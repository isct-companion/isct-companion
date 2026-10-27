//! A backup expires only after 30 days and four strictly newer saved backups.
use chrono::{DateTime, Duration, Utc};

pub(crate) fn expired<'a>(
    dates: impl IntoIterator<Item = &'a str>,
    now: DateTime<Utc>,
) -> Vec<usize> {
    let mut dated = dates
        .into_iter()
        .enumerate()
        .filter_map(|(index, date)| {
            let date = DateTime::parse_from_rfc3339(date).ok()?.with_timezone(&Utc);
            // Unknown/future timestamps are never grounds for deleting another file.
            (date <= now).then_some((index, date))
        })
        .collect::<Vec<_>>();
    dated.sort_by_key(|&(_, date)| std::cmp::Reverse(date));
    let cutoff = now - Duration::days(30);
    let mut group_start = 0;
    let mut expired = Vec::new();
    for (rank, (index, date)) in dated.iter().enumerate() {
        if rank == 0 || dated[rank - 1].1 != *date {
            group_start = rank;
        }
        if group_start >= 4 && *date < cutoff {
            expired.push(*index);
        }
    }
    expired
}

#[cfg(test)]
#[path = "../tests/backup__retention.rs"]
mod tests;
