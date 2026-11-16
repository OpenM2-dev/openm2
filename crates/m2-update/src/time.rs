//! Times in manifests: seconds since 1970 in UTC, written
//! `YYYY-MM-DDTHH:MM:SSZ`.

use std::fmt;
use std::str::FromStr;

use crate::UpdateError;

/// A moment in UTC, in whole seconds since 1970-01-01T00:00:00Z.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Timestamp(u64);

/// The last second that can be written: 9999-12-31T23:59:59Z.
const LAST: u64 = 253_402_300_799;

const DAY: u64 = 86_400;

impl Timestamp {
    /// From seconds since 1970, capped at the last second of 9999.
    pub fn from_unix(seconds: u64) -> Self {
        Self(seconds.min(LAST))
    }

    /// Seconds since 1970.
    pub fn unix(self) -> u64 {
        self.0
    }

    /// `days` later, capped at the last second of 9999.
    pub fn plus_days(self, days: u64) -> Self {
        Self::from_unix(self.0.saturating_add(days.saturating_mul(DAY)))
    }
}

/// Days since 1970-01-01 of a date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year / 400;
    let year_of_era = year - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    // 719,468 days from 0000-03-01 to 1970-01-01.
    era * 146_097 + day_of_era - 719_468
}

/// The date of a day since 1970-01-01 (`civil_from_days`).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let days = days + 719_468;
    let era = days / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = civil_from_days(self.0 / DAY);
        let second = self.0 % DAY;
        write!(
            f,
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            second / 3600,
            second / 60 % 60,
            second % 60
        )
    }
}

impl FromStr for Timestamp {
    type Err = UpdateError;

    /// Reads exactly `YYYY-MM-DDTHH:MM:SSZ`, from 1970 to 9999.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || UpdateError::Time(crate::shown(text));
        let bytes = text.as_bytes();
        let [
            y1,
            y2,
            y3,
            y4,
            b'-',
            mo1,
            mo2,
            b'-',
            d1,
            d2,
            b'T',
            h1,
            h2,
            b':',
            mi1,
            mi2,
            b':',
            s1,
            s2,
            b'Z',
        ] = *bytes
        else {
            return Err(bad());
        };
        let number = |digits: &[u8]| {
            digits.iter().try_fold(0u64, |value, digit| {
                digit
                    .is_ascii_digit()
                    .then(|| value * 10 + u64::from(digit - b'0'))
            })
        };
        let year = number(&[y1, y2, y3, y4]).ok_or_else(bad)?;
        let month = number(&[mo1, mo2]).ok_or_else(bad)?;
        let day = number(&[d1, d2]).ok_or_else(bad)?;
        let hour = number(&[h1, h2]).ok_or_else(bad)?;
        let minute = number(&[mi1, mi2]).ok_or_else(bad)?;
        let second = number(&[s1, s2]).ok_or_else(bad)?;
        if year < 1970
            || !(1..=12).contains(&month)
            || day < 1
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return Err(bad());
        }
        Ok(Self(
            days_from_civil(year, month, day) * DAY + hour * 3600 + minute * 60 + second,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_moments() {
        for (text, seconds) in [
            ("1970-01-01T00:00:00Z", 0),
            ("2000-02-29T12:34:56Z", 951_827_696),
            ("2026-10-22T00:00:00Z", 1_792_627_200),
            ("2038-01-19T03:14:08Z", 2_147_483_648),
            ("9999-12-31T23:59:59Z", LAST),
        ] {
            let time: Timestamp = text.parse().unwrap();
            assert_eq!(time.unix(), seconds, "{text}");
            assert_eq!(time.to_string(), text);
        }
    }

    #[test]
    fn every_day_both_ways() {
        // Every day from 1970 to past 2100 (not a leap year) reads back.
        for day in 0..50_000 {
            let time = Timestamp::from_unix(day * DAY + 3661);
            assert_eq!(time.to_string().parse::<Timestamp>().unwrap(), time);
        }
    }

    #[test]
    fn other_writings_are_refused() {
        for text in [
            "",
            "2026-10-22",
            "2026-10-22T00:00:00",
            "2026-10-22T00:00:00+00:00",
            "2026-10-22 00:00:00Z",
            "2026-10-22t00:00:00z",
            "2026-10-22T00:00:00.5Z",
            "1969-12-31T23:59:59Z",
            "2026-13-01T00:00:00Z",
            "2026-00-01T00:00:00Z",
            "2026-02-29T00:00:00Z",
            "2100-02-29T00:00:00Z",
            "2026-04-31T00:00:00Z",
            "2026-10-22T24:00:00Z",
            "2026-10-22T00:60:00Z",
            "2026-10-22T00:00:60Z",
            "+026-10-22T00:00:00Z",
        ] {
            assert!(text.parse::<Timestamp>().is_err(), "{text:?}");
        }
        assert!("2024-02-29T00:00:00Z".parse::<Timestamp>().is_ok());
        assert!("2000-02-29T00:00:00Z".parse::<Timestamp>().is_ok());
    }

    #[test]
    fn days_are_added_up_to_the_last_second() {
        let start: Timestamp = "2026-10-22T08:00:00Z".parse().unwrap();
        assert_eq!(start.plus_days(90).to_string(), "2027-01-20T08:00:00Z");
        assert_eq!(start.plus_days(u64::MAX).unix(), LAST);
        assert_eq!(Timestamp::from_unix(u64::MAX).unix(), LAST);
    }
}
