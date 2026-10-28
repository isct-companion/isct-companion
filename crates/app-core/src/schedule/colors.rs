use serde_json::Value;

use super::text;

// The palette is a design token shared with every shell (design/tokens/course.json).
include!("course_palette.rs");

const IMPORT_DEFAULTS: [&str; 2] = ["#526e91", "#556b8d"];

pub fn display_color(course: &Value) -> String {
    if let Some(color) = course.pointer("/overrides/color").and_then(Value::as_str) {
        if !color.is_empty() {
            return color.into();
        }
    }
    if let Some(color) = text(course, "color") {
        if !color.is_empty() && !IMPORT_DEFAULTS.contains(&color.to_ascii_lowercase().as_str()) {
            return color.into();
        }
    }
    let code = text(course, "code").unwrap_or_default();
    let key = code
        .split(['-', '['])
        .next()
        .filter(|value| !value.is_empty())
        .or_else(|| text(course, "id"))
        .unwrap_or_default();
    palette_color(key).into()
}

/// A weekly personal event's colour: the one the person chose, else a
/// palette colour picked by its title, so one meeting keeps its colour in
/// every quarter it is copied to.
pub fn event_color(event: &Value) -> String {
    text(event, "color")
        .filter(|color| is_hex_color(color))
        .map(str::to_owned)
        .unwrap_or_else(|| palette_color(text(event, "title").unwrap_or_default().trim()).into())
}

pub(crate) fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn palette_color(key: &str) -> &'static str {
    let hash = key.chars().fold(2_166_136_261_u32, |value, character| {
        let mut units = [0_u16; 2];
        let code_unit = character.encode_utf16(&mut units)[0];
        (value ^ u32::from(code_unit)).wrapping_mul(16_777_619)
    });
    COURSE_PALETTE[hash as usize % COURSE_PALETTE.len()]
}
