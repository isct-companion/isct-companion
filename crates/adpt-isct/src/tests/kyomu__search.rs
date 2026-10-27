use super::*;
use ControlTag::{Button, Input, Select, Textarea};

fn control(tag: ControlTag, kind: &str, name: &str, value: Option<&str>) -> FormControl {
    FormControl {
        tag,
        kind: kind.into(),
        name: name.into(),
        value: value.map(str::to_string),
        checked: false,
        disabled: false,
        multiple: false,
        list_box: false,
        options: Vec::new(),
    }
}

fn option(value: &str, selected: bool, disabled: bool) -> SelectOption {
    SelectOption {
        value: value.into(),
        text: value.into(),
        selected,
        disabled,
    }
}

fn pairs(fields: &[(&str, &str)]) -> Vec<(String, String)> {
    fields
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

#[test]
fn form_data_is_what_the_browser_posts_without_a_submitter() {
    let checked = |control: FormControl| FormControl {
        checked: true,
        ..control
    };
    let select = |name: &str, options: Vec<SelectOption>| FormControl {
        options,
        ..control(Select, "", name, None)
    };
    let controls = [
        control(Input, "hidden", "__VIEWSTATE", Some("state")),
        control(Input, "text", "ctl00$Name", None),
        checked(control(Input, "checkbox", "chkA", None)),
        control(Input, "checkbox", "chkB", Some("x")),
        checked(control(Input, "radio", "grp", Some("1"))),
        checked(control(Input, "radio", "grp", Some("2"))),
        control(Input, "submit", "btnSearch", Some("検索")),
        control(Button, "", "button", Some("b")),
        control(Input, "image", "image", None),
        FormControl {
            disabled: true,
            ..control(Input, "text", "off", Some("v"))
        },
        control(Input, "text", "", Some("nameless")),
        select(
            "size",
            vec![option("10", false, false), option("50", false, false)],
        ),
        select(
            "single",
            vec![option("a", true, false), option("b", true, false)],
        ),
        FormControl {
            multiple: true,
            list_box: true,
            ..select(
                "many",
                vec![
                    option("a", true, false),
                    option("b", false, false),
                    option("c", true, true),
                ],
            )
        },
        select(
            "skip",
            vec![option("x", false, true), option("y", false, false)],
        ),
        control(Textarea, "", "note", Some("line")),
    ];
    // Unchecked boxes, buttons, submit and image inputs, disabled and
    // nameless controls are absent; a checkbox without a value posts "on";
    // only the last checked radio of a group posts; a drop-down with nothing
    // selected posts its first enabled option, a single select its last
    // selected one, a multiple select every enabled selected one.
    assert_eq!(
        form_data(&controls),
        pairs(&[
            ("__VIEWSTATE", "state"),
            ("ctl00$Name", ""),
            ("chkA", "on"),
            ("grp", "2"),
            ("size", "10"),
            ("single", "b"),
            ("many", "a"),
            ("skip", "y"),
            ("note", "line"),
        ])
    );
}

#[test]
fn page_size_postback_switches_the_grid_to_its_largest_choice() {
    let fields = pairs(&[
        ("__VIEWSTATE", "s"),
        ("__EVENTTARGET", ""),
        ("grid$size", "10"),
        ("grid$size", "10"),
    ]);
    let choices: Vec<String> = ["10", "50", "2000", "100"].map(String::from).to_vec();
    assert_eq!(
        expand_postback(fields.clone(), "grid$size", Some("10"), &choices),
        Some(pairs(&[
            ("__VIEWSTATE", "s"),
            ("__EVENTTARGET", "grid$size"),
            ("grid$size", "2000"),
            ("__EVENTARGUMENT", ""),
        ]))
    );
    // Already showing its largest page: nothing to post.
    assert_eq!(
        expand_postback(fields.clone(), "grid$size", Some("2000"), &choices),
        None
    );
    // No numeric size is a paging control this reader does not know.
    assert_eq!(
        expand_postback(fields, "grid$size", Some("10"), &["all".to_string()]),
        None
    );
}

#[test]
fn a_search_is_truncated_at_the_school_cap_or_when_another_page_remains() {
    assert!(!is_truncated(MAX_SEARCH_ROWS - 1, false));
    assert!(is_truncated(MAX_SEARCH_ROWS, false));
    assert!(is_truncated(12, true));
}
