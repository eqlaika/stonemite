use std::fmt;
use std::str::FromStr;

/// Maximum number of additional public releases supported on one date.
pub const MAX_DAILY_REVISION: u8 = 99;

/// Stonemite's public calendar version: `YYYY.MM.DD[.N]`.
///
/// The first release on a date omits the revision; later releases use `.1`
/// through `.99`. Ordering is chronological, then by the daily revision.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CalVer {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub revision: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseCalVerError(String);

impl fmt::Display for ParseCalVerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseCalVerError {}

impl CalVer {
    /// Parse a public version, accepting an optional Git tag `v` prefix.
    pub fn parse(input: &str) -> Result<Self, ParseCalVerError> {
        input.parse()
    }

    /// Return the order-preserving SemVer used by Cargo and Tauri internally.
    ///
    /// The patch component packs `DD` and the daily revision as `DDNN`, so
    /// `2026.08.22.1` maps to `2026.8.2201`. Public UI and release metadata
    /// always use the canonical calendar version instead.
    #[allow(dead_code)] // Used by build.rs; the runtime module only compares public versions.
    pub fn cargo_version(self) -> String {
        let patch = u16::from(self.day) * 100 + u16::from(self.revision);
        format!("{}.{}.{}", self.year, self.month, patch)
    }
}

impl FromStr for CalVer {
    type Err = ParseCalVerError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.strip_prefix('v').unwrap_or(input);
        let parts: Vec<&str> = input.split('.').collect();
        if !(parts.len() == 3 || parts.len() == 4) {
            return Err(invalid(input));
        }
        if !is_fixed_digits(parts[0], 4)
            || !is_fixed_digits(parts[1], 2)
            || !is_fixed_digits(parts[2], 2)
        {
            return Err(invalid(input));
        }

        let year = parse_component(parts[0], input)?;
        let month = parse_component(parts[1], input)?;
        let day = parse_component(parts[2], input)?;
        let revision = if let Some(value) = parts.get(3) {
            if value.is_empty()
                || !value.bytes().all(|byte| byte.is_ascii_digit())
                || value.starts_with('0')
            {
                return Err(invalid(input));
            }
            let revision: u8 = parse_component(value, input)?;
            if revision == 0 || revision > MAX_DAILY_REVISION {
                return Err(invalid(input));
            }
            revision
        } else {
            0
        };

        if year == 0 || !(1..=12).contains(&month) {
            return Err(invalid(input));
        }
        let max_day = days_in_month(year, month);
        if day == 0 || day > max_day {
            return Err(invalid(input));
        }

        Ok(Self {
            year,
            month,
            day,
            revision,
        })
    }
}

impl fmt::Display for CalVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}.{:02}.{:02}", self.year, self.month, self.day)?;
        if self.revision > 0 {
            write!(f, ".{}", self.revision)?;
        }
        Ok(())
    }
}

fn parse_component<T>(value: &str, original: &str) -> Result<T, ParseCalVerError>
where
    T: FromStr,
{
    value.parse().map_err(|_| invalid(original))
}

fn is_fixed_digits(value: &str, width: usize) -> bool {
    value.len() == width && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn invalid(input: &str) -> ParseCalVerError {
    ParseCalVerError(format!(
        "invalid Stonemite version {input:?}; expected YYYY.MM.DD or YYYY.MM.DD.N (N=1-{MAX_DAILY_REVISION})"
    ))
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_canonical_versions() {
        let first = CalVer::parse("2026.08.22").unwrap();
        assert_eq!(first.to_string(), "2026.08.22");
        assert_eq!(first.revision, 0);

        let second = CalVer::parse("v2026.08.22.1").unwrap();
        assert_eq!(second.to_string(), "2026.08.22.1");
        assert_eq!(second.revision, 1);
    }

    #[test]
    fn rejects_noncanonical_and_impossible_versions() {
        for version in [
            "2026.8.22",
            "2026.08.22.0",
            "2026.08.22.01",
            "2026.08.22.100",
            "2026.02.29",
            "2026.13.01",
            "0.5.0",
            "2026.08.22-alpha",
        ] {
            assert!(CalVer::parse(version).is_err(), "accepted {version}");
        }
        assert!(CalVer::parse("2028.02.29").is_ok());
        assert!(CalVer::parse("2000.02.29").is_ok());
        assert!(CalVer::parse("2100.02.29").is_err());
    }

    #[test]
    fn orders_dates_and_same_day_revisions() {
        let versions = [
            "2026.08.22",
            "2026.08.22.1",
            "2026.08.22.2",
            "2026.08.23",
            "2026.09.01",
            "2027.01.01",
        ]
        .map(|version| CalVer::parse(version).unwrap());

        assert!(versions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn cargo_encoding_is_order_preserving() {
        let cases = [
            ("2026.08.22", "2026.8.2200"),
            ("2026.08.22.1", "2026.8.2201"),
            ("2026.08.22.99", "2026.8.2299"),
            ("2026.08.23", "2026.8.2300"),
        ];

        for (public, internal) in cases {
            assert_eq!(CalVer::parse(public).unwrap().cargo_version(), internal);
        }
    }
}
