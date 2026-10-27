use std::{fmt, str::FromStr};

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::ScheduleError;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct Term {
    pub year: i32,
    pub quarter: u8,
}

impl Term {
    pub fn parse(raw: &str) -> Result<Self, ScheduleError> {
        raw.parse()
    }

    /// The quarter `date` falls in by month: April–May 1Q, June–September
    /// 2Q, October–November 3Q, December–March 4Q of the academic year that
    /// started in April. Approximate; the academic calendar sets the real
    /// bounds.
    pub fn by_month(date: NaiveDate) -> Self {
        let quarter = match date.month() {
            4 | 5 => 1,
            6..=9 => 2,
            10 | 11 => 3,
            _ => 4,
        };
        Self {
            year: date.year() - i32::from(date.month() < 4),
            quarter,
        }
    }
}

/// Today's date in Japan.
pub fn japan_today(now: DateTime<Utc>) -> NaiveDate {
    FixedOffset::east_opt(9 * 3600)
        .map(|offset| now.with_timezone(&offset).date_naive())
        .unwrap_or_else(|| now.date_naive())
}

impl FromStr for Term {
    type Err = ScheduleError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let Some((year, quarter)) = raw.split_once('-') else {
            return Err(ScheduleError::InvalidInput("Invalid quarter.".into()));
        };
        let year = year
            .parse::<i32>()
            .map_err(|_| ScheduleError::InvalidInput("Invalid quarter.".into()))?;
        let quarter = quarter
            .strip_suffix('Q')
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|value| (1..=4).contains(value))
            .ok_or_else(|| ScheduleError::InvalidInput("Invalid quarter.".into()))?;
        if year.to_string().len() != 4 {
            return Err(ScheduleError::InvalidInput("Invalid quarter.".into()));
        }
        Ok(Self { year, quarter })
    }
}

impl fmt::Display for Term {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}-{}Q", self.year, self.quarter)
    }
}

pub fn parse_civil_date(raw: &str) -> Result<NaiveDate, ScheduleError> {
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| ScheduleError::InvalidInput("Choose a valid date.".into()))?;
    if date.format("%Y-%m-%d").to_string() != raw {
        return Err(ScheduleError::InvalidInput("Choose a valid date.".into()));
    }
    Ok(date)
}

pub fn add_days(raw: &str, days: i64) -> Result<String, ScheduleError> {
    parse_civil_date(raw)?
        .checked_add_signed(Duration::days(days))
        .map(|date| date.to_string())
        .ok_or_else(|| ScheduleError::InvalidInput("Date is out of range.".into()))
}

pub fn tokyo_date(value: DateTime<FixedOffset>) -> String {
    FixedOffset::east_opt(9 * 60 * 60)
        .map(|jst| value.with_timezone(&jst).date_naive().to_string())
        .unwrap_or_else(|| value.date_naive().to_string())
}

pub(crate) fn weekday(raw: &str) -> Result<u8, ScheduleError> {
    Ok(parse_civil_date(raw)?.weekday().number_from_monday() as u8)
}
