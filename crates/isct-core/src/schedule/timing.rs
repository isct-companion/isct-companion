use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::ScheduleError;

/// One paired block in the teaching day. Period numbers identify the block;
/// individual period lengths are intentionally not inferred.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TeachingPeriod {
    pub periods: [u8; 2],
    pub start_time: String,
    pub end_time: String,
}

pub fn default_teaching_periods() -> Vec<TeachingPeriod> {
    [
        ([1, 2], "08:50", "10:30"),
        ([3, 4], "10:45", "12:25"),
        ([5, 6], "13:30", "15:10"),
        ([7, 8], "15:25", "17:05"),
        ([9, 10], "17:15", "18:55"),
        ([11, 12], "19:05", "20:45"),
    ]
    .into_iter()
    .map(|(periods, start, end)| TeachingPeriod {
        periods,
        start_time: start.into(),
        end_time: end.into(),
    })
    .collect()
}

pub fn validate_teaching_periods(periods: &[TeachingPeriod]) -> Result<(), ScheduleError> {
    if periods.len() != 6 {
        return Err(ScheduleError::InvalidInput(
            "Teaching periods must define all pairs from 1–2 through 11–12.".into(),
        ));
    }
    let mut previous_end = None;
    for (index, slot) in periods.iter().enumerate() {
        let first = (index * 2 + 1) as u8;
        if slot.periods != [first, first + 1] {
            return Err(ScheduleError::InvalidInput(
                "Teaching periods must define each pair once, in order from 1–2 through 11–12."
                    .into(),
            ));
        }
        let parse = |clock: &str| {
            NaiveTime::parse_from_str(clock, "%H:%M")
                .ok()
                .filter(|time| time.format("%H:%M").to_string() == clock)
                .ok_or_else(|| {
                    ScheduleError::InvalidInput(
                        "Teaching-period times must use 24-hour HH:MM format.".into(),
                    )
                })
        };
        let start = parse(&slot.start_time)?;
        let end = parse(&slot.end_time)?;
        if start >= end || previous_end.is_some_and(|previous| start < previous) {
            return Err(ScheduleError::InvalidInput(
                "Teaching-period times must be ordered and may not overlap.".into(),
            ));
        }
        previous_end = Some(end);
    }
    Ok(())
}

/// Missing legacy data selects the product's official table. Once a table is
/// stored, the whole table must be valid; individual entries never fall back.
pub fn teaching_periods(workspace: &Value) -> Result<Vec<TeachingPeriod>, ScheduleError> {
    let Some(value) = workspace
        .pointer("/academicProfile/teachingPeriods")
        .filter(|value| !value.is_null())
    else {
        return Ok(default_teaching_periods());
    };
    let periods: Vec<TeachingPeriod> = serde_json::from_value(value.clone()).map_err(|error| {
        ScheduleError::InvalidInput(format!("Teaching periods are invalid: {error}"))
    })?;
    validate_teaching_periods(&periods)?;
    Ok(periods)
}

pub(crate) fn clock_for_periods(
    teaching_periods: &[TeachingPeriod],
    periods: [u8; 2],
) -> Result<(&str, &str), ScheduleError> {
    if periods[0] == 0 || periods[0] > periods[1] {
        return Err(ScheduleError::InvalidInput(
            "Choose periods from 1 to 12 in ascending order.".into(),
        ));
    }
    let first = teaching_periods
        .iter()
        .find(|slot| slot.periods.contains(&periods[0]));
    let last = teaching_periods
        .iter()
        .find(|slot| slot.periods.contains(&periods[1]));
    match (first, last) {
        (Some(first), Some(last)) => Ok((&first.start_time, &last.end_time)),
        _ => Err(ScheduleError::InvalidInput(
            "Choose periods from 1 to 12 in ascending order.".into(),
        )),
    }
}

pub(crate) fn validate_period_range(periods: [u8; 2]) -> Result<(), ScheduleError> {
    clock_for_periods(&default_teaching_periods(), periods).map(|_| ())
}

#[cfg(test)]
#[path = "../tests/schedule__timing.rs"]
mod tests;
