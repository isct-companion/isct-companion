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
