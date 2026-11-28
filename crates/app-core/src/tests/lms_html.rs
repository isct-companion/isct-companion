//! Real teacher HTML is a course's own, so none is committed. The
//! `moodle_ws_captures` test reads the `core_course_get_contents.json`
//! saved as described in `net__moodle__mapping.rs`, from the directory in
//! `MOODLE_WS_CAPTURES`, and is compiled only when that variable names a
//! directory (`build.rs`); a missing file fails the test. The other tests feed the sanitizer generic markup: script,
//! event handlers and `javascript:` links never appear in a capture, so a
//! capture cannot show they are dropped.
//!
//! `MOODLE_WS_CAPTURES=/path/to/dir cargo test -p app-core --lib schedule::lms_html`
use super::*;

#[test]
fn teacher_html_keeps_text_and_safe_links_only() {
    let html = concat!(
        r#"<div class="summarytext" onclick="steal()"><h1>Week 1</h1>"#,
        r#"<p style="color:red">Read <b>this</b> &amp; <a href="https://lms.s.isct.ac.jp/x.pdf" target="_blank">that</a>.</p>"#,
        r#"<a href="javascript:alert(1)">bad</a><a href="http://plain.example/">plain</a>"#,
        r#"<a href="https://lms.s.isct.ac.jp/2026/course/view.php?id=8#coursecontentcollapse3">fold</a>"#,
        r#"<img src="https://lms.s.isct.ac.jp/pluginfile.php/1/diagram.png" alt="Diagram">"#,
        r#"<img src="https://lms.s.isct.ac.jp/theme/image.php/icon">"#,
        r#"<table><tr><td colspan="2" width="9">cell</td></tr></table>"#,
        r#"<script>steal()</script><form><input value="x"></form><iframe src="https://evil"></iframe>"#,
        r#"<div data-region="activity-dates">Opened: today</div></div>"#,
    );
    assert_eq!(
        safe_html(html, true),
        concat!(
            "<div>Week 1",
            r#"<p>Read <b>this</b> &amp; <a href="https://lms.s.isct.ac.jp/x.pdf">that</a>.</p>"#,
            "<a>bad</a><a>plain</a>",
            r#"<a href="https://lms.s.isct.ac.jp/pluginfile.php/1/diagram.png">Diagram</a>"#,
            r#"<table><tbody><tr><td colspan="2">cell</td></tr></tbody></table>"#,
            "</div>",
        )
    );
    // Activity dates stay where they are not asked to go.
    assert!(safe_html(r#"<div data-region="activity-dates">Due</div>"#, false).contains("Due"));
}

#[test]
fn language_projection_uses_explicit_markers_and_preserves_the_sanitized_original() {
    let source = concat!(
        r#"<p>Unmarked announcement</p><p lang="ja">日本語</p>"#,
        r#"<p lang="en-GB"><a href="https://lms.s.isct.ac.jp/2026/pluginfile.php/1/sheet.pdf">English file</a></p>"#,
        r#"<div lang="ja">別の文 <span lang="en">Nested English</span></div>"#,
        r#"<p>連絡先：A<br>Contact: B</p><script>unsafe()</script>"#,
    );
    let view = project_html(source, false, Some(ContentLanguage::En));
    assert!(view.preferred.contains("Unmarked announcement"));
    assert!(view.preferred.contains("Nested English"));
    assert!(view.preferred.contains("English file</a>") && view.preferred.contains("sheet.pdf"));
    assert!(view.preferred.contains("連絡先：A<br>Contact: B"));
    assert!(!view.preferred.contains("日本語") && !view.preferred.contains("別の文"));
    let original = view.original.unwrap();
    assert!(original.contains("別の文") && original.contains("English file"));
    assert!(!original.contains("unsafe()"));
    let only = r#"<p lang="ja">日本語</p><script lang="en">unsafe()</script>"#;
    let view = project_html(only, false, Some(ContentLanguage::En));
    assert_eq!(view.preferred, r#"<p lang="ja">日本語</p>"#);
    assert!(view.original.is_none());
    assert!(project_html(source, false, None)
        .preferred
        .contains("日本語"));
    // The sanitized original keeps recognized language markers only.
    assert_eq!(
        safe_html(r#"<p lang="EN">x</p><p lang="fr">y</p>"#, false),
        r#"<p lang="en">x</p><p>y</p>"#
    );
}

/// Every section summary and activity description of a real course comes
/// out inert, and marking where the course is in a lesson table only adds
/// the row classes, whichever lesson it is at.
#[test]
#[cfg(moodle_ws_captures)]
fn captured_teacher_html_stays_inert_and_schedule_marks_only_add_classes() {
    let path =
        std::path::Path::new(env!("MOODLE_WS_CAPTURES")).join("core_course_get_contents.json");
    let raw = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let contents: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let mut bodies = Vec::new();
    for section in contents.as_array().unwrap() {
        bodies.extend(section["summary"].as_str());
        for module in section["modules"].as_array().into_iter().flatten() {
            bodies.extend(module["description"].as_str());
        }
    }
    assert!(
        bodies.iter().any(|body| !body.trim().is_empty()),
        "the captured course has teacher text"
    );
    let marks = Regex::new(r#" class="lmsRow[A-Za-z ]*""#).unwrap();
    for body in bodies {
        let safe = safe_html(body, true);
        let lower = safe.to_lowercase();
        for inert in [
            "<script",
            "<iframe",
            "<form",
            "<style",
            "javascript:",
            " style=",
            " onclick=",
        ] {
            assert!(!lower.contains(inert), "{inert} in {safe}");
        }
        for lesson in 1..=16 {
            assert_eq!(
                marks.replace_all(&mark_schedule(&safe, Some(lesson)), ""),
                safe
            );
        }
    }
}
