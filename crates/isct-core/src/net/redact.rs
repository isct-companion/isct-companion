//! Removes credentials from URLs and text before they reach logs, errors,
//! diagnostics exports or stored records. The one redaction rule for Core.

use std::sync::OnceLock;

/// Query parameters whose values are credentials.
const SECRET_PARAMS: [&str; 5] = ["sesskey", "wstoken", "token", "privatetoken", "passport"];
const REDACTED: &str = "[redacted]";
const MOODLE_MOBILE: &str = "moodlemobile://";

/// `raw` with credential query values replaced. Works on the text, never a
/// parsed URL: `moodlemobile://token=…` is not a URL, and a URL parser reads
/// its token as the host.
pub fn redact_url(raw: &str) -> String {
    if raw
        .get(..MOODLE_MOBILE.len())
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case(MOODLE_MOBILE))
    {
        return format!("{}token={REDACTED}", &raw[..MOODLE_MOBILE.len()]);
    }
    let Some(start) = raw.find(['?', '#']) else {
        return raw.to_owned();
    };
    let (base, mut rest) = raw.split_at(start);
    let mut out = String::with_capacity(raw.len());
    out.push_str(base);
    // `rest` starts with `?` (a query, ending at `#`) or `#` (the fragment).
    while let Some(marker) = rest.chars().next() {
        let body = &rest[marker.len_utf8()..];
        let end = if marker == '?' {
            body.find('#').unwrap_or(body.len())
        } else {
            body.len()
        };
        out.push(marker);
        out.push_str(&redact_pairs(&body[..end]));
        rest = &body[end..];
    }
    out
}

fn redact_pairs(pairs: &str) -> String {
    pairs
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, _))
                if SECRET_PARAMS
                    .iter()
                    .any(|secret| name.eq_ignore_ascii_case(secret)) =>
            {
                format!("{name}={REDACTED}")
            }
            _ => pair.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

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
