//! Recovery archives: the robot's map store as a bzip2-compressed tar file.
//!
//! The robot uploads one archive per recovery entry and loads one when a map is restored
//! (`docs/research/maps.md` §12.2). An edit replaces some members and must leave the rest of the
//! file as the robot wrote it, so this module keeps every tar header block as raw bytes. Writing
//! changes only the size and checksum fields of a replaced member.

use crate::{decode_err, Result};
use dreame_core::Error;
use std::io::{Read, Write};

const BLOCK: usize = 512;
const SIZE: std::ops::Range<usize> = 124..136;
const CHECKSUM: std::ops::Range<usize> = 148..156;
const TYPE: usize = 156;

/// One tar record: a header block and its data.
#[derive(Debug, Clone)]
struct Record {
    header: [u8; BLOCK],
    /// The member's path, after a GNU long-name record if one came before.
    name: String,
    data: Vec<u8>,
}

impl Record {
    fn kind(&self) -> u8 {
        self.header[TYPE]
    }
    fn is_file(&self) -> bool {
        matches!(self.kind(), b'0' | 0)
    }
}

/// A decoded recovery archive.
#[derive(Debug, Clone)]
pub struct Archive {
    records: Vec<Record>,
    /// The end-of-archive blocks, kept so an unedited archive encodes to the same tar bytes.
    tail: Vec<u8>,
}

fn octal(field: &[u8]) -> Result<u64> {
    let s: String = field.iter().take_while(|b| **b != 0).map(|b| *b as char).collect();
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s, 8).map_err(|_| decode_err(format!("bad octal field {s:?} in a tar header")))
}

fn checksum(header: &[u8; BLOCK]) -> u32 {
    header.iter().enumerate().map(|(i, b)| if CHECKSUM.contains(&i) { 32 } else { *b as u32 }).sum()
}

fn text(field: &[u8]) -> String {
    String::from_utf8_lossy(&field[..field.iter().position(|b| *b == 0).unwrap_or(field.len())]).into_owned()
}

impl Archive {
    /// Decodes a `.tbz2` file.
    pub fn decode(tbz2: &[u8]) -> Result<Self> {
        let mut tar = Vec::new();
        bzip2::read::BzDecoder::new(tbz2)
            .read_to_end(&mut tar)
            .map_err(|e| decode_err(format!("archive is not bzip2: {e}")))?;
        Self::decode_tar(&tar)
    }

    /// Decodes an uncompressed tar file.
    pub fn decode_tar(tar: &[u8]) -> Result<Self> {
        let mut records = Vec::new();
        let mut long_name: Option<String> = None;
        let mut i = 0;
        while i + BLOCK <= tar.len() {
            let header: [u8; BLOCK] = tar[i..i + BLOCK].try_into().expect("block size");
            if header.iter().all(|b| *b == 0) {
                break;
            }
            if octal(&header[CHECKSUM])? != checksum(&header) as u64 {
                return Err(decode_err(format!("tar header at byte {i} has a bad checksum")));
            }
            let size = octal(&header[SIZE])? as usize;
            let start = i + BLOCK;
            let end = start + size;
            if end > tar.len() {
                return Err(decode_err(format!("tar member at byte {i} runs past the end")));
            }
            let data = tar[start..end].to_vec();
            let name = match header[TYPE] {
                b'L' | b'K' => String::new(),
                _ => long_name.take().unwrap_or_else(|| text(&header[..100])),
            };
            if header[TYPE] == b'L' {
                long_name = Some(text(&data));
            }
            if matches!(header[TYPE], b'x' | b'g') {
                return Err(Error::Unsupported("pax headers in a recovery archive".into()));
            }
            records.push(Record { header, name, data });
            i = start + size.div_ceil(BLOCK) * BLOCK;
        }
        if i >= tar.len() {
            return Err(decode_err("tar file has no end-of-archive blocks"));
        }
        Ok(Self { records, tail: tar[i..].to_vec() })
    }

    /// Encodes the archive as a `.tbz2` file at the highest compression level.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
        enc.write_all(&self.encode_tar())
            .and_then(|_| enc.finish())
            .map_err(|e| decode_err(format!("bzip2 failed: {e}")))
    }

    /// Encodes the archive as an uncompressed tar file.
    pub fn encode_tar(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for r in &self.records {
            out.extend_from_slice(&r.header);
            out.extend_from_slice(&r.data);
            out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
        }
        out.extend_from_slice(&self.tail);
        out
    }

    /// The paths of the regular files, in archive order.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.records.iter().filter(|r| r.is_file()).map(|r| r.name.as_str())
    }

    /// The content of a regular file.
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.records.iter().find(|r| r.is_file() && r.name == name).map(|r| r.data.as_slice())
    }

    /// Replaces the content of an existing regular file. Its header keeps every field except the
    /// size and the checksum.
    pub fn set(&mut self, name: &str, data: Vec<u8>) -> Result<()> {
        let r = self
            .records
            .iter_mut()
            .find(|r| r.is_file() && r.name == name)
            .ok_or_else(|| decode_err(format!("the archive has no file {name}")))?;
        let size = format!("{:011o}\0", data.len());
        if size.len() != SIZE.len() {
            return Err(Error::Unsupported(format!("{name} is too large for a tar size field")));
        }
        r.header[SIZE].copy_from_slice(size.as_bytes());
        let sum = format!("{:06o}\0 ", checksum(&r.header));
        r.header[CHECKSUM].copy_from_slice(sum.as_bytes());
        r.data = data;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-file GNU tar, built by hand: header, data, two zero blocks.
    fn tiny_tar(name: &str, data: &[u8]) -> Vec<u8> {
        let mut h = [0u8; BLOCK];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..108].copy_from_slice(b"0000644\0");
        h[SIZE].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        h[TYPE] = b'0';
        h[257..265].copy_from_slice(b"ustar  \0");
        let sum = format!("{:06o}\0 ", checksum(&h));
        h[CHECKSUM].copy_from_slice(sum.as_bytes());
        let mut t = h.to_vec();
        t.extend_from_slice(data);
        t.resize(t.len().div_ceil(BLOCK) * BLOCK + 2 * BLOCK, 0);
        t
    }

    #[test]
    fn round_trip_keeps_bytes() {
        let t = tiny_tar("a/b.txt", b"hello");
        let a = Archive::decode_tar(&t).unwrap();
        assert_eq!(a.files().collect::<Vec<_>>(), ["a/b.txt"]);
        assert_eq!(a.get("a/b.txt"), Some(&b"hello"[..]));
        assert_eq!(a.encode_tar(), t);
        let back = Archive::decode(&a.encode().unwrap()).unwrap();
        assert_eq!(back.encode_tar(), t);
    }

    #[test]
    fn set_updates_size_and_checksum() {
        let mut a = Archive::decode_tar(&tiny_tar("x", b"hello")).unwrap();
        a.set("x", vec![7; 700]).unwrap();
        let t = a.encode_tar();
        assert_eq!(t, {
            let mut want = tiny_tar("x", &[7; 700]);
            // The hand-built header has the same fields, so only the data and padding differ.
            want.truncate(t.len());
            want
        });
        let b = Archive::decode_tar(&t).unwrap();
        assert_eq!(b.get("x").unwrap().len(), 700);
        assert!(a.set("missing", vec![]).is_err());
    }

    #[test]
    fn bad_checksum_is_an_error() {
        let mut t = tiny_tar("x", b"hello");
        t[0] = b'y';
        assert!(Archive::decode_tar(&t).is_err());
    }
}
