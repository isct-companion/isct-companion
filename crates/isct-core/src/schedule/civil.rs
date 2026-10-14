use std::{fmt, str::FromStr};

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate};
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

pub(crate) fn clock_for_periods(
    periods: [u8; 2],
) -> Result<(&'static str, &'static str), ScheduleError> {
    const TIMES: [(&str, &str); 5] = [
        ("08:50", "10:30"),
        ("10:45", "12:25"),
        ("13:30", "15:10"),
        ("15:25", "17:05"),
        ("17:15", "18:55"),
    ];
    let valid = |period: u8| (1..=10).contains(&period);
    if !valid(periods[0]) || !valid(periods[1]) || periods[0] > periods[1] {
        return Err(ScheduleError::InvalidInput(
            "Choose periods from 1 to 10 in ascending order.".into(),
        ));
    }
    let first = TIMES[((periods[0] - 1) / 2) as usize].0;
    let last = TIMES[((periods[1] - 1) / 2) as usize].1;
    Ok((first, last))
}
