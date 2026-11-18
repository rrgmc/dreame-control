//! Station operations for START_WASHING (4.a4): the 4.10 value
//! `"<op>,<flag>"` (device-model.md 4.7).

use super::decode_err;
use dreame_core::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StationOp {
    /// `2,1`
    ReturnAndWash,
    /// `1,1`
    ResumeWashing,
    /// `1,0`
    PauseWashing,
    /// `3,1`
    StartDrying,
    /// `3,0`
    StopDrying,
    /// `14,1`
    StartDustBagDrying,
    /// `14,0`
    StopDustBagDrying,
    /// `7,1`
    Drain,
    /// `9,1`
    DrainCleanWaterTank,
    /// `5,1`
    WashboardSelfClean,
    /// Any other pair, sent as given.
    Raw(u32, u32),
}

impl StationOp {
    pub fn pair(self) -> (u32, u32) {
        match self {
            Self::ReturnAndWash => (2, 1),
            Self::ResumeWashing => (1, 1),
            Self::PauseWashing => (1, 0),
            Self::StartDrying => (3, 1),
            Self::StopDrying => (3, 0),
            Self::StartDustBagDrying => (14, 1),
            Self::StopDustBagDrying => (14, 0),
            Self::Drain => (7, 1),
            Self::DrainCleanWaterTank => (9, 1),
            Self::WashboardSelfClean => (5, 1),
            Self::Raw(a, b) => (a, b),
        }
    }

    pub fn encode(self) -> String {
        let (a, b) = self.pair();
        format!("{a},{b}")
    }

    pub fn parse(s: &str) -> Result<Self> {
        let (a, b) = s.split_once(',').ok_or_else(|| decode_err("station op", s))?;
        let a: u32 = a.trim().parse().map_err(|_| decode_err("station op", s))?;
        let b: u32 = b.trim().parse().map_err(|_| decode_err("station op", s))?;
        let all = [
            Self::ReturnAndWash,
            Self::ResumeWashing,
            Self::PauseWashing,
            Self::StartDrying,
            Self::StopDrying,
            Self::StartDustBagDrying,
            Self::StopDustBagDrying,
            Self::Drain,
            Self::DrainCleanWaterTank,
            Self::WashboardSelfClean,
        ];
        Ok(all.into_iter().find(|o| o.pair() == (a, b)).unwrap_or(Self::Raw(a, b)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        assert_eq!(StationOp::ReturnAndWash.encode(), "2,1");
        assert_eq!(StationOp::StopDrying.encode(), "3,0");
        assert_eq!(StationOp::parse("14,1").unwrap(), StationOp::StartDustBagDrying);
        assert_eq!(StationOp::parse("8,1").unwrap(), StationOp::Raw(8, 1));
        assert!(StationOp::parse("x").is_err());
    }
}
