use std::fmt;
use std::str::FromStr;

use crate::error::Error;

/// Game version (`major.minor.patch`); ordered numerically, so `1.21.11 < 26.1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u16, pub u16, pub u16);

impl Version {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self(major, minor, patch)
    }
}

impl FromStr for Version {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let err = || Error::UnknownVersion(s.to_string());
        let mut parts = s.trim().split('.');
        let mut next = |required: bool| -> Result<u16, Error> {
            match parts.next() {
                Some(p) => p.parse().map_err(|_| err()),
                None if required => Err(err()),
                None => Ok(0),
            }
        };
        let version = Version(next(true)?, next(true)?, next(false)?);
        if parts.next().is_some() {
            return Err(err());
        }
        Ok(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.2 == 0 {
            write!(f, "{}.{}", self.0, self.1)
        } else {
            write!(f, "{}.{}.{}", self.0, self.1, self.2)
        }
    }
}

/// Resource pack format. Since 1.21.9 formats have a minor component (e.g. `69.0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackFormat(pub u32, pub u32);

/// First format that uses `min_format`/`max_format` in `pack.mcmeta` (25w31a).
pub const MIN_MAX_FORMAT_SINCE: PackFormat = PackFormat(65, 0);

/// Every supported release and its resource pack format, oldest first.
const RELEASES: &[(Version, PackFormat)] = &[
    (Version(1, 7, 2), PackFormat(1, 0)),
    (Version(1, 7, 10), PackFormat(1, 0)),
    (Version(1, 8, 0), PackFormat(1, 0)),
    (Version(1, 8, 9), PackFormat(1, 0)),
    (Version(1, 9, 0), PackFormat(2, 0)),
    (Version(1, 9, 4), PackFormat(2, 0)),
    (Version(1, 10, 0), PackFormat(2, 0)),
    (Version(1, 10, 2), PackFormat(2, 0)),
    (Version(1, 11, 0), PackFormat(3, 0)),
    (Version(1, 11, 2), PackFormat(3, 0)),
    (Version(1, 12, 0), PackFormat(3, 0)),
    (Version(1, 12, 2), PackFormat(3, 0)),
    (Version(1, 13, 0), PackFormat(4, 0)),
    (Version(1, 13, 2), PackFormat(4, 0)),
    (Version(1, 14, 0), PackFormat(4, 0)),
    (Version(1, 14, 4), PackFormat(4, 0)),
    (Version(1, 15, 0), PackFormat(5, 0)),
    (Version(1, 15, 2), PackFormat(5, 0)),
    (Version(1, 16, 0), PackFormat(5, 0)),
    (Version(1, 16, 1), PackFormat(5, 0)),
    (Version(1, 16, 2), PackFormat(6, 0)),
    (Version(1, 16, 5), PackFormat(6, 0)),
    (Version(1, 17, 0), PackFormat(7, 0)),
    (Version(1, 17, 1), PackFormat(7, 0)),
    (Version(1, 18, 0), PackFormat(8, 0)),
    (Version(1, 18, 2), PackFormat(8, 0)),
    (Version(1, 19, 0), PackFormat(9, 0)),
    (Version(1, 19, 2), PackFormat(9, 0)),
    (Version(1, 19, 3), PackFormat(12, 0)),
    (Version(1, 19, 4), PackFormat(13, 0)),
    (Version(1, 20, 0), PackFormat(15, 0)),
    (Version(1, 20, 1), PackFormat(15, 0)),
    (Version(1, 20, 2), PackFormat(18, 0)),
    (Version(1, 20, 3), PackFormat(22, 0)),
    (Version(1, 20, 4), PackFormat(22, 0)),
    (Version(1, 20, 5), PackFormat(32, 0)),
    (Version(1, 20, 6), PackFormat(32, 0)),
    (Version(1, 21, 0), PackFormat(34, 0)),
    (Version(1, 21, 1), PackFormat(34, 0)),
    (Version(1, 21, 2), PackFormat(42, 0)),
    (Version(1, 21, 3), PackFormat(42, 0)),
    (Version(1, 21, 4), PackFormat(46, 0)),
    (Version(1, 21, 5), PackFormat(55, 0)),
    (Version(1, 21, 6), PackFormat(63, 0)),
    (Version(1, 21, 7), PackFormat(64, 0)),
    (Version(1, 21, 8), PackFormat(64, 0)),
    (Version(1, 21, 9), PackFormat(69, 0)),
    (Version(1, 21, 10), PackFormat(69, 0)),
    (Version(1, 21, 11), PackFormat(75, 0)),
];

pub fn releases() -> impl Iterator<Item = (Version, PackFormat)> {
    RELEASES.iter().copied()
}

pub fn oldest() -> Version {
    RELEASES[0].0
}

pub fn newest() -> Version {
    RELEASES[RELEASES.len() - 1].0
}

/// Resource pack format for a supported release; unknown patch releases resolve to the
/// closest older listed release (e.g. `1.16.4` → `1.16.2`'s format).
pub fn pack_format(version: Version) -> Result<PackFormat, Error> {
    if version < oldest() || version > newest() {
        return Err(Error::UnknownVersion(version.to_string()));
    }
    Ok(RELEASES.iter().rev().find(|(v, _)| *v <= version).expect("in range").1)
}

/// Newest release that uses `format`, or the closest older one for formats between releases
/// (snapshot formats). Formats newer than every release map to the newest release.
pub fn version_for_format(format: PackFormat) -> Version {
    RELEASES
        .iter()
        .rev()
        .find(|(_, f)| *f <= format)
        .map_or(oldest(), |(v, _)| *v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_orders() {
        let a: Version = "1.21.11".parse().unwrap();
        let b: Version = "26.1".parse().unwrap();
        assert!(a < b);
        assert_eq!(b.to_string(), "26.1");
        assert!("1".parse::<Version>().is_err());
        assert!("1.2.3.4".parse::<Version>().is_err());
    }

    #[test]
    fn table_is_sorted() {
        for w in RELEASES.windows(2) {
            assert!(w[0].0 < w[1].0 && w[0].1 <= w[1].1, "{:?}", w);
        }
    }

    #[test]
    fn formats() {
        assert_eq!(pack_format(Version(1, 16, 4)).unwrap(), PackFormat(6, 0));
        assert_eq!(version_for_format(PackFormat(6, 0)), Version(1, 16, 5));
        assert_eq!(version_for_format(PackFormat(10, 0)), Version(1, 19, 2));
        assert!(pack_format(Version(1, 6, 4)).is_err());
    }
}
