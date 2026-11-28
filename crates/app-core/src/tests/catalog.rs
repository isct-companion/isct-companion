//! The stored catalog over the official search's real grid: the committed
//! Japanese capture (`tests/fixtures/kyomu/course-search.html`, provenance
//! beside it), and for English names an English capture of the same search
//! that the `kyomu_search_en_capture` test reads from the file
//! `KYOMU_SEARCH_EN_HTML` names, compiled only when that file exists
//! (`build.rs`): switch 教務Web to English (the header's English link), open
//! the URL `content-provenance.json` names for `course-search.html`, and save
//! the result grid's outerHTML as that fixture was saved. It is public
//! catalogue text, so it can be committed beside the Japanese one with its
//! own provenance entry, after which the test reads it with `include_str!`
//! and runs on every commit.
//!
//! `KYOMU_SEARCH_EN_HTML=/path/to/grid.html cargo test -p app-core --lib catalog::tests`
use super::*;
#[cfg(kyomu_search_en_capture)]
use adpt_isct::kyomu::search::read_english;
use adpt_isct::kyomu::search::{read_search, SearchRead};

const SEARCH: &str = include_str!("../../tests/fixtures/kyomu/course-search.html");
const FETCHED_AT: &str = "2026-10-16T00:00:00Z";

fn rows(read: SearchRead) -> Vec<serde_json::Value> {
    let SearchRead::Results(results) = read else {
        panic!("the captured grid was not read as results");
    };
    results.courses
}

fn temporary_store() -> (data::DataStore, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("isct-source-{}", uuid::Uuid::new_v4()));
    let store = data::DataStore::initialize_dir(root.clone()).unwrap().0;
    (store, root)
}

/// A department read from the real grid is stored once per offering under
/// its year and department, and the same page read again changes nothing.
/// Each stored row plans as a 教務 course with one meeting per weekly slot.
#[test]
fn the_captured_search_grid_stores_once_and_plans_each_course() {
    let japanese = rows(read_search(SEARCH, true, FETCHED_AT).unwrap());
    let (store, root) = temporary_store();
    assert_eq!(
        store.catalog_upsert_offerings(&japanese).unwrap(),
        japanese.len()
    );
    assert_eq!(store.catalog_upsert_offerings(&japanese).unwrap(), 0);
    let stored = offerings(&store, 2026).unwrap();
    assert_eq!(stored.len(), japanese.len());
    assert!(offerings(&store, 2025).unwrap().is_empty());
    for row in &stored {
        assert!(row["unitCode"].is_string(), "{row}");
        let term = format!("2026-{}", row["quarters"][0].as_str().unwrap());
        let planned = crate::workspace::planned_catalog_course(
            row,
            "planned",
            &term,
            crate::locale::ContentLanguage::Ja,
            &crate::schedule::default_teaching_periods(),
        )
        .unwrap();
        assert_eq!(planned["source"]["provider"], "kyomu");
        assert_eq!(
            planned["meetings"].as_array().unwrap().len(),
            row["schedule"].as_array().unwrap().len(),
            "{row}"
        );
    }
    // Windows cannot delete a folder with files still open.
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

/// The English page adds English titles and instructor names to the rows the
/// Japanese page stored, and only to those; reading the Japanese page again
/// keeps them.
#[test]
#[cfg(kyomu_search_en_capture)]
fn english_names_from_the_english_page_survive_japanese_rereads() {
    let path = env!("KYOMU_SEARCH_EN_HTML");
    let html = std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{path}: {error}"));
    let english = rows(read_english(&html, true, FETCHED_AT).unwrap());
    let japanese = rows(read_search(SEARCH, true, FETCHED_AT).unwrap());
    let (store, root) = temporary_store();
    let merge_english = |rows: &[serde_json::Value]| {
        store
            .write(None, |transaction| {
                data::DataStore::catalog_merge_english_in(transaction, rows)
            })
            .unwrap()
    };
    // English before the Japanese row exists adds nothing.
    assert_eq!(merge_english(english.as_slice()), 0);
    store.catalog_upsert_offerings(&japanese).unwrap();
    assert_eq!(merge_english(english.as_slice()), japanese.len());
    store.catalog_upsert_offerings(&japanese).unwrap();
    for row in &english {
        let id = row["id"].as_str().unwrap();
        let merged = store.catalog_offering(id).unwrap().unwrap();
        assert_eq!(merged["title"]["en"], row["title"]["en"], "{id}");
        assert!(!merged["title"]["ja"].as_str().unwrap().is_empty(), "{id}");
        let names = merged["instructors"].as_array().unwrap();
        assert!(names.iter().all(|name| name["en"].is_string()), "{id}");
    }
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
