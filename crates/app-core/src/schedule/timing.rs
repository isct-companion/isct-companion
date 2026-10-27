use chrono::{NaiveTime, Timelike};
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

/// A gap this long between two blocks (lunch) keeps a band of its own; a
/// shorter break folds into the rule between two rows.
const BREAK_BAND_MINUTES: u32 = 30;
/// The lunch band's height, in period rows.
const BREAK_BAND_ROWS: f64 = 0.5;

/// Where times sit on a timetable whose period rows all have one height, so
/// desktop and mobile place classes, empty-slot selections and the now line
/// the same way. Offsets are in period rows from the first period's start.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TimetableAxis {
    /// Height of the whole axis in rows.
    pub length: f64,
    /// Clock times and their offsets, ascending; a time between two stops
    /// interpolates linearly, one outside them clamps to the nearest end.
    pub stops: Vec<AxisStop>,
    /// Teaching blocks and break bands, top to bottom.
    pub rows: Vec<AxisRow>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AxisStop {
    /// Minutes since midnight.
    pub minutes: u32,
    pub offset: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AxisRow {
    /// The block's periods; absent for a break band.
    pub periods: Option<[u8; 2]>,
    pub start_time: String,
    pub end_time: String,
    pub top: f64,
    pub bottom: f64,
}

fn clock_minutes(raw: &str) -> Result<u32, ScheduleError> {
    let time = NaiveTime::parse_from_str(raw, "%H:%M")
        .map_err(|_| ScheduleError::InvalidInput(format!("Invalid clock time: {raw}")))?;
    Ok(time.hour() * 60 + time.minute())
}

/// Every period is one row; a block of two periods is two rows. A class
/// outside the teaching day (`earliest`, `latest`: the week's first start and
/// last end) widens the axis at the first or last block's pace instead of
/// being cut off.
pub fn timetable_axis(
    periods: &[TeachingPeriod],
    earliest: Option<&str>,
    latest: Option<&str>,
) -> Result<TimetableAxis, ScheduleError> {
    validate_teaching_periods(periods)?;
    let mut stops = Vec::new();
    let mut rows = Vec::new();
    let mut offset = 0.0;
    let mut previous: Option<(u32, &str)> = None;
    for block in periods {
        let start = clock_minutes(&block.start_time)?;
        let end = clock_minutes(&block.end_time)?;
        if let Some((previous_end, previous_clock)) = previous {
            if start - previous_end >= BREAK_BAND_MINUTES {
                rows.push(AxisRow {
                    periods: None,
                    start_time: previous_clock.into(),
                    end_time: block.start_time.clone(),
                    top: offset,
                    bottom: offset + BREAK_BAND_ROWS,
                });
                offset += BREAK_BAND_ROWS;
            }
        }
        let height = f64::from(block.periods[1].saturating_sub(block.periods[0]) + 1);
        stops.push(AxisStop {
            minutes: start,
            offset,
        });
        stops.push(AxisStop {
            minutes: end,
            offset: offset + height,
        });
        rows.push(AxisRow {
            periods: Some(block.periods),
            start_time: block.start_time.clone(),
            end_time: block.end_time.clone(),
            top: offset,
            bottom: offset + height,
        });
        offset += height;
        previous = Some((end, &block.end_time));
    }
    let pace = |block: Option<&TeachingPeriod>| -> Result<f64, ScheduleError> {
        let Some(block) = block else { return Ok(1.0) };
        let minutes = clock_minutes(&block.end_time)? - clock_minutes(&block.start_time)?;
        let rows = f64::from(block.periods[1].saturating_sub(block.periods[0]) + 1);
        Ok(f64::from(minutes.max(1)) / rows)
    };
    if let (Some(raw), Some(first)) = (earliest, stops.first().map(|stop| stop.minutes)) {
        let earliest = clock_minutes(raw)?;
        if earliest < first {
            let lead = f64::from(first - earliest) / pace(periods.first())?;
            for stop in &mut stops {
                stop.offset += lead;
            }
            for row in &mut rows {
                row.top += lead;
                row.bottom += lead;
            }
            stops.insert(
                0,
                AxisStop {
                    minutes: earliest,
                    offset: 0.0,
                },
            );
            offset += lead;
        }
    }
    if let (Some(raw), Some(last)) = (latest, stops.last().map(|stop| stop.minutes)) {
        let latest = clock_minutes(raw)?;
        if latest > last {
            offset += f64::from(latest - last) / pace(periods.last())?;
            stops.push(AxisStop {
                minutes: latest,
                offset,
            });
        }
    }
    Ok(TimetableAxis {
        length: offset,
        stops,
        rows,
    })
}

impl TimetableAxis {
    /// The offset of a clock time (minutes since midnight).
    pub fn offset(&self, minutes: u32) -> f64 {
        let (Some(first), Some(last)) = (self.stops.first(), self.stops.last()) else {
            return 0.0;
        };
        if minutes <= first.minutes {
            return first.offset;
        }
        if minutes >= last.minutes {
            return last.offset;
        }
        self.stops
            .windows(2)
            .find(|pair| minutes <= pair[1].minutes)
            .map_or(last.offset, |pair| {
                let span = f64::from(pair[1].minutes - pair[0].minutes).max(1.0);
                pair[0].offset
                    + (pair[1].offset - pair[0].offset) * f64::from(minutes - pair[0].minutes)
                        / span
            })
    }
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
