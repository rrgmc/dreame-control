//! Cloud regions and brands (protocols.md sections 1 and 2.3).

use dreame_core::{AppConstants, Error};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// REST API port, the same for every brand.
pub const API_PORT: u16 = 13267;

/// A Dreamehome cloud region. The region belongs to the account, not to the
/// user's location; an account registered outside the US can live in `us`, for example.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Region {
    Us,
    Eu,
    Cn,
    Ru,
    Sg,
    Kr,
    By,
    /// A region code this crate does not know, such as `in`. The host is
    /// built from the code like the known regions.
    Other(String),
    /// An explicit host (`host[:port]`) or base URL (`https://host:port`).
    /// It is used as is and never replaced by the region from the token.
    Custom(String),
}

impl Region {
    /// The region code, or `None` for a custom host.
    pub fn code(&self) -> Option<&str> {
        Some(match self {
            Self::Us => "us",
            Self::Eu => "eu",
            Self::Cn => "cn",
            Self::Ru => "ru",
            Self::Sg => "sg",
            Self::Kr => "kr",
            Self::By => "by",
            Self::Other(c) => c,
            Self::Custom(_) => return None,
        })
    }

    /// Parses a region code or host. Codes are case-insensitive. The aliases
    /// `de` (for `eu`) and `tw` (for `cn`) are accepted. A value that contains
    /// `.`, `:` or `/` is a custom host.
    pub fn parse(s: &str) -> Self {
        let t = s.trim();
        if t.contains(['.', ':', '/']) {
            return Self::Custom(t.to_string());
        }
        match t.to_ascii_lowercase().as_str() {
            "us" => Self::Us,
            "eu" | "de" => Self::Eu,
            "cn" | "tw" => Self::Cn,
            "ru" => Self::Ru,
            "sg" => Self::Sg,
            "kr" => Self::Kr,
            "by" => Self::By,
            other => Self::Other(other.to_string()),
        }
    }

    /// Regions in the order to try when the account's region is unknown:
    /// `us`, `eu`, `sg`, then the rest (protocols.md section 1.3).
    pub fn default_order() -> Vec<Region> {
        vec![Self::Us, Self::Eu, Self::Sg, Self::Cn, Self::Ru, Self::Kr, Self::By]
    }

    pub fn is_custom(&self) -> bool {
        matches!(self, Self::Custom(_))
    }

    /// Base URL of the REST API, without a trailing slash.
    pub fn base_url(&self, brand: Brand) -> String {
        match self {
            Self::Custom(h) if h.starts_with("http://") || h.starts_with("https://") => {
                h.trim_end_matches('/').to_string()
            }
            Self::Custom(h) => format!("https://{}", h.trim_end_matches('/')),
            other => format!("https://{}.{}:{API_PORT}", other.code().unwrap_or("us"), brand.host_suffix()),
        }
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Custom(h) => f.write_str(h),
            other => f.write_str(other.code().unwrap_or_default()),
        }
    }
}

impl std::str::FromStr for Region {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        if s.trim().is_empty() {
            return Err(Error::Decode("empty region".into()));
        }
        Ok(Self::parse(s))
    }
}

impl Serialize for Region {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Region {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// App brand. MOVA and TROUVER share the Dreame cloud software with their own
/// hosts and constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Brand {
    #[default]
    Dreame,
    Mova,
    Trouver,
}

impl Brand {
    /// Host name suffix after the region code.
    pub fn host_suffix(self) -> &'static str {
        match self {
            Self::Dreame => "iot.dreame.tech",
            Self::Mova => "iot.mova-tech.com",
            Self::Trouver => "iot.trouver-tech.com",
        }
    }

    /// The app constants of this brand (protocols.md section 2.3).
    pub fn constants(self) -> AppConstants {
        let d = AppConstants::default();
        match self {
            Self::Dreame => d,
            Self::Mova => AppConstants {
                basic_auth: "Basic bW92YV9hcHA6VjdLb0NoTFc4dkhBQ3FHYg==".into(),
                sign_secret: "gigxlmqwZ]7oWZUF".into(),
                meta_version: "cv=a_10605530".into(),
                user_agent: "Dart/3.10 (dart:io)".into(),
                tenant_id: "000002".into(),
                ..d
            },
            Self::Trouver => AppConstants {
                basic_auth: "Basic dHJvdXZlcl9hcHA6NXB0MGV6d0w1TzRHMG96aw==".into(),
                sign_secret: "MMd02jyRqzuf1QBS".into(),
                meta_version: "cv=a_10300532".into(),
                user_agent: "Dart/3.6 (dart:io)".into(),
                tenant_id: "000005".into(),
                ..d
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(Region::Us.base_url(Brand::Dreame), "https://us.iot.dreame.tech:13267");
        assert_eq!(Region::Eu.base_url(Brand::Mova), "https://eu.iot.mova-tech.com:13267");
        assert_eq!(Region::parse("in").base_url(Brand::Dreame), "https://in.iot.dreame.tech:13267");
        assert_eq!(Region::parse("127.0.0.1:8080").base_url(Brand::Dreame), "https://127.0.0.1:8080");
        assert_eq!(Region::parse("http://localhost:1/").base_url(Brand::Dreame), "http://localhost:1");
    }

    #[test]
    fn parse_and_display() {
        assert_eq!(Region::parse("US"), Region::Us);
        assert_eq!(Region::parse("de"), Region::Eu);
        assert_eq!(Region::parse("tw"), Region::Cn);
        assert_eq!(Region::parse("in"), Region::Other("in".into()));
        for r in Region::default_order() {
            assert_eq!(Region::parse(&r.to_string()), r);
        }
        assert!("".parse::<Region>().is_err());
        let json = serde_json::to_string(&Region::Sg).unwrap();
        assert_eq!(json, "\"sg\"");
        assert_eq!(serde_json::from_str::<Region>(&json).unwrap(), Region::Sg);
    }

    #[test]
    fn default_order_starts_with_us_eu_sg() {
        assert_eq!(&Region::default_order()[..3], &[Region::Us, Region::Eu, Region::Sg]);
    }

    #[test]
    fn brand_constants() {
        assert_eq!(Brand::Dreame.constants(), AppConstants::default());
        assert_eq!(Brand::Mova.constants().tenant_id, "000002");
        assert_eq!(Brand::Trouver.constants().password_salt, AppConstants::default().password_salt);
    }
}
