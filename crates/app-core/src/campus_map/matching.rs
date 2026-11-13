//! Reads a course's free-text room string into the rooms and buildings it
//! mentions. Nothing here knows which of them the map has: `map` decides.
//! The strings are the school's, as in `S7-207, S7-209S8-110, S8-108,109`,
//! `南4号館3階第2演習室` or `石川台1号館 204,253号室`.
use regex::{Captures, Regex};
use unicode_normalization::UnicodeNormalization;

/// The export's alias key for a spelling (contract v1): Unicode NFKC, every
/// whitespace character removed, then upper case. The exporter applies the
/// same to every key, so a lookup needs nothing else.
pub(super) fn alias_key(raw: &str) -> String {
    raw.nfkc()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase()
}

/// One room or building the string mentions.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct Mention {
    /// A building code: `W9`, `S4`, `M`.
    pub building: Option<String>,
    /// A room id in the data's form: `W9-202`, `M-B07`.
    pub room: Option<String>,
    /// A floor's short name: `3F`, `B1`.
    pub level: Option<String>,
    /// The mention's words, normalized, where a named room may be found.
    pub text: String,
}

pub(super) struct Patterns {
    glued: Regex,
    lecture_hall: Regex,
    numbered_hall: Regex,
    code: Regex,
    building: Regex,
    bare_room: Regex,
    only_level: Regex,
    level: Regex,
    room: Regex,
}

impl Patterns {
    pub(super) fn new() -> Result<Self, regex::Error> {
        Ok(Self {
            // A room id running straight into the next building's code. The
            // leading hyphen keeps `W8E-101` whole: its digit starts no room.
            glued: Regex::new(r"(-\s*[A-Z]?\d+)([A-Z]{1,3}\d{0,2}[EW]?-)")?,
            lecture_hall: Regex::new(r"(?:大岡山)?(西|南)講義棟\s*(\d?)")?,
            numbered_hall: Regex::new(
                r"(?:大岡山)?(西|南|北|石川台|石)\s*(\d+)\s*(?:号館)?(?:\(([EW])\))?",
            )?,
            code: Regex::new(r"(?:^|[^A-Z0-9])([A-Z]{1,3}\d{0,2}[EW]?)\s*-\s*([A-Z]?\d+[A-Z]?)")?,
            building: Regex::new(
                r"(?:^|[^A-Z0-9])(WL\d|SL|W\d{1,2}[EW]?|S\d{1,2}|I\d{1,2}|N\d{1,2}|[GBJ]\d|M)(?:$|[^A-Z0-9])",
            )?,
            bare_room: Regex::new(r"^([A-Z]?\d+[A-Z]?)\s*(?:号室|演習室|室)?\s*(?:他|等)?$")?,
            only_level: Regex::new(r"^(地下|B)?(\d+)\s*(?:階|F)$")?,
            level: Regex::new(r"(地下|B)?(\d+)\s*(?:階|F)")?,
            room: Regex::new(r"(?:^|[^第A-Z0-9])([A-Z]?\d+[A-Z]?)(?:$|[^A-Z0-9-])")?,
        })
    }

    pub(super) fn mentions(&self, raw: &str) -> Vec<Mention> {
        let (outer, notes) = strip_notes(&normalize(raw));
        let mut mentions = self.parse(&outer);
        // `第2ラボ(W3,4F)`: the building is only in the note.
        if mentions.iter().all(|mention| mention.building.is_none()) {
            for note in notes {
                mentions.extend(self.parse(&note));
            }
        }
        mentions
    }

    fn parse(&self, text: &str) -> Vec<Mention> {
        let text = self.glued.replace_all(text, "${1},${2}");
        let mut mentions = Vec::new();
        // A bare `110` belongs to the building last named.
        let mut current = None;
        for segment in text.split([',', '/', '、', ';']) {
            self.segment(segment, &mut current, &mut mentions);
        }
        mentions
    }

