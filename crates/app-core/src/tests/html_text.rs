use super::*;

// Instructions written in the LMS editor keep their paragraphs, breaks and
// list items as lines; markup, scripts and runs of spaces do not reach the
// reader.
#[test]
fn keeps_paragraphs_breaks_and_list_items_and_drops_markup() {
    let html = r#"<p>Submit <b>one</b>   PDF.</p>
        <p>Name it<br>ID_name.pdf</p>
        <ul><li>Part A</li><li>Part <i>B</i></li></ul>
        <script>alert(1)</script><div>Due &amp; late rules</div>"#;
    assert_eq!(
        plain_text(html),
        "Submit one PDF.\n\nName it\nID_name.pdf\n\n- Part A\n- Part B\n\nDue & late rules"
    );
    assert_eq!(plain_text("plain words"), "plain words");
    assert_eq!(plain_text(""), "");
}
