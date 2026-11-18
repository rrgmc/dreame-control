//! Consumables: one service each, with life percent and time left in p1 and
//! p2 (the order varies) and reset as aiid 1 (device-model.md 2.9).

use dreame_core::{ActionId, PropId};
use serde::Serialize;

/// Which piid holds the percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Order {
    /// p1 time left, p2 percent.
    TimeFirst,
    /// p1 percent, p2 time left.
    PercentFirst,
}

macro_rules! consumables {
    ($($var:ident = $siid:literal, $order:ident, $life:expr, $label:literal;)*) => {
        /// A consumable service.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
        pub enum Consumable { $($var,)* }

        impl Consumable {
            pub const ALL: &'static [Self] = &[$(Self::$var,)*];
            pub const fn siid(self) -> u32 { match self { $(Self::$var => $siid,)* } }
            const fn order(self) -> Order { match self { $(Self::$var => Order::$order,)* } }
            /// Tasshack's lifetime in hours for a local reset, where known.
            pub const fn lifetime(self) -> Option<u32> { match self { $(Self::$var => $life,)* } }
            pub const fn label(self) -> &'static str { match self { $(Self::$var => $label,)* } }
            pub fn from_siid(siid: u32) -> Option<Self> {
                match siid { $($siid => Some(Self::$var),)* _ => None }
            }
        }
    };
}

consumables! {
    MainBrush = 9, TimeFirst, Some(300), "main brush";
    SideBrush = 10, TimeFirst, Some(200), "side brush";
    Filter = 11, PercentFirst, Some(150), "filter";
    Sensor = 16, PercentFirst, Some(30), "sensors";
    TankFilter = 17, PercentFirst, Some(30), "tank filter";
    MopPad = 18, PercentFirst, None, "mop pad";
    SilverIon = 19, TimeFirst, None, "silver ion";
    Detergent = 20, PercentFirst, None, "detergent";
    Squeegee = 24, TimeFirst, None, "squeegee";
    DirtyWaterChannel = 25, TimeFirst, None, "dirty water channel";
    OnboardDirtyWaterTank = 26, TimeFirst, None, "onboard dirty water tank";
    Deodorizer = 29, TimeFirst, None, "deodorizer";
    Wheel = 30, TimeFirst, Some(30), "wheel";
    ScaleInhibitor = 31, TimeFirst, None, "scale inhibitor";
    FluffingRoller = 32, PercentFirst, None, "fluffing roller";
    RollerMopFilter = 33, PercentFirst, None, "roller mop filter";
    WaterOutletFilter = 35, TimeFirst, None, "water outlet filter";
    Track = 36, TimeFirst, None, "track";
    Washboard = 37, TimeFirst, None, "washboard";
    DockFilter = 38, TimeFirst, None, "dock filter";
}

impl Consumable {
    pub const fn percent_prop(self) -> PropId {
        match self.order() {
            Order::TimeFirst => PropId::new(self.siid(), 2),
            Order::PercentFirst => PropId::new(self.siid(), 1),
        }
    }

    pub const fn time_prop(self) -> PropId {
        match self.order() {
            Order::TimeFirst => PropId::new(self.siid(), 1),
            Order::PercentFirst => PropId::new(self.siid(), 2),
        }
    }

    pub const fn reset_action(self) -> ActionId {
        ActionId::new(self.siid(), 1)
    }

    /// Which field a property is, if it belongs to a consumable.
    pub fn field_of(id: PropId) -> Option<(Self, ConsumableField)> {
        let c = Self::from_siid(id.siid)?;
        if id == c.percent_prop() {
            Some((c, ConsumableField::Percent))
        } else if id == c.time_prop() {
            Some((c, ConsumableField::TimeLeft))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumableField {
    Percent,
    TimeLeft,
}

/// Remaining life. The time unit is hours for siids 9, 10 and 11; for 16
/// and 30 hours or days is still open (device-model.md 7, item 9).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ConsumableLife {
    pub percent: Option<i64>,
    pub time_left: Option<i64>,
}

impl ConsumableLife {
    pub fn set(&mut self, field: ConsumableField, v: i64) {
        match field {
            ConsumableField::Percent => self.percent = Some(v),
            ConsumableField::TimeLeft => self.time_left = Some(v),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r2579a_values_and_orders() {
        // (siid, p1, p2, expected percent, expected time) from device-model.md 2.9.
        let cases = [(9, 192, 64, 64, 192), (10, 92, 46, 46, 92), (11, 28, 42, 28, 42), (16, 45, 13, 45, 13), (30, 20, 68, 68, 20)];
        for (siid, p1, p2, pct, time) in cases {
            let mut life = ConsumableLife::default();
            for (piid, v) in [(1, p1), (2, p2)] {
                let (c, f) = Consumable::field_of(PropId::new(siid, piid)).unwrap();
                assert_eq!(c.siid(), siid);
                life.set(f, v);
            }
            assert_eq!(life, ConsumableLife { percent: Some(pct), time_left: Some(time) }, "siid {siid}");
        }
        // Main brush: 192 h of 300 h is 64 %.
        assert_eq!(192 * 100 / Consumable::MainBrush.lifetime().unwrap(), 64);
    }

    #[test]
    fn reset_and_lookup() {
        assert_eq!(Consumable::Filter.reset_action(), ActionId::new(11, 1));
        assert_eq!(Consumable::from_siid(12), None);
        assert_eq!(Consumable::field_of(PropId::new(9, 99)), None);
        assert_eq!(Consumable::ALL.len(), 20);
    }
}