    fn segment(&self, segment: &str, current: &mut Option<String>, out: &mut Vec<Mention>) {
        // `I3-301~3`, `I3-201A~C`: the first of a range stands for it.
        let words = segment.split('~').next().unwrap_or_default().trim();
        if words.is_empty() {
            return;
        }
        if let Some(found) = self.only_level.captures(words) {
            if let Some(last) = out.last_mut() {
                last.level = Some(level_name(&found));
            }
            return;
        }
        // `W3` alone is a building, though it reads like a room number too.
        let bare = self.bare_room.captures(words);
        if let Some(found) = bare.filter(|_| !self.building.is_match(words)) {
            if let Some(building) = current.as_deref() {
                out.push(Mention {
                    building: Some(building.to_owned()),
                    room: Some(format!("{building}-{}", &found[1])),
                    text: words.to_owned(),
                    ..Mention::default()
                });
            }
            return;
        }
        let coded = self.with_building_codes(words);
        let level = self.level.captures(&coded).map(|found| level_name(&found));
        let mut any = false;
        for found in self.code.captures_iter(&coded) {
            any = true;
            *current = Some(found[1].to_owned());
            out.push(Mention {
                building: Some(found[1].to_owned()),
                room: Some(format!("{}-{}", &found[1], &found[2])),
                level: level.clone(),
                text: words.to_owned(),
            });
        }
        if any {
            return;
        }
        let Some(found) = self.building.captures(&coded) else {
            *current = None;
            out.push(Mention {
                level,
                text: words.to_owned(),
                ..Mention::default()
            });
            return;
        };
        let building = found[1].to_owned();
        let rest = coded[found.get(1).map_or(0, |name| name.end())..].to_owned();
        let rest = self.level.replace_all(&rest, " ");
        let room = self
            .room
            .captures(&rest)
            .map(|room| format!("{building}-{}", &room[1]));
        *current = Some(building.clone());
        out.push(Mention {
            building: Some(building),
            room,
            level,
            text: words.to_owned(),
        });
    }

    /// `南4号館` as `S4`, `石3` as `I3`, `本館` as `M`.
    fn with_building_codes(&self, words: &str) -> String {
        let words = self
            .lecture_hall
            .replace_all(words, |found: &Captures<'_>| match (&found[1], &found[2]) {
                ("南", _) => " SL ".to_owned(),
                (_, number) => format!(" WL{number} "),
            });
        let words = self
            .numbered_hall
            .replace_all(&words, |found: &Captures<'_>| {
                let letter = match &found[1] {
                    "西" => "W",
                    "南" => "S",
                    "北" => "N",
                    _ => "I",
                };
                let wing = found.get(3).map_or("", |wing| wing.as_str());
                format!(" {letter}{}{wing} ", &found[2])
            });
        words.replace("本館", " M ")
    }
}

/// Half-width text, one kind of hyphen and tilde, upper case Latin.
fn normalize(raw: &str) -> String {
    raw.nfkc()
        .map(|c| match c {
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            '\u{301C}' => '~',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

/// The text outside brackets, and what the brackets held. `西8号館(E)` keeps
/// its bracket: that one is part of the building's name.
fn strip_notes(text: &str) -> (String, Vec<String>) {
    let mut outer = String::new();
    let mut notes = Vec::new();
    let mut note = String::new();
    let mut depth = 0_usize;
    for c in text.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if note == "E" || note == "W" {
                        outer.push_str(&format!("({note})"));
                        note.clear();
                    } else {
                        notes.push(std::mem::take(&mut note));
                    }
                }
            }
            c if depth > 0 => note.push(c),
            c => outer.push(c),
        }
    }
    if !note.is_empty() {
        notes.push(note);
    }
    (outer, notes)
}

/// A floor's short name, as the data writes it.
fn level_name(found: &Captures<'_>) -> String {
    match found.get(1) {
        Some(_) => format!("B{}", &found[2]),
        None => format!("{}F", &found[2]),
    }
}
