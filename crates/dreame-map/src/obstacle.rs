//! Obstacles the robot's camera recognised, from the map trailer's `ai_obstacle` list, and the
//! decryption of their photos.
//!
//! Each entry is a list of strings: `[x, y, type, confidence, key, file, pwd, box x4,
//! upload status, small object, ignore status]`. Older firmware sends fewer fields. The photo is
//! a JPEG in cloud storage under `file + "-" + key`, encrypted with AES-128-ECB, key = MD5(pwd).
//! See docs/research/remote-and-camera.md.

use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};
use aes::Aes128;
use md5::{Digest, Md5};
use serde_json::Value;

/// Obstacle type the plugin treats as a pet.
pub const PET: u32 = 158;

/// One recognised obstacle.
#[derive(Debug, Clone, PartialEq)]
pub struct Obstacle {
    /// Position in map millimetres.
    pub x: f64,
    pub y: f64,
    /// The robot's obstacle class code. [`PET`] is a pet.
    pub kind: u32,
    /// Recognition confidence, 0 to 1.
    pub confidence: f64,
    /// Photo details; absent when the robot took no photo.
    pub photo: Option<PhotoRef>,
}

/// Where an obstacle photo is stored and how to decrypt it.
#[derive(Clone, PartialEq)]
pub struct PhotoRef {
    pub key: String,
    pub file: String,
    pub pwd: String,
    /// 1 uploading, 2 in the cloud, 3 upload failed; absent on older firmware.
    pub upload_status: Option<u8>,
}

// The password is a secret: keep it out of logs.
impl std::fmt::Debug for PhotoRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhotoRef").field("upload_status", &self.upload_status).finish_non_exhaustive()
    }
}

impl PhotoRef {
    /// The cloud object name: `file + "-" + key`.
    pub fn object(&self) -> String {
        format!("{}-{}", self.file, self.key)
    }

    /// Whether the robot reported the upload as failed.
    pub fn upload_failed(&self) -> bool {
        self.upload_status == Some(3)
    }

    /// Decrypts a downloaded photo into JPEG bytes.
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, PhotoError> {
        decrypt_photo(&self.pwd, data)
    }
}

impl Obstacle {
    pub fn is_pet(&self) -> bool {
        self.kind == PET
    }
}

/// Parses the trailer's `ai_obstacle` value. Entries that do not parse are skipped.
pub fn parse(v: &Value) -> Vec<Obstacle> {
    v.as_array().map(|a| a.iter().filter_map(parse_one).collect()).unwrap_or_default()
}

fn parse_one(e: &Value) -> Option<Obstacle> {
    let f: Vec<String> = e
        .as_array()?
        .iter()
        .map(|x| match x {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect();
    let num = |i: usize| f.get(i).and_then(|s| s.parse::<f64>().ok());
    // A trailing "obstacleignore:x:y" field is not part of the numbered layout.
    let size = f.len() - usize::from(f.last().is_some_and(|s| s.contains("obstacleignore:")));
    let photo = (size >= 7 && !f[5].is_empty()).then(|| PhotoRef {
        key: f[4].clone(),
        file: f[5].clone(),
        pwd: f[6].clone(),
        upload_status: (size >= 13).then(|| f[11].parse().ok()).flatten(),
    });
    Some(Obstacle {
        x: num(0)?,
        y: num(1)?,
        kind: f.get(2)?.parse().ok()?,
        confidence: num(3).unwrap_or(0.0),
        photo,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotoError {
    /// The length is not a whole number of AES blocks.
    Length(usize),
    /// The padding is not valid PKCS#7, so the password is probably wrong.
    Padding,
}

impl std::fmt::Display for PhotoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Length(n) => write!(f, "photo is {n} bytes, not a multiple of 16"),
            Self::Padding => write!(f, "photo did not decrypt (bad padding)"),
        }
    }
}

impl std::error::Error for PhotoError {}

/// AES-128-ECB with PKCS#7 padding; the key is the 16 raw bytes of MD5(pwd).
pub fn decrypt_photo(pwd: &str, data: &[u8]) -> Result<Vec<u8>, PhotoError> {
    if data.is_empty() || !data.len().is_multiple_of(16) {
        return Err(PhotoError::Length(data.len()));
    }
    let key = Md5::digest(pwd.as_bytes());
    let cipher = Aes128::new(&key);
    let mut out = data.to_vec();
    for block in out.as_chunks_mut::<16>().0 {
        cipher.decrypt_block(GenericArray::from_mut_slice(block.as_mut_slice()));
    }
    let pad = *out.last().unwrap() as usize;
    if pad == 0 || pad > 16 || !out[out.len() - pad..].iter().all(|&b| b as usize == pad) {
        return Err(PhotoError::Padding);
    }
    out.truncate(out.len() - pad);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncrypt;
    use serde_json::json;

    fn encrypt(pwd: &str, plain: &[u8]) -> Vec<u8> {
        let cipher = Aes128::new(&Md5::digest(pwd.as_bytes()));
        let pad = 16 - plain.len() % 16;
        let mut out = plain.to_vec();
        out.extend(std::iter::repeat_n(pad as u8, pad));
        for block in out.as_chunks_mut::<16>().0 {
            cipher.encrypt_block(GenericArray::from_mut_slice(block.as_mut_slice()));
        }
        out
    }

    #[test]
    fn parses_a_full_entry() {
        let v = json!([["120", "-340", "158", "0.91", "k1", "/ai/x.jpg", "pw", "1", "2", "3", "4", "2", "1", "0"]]);
        let o = &parse(&v)[0];
        assert_eq!((o.x, o.y, o.kind), (120.0, -340.0, 158));
        assert!(o.is_pet());
        let p = o.photo.as_ref().unwrap();
        assert_eq!(p.object(), "/ai/x.jpg-k1");
        assert_eq!(p.upload_status, Some(2));
        assert!(!format!("{p:?}").contains("pw"));
    }

    #[test]
    fn short_entries_have_no_photo_or_status() {
        let v = json!([["1", "2", "139", "0.5"], ["1", "2", "139", "0.5", "k", "/f", "p"], ["x"]]);
        let o = parse(&v);
        assert_eq!(o.len(), 2);
        assert!(o[0].photo.is_none());
        assert_eq!(o[1].photo.as_ref().unwrap().upload_status, None);
    }

    #[test]
    fn ignore_suffix_does_not_shift_fields() {
        let v = json!([["1", "2", "139", "0.5", "k", "/f", "p", "1", "2", "3", "4", "3", "0", "0", "obstacleignore:5:6"]]);
        assert!(parse(&v)[0].photo.as_ref().unwrap().upload_failed());
    }

    #[test]
    fn decrypt_round_trip() {
        let jpeg = b"\xff\xd8\xff\xe0 a small fake jpeg body";
        let enc = encrypt("secret", jpeg);
        assert_eq!(decrypt_photo("secret", &enc).unwrap(), jpeg);
        assert!(decrypt_photo("other", &enc).is_err());
        assert_eq!(decrypt_photo("secret", &enc[..15]), Err(PhotoError::Length(15)));
    }
}
