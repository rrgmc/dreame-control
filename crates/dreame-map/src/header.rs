//! The 27-byte frame header (maps.md §4).
//!
//! All fields are little-endian signed 16-bit integers except `frame_type`, which is one ASCII byte.

use crate::{decode_err, Result};

/// Header length in bytes.
pub const HEADER_LEN: usize = 27;

/// A position value of 32767 means "absent".
pub const ABSENT: i16 = 32767;

/// Frame type byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameType {
    /// `I`: a full snapshot.
    I,
    /// `P`: a difference against the previous frame.
    P,
    /// `W`: a Wi-Fi signal map.
    W,
    /// Any other byte, kept so the header round-trips.
    Other(u8),
}

impl FrameType {
    pub fn from_byte(b: u8) -> Self {
        match b {
            b'I' => Self::I,
            b'P' => Self::P,
            b'W' => Self::W,
            other => Self::Other(other),
        }
    }

    pub fn to_byte(self) -> u8 {
        match self {
            Self::I => b'I',
            Self::P => b'P',
            Self::W => b'W',
            Self::Other(b) => b,
        }
    }
}

/// A position in mm and an angle in degrees, as stored in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pose {
    pub x: i16,
    pub y: i16,
    pub angle: i16,
}

impl Pose {
    pub const ABSENT: Pose = Pose {
        x: ABSENT,
        y: ABSENT,
        angle: ABSENT,
    };

    pub fn new(x: i16, y: i16, angle: i16) -> Self {
        Self { x, y, angle }
    }

    /// True when x or y carries the "absent" marker.
    pub fn is_absent(&self) -> bool {
        self.x == ABSENT || self.y == ABSENT
    }

    /// The pose, or `None` when it is marked absent.
    pub fn present(self) -> Option<Pose> {
        (!self.is_absent()).then_some(self)
    }
}

/// The decoded header. Raw values are kept so the header re-encodes byte for byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub map_id: i16,
    pub frame_id: i16,
    pub frame_type: FrameType,
    /// Robot pose. Use [`Pose::present`] to treat 32767 as absent.
    pub robot: Pose,
    /// Charger pose. Use [`Pose::present`] to treat 32767 as absent.
    pub charger: Pose,
    /// Millimetres per pixel.
    pub grid_size: i16,
    pub width: i16,
    pub height: i16,
    /// World x of column 0, in mm.
    pub left: i16,
    /// World y of row 0, in mm. Row 0 is the minimum y.
    pub top: i16,
}

impl Default for Header {
    fn default() -> Self {
        Self {
            map_id: 0,
            frame_id: 0,
            frame_type: FrameType::I,
            robot: Pose::ABSENT,
            charger: Pose::ABSENT,
            grid_size: 50,
            width: 0,
            height: 0,
            left: 0,
            top: 0,
        }
    }
}

fn i16_at(b: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([b[o], b[o + 1]])
}

impl Header {
    /// Parses the first 27 bytes of `raw`.
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() < HEADER_LEN {
            return Err(decode_err(format!(
                "frame too short for header: {} bytes",
                raw.len()
            )));
        }
        Ok(Self {
            map_id: i16_at(raw, 0),
            frame_id: i16_at(raw, 2),
            frame_type: FrameType::from_byte(raw[4]),
            robot: Pose::new(i16_at(raw, 5), i16_at(raw, 7), i16_at(raw, 9)),
            charger: Pose::new(i16_at(raw, 11), i16_at(raw, 13), i16_at(raw, 15)),
            grid_size: i16_at(raw, 17),
            width: i16_at(raw, 19),
            height: i16_at(raw, 21),
            left: i16_at(raw, 23),
            top: i16_at(raw, 25),
        })
    }

    /// Encodes the header as 27 bytes.
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        let mut put = |o: usize, v: i16| out[o..o + 2].copy_from_slice(&v.to_le_bytes());
        put(0, self.map_id);
        put(2, self.frame_id);
        put(5, self.robot.x);
        put(7, self.robot.y);
        put(9, self.robot.angle);
        put(11, self.charger.x);
        put(13, self.charger.y);
        put(15, self.charger.angle);
        put(17, self.grid_size);
        put(19, self.width);
        put(21, self.height);
        put(23, self.left);
        put(25, self.top);
        out[4] = self.frame_type.to_byte();
        out
    }

    /// Number of pixel bytes that follow the header. Negative sizes count as 0.
    pub fn pixel_count(&self) -> usize {
        self.width.max(0) as usize * self.height.max(0) as usize
    }

    /// An empty map is 2 x 2 with all pixels zero (maps.md §4). This checks only the size.
    pub fn is_empty_size(&self) -> bool {
        self.width == 2 && self.height == 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trip() {
        let h = Header {
            map_id: 4,
            frame_id: 350,
            frame_type: FrameType::I,
            robot: Pose::new(-300, 400, 45),
            charger: Pose::new(-300, 400, 45),
            grid_size: 50,
            width: 185,
            height: 296,
            left: -2000,
            top: -3000,
        };
        let b = h.to_bytes();
        assert_eq!(b[4], b'I');
        assert_eq!(&b[0..2], &4i16.to_le_bytes());
        assert_eq!(Header::parse(&b).unwrap(), h);
        assert_eq!(h.pixel_count(), 185 * 296);
    }

    #[test]
    fn absent_positions() {
        let mut h = Header {
            robot: Pose::new(ABSENT, ABSENT, 0),
            ..Default::default()
        };
        let p = Header::parse(&h.to_bytes()).unwrap();
        assert!(p.robot.present().is_none());
        assert!(p.charger.is_absent());
        h.robot = Pose::new(10, -20, 90);
        assert_eq!(
            Header::parse(&h.to_bytes()).unwrap().robot.present(),
            Some(Pose::new(10, -20, 90))
        );
    }

    #[test]
    fn frame_types() {
        for &b in b"IPWX" {
            assert_eq!(FrameType::from_byte(b).to_byte(), b);
        }
        assert!(Header::parse(&[0u8; 26]).is_err());
    }
}
