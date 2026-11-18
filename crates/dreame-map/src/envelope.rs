//! Text envelope around a raw frame (maps.md §3).
//!
//! ```text
//! text -> URL-safe base64 -> [AES-256-CBC, only with a key] -> zlib -> raw frame bytes
//! ```
//!
//! The AES key is the first 32 ASCII characters of the lowercase hex SHA-256 of the key string.
//! The IV is a 16-character ASCII string. When the IV is unknown, every entry of [`KNOWN_IVS`] is
//! tried until zlib succeeds.
//!
//! The per-model IV table comes from tasshack. The official app does not have one: it uses one
//! global IV, which is empty in the r2579a plugin, so it can only read maps sent without a key
//! (map-plugin.md §4). r2579a sends no key. The IV search is kept for other models.

use crate::{decode_err, Result};
use aes::cipher::{block_padding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

/// AES IVs from Tasshack's `DEVICE_INFO` table (one per model family).
pub const KNOWN_IVS: [&str; 13] = [
    "3F0ji4ufBMaH1ThM",
    "4sCv3Q2BtbWVBIB2",
    "6PFiLPYMHLylp7RR",
    "8qnS9dqgT3CppGe1",
    "FmnfaI2pbem0k75t",
    "NRwnBj5FsNPgBNbT",
    "OFULk9To37qRdXY3",
    "RNO4p35b2QKaovHC",
    "dndRQ3z8ACjDdDMo",
    "nf3Zi2Mq8jD5AAOm",
    "ojxGnogHfVuefVfx",
    "qFKhvoAqRFTPfKN6",
    "wRy05fYLQJMRH6Mj",
];

/// AES parameters for encoding or for a known model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AesParams {
    /// The key string as sent by the robot (not the derived key bytes).
    pub key: String,
    /// The 16-character ASCII IV.
    pub iv: String,
}

/// Result of [`decode`]: the raw frame bytes and how they were unwrapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub raw: Vec<u8>,
    /// AES parameters that decrypted the envelope, or `None` for a plain envelope.
    pub aes: Option<AesParams>,
}

/// Splits `"<b64>,<key>"` into its parts. Text without a comma has no key.
pub fn split_key(text: &str) -> (&str, Option<&str>) {
    match text.split_once(',') {
        Some((b, k)) if !k.trim().is_empty() => (b, Some(k.trim())),
        Some((b, _)) => (b, None),
        None => (text, None),
    }
}

/// Decodes URL-safe base64. Whitespace is ignored and padding is optional.
pub fn base64_decode(text: &str) -> Result<Vec<u8>> {
    let s: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '_' => '/',
            '-' => '+',
            c => c,
        })
        .collect();
    base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(s.trim_end_matches('='))
        .map_err(|e| decode_err(format!("base64: {e}")))
}

/// Encodes bytes as base64 with `+` replaced by `-` and `/` by `_`, padding kept.
pub fn base64_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE.encode(data)
}

/// Derives the 32 AES key bytes from a key string.
pub fn aes_key(key: &str) -> [u8; 32] {
    let hex = format!("{:x}", Sha256::digest(key.as_bytes()));
    let mut out = [0u8; 32];
    out.copy_from_slice(&hex.as_bytes()[..32]);
    out
}

fn check_iv(iv: &str) -> Result<()> {
    if iv.len() != 16 {
        return Err(decode_err(format!(
            "AES IV must be 16 bytes, got {}",
            iv.len()
        )));
    }
    Ok(())
}

/// Decrypts AES-256-CBC without removing padding (zlib ignores trailing bytes).
pub fn aes_decrypt(data: &[u8], key: &str, iv: &str) -> Result<Vec<u8>> {
    check_iv(iv)?;
    let n = data.len() / 16 * 16;
    let mut buf = data[..n].to_vec();
    let dec = cbc::Decryptor::<aes::Aes256>::new_from_slices(&aes_key(key), iv.as_bytes())
        .map_err(|e| decode_err(format!("aes: {e}")))?;
    dec.decrypt_padded_mut::<block_padding::NoPadding>(&mut buf)
        .map_err(|e| decode_err(format!("aes: {e}")))?;
    Ok(buf)
}

/// Encrypts with AES-256-CBC and PKCS#7 padding.
pub fn aes_encrypt(data: &[u8], key: &str, iv: &str) -> Result<Vec<u8>> {
    check_iv(iv)?;
    let len = data.len();
    let mut buf = vec![0u8; (len / 16 + 1) * 16];
    buf[..len].copy_from_slice(data);
    let enc = cbc::Encryptor::<aes::Aes256>::new_from_slices(&aes_key(key), iv.as_bytes())
        .map_err(|e| decode_err(format!("aes: {e}")))?;
    let out = enc
        .encrypt_padded_mut::<block_padding::Pkcs7>(&mut buf, len)
        .map_err(|e| decode_err(format!("aes: {e}")))?;
    Ok(out.to_vec())
}

