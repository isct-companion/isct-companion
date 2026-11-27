//! Whether an exam meets somewhere other than its course's classes. The
//! school writes one room string per meeting and per exam row, sometimes
//! several rooms in one (`S8-108,109`, `W8E-307、W8E-308`), and the two
//! sources spell the same room with different widths and hyphens.
use unicode_normalization::UnicodeNormalization;

/// The rooms one string names, each in one spelling: NFKC, no whitespace,
/// upper case, one kind of hyphen. A bare `109` after `S8-108` is `S8-109`.
fn rooms(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for segment in raw.split([',', '、', '，', '/', ';', '・']) {
        let room: String = segment
            .nfkc()
            .filter(|c| !c.is_whitespace())
            .map(|c| match c {
                '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
                c => c.to_ascii_uppercase(),
            })
            .collect();
        if room.is_empty() {
            continue;
        }
        let building = out
            .last()
            .and_then(|last| last.rsplit_once('-'))
            .map(|(building, _)| building.to_owned());
        match building {
            Some(building)
                if !room.contains('-') && room.starts_with(|c: char| c.is_ascii_digit()) =>
            {
                out.push(format!("{building}-{room}"))
            }
            _ => out.push(room),
        }
    }
    out
}

/// The course's usual rooms, when `exam_room` names a room none of them is.
/// `None` when the exam has no room, the course has none to compare with, or
/// every room of the exam is one the course meets in.
pub(super) fn usual_room_if_elsewhere(exam_room: &str, usual: &[&str]) -> Option<String> {
    let held = rooms(exam_room);
    let known: Vec<String> = usual.iter().flat_map(|room| rooms(room)).collect();
    if held.is_empty() || known.is_empty() || held.iter().all(|room| known.contains(room)) {
        return None;
    }
    let mut shown: Vec<&str> = Vec::new();
    for room in usual
        .iter()
        .map(|room| room.trim())
        .filter(|room| !room.is_empty())
    {
        if !shown.contains(&room) {
            shown.push(room);
        }
    }
    Some(shown.join(" / "))
}
