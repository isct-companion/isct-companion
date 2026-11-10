//! Removes credentials from text before they reach logs, errors,
//! diagnostics exports or stored records. The one redaction rule for Core.

use std::sync::OnceLock;

const REDACTED: &str = "[redacted]";

/// `text` with every `name=value` / `"name": "value"` credential, bearer
/// token, cookie header and `moodlemobile://token=` value replaced. Fails
/// closed: if the rule cannot be built the whole text is withheld.
pub fn redact_text(text: &str) -> String {
    static PATTERN: OnceLock<Result<regex::Regex, regex::Error>> = OnceLock::new();
    match PATTERN.get_or_init(|| {
        regex::Regex::new(concat!(
            r#"(?i)((?:password|passwd|secret|otp_secret|wstoken|privatetoken|access_token|refresh_token|token|sesskey|passport|code)["']?\s*[:=]\s*["']?)[^&\s"',}]+"#,
            r#"|(bearer\s+)[a-z0-9._~+/=-]+"#,
            r#"|((?:set-)?cookie\s*:\s*)[^\r\n]+"#,
            r#"|(moodlemobile://)\S+"#,
        ))
    }) {
        Ok(pattern) => pattern
            .replace_all(text, |captures: &regex::Captures| {
                match captures.get(4) {
                    Some(scheme) => format!("{}token={REDACTED}", scheme.as_str()),
                    None => format!(
                        "{}{}{}{REDACTED}",
                        captures.get(1).map_or("", |m| m.as_str()),
                        captures.get(2).map_or("", |m| m.as_str()),
                        captures.get(3).map_or("", |m| m.as_str()),
                    ),
                }
            })
            .into_owned(),
        Err(_) => "[Log message unavailable]".into(),
    }
}

#[cfg(test)]
#[path = "../tests/net__redact.rs"]
mod tests;