/// Inflates zlib data, falling back to raw deflate as the Homey app does.
pub fn inflate(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    if flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .is_ok()
        && !out.is_empty()
    {
        return Ok(out);
    }
    out.clear();
    flate2::read::DeflateDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| decode_err(format!("zlib: {e}")))?;
    if out.is_empty() {
        return Err(decode_err("zlib: empty output"));
    }
    Ok(out)
}

fn inflate_zlib(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .ok()?;
    (!out.is_empty()).then_some(out)
}

/// Compresses with zlib at the default level.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).expect("writing to a Vec cannot fail");
    enc.finish().expect("writing to a Vec cannot fail")
}

/// Decodes an envelope to raw frame bytes.
///
/// `key` overrides a key given as a `",<key>"` suffix in `text`. `iv` selects one IV; without it
/// every IV in [`KNOWN_IVS`] is tried. A plain (unencrypted) envelope is always tried first.
pub fn decode(text: &str, key: Option<&str>, iv: Option<&str>) -> Result<Decoded> {
    let (body, suffix_key) = split_key(text.trim());
    let key = key.or(suffix_key);
    let data = base64_decode(body)?;
    // Only zlib (header check and Adler-32) counts here: raw deflate can accept noise.
    if let Some(raw) = inflate_zlib(&data) {
        return Ok(Decoded { raw, aes: None });
    }
    let Some(key) = key else {
        return inflate(&data).map(|raw| Decoded { raw, aes: None });
    };
    let ivs: Vec<&str> = match iv {
        Some(iv) => vec![iv],
        None => KNOWN_IVS.to_vec(),
    };
    for iv in ivs {
        if let Ok(dec) = aes_decrypt(&data, key, iv) {
            if let Some(raw) = inflate_zlib(&dec) {
                return Ok(Decoded {
                    raw,
                    aes: Some(AesParams {
                        key: key.to_string(),
                        iv: iv.to_string(),
                    }),
                });
            }
        }
    }
    Err(decode_err(
        "envelope: zlib failed and no IV decrypted the frame",
    ))
}

/// Encodes raw frame bytes as an envelope. With `aes`, the result carries no key suffix; append
/// `",<key>"` yourself if the consumer expects it.
pub fn encode(raw: &[u8], aes: Option<&AesParams>) -> Result<String> {
    let z = deflate(raw);
    let data = match aes {
        Some(p) => aes_encrypt(&z, &p.key, &p.iv)?,
        None => z,
    };
    Ok(base64_encode(&data))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &[u8] = b"hello map frame with some bytes \x00\x01\x02\xff";

    #[test]
    fn plain_round_trip() {
        let text = encode(RAW, None).unwrap();
        assert!(text.starts_with("eJ") || text.starts_with("eF") || text.starts_with("eN"));
        assert!(!text.contains('+') && !text.contains('/'));
        let d = decode(&text, None, None).unwrap();
        assert_eq!(d.raw, RAW);
        assert!(d.aes.is_none());
        // Padding is optional.
        let d = decode(text.trim_end_matches('='), None, None).unwrap();
        assert_eq!(d.raw, RAW);
    }

    #[test]
    fn aes_round_trip_with_suffix_key_and_iv_search() {
        let p = AesParams {
            key: "secret-key".into(),
            iv: KNOWN_IVS[7].into(),
        };
        let text = encode(RAW, Some(&p)).unwrap();
        // Key from the suffix, IV found by trying all.
        let d = decode(&format!("{text},secret-key"), None, None).unwrap();
        assert_eq!(d.raw, RAW);
        assert_eq!(d.aes, Some(p.clone()));
        // Explicit key and IV.
        let d = decode(&text, Some("secret-key"), Some(KNOWN_IVS[7])).unwrap();
        assert_eq!(d.raw, RAW);
        // Wrong key fails.
        assert!(decode(&text, Some("other"), None).is_err());
        // No key fails.
        assert!(decode(&text, None, None).is_err());
    }

    #[test]
    fn key_derivation_is_hex_prefix() {
        let k = aes_key("abc");
        // sha256("abc") starts with ba7816bf8f01cfea414140de5dae2223.
        assert_eq!(&k, b"ba7816bf8f01cfea414140de5dae2223");
    }

    #[test]
    fn split_key_forms() {
        assert_eq!(split_key("abc,def"), ("abc", Some("def")));
        assert_eq!(split_key("abc,"), ("abc", None));
        assert_eq!(split_key("abc"), ("abc", None));
    }

    #[test]
    fn raw_deflate_fallback() {
        let mut enc =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(RAW).unwrap();
        let text = base64_encode(&enc.finish().unwrap());
        assert_eq!(decode(&text, None, None).unwrap().raw, RAW);
    }
}
