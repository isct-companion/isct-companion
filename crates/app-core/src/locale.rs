//! The language school text reads in, and the one rule that picks it
//! (`approach/content-language-in-core`). Read models carry the picked text,
//! so no shell chooses between Japanese and English itself.
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

/// The language school text (course names, rooms, calendar labels) is shown
/// in. It is independent of the UI language unless left on automatic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentLanguage {
    Ja,
    En,
}

impl ContentLanguage {
    /// `settings.contentLanguage` when it names a language; on automatic,
    /// Japanese for a Japanese or Chinese UI and English otherwise.
    pub fn of(workspace: &Value) -> Self {
        let setting = |key: &str| {
            workspace
                .pointer(&format!("/settings/{key}"))
                .and_then(Value::as_str)
        };
        match setting("contentLanguage") {
            Some("ja") => Self::Ja,
            Some("en") => Self::En,
            _ if matches!(setting("locale"), Some("ja" | "zh")) => Self::Ja,
            _ => Self::En,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Ja => "ja",
            Self::En => "en",
        }
    }
}

/// School text as Isct-Data publishes it and the stored records keep it:
/// Japanese, English, or both. Either may be missing.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(deny_unknown_fields)]
pub struct Localized {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub ja: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub en: Option<String>,
}

impl Localized {
    /// This text in `language`, by [`pick`]'s rule.
    pub fn pick(&self, language: ContentLanguage) -> Option<&str> {
        choose(self.ja.as_deref(), self.en.as_deref(), language)
    }
}

/// `value` in `language`, or else in the other language; never a made-up
/// translation. A plain string is text in one language and reads as it is.
/// Blank text counts as missing.
pub fn pick(value: Option<&Value>, language: ContentLanguage) -> Option<&str> {
    let value = value?;
    if let Some(text) = value.as_str() {
        return Some(text).filter(|text| present(text));
    }
    let text = |key| value.get(key).and_then(Value::as_str);
    choose(text("ja"), text("en"), language)
}

fn choose<'a>(
    ja: Option<&'a str>,
    en: Option<&'a str>,
    language: ContentLanguage,
) -> Option<&'a str> {
    let (first, second) = match language {
        ContentLanguage::Ja => (ja, en),
        ContentLanguage::En => (en, ja),
    };
    first
        .filter(|text| present(text))
        .or(second.filter(|text| present(text)))
}

fn present(text: &str) -> bool {
    !text.trim().is_empty()
}
