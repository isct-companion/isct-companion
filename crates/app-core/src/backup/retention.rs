//! Cloud history thins out with age, the way Time Machine keeps backups: the
//! newest copy of each hour for a day, the newest of each day for 30 days,
//! nothing older. The four newest copies always stay, so a workspace nobody
//! touched for months still has its last versions.
use chrono::{DateTime, Duration, Utc};
use std::collections::HashSet;

const ALWAYS_KEPT: usize = 4;

/// Indexes of `dates` the history drops at `now`. Buckets are fixed UTC hours
/// and days, so a copy kept for its hour is not dropped later for the same
/// hour. Unknown and future dates are never dropped and never count as newer.
pub(crate) fn expired<'a>(
    dates: impl IntoIterator<Item = &'a str>,
    now: DateTime<Utc>,
) -> Vec<usize> {
    let mut dated = dates
        .into_iter()
        .enumerate()
        .filter_map(|(index, date)| {
            let date = DateTime::parse_from_rfc3339(date).ok()?.with_timezone(&Utc);
            (date <= now).then_some((index, date))
        })
        .collect::<Vec<_>>();
    // Newest first; equal dates keep their given order.
    dated.sort_by_key(|&(index, date)| (std::cmp::Reverse(date), index));
    let (mut hours, mut days) = (HashSet::new(), HashSet::new());
    let mut expired = Vec::new();
    for (rank, (index, date)) in dated.into_iter().enumerate() {
        let age = now - date;
        let hour = date.timestamp().div_euclid(3_600);
        let day = date.timestamp().div_euclid(86_400);
        let keep = rank < ALWAYS_KEPT
            || (age <= Duration::days(1) && !hours.contains(&hour))
            || (age <= Duration::days(30) && !days.contains(&day));
        if keep {
            hours.insert(hour);
            days.insert(day);
        } else {
            expired.push(index);
        }
    }
    expired.sort_unstable();
    expired
}

#[cfg(test)]
#[path = "../tests/backup__retention.rs"]
mod tests;
