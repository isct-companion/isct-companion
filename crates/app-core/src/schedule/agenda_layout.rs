//! Geometry and display metadata for resolved calendar occurrences.

use chrono::{NaiveTime, Timelike};

use super::alternation;
use super::occurrence::{mark_conflicts, overlap};
use super::{GridSlot, Occurrence, OccurrenceKind, ScheduleError, SessionMark, TeachingPeriod};

pub(super) fn finalize_agenda(
    mut blocks: Vec<Occurrence>,
    periods: &[TeachingPeriod],
) -> Result<Vec<Occurrence>, ScheduleError> {
    for block in &mut blocks {
        block.conflict = None;
        block.conflict_with = None;
    }
    alternation::pair_blocks(&mut blocks)?;
    mark_conflicts(&mut blocks)?;
    for i in 0..blocks.len() {
        if blocks[i].kind != OccurrenceKind::Exam {
            continue;
        }
        let collisions: Vec<_> = (0..blocks.len())
            .filter(|&j| {
                j != i
                    && blocks[j].cancelled != Some(true)
                    && blocks[j].course_id != blocks[i].course_id
                    && overlap(&blocks[i], &blocks[j]).unwrap_or(false)
            })
            .collect();
        let names: Vec<_> = collisions
            .into_iter()
            .map(|j| {
                blocks[j].conflict = Some(true);
                blocks[j].title.clone()
            })
            .collect();
        if !names.is_empty() {
            blocks[i].conflict = Some(true);
            blocks[i].conflict_with = Some(names)
        }
    }
    blocks.sort_by(|a, b| {
        a.start_time
            .cmp(&b.start_time)
            .then((a.kind == OccurrenceKind::Exam).cmp(&(b.kind == OccurrenceKind::Exam)))
            .then(a.key.cmp(&b.key))
    });
    place_on_grid(&mut blocks, periods);
    for block in &mut blocks {
        block.mark = session_mark(block);
    }
    Ok(blocks)
}

fn session_mark(block: &Occurrence) -> Option<SessionMark> {
    let mark = match (block.kind, block.cancelled, block.quiz) {
        (OccurrenceKind::Event, ..) => return None,
        (_, Some(true), _) => SessionMark::Cancelled,
        (OccurrenceKind::Exam, _, Some(true)) => SessionMark::Quiz,
        (OccurrenceKind::Exam, ..) => SessionMark::Exam,
        (OccurrenceKind::Course, ..) => SessionMark::Class,
    };
    (block.planned != Some(true) || mark != SessionMark::Class).then_some(mark)
}

/// The grid a timetable draws: a class or exam whose times fall inside
/// teaching blocks fills the whole blocks it touches (an exam at 09:30–10:20
/// fills 1–2限), and blocks that then overlap sit side by side, the earlier
/// one on the left. `grid` stays empty for a block drawn at its own times
/// on its own.
fn place_on_grid(blocks: &mut [Occurrence], periods: &[TeachingPeriod]) {
    let clock = |raw: &str| {
        NaiveTime::parse_from_str(raw, "%H:%M")
            .ok()
            .map(|time| time.hour() * 60 + time.minute())
    };
    let teaching: Vec<_> = periods
        .iter()
        .filter_map(|period| Some((clock(&period.start_time)?, clock(&period.end_time)?, period)))
        .collect();
    let spans: Vec<Option<(String, String, u32, u32)>> = blocks
        .iter()
        .map(|block| {
            let (start, end) = (clock(&block.start_time)?, clock(&block.end_time)?);
            if end <= start || block.end_date.is_some() {
                return None;
            }
            let snaps = matches!(block.kind, OccurrenceKind::Course | OccurrenceKind::Exam);
            let from = teaching
                .iter()
                .find(|(from, to, _)| snaps && start >= *from && start < *to);
            let to = teaching
                .iter()
                .find(|(from, to, _)| snaps && end > *from && end <= *to);
            Some((
                from.map_or_else(|| block.start_time.clone(), |p| p.2.start_time.clone()),
                to.map_or_else(|| block.end_time.clone(), |p| p.2.end_time.clone()),
                from.map_or(start, |p| p.0),
                to.map_or(end, |p| p.1),
            ))
        })
        .collect();
    let mut order: Vec<usize> = (0..blocks.len()).filter(|&i| spans[i].is_some()).collect();
    order.sort_by_key(|&i| spans[i].as_ref().map(|span| (span.2, u32::MAX - span.3)));
    let mut lanes = vec![(0u8, 1u8); blocks.len()];
    let mut cluster: Vec<usize> = Vec::new();
    let mut lane_ends: Vec<u32> = Vec::new();
    let mut cluster_end = 0;
    let close = |lanes: &mut [(u8, u8)], cluster: &mut Vec<usize>, lane_ends: &mut Vec<u32>| {
        let count = u8::try_from(lane_ends.len().max(1)).unwrap_or(u8::MAX);
        for &member in cluster.iter() {
            lanes[member].1 = count;
        }
        cluster.clear();
        lane_ends.clear();
    };
    for &i in &order {
        let Some((_, _, start, end)) = spans[i] else {
            continue;
        };
        if !cluster.is_empty() && start >= cluster_end {
            close(&mut lanes, &mut cluster, &mut lane_ends);
        }
        let lane = match lane_ends.iter().position(|&lane_end| lane_end <= start) {
            Some(free) => {
                lane_ends[free] = end;
                free
            }
            None => {
                lane_ends.push(end);
                lane_ends.len() - 1
            }
        };
        lanes[i].0 = u8::try_from(lane).unwrap_or(u8::MAX);
        cluster.push(i);
        cluster_end = cluster_end.max(end);
        if cluster.len() == 1 {
            cluster_end = end;
        }
    }
    close(&mut lanes, &mut cluster, &mut lane_ends);
    for (i, block) in blocks.iter_mut().enumerate() {
        block.grid = spans[i].as_ref().and_then(|(from, to, _, _)| {
            let (lane, count) = lanes[i];
            (count > 1 || *from != block.start_time || *to != block.end_time).then(|| GridSlot {
                start_time: from.clone(),
                end_time: to.clone(),
                lane,
                lanes: count,
            })
        });
    }
}
