use std::str::FromStr;
use regex::Regex;
use serde::Serialize;
use once_cell::sync::Lazy;
static VERSION_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?P<major>[0-9]+)\.(?P<minor>[0-9]+)\.(?P<patch>[0-9]+)(?:(?P<type_str>[a-z])(?P<type_number>\d+)?)?.*$").unwrap()
});
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[repr(u8)]
pub enum UnityVersionType {
    Alpha = 0,
    Beta = 1,
    China = 2,
    Final = 3,
    Patch = 4,
    Experimental = 5,
    Unknown = 255,
}
impl From<&str> for UnityVersionType {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "a" => Self::Alpha,
            "b" => Self::Beta,
            "c" => Self::China,
            "f" => Self::Final,
            "p" => Self::Patch,
            "x" => Self::Experimental,
            _ => Self::Unknown,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct UnityVersion(u64);
impl UnityVersion {
    pub const fn new(major: u16, minor: u16, patch: u16, ver_type: UnityVersionType, type_number: u8) -> Self {
        let val = ((major as u64) << 48) |
                  ((minor as u64) << 32) |
                  ((patch as u64) << 16) |
                  ((ver_type as u64) << 8) |
                  (type_number as u64);
        Self(val)
    }
    pub const fn empty() -> Self {
        Self::new(0, 0, 0, UnityVersionType::Unknown, 0)
    }
    pub fn major(&self) -> u16 { ((self.0 >> 48) & 0xFFFF) as u16 }
    pub fn minor(&self) -> u16 { ((self.0 >> 32) & 0xFFFF) as u16 }
    pub fn patch(&self) -> u16 { ((self.0 >> 16) & 0xFFFF) as u16 }
    pub fn ver_type(&self) -> UnityVersionType {
        match (self.0 >> 8) & 0xFF {
            0 => UnityVersionType::Alpha,
            1 => UnityVersionType::Beta,
            2 => UnityVersionType::China,
            3 => UnityVersionType::Final,
            4 => UnityVersionType::Patch,
            5 => UnityVersionType::Experimental,
            _ => UnityVersionType::Unknown,
        }
    }
    pub fn type_number(&self) -> u8 { (self.0 & 0xFF) as u8 }
    pub fn is_empty(&self) -> bool {
        self.major() == 0
    }
    pub fn guess_from_format(format_version: u32) -> Self {
        match format_version {
            0..=10 => Self::new(4, 0, 0, UnityVersionType::Final, 0),
            11..=14 => Self::new(5, 0, 0, UnityVersionType::Final, 0),
            15 => Self::new(5, 5, 0, UnityVersionType::Final, 0),
            16 => Self::new(5, 6, 0, UnityVersionType::Final, 0),
            17..=19 => Self::new(2017, 4, 0, UnityVersionType::Final, 0),
            20..=21 => Self::new(2019, 4, 0, UnityVersionType::Final, 0),
            22.. => Self::new(2021, 3, 0, UnityVersionType::Final, 0),
        }
    }
}
impl PartialEq<(u16, u16)> for UnityVersion {
    fn eq(&self, other: &(u16, u16)) -> bool {
        self.major() == other.0 && self.minor() == other.1
    }
}
impl PartialOrd<(u16, u16)> for UnityVersion {
    fn partial_cmp(&self, other: &(u16, u16)) -> Option<std::cmp::Ordering> {
        match self.major().partial_cmp(&other.0) {
            Some(std::cmp::Ordering::Equal) => self.minor().partial_cmp(&other.1),
            ord => ord,
        }
    }
}
impl PartialEq<(u16, u16, u16)> for UnityVersion {
    fn eq(&self, other: &(u16, u16, u16)) -> bool {
        self.major() == other.0 && self.minor() == other.1 && self.patch() == other.2
    }
}
impl PartialOrd<(u16, u16, u16)> for UnityVersion {
    fn partial_cmp(&self, other: &(u16, u16, u16)) -> Option<std::cmp::Ordering> {
        match self.major().partial_cmp(&other.0) {
            Some(std::cmp::Ordering::Equal) => match self.minor().partial_cmp(&other.1) {
                Some(std::cmp::Ordering::Equal) => self.patch().partial_cmp(&other.2),
                ord => ord,
            },
            ord => ord,
        }
    }
}
impl FromStr for UnityVersion {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(caps) = VERSION_PATTERN.captures(s) {
            let major = caps["major"].parse::<u16>().map_err(|e| e.to_string())?;
            let minor = caps["minor"].parse::<u16>().map_err(|e| e.to_string())?;
            let patch = caps["patch"].parse::<u16>().map_err(|e| e.to_string())?;
            let type_str = caps.name("type_str").map(|m| m.as_str()).unwrap_or("f");
            let type_number = caps
                .name("type_number")
                .map(|m| m.as_str().parse::<u8>().unwrap_or(0))
                .unwrap_or(0);
            Ok(Self::new(major, minor, patch, UnityVersionType::from(type_str), type_number))
        } else {
            if s.is_empty() || s == "0.0.0" {
                Ok(Self::empty())
            } else {
                Err(format!("Invalid Unity version string: {}", s))
            }
        }
    }
}
impl std::fmt::Display for UnityVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_empty() {
            write!(f, "0.0.0")
        } else {
            let type_char = match self.ver_type() {
                UnityVersionType::Alpha => 'a',
                UnityVersionType::Beta => 'b',
                UnityVersionType::China => 'c',
                UnityVersionType::Final => 'f',
                UnityVersionType::Patch => 'p',
                UnityVersionType::Experimental => 'x',
                UnityVersionType::Unknown => 'u',
            };
            write!(f, "{}.{}.{}{}{}", self.major(), self.minor(), self.patch(), type_char, self.type_number())
        }
    }
}
impl Default for UnityVersion {
    fn default() -> Self {
        Self::empty()
    }
}
