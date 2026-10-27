use super::*;

fn item(code: &str) -> GuideItem {
    GuideItem {
        offering_id: "202602431".into(),
        code: code.into(),
        title: "Synthetic".into(),
        day: Some(1),
        slot: "3Q 月3-4".into(),
    }
}

#[test]
fn guide_never_submits_or_posts_back() {
    let script = script(&[item("CSC.T273")]).unwrap();
    for forbidden in [
        "__doPostBack",
        "submit(",
        ".click(",
        "$.ajax",
        "fetch(",
        ".value =",
    ] {
        assert!(!script.contains(forbidden), "{forbidden}");
    }
}

#[test]
fn rejects_unbounded_or_malformed_items() {
    assert!(validate(vec![item("CSC.T273")]).is_ok());
    let mut bad = item("CSC.T273");
    bad.offering_id = "javascript".into();
    assert!(validate(vec![bad]).is_err());
    let mut bad = item("CSC.T273\n");
    bad.day = Some(1);
    assert!(validate(vec![bad]).is_err());
    assert!(validate((0..41).map(|_| item("X")).collect()).is_err());
    assert!(script(&[]).is_none());
}

#[test]
fn queue_is_embedded_as_json() {
    let script = script(&[item("A</script>")]).unwrap();
    assert!(script.contains(r#""code":"A</script>""#));
    assert!(!script.contains("__QUEUE__"));
}
