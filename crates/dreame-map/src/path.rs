//! The robot path string `tr` (maps.md §7.3).
//!
//! The string is a sequence of tokens `<op><x>,<y>` with integer mm:
//!
//! - `S` sweep, `M` mop, `W` sweep and mop: absolute point that starts a new stroke and sets the
//!   cleaning mode from there on.
//! - `L`: relative move from the previous point, drawn as a line.
//! - `l` (P-frames only): absolute point drawn as a line from the previous frame's last point.
//!
//! Each `tr` string starts from `(0, 0)`, as in Tasshack. [`Path::append_tr`] adds a P-frame path
//! to an existing path.

use crate::{decode_err, Result};
use std::fmt::Write as _;

/// Token operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathOp {
    /// `S`
    Sweep,
    /// `M`
    Mop,
    /// `W`
    SweepAndMop,
    /// `L`, relative.
    Line,
    /// `l`, absolute, joined to the previous point.
    JoinedLine,
}

impl PathOp {
    pub fn from_char(c: char) -> Option<Self> {
        Some(match c {
            'S' => Self::Sweep,
            'M' => Self::Mop,
            'W' => Self::SweepAndMop,
            'L' => Self::Line,
            'l' => Self::JoinedLine,
            _ => return None,
        })
    }

    pub fn to_char(self) -> char {
        match self {
            Self::Sweep => 'S',
            Self::Mop => 'M',
            Self::SweepAndMop => 'W',
            Self::Line => 'L',
            Self::JoinedLine => 'l',
        }
    }

    /// True for `S`, `M` and `W`, which start a new stroke.
    pub fn starts_stroke(self) -> bool {
        matches!(self, Self::Sweep | Self::Mop | Self::SweepAndMop)
    }
}

/// One token as written on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathToken {
    pub op: PathOp,
    pub x: i64,
    pub y: i64,
}

/// Cleaning mode of a stroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PathMode {
    #[default]
    Sweep,
    Mop,
    SweepAndMop,
}

/// One resolved point in world mm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathPoint {
    pub x: i64,
    pub y: i64,
    /// True when a line is drawn from the previous point; false starts a new stroke.
    pub connected: bool,
    /// Cleaning mode in effect at this point.
    pub mode: PathMode,
}

/// Parses tokens. Separators other than the token characters (spaces, commas between tokens)
/// are tolerated; anything else is an error.
pub fn parse_tokens(s: &str) -> Result<Vec<PathToken>> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    let num = |i: &mut usize| -> Result<i64> {
        let start = *i;
        if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
            *i += 1;
        }
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        s[start..*i]
            .parse::<i64>()
            .map_err(|_| decode_err(format!("path: bad number at byte {start}")))
    };
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_whitespace() || c == ',' || c == ';' {
            i += 1;
            continue;
        }
        let op = PathOp::from_char(c)
            .ok_or_else(|| decode_err(format!("path: unexpected {c:?} at byte {i}")))?;
        i += 1;
        let x = num(&mut i)?;
        if i >= b.len() || b[i] != b',' {
            return Err(decode_err(format!("path: expected ',' at byte {i}")));
        }
        i += 1;
        let y = num(&mut i)?;
        out.push(PathToken { op, x, y });
    }
    Ok(out)
}

/// Formats tokens back to the wire form.
pub fn format_tokens(tokens: &[PathToken]) -> String {
    let mut s = String::with_capacity(tokens.len() * 8);
    for t in tokens {
        let _ = write!(s, "{}{},{}", t.op.to_char(), t.x, t.y);
    }
    s
}

/// A resolved path.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Path {
    pub points: Vec<PathPoint>,
}

impl Path {
    /// Parses a `tr` string.
    pub fn parse(tr: &str) -> Result<Self> {
        let mut p = Path::default();
        p.append_tr(tr)?;
        Ok(p)
    }

    /// Appends the points of another `tr` string. Relative moves restart from `(0, 0)` for each
    /// string, as in Tasshack; the mode carries over from the existing path.
    pub fn append_tr(&mut self, tr: &str) -> Result<()> {
        let tokens = parse_tokens(tr)?;
        self.append_tokens(&tokens);
        Ok(())
    }

