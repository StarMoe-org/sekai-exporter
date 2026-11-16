//! Times in UTC, as publications and signed requests write them.

use std::time::{SystemTime, UNIX_EPOCH};

/// A time in UTC, to the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Utc {
    pub(crate) year: u64,
    pub(crate) month: u64,
    pub(crate) day: u64,
    pub(crate) hour: u64,
    pub(crate) minute: u64,
    pub(crate) second: u64,
}

impl Utc {
    /// `time` in UTC. A time before 1970 is taken as 1970-01-01T00:00:00Z.
    pub(crate) fn of(time: SystemTime) -> Self {
        let seconds = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let (year, month, day) = civil(seconds / 86_400);
        let of_day = seconds % 86_400;
        Self {
            year,
            month,
            day,
            hour: of_day / 3600,
            minute: of_day % 3600 / 60,
            second: of_day % 60,
        }
    }

    /// The time that the RFC 3339 text `text` names in UTC, `YYYY-MM-DDTHH:MM:SS` with an optional
    /// fraction of a second, which is dropped, and then `Z`, as S3 listings give times. `None` for
    /// any other text, or a time before 1970.
    #[cfg(feature = "s3")]
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.strip_suffix('Z')?;
        let whole = text
            .split_once('.')
            .map_or(text, |(whole, _fraction)| whole);
        let (date, time) = whole.split_once('T')?;
        let [year, month, day] = three(date, '-')?;
        let [hour, minute, second] = three(time, ':')?;
        let valid = year >= 1970
            && (1..=12).contains(&month)
            && (1..=31).contains(&day)
            && hour < 24
            && minute < 60
            && second < 60;
        valid.then_some(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    /// The time as a system time.
    #[cfg(feature = "s3")]
    pub(crate) fn time(&self) -> SystemTime {
        let days = days(self.year, self.month, self.day);
        let seconds = days * 86_400 + self.hour * 3600 + self.minute * 60 + self.second;
        UNIX_EPOCH + std::time::Duration::from_secs(seconds)
    }

    /// The time as RFC 3339, `YYYY-MM-DDTHH:MM:SSZ`.
    pub(crate) fn rfc3339(&self) -> String {
        let Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        } = self;
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
    }
}

/// The three numbers of `text` between `separator`s.
#[cfg(feature = "s3")]
fn three(text: &str, separator: char) -> Option<[u64; 3]> {
    let mut numbers = text.split(separator).map(|number| number.parse().ok());
    let three = [numbers.next()??, numbers.next()??, numbers.next()??];
    numbers.next().is_none().then_some(three)
}

/// The days from 1970-01-01 to the date `year`-`month`-`day`, from 1970 on: the inverse of
/// [`civil`].
#[cfg(feature = "s3")]
fn days(year: u64, month: u64, day: u64) -> u64 {
    let year = year - u64::from(month <= 2);
    let era = year / 400;
    let year_of_era = year % 400;
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The date `days` days after 1970-01-01.
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn times_are_written_in_utc() {
        let at = |seconds| Utc::of(UNIX_EPOCH + Duration::from_secs(seconds)).rfc3339();
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
        assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(at(1_735_689_599), "2024-12-31T23:59:59Z");
    }

    #[cfg(feature = "s3")]
    #[test]
    fn listed_times_are_read_back() {
        for seconds in [0, 951_782_400, 1_735_689_599, 4_107_542_400] {
            let time = UNIX_EPOCH + Duration::from_secs(seconds);
            let text = Utc::of(time).rfc3339();
            assert_eq!(
                Utc::parse(&text).map(|utc| utc.time()),
                Some(time),
                "{text}"
            );
        }
        let listed = Utc::parse("2026-11-16T14:25:24.123Z").map(|utc| utc.rfc3339());
        assert_eq!(listed.as_deref(), Some("2026-11-16T14:25:24Z"));
        for text in [
            "2026-11-16T14:25:24",
            "2026-13-01T00:00:00Z",
            "1969-12-31T23:59:59Z",
        ] {
            assert_eq!(Utc::parse(text), None, "{text}");
        }
        assert_eq!(Utc::parse("2026-11-16T14:25Z"), None);
    }
}
