//! A raw frame: header, pixel bytes and typed trailer.
//!
//! Layout: 27-byte header, `width * height` pixel bytes (row-major, row 0 at `top`), then the
//! UTF-8 JSON trailer.

use crate::envelope::{self, AesParams};
use crate::grid::{self, Encoding, Grid};
use crate::header::{FrameType, Header, HEADER_LEN};
use crate::trailer::Trailer;
use crate::{decode_err, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub header: Header,
    /// Raw pixel bytes in the frame's wire encoding.
    pub pixels: Vec<u8>,
    pub trailer: Trailer,
    /// AES parameters the envelope used, if it was encrypted. [`Frame::encode`] reuses them.
    pub aes: Option<AesParams>,
}

impl Frame {
    /// Parses raw (inflated) frame bytes.
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let header = Header::parse(raw)?;
        let n = header.pixel_count();
        if raw.len() < HEADER_LEN + n {
            return Err(decode_err(format!(
                "frame shorter than header claims ({} < {})",
                raw.len(),
                HEADER_LEN + n
            )));
        }
        let pixels = raw[HEADER_LEN..HEADER_LEN + n].to_vec();
        let trailer = Trailer::from_slice(&raw[HEADER_LEN + n..])?;
        Ok(Self {
            header,
            pixels,
            trailer,
            aes: None,
        })
    }

    /// Serializes the frame to raw bytes. The pixel buffer must match the header size.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if self.pixels.len() != self.header.pixel_count() {
            return Err(decode_err(format!(
                "pixel buffer has {} bytes, header says {} x {}",
                self.pixels.len(),
                self.header.width,
                self.header.height
            )));
        }
        let mut out = Vec::with_capacity(HEADER_LEN + self.pixels.len() + 256);
        out.extend_from_slice(&self.header.to_bytes());
        out.extend_from_slice(&self.pixels);
        if self.trailer != Trailer::default() {
            out.extend_from_slice(self.trailer.to_json().as_bytes());
        }
        Ok(out)
    }

    /// Decodes an envelope (see [`envelope::decode`]) and parses the frame.
    pub fn decode(text: &str, key: Option<&str>, iv: Option<&str>) -> Result<Self> {
        let d = envelope::decode(text, key, iv)?;
        let mut f = Self::parse(&d.raw)?;
        f.aes = d.aes;
        Ok(f)
    }

    /// Encodes the frame as an envelope, encrypted when [`Frame::aes`] is set.
    pub fn encode(&self) -> Result<String> {
        envelope::encode(&self.to_bytes()?, self.aes.as_ref())
    }

    pub fn is_i_frame(&self) -> bool {
        self.header.frame_type == FrameType::I
    }

    pub fn is_p_frame(&self) -> bool {
        self.header.frame_type == FrameType::P
    }

    /// World x of column 0. The trailer's `origin` overrides the header (maps.md §4).
    pub fn left(&self) -> i32 {
        self.trailer
            .origin
            .map_or(self.header.left as i32, |o| o[0] as i32)
    }

    /// World y of row 0. The trailer's `origin` overrides the header.
    pub fn top(&self) -> i32 {
        self.trailer
            .origin
            .map_or(self.header.top as i32, |o| o[1] as i32)
    }

    /// The pixel encoding for this frame. `saved` marks a saved map (6.8 list, `rism`, recovery
    /// preview), which carries no live-state flags.
    pub fn encoding(&self, map_version: u8, saved: bool) -> Encoding {
        grid::select_encoding(map_version, self.header.frame_type, &self.trailer, saved)
    }

    /// Decodes the pixel grid with the given encoding.
    pub fn grid(&self, encoding: Encoding) -> Grid {
        Grid::decode(
            &self.pixels,
            self.header.width.max(0) as usize,
            self.header.height.max(0) as usize,
            self.left(),
            self.top(),
            self.header.grid_size as i32,
            encoding,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::Pose;

    pub(crate) fn sample_frame() -> Frame {
        let header = Header {
            map_id: 1,
            frame_id: 0,
            frame_type: FrameType::I,
            robot: Pose::new(0, 0, 0),
            charger: Pose::new(-300, 400, 45),
            grid_size: 50,
            width: 4,
            height: 3,
            left: -100,
            top: -50,
        };
        let trailer =
            Trailer::from_value(serde_json::json!({"origin": [-100, -50], "delsr": [2], "zzz": 1}))
                .unwrap();
        Frame {
            header,
            pixels: vec![0, 1, 1, 0x81, 0, 0x41, 2, 0x82, 0, 0, 0, 0],
            trailer,
            aes: None,
        }
    }

    #[test]
    fn encode_decode_round_trip() {
        let f = sample_frame();
        let text = f.encode().unwrap();
        let g = Frame::decode(&text, None, None).unwrap();
        assert_eq!(g, f);
        assert_eq!(g.to_bytes().unwrap(), f.to_bytes().unwrap());
    }

    #[test]
    fn encrypted_round_trip() {
        let mut f = sample_frame();
        f.aes = Some(AesParams {
            key: "k".into(),
            iv: envelope::KNOWN_IVS[0].into(),
        });
        let text = f.encode().unwrap();
        let g = Frame::decode(&format!("{text},k"), None, None).unwrap();
        assert_eq!(g, f);
    }

    #[test]
    fn short_frame_is_an_error() {
        let f = sample_frame();
        let bytes = f.to_bytes().unwrap();
        assert!(Frame::parse(&bytes[..30]).is_err());
    }

    #[test]
    fn origin_overrides_header() {
        let mut f = sample_frame();
        f.trailer.origin = Some([500, 600]);
        assert_eq!((f.left(), f.top()), (500, 600));
        f.trailer.origin = None;
        assert_eq!((f.left(), f.top()), (-100, -50));
    }
}
