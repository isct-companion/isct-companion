use url::Url;

/// The resident school WebView still imports this historical symbol. Its
/// implementation now lives in the typed LMS ingestion module.
pub const T2SCHOLA_EXTRACT_SCRIPT: &str = crate::lms_ingest::LMS_EXTRACT_SCRIPT;

/// Shared allowlist passed to Android's synchronous navigation delegate.
pub(crate) const SCHOOL_HOSTS: &[&str] = &[
    "isct.ex-tic.com",
    "portal.isct.ac.jp",
    "portal.titech.ac.jp",
    "lms.s.isct.ac.jp",
    "t2schola.titech.ac.jp",
    "syllabus.s.isct.ac.jp",
    "portal.nap.gsic.titech.ac.jp",
    "rp.nap.gsic.titech.ac.jp",
    "mailv4.m.titech.ac.jp",
    "kyomu0.gakumu.titech.ac.jp",
    "request.libra.titech.ac.jp",
    "portal.t4.gsic.titech.ac.jp",
];

pub fn sanitize_school_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    if !url
        .host_str()
        .is_some_and(|host| SCHOOL_HOSTS.contains(&host))
    {
        return None;
    }
    url.set_fragment(None);
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, value)| {
            let safe_identifier = value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
            (matches!(key.as_ref(), "id" | "course" | "page" | "section") && safe_identifier)
                // Syllabus course pages: `courses?jwc={9-digit offering id}`.
                || (key == "jwc" && value.len() == 9 && value.bytes().all(|b| b.is_ascii_digit()))
                || (key == "lang" && matches!(value.as_ref(), "ja" | "en"))
                || (key == "plang" && matches!(value.as_ref(), "jpn" | "eng"))
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !kept.is_empty() {
        url.query_pairs_mut().extend_pairs(kept);
    }
    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syllabus_course_links_keep_their_offering_id() {
        assert_eq!(
            sanitize_school_url("https://syllabus.s.isct.ac.jp/courses?jwc=202602431&x=1")
                .as_deref(),
            Some("https://syllabus.s.isct.ac.jp/courses?jwc=202602431")
        );
        assert_eq!(
            sanitize_school_url("https://syllabus.s.isct.ac.jp/courses?jwc=javascript").as_deref(),
            Some("https://syllabus.s.isct.ac.jp/courses")
        );
    }

    #[test]
    fn rejects_credentials_and_non_school_hosts() {
        assert!(sanitize_school_url("https://user:secret@lms.s.isct.ac.jp/2025/").is_none());
        assert!(sanitize_school_url("https://example.com/").is_none());
        assert!(sanitize_school_url("http://lms.s.isct.ac.jp/2025/").is_none());
    }
}