    /// Appends resolved tokens.
    pub fn append_tokens(&mut self, tokens: &[PathToken]) {
        let mut mode = self.points.last().map(|p| p.mode).unwrap_or_default();
        let (mut x, mut y) = (0i64, 0i64);
        for t in tokens {
            let connected = match t.op {
                PathOp::Line => {
                    x += t.x;
                    y += t.y;
                    true
                }
                PathOp::JoinedLine => {
                    x = t.x;
                    y = t.y;
                    true
                }
                op => {
                    mode = match op {
                        PathOp::Mop => PathMode::Mop,
                        PathOp::SweepAndMop => PathMode::SweepAndMop,
                        _ => PathMode::Sweep,
                    };
                    x = t.x;
                    y = t.y;
                    false
                }
            };
            self.points.push(PathPoint {
                x,
                y,
                connected: connected && !self.points.is_empty(),
                mode,
            });
        }
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Splits the path into strokes: runs of connected points.
    pub fn strokes(&self) -> Vec<&[PathPoint]> {
        let mut out = Vec::new();
        let mut start = 0;
        for i in 1..=self.points.len() {
            if i == self.points.len() || !self.points[i].connected {
                if i > start {
                    out.push(&self.points[start..i]);
                }
                start = i;
            }
        }
        out
    }

    /// Total drawn length in mm.
    pub fn length_mm(&self) -> f64 {
        self.points
            .windows(2)
            .filter(|w| w[1].connected)
            .map(|w| (((w[1].x - w[0].x).pow(2) + (w[1].y - w[0].y).pow(2)) as f64).sqrt())
            .sum()
    }

    /// Encodes the path as a `tr` string: `S`, `M` or `W` at each stroke start (and at a mode
    /// change), relative `L` elsewhere.
    pub fn to_tr(&self) -> String {
        let mut tokens = Vec::with_capacity(self.points.len());
        let mut prev: Option<PathPoint> = None;
        for p in &self.points {
            let tok = match prev {
                Some(q) if p.connected && q.mode == p.mode => PathToken {
                    op: PathOp::Line,
                    x: p.x - q.x,
                    y: p.y - q.y,
                },
                _ => {
                    let op = match p.mode {
                        PathMode::Sweep => PathOp::Sweep,
                        PathMode::Mop => PathOp::Mop,
                        PathMode::SweepAndMop => PathOp::SweepAndMop,
                    };
                    PathToken { op, x: p.x, y: p.y }
                }
            };
            tokens.push(tok);
            prev = Some(*p);
        }
        format_tokens(&tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip() {
        let s = "S100,-200L6,52L5,50M0,0L-10,-10";
        let t = parse_tokens(s).unwrap();
        assert_eq!(t.len(), 5);
        assert_eq!(
            t[0],
            PathToken {
                op: PathOp::Sweep,
                x: 100,
                y: -200
            }
        );
        assert_eq!(
            t[4],
            PathToken {
                op: PathOp::Line,
                x: -10,
                y: -10
            }
        );
        assert_eq!(format_tokens(&t), s);
    }

    #[test]
    fn relative_moves_and_modes() {
        let p = Path::parse("S100,-200L6,52L5,50M0,0L-10,-10").unwrap();
        let xy: Vec<_> = p
            .points
            .iter()
            .map(|q| (q.x, q.y, q.connected, q.mode))
            .collect();
        assert_eq!(
            xy,
            vec![
                (100, -200, false, PathMode::Sweep),
                (106, -148, true, PathMode::Sweep),
                (111, -98, true, PathMode::Sweep),
                (0, 0, false, PathMode::Mop),
                (-10, -10, true, PathMode::Mop),
            ]
        );
        assert_eq!(p.strokes().len(), 2);
        assert_eq!(p.strokes()[0].len(), 3);
        assert_eq!(Path::parse(&p.to_tr()).unwrap(), p);
    }

    #[test]
    fn p_frame_joined_line_appends() {
        // The P-frame form seen on r2579a: `l` absolute, then relative `L`.
        let mut p = Path::parse("S0,0L100,0").unwrap();
        p.append_tr("l-661,3224L6,52L5,50").unwrap();
        assert_eq!(p.len(), 5);
        let last = p.points[4];
        assert_eq!((last.x, last.y, last.connected), (-650, 3326, true));
        assert!(p.points[2].connected);
        assert_eq!(p.strokes().len(), 1);
        // A leading `l` on an empty path cannot connect.
        let q = Path::parse("l5,5L1,1").unwrap();
        assert!(!q.points[0].connected);
    }

    #[test]
    fn errors_and_length() {
        assert!(parse_tokens("X1,2").is_err());
        assert!(parse_tokens("L1").is_err());
        assert!(parse_tokens("L1,").is_err());
        assert!(parse_tokens("").unwrap().is_empty());
        let p = Path::parse("S0,0L3,4L0,10S100,100L1,0").unwrap();
        assert_eq!(p.length_mm(), 16.0);
    }
}
