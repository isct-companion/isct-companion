use super::*;
use ControlTag::{Button, Input, Select, Textarea};

/// A result grid captured from the official search (public catalogue text;
/// URL, date and hash in `content-provenance.json` beside it): 2026's CSC.T
/// courses of 3Q and 4Q.
const SEARCH: &str = include_str!("../../../app-core/tests/fixtures/kyomu/course-search.html");

/// Every row of a real grid becomes one stored course: its own offering id
/// and year, the code and quarters the search asked for, a timetable slot per
/// line, credits as JSON integers (stored row hashes must not churn), and
/// the syllabus link. The grid shows all it holds, so it is not truncated.
/// The same Japanese page is refused as English, so a session still in
/// Japanese never stores Japanese names as English ones.
#[test]
fn a_captured_search_grid_reads_every_row() {
    let SearchRead::Results(results) = read_search(SEARCH, true, "2026-10-16T00:00:00Z").unwrap()
    else {
        panic!("the captured grid was not read as results");
    };
    assert_eq!(
        results.courses.len(),
        14,
        "content-provenance.json counts 14"
    );
    assert!(!results.truncated);
    let mut ids = HashSet::new();
    for course in &results.courses {
        let id = course["id"].as_str().unwrap();
        assert!(nine_digits(id) && ids.insert(id), "{course}");
        assert_eq!(course["year"], 2026, "{course}");
        assert!(
            course["code"].as_str().unwrap().starts_with("CSC.T"),
            "{course}"
        );
        let quarters = course["quarters"].as_array().unwrap();
        assert!(
            !quarters.is_empty() && quarters.iter().all(|q| q == "3Q" || q == "4Q"),
            "{course}"
        );
        assert!(
            !course["title"]["ja"].as_str().unwrap().is_empty(),
            "{course}"
        );
        assert!(
            !course["instructors"].as_array().unwrap().is_empty(),
            "{course}"
        );
        let slots = course["schedule"].as_array().unwrap();
        assert!(!slots.is_empty(), "{course}");
        for slot in slots {
            assert_eq!(slot["kind"], "weekly", "{course}");
            let periods = slot["periods"].as_array().unwrap();
            let (from, to) = (periods[0].as_u64().unwrap(), periods[1].as_u64().unwrap());
            assert!((1..=to).contains(&from) && to <= 12, "{course}");
        }
        for part in ["lecture", "exercise", "lab"] {
            assert!(course["credits"][part].is_u64(), "{course}");
        }
        assert!(
            course["syllabusUrl"].as_str().unwrap().contains(id),
            "{course}"
        );
    }
    assert!(read_english(SEARCH, true, "2026-10-16T00:00:00Z").is_err());
}

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
