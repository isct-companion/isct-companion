//! The map's search: buildings by code and name, rooms by code and every
//! alias the export lists. One small in-memory index, built with the map;
//! a query is normalized like an alias key (`alias_key`).
use super::matching::alias_key;
use std::collections::HashMap;

/// At most this many answers; the list is for picking, not browsing.
const MAX_RESULTS: usize = 30;

/// A searchable place, by export index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum Found {
    Building(usize),
    Room(usize),
}

/// How a key names its place: a code is the exact identifier, a name or
/// alias another spelling of it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Spelling {
    Code,
    Alias,
}

struct Entry {
    key: String,
    spelling: Spelling,
    found: Found,
}

#[derive(Default)]
pub(super) struct Index {
    entries: Vec<Entry>,
}

impl Index {
    pub(super) fn add_code(&mut self, code: &str, found: Found) {
        self.add(code, Spelling::Code, found);
    }

    pub(super) fn add_alias(&mut self, spelling: &str, found: Found) {
        self.add(spelling, Spelling::Alias, found);
    }

    fn add(&mut self, raw: &str, spelling: Spelling, found: Found) {
        let key = alias_key(raw);
        if !key.is_empty() {
            self.entries.push(Entry {
                key,
                spelling,
                found,
            });
        }
    }

    /// Places matching `query`, best first: an exact code, an exact alias or
    /// name, a key starting with the query, then a key containing it. Ties
    /// go to the shorter key (the closer spelling), then buildings before
    /// rooms; each place shows once, at its best rank.
    pub(super) fn search(&self, query: &str) -> Vec<Found> {
        let query = alias_key(query);
        if query.is_empty() {
            return Vec::new();
        }
        let mut best: HashMap<Found, (u8, usize)> = HashMap::new();
        for entry in &self.entries {
            let rank = if entry.key == query {
                match entry.spelling {
                    Spelling::Code => 0,
                    Spelling::Alias => 1,
                }
            } else if entry.key.starts_with(&query) {
                2
            } else if entry.key.contains(&query) {
                3
            } else {
                continue;
            };
            let score = (rank, entry.key.chars().count());
            best.entry(entry.found)
                .and_modify(|current| *current = (*current).min(score))
                .or_insert(score);
        }
        let mut found: Vec<(Found, (u8, usize))> = best.into_iter().collect();
        found.sort_by_key(|(place, score)| {
            let (kind, index) = match place {
                Found::Building(index) => (0, *index),
                Found::Room(index) => (1, *index),
            };
            (*score, kind, index)
        });
        found
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(place, _)| place)
            .collect()
    }
}
