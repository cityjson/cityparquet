use crate::error::{CityParquetError, Result};

/// A CityJSON Level of Detail such as `1`, `2`, or `2.2` (major, optional minor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lod {
    major: u8,
    minor: Option<u8>,
}

impl Lod {
    pub fn parse(s: &str) -> Result<Self> {
        let err = || CityParquetError::Lod(s.to_string());
        let mut parts = s.split('.');
        let major = parts.next().filter(|p| !p.is_empty()).ok_or_else(err)?;
        let major: u8 = major.parse().map_err(|_| err())?;
        let minor = match parts.next() {
            Some(m) => Some(m.parse().map_err(|_| err())?),
            None => None,
        };
        if parts.next().is_some() {
            return Err(err());
        }
        Ok(Self { major, minor })
    }

    /// CityParquet geometry column suffix, e.g. `lod2_2` for LoD 2.2.
    pub fn column_suffix(&self) -> String {
        match self.minor {
            Some(minor) => format!("lod{}_{minor}", self.major),
            None => format!("lod{}", self.major),
        }
    }

    pub fn from_column_suffix(suffix: &str) -> Option<Self> {
        let rest = suffix.strip_prefix("lod")?;
        Self::parse(&rest.replace('_', ".")).ok()
    }
}

impl std::fmt::Display for Lod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.minor {
            Some(minor) => write!(f, "{}.{minor}", self.major),
            None => write!(f, "{}", self.major),
        }
    }
}

#[cfg(test)]
mod lod_tests {
    use super::*;

    #[test]
    fn parses_and_displays() {
        assert_eq!(Lod::parse("2").unwrap().to_string(), "2");
        assert_eq!(Lod::parse("2.2").unwrap().to_string(), "2.2");
        assert!(Lod::parse("").is_err());
        assert!(Lod::parse("2.x").is_err());
        assert!(Lod::parse("2.2.2").is_err());
    }

    #[test]
    fn column_suffix_round_trip() {
        let lod = Lod::parse("2.2").unwrap();
        assert_eq!(lod.column_suffix(), "lod2_2");
        assert_eq!(Lod::from_column_suffix("lod2_2"), Some(lod));
        assert_eq!(
            Lod::from_column_suffix("lod1"),
            Some(Lod::parse("1").unwrap())
        );
        assert_eq!(Lod::from_column_suffix("geometry"), None);
    }

    #[test]
    fn orders_numerically() {
        let mut lods = [
            Lod::parse("2.2").unwrap(),
            Lod::parse("1.3").unwrap(),
            Lod::parse("2").unwrap(),
        ];
        lods.sort();
        let s: Vec<String> = lods.iter().map(|l| l.to_string()).collect();
        assert_eq!(s, ["1.3", "2", "2.2"]);
    }
}
