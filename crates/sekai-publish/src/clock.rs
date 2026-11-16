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
}
