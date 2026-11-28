//! 教務Web's student pages carry a person's grades and records, so none is
//! committed. The `kyomu_captures` test reads pages saved from one signed-in
//! account from the directory in `KYOMU_CAPTURES`, kept outside the
//! repository, and is compiled only when that directory exists, and a file
//! missing from it fails its test
//! (`build.rs`):
//!
//! - `student-top.html`: <https://kyomu0.gakumu.titech.ac.jp/Titech/Student/>;
//! - `<kind>.html` for each kind in [`PAGES`] (`gpa`, `registrations`,
//!   `declared`, `profile`, `grades`, `requirements`): the page the student
//!   menu links for it (成績一覧 is `PID2_0` for 学士, `PID2_2` for 修士).
//!
//! Save each page as the server sent it (view-source, then save), not as the
//! browser rendered it, or print the HTML that the ignored
//! `connect_and_read` in `app-core`'s `school_session__login.rs` returns for
//! that address. A run per programme (学士, 修士) covers each one's menu.
//!
//! `KYOMU_CAPTURES=/path/to/dir cargo test -p adpt-isct --lib kyomu::tests`
use super::*;
#[cfg(kyomu_captures)]
use std::collections::HashSet;

/// The pages a read may fetch are a closed list, matched without regard to
/// case as the school's IIS does (the menu's own links use lower-case
/// escapes); a page of another programme is not guessed.
#[test]
fn only_the_listed_pages_are_approved_in_any_case() {
    for page in &PAGES {
        for path in page.paths() {
            assert_eq!(expected_kind(&path.to_lowercase()), Some(page.kind));
        }
    }
    let grades = Url::parse(&page_url("grades").unwrap()).unwrap();
    assert_eq!(
        expected_kind(&grades.path().replace("PID2_0", "PID2_5")),
        None
    );
    assert_eq!(expected_kind(Url::parse(STUDENT_TOP).unwrap().path()), None);
}

#[cfg(kyomu_captures)]
fn capture(name: &str) -> String {
    let path = std::path::Path::new(env!("KYOMU_CAPTURES")).join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Each table keeps one value per measure column, a blank one in place
/// rather than shifted, and every row a label of its own (1.3.1 refused the
/// master's 修了要件 page for rows it could not name).
#[cfg(kyomu_captures)]
fn assert_requirement_tables(data: &Value) {
    let tables = data["tables"].as_array().unwrap();
    assert!(!tables.is_empty(), "a requirements page lists its tables");
    for table in tables {
        let headers = table["headers"].as_array().unwrap().len();
        let mut labels = HashSet::new();
        for row in table["rows"].as_array().unwrap() {
            assert_eq!(
                row["values"].as_array().unwrap().len(),
                headers - 1,
                "{table}"
            );
            assert!(
                labels.insert(row["label"].as_str().unwrap()),
                "labels name one row each: {table}"
            );
        }
    }
}

/// The whole read of one account's pages: the student menu names each
/// page's own address (issue #64: a 修士 account's 科目から and 成績一覧
/// are `PID1_2` and `PID2_2`), each page's reader accepts it, and what it
/// read passes the trust boundary only from the page it names.
#[test]
#[cfg(kyomu_captures)]
fn captured_student_pages_read_through_the_menu_and_the_trust_boundary() {
    let menu = Menu::read(STUDENT_TOP, &capture("student-top.html")).unwrap();
    for page in &PAGES {
        let kind = page.kind;
        let url = menu
            .url(kind)
            .unwrap_or_else(|error| panic!("{kind}: {error}"));
        assert_eq!(expected_kind(Url::parse(&url).unwrap().path()), Some(kind));
        let payload = extract_page(&url, &capture(&format!("{kind}.html")));
        let (stage, _) = parse_payload(&serde_json::to_string(&payload).unwrap())
            .unwrap_or_else(|error| panic!("{kind}: {error}"));
        match kind {
            // Open in every season, whatever the 申告 window.
            "grades" | "requirements" | "profile" => {
                assert_eq!(stage, KyomuStage::Ready, "{kind}")
            }
            _ => assert!(
                matches!(stage, KyomuStage::Ready | KyomuStage::Unavailable),
                "{kind}"
            ),
        }
        if kind == "requirements" {
            assert_requirement_tables(&payload.data);
        }

        // The same read from another host, or labelled as another page, is
        // refused.
        let foreign = KyomuPayload {
            source_url: url.replace(HOST, "kyomu0.gakumu.titech.ac.jp.invalid"),
            ..payload.clone()
        };
        assert!(parse_payload(&serde_json::to_string(&foreign).unwrap()).is_err());
        let relabelled = KyomuPayload {
            kind: if kind == "grades" { "gpa" } else { "grades" }.into(),
            ..payload
        };
        assert!(parse_payload(&serde_json::to_string(&relabelled).unwrap()).is_err());
    }
}
