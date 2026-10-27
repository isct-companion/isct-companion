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
fn schedule_tables_mark_where_the_course_is() {
    let table = safe_html(
        concat!(
            "<table><tr><th>日付</th><th>授業回目</th></tr>",
            "<tr><td>10/2</td><td>1回目</td></tr><tr><td>10/16</td><td>休講</td></tr>",
            "<tr><td>10/20</td><td>2回目</td></tr><tr><td>10/23</td><td>3回目</td></tr></table>",
        ),
        false,
    );
    // Earlier rows past, a cancelled one off (and past), the current row now.
    assert_eq!(
        mark_schedule(&table, Some(2)),
        concat!(
            "<table><tbody><tr><th>日付</th><th>授業回目</th></tr>",
            r#"<tr class="lmsRowPast"><td>10/2</td><td>1回目</td></tr>"#,
            r#"<tr class="lmsRowOff lmsRowPast"><td>10/16</td><td>休講</td></tr>"#,
            r#"<tr class="lmsRowNow"><td>10/20</td><td>2回目</td></tr>"#,
            "<tr><td>10/23</td><td>3回目</td></tr></tbody></table>",
        )
    );
    // No current lesson, no table, or a table with one numbered row: as is.
    assert_eq!(mark_schedule(&table, None), table);
    assert_eq!(mark_schedule("<p>no table</p>", Some(2)), "<p>no table</p>");
    let single = safe_html(
        "<table><tr><td>1回目</td><td>intro</td></tr><tr><td>reading</td><td>—</td></tr></table>",
        false,
    );
    assert_eq!(mark_schedule(&single, Some(2)), single);
}

#[test]
fn lesson_text_reads_as_a_summary_and_labels_as_pictures() {
    assert_eq!(
        lesson_summary("<p>授業計画：正規文法<br>課題：確認する</p>"),
        "正規文法"
    );
    assert_eq!(lesson_summary(""), "");
    let picture = r#"<p><a href="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_label/intro/s.png"><img src="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_label/intro/s_s.png" alt=""></a></p>"#;
    assert!(image_only(picture));
    assert!(!image_only(&format!("{picture}<p>Scan to join</p>")));
    assert!(!image_only(
        r#"<img src="https://lms.s.isct.ac.jp/theme/image.php/icon">"#
    ));
    // Bilingual blocks keep their language for the shells' filter.
    assert_eq!(
        safe_html(r#"<p lang="EN">x</p><p lang="fr">y</p>"#, false),
        r#"<p lang="en">x</p><p>y</p>"#
    );
}
