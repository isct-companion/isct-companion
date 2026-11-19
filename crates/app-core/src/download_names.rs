//! File names for completed downloads, independent of how bytes arrive.

// Leave room for the extension and collision number on desktop filesystems.
const NAME_LIMIT: usize = 200;

pub(crate) fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 && name.len() - index <= 16 => name.split_at(index),
        _ => (name, ""),
    }
}

/// Turns a suggested name into one path component safe for this target.
/// A supplied PDF MIME type requires a PDF suffix.
pub fn safe_name(suggested: &str, mime_type: &str) -> String {
    let cleaned: String = suggested
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| match character {
            '/' | '\\' | ':' => '_',
            #[cfg(target_os = "windows")]
            '<' | '>' | '"' | '|' | '?' | '*' => '_',
            character => character,
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let mut name = if cleaned.is_empty() {
        "download"
    } else {
        cleaned
    }
    .to_owned();
    if mime_type.eq_ignore_ascii_case("application/pdf")
        && !name.to_ascii_lowercase().ends_with(".pdf")
    {
        name.push_str(".pdf");
    }
    #[cfg(target_os = "windows")]
    if reserved_windows_name(&name) {
        name.insert(0, '_');
    }
    if name.len() <= NAME_LIMIT {
        return name;
    }
    let (stem, extension) = split_extension(&name);
    let mut keep = NAME_LIMIT - extension.len();
    while !stem.is_char_boundary(keep) {
        keep -= 1;
    }
    format!("{}{extension}", stem[..keep].trim_end())
}

#[cfg(target_os = "windows")]
fn reserved_windows_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|number| {
                matches!(
                    number,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

/// The original name followed by numbered alternatives before its extension.
pub fn name_candidates(name: &str) -> impl Iterator<Item = String> + '_ {
    let (stem, extension) = split_extension(name);
    std::iter::once(name.to_string())
        .chain((1..1000).map(move |number| format!("{stem} ({number}){extension}")))
}

#[cfg(test)]
#[path = "tests/download_names.rs"]
mod tests;
