//! The tests over files kept outside the repository (pages and answers saved
//! from a real account, the published campus map) compile only when their
//! directory or file is given (`capture-gate`); a page missing from a
//! directory fails its test.
fn main() {
    use capture_gate::{dir, file, value};

    dir("kyomu_captures", "KYOMU_CAPTURES");
    file("kyomu_search_en_capture", "KYOMU_SEARCH_EN_HTML");
    dir("moodle_ws_captures", "MOODLE_WS_CAPTURES");
    value("moodle_course_id", "MOODLE_COURSE_ID");
    dir("moodle_envelope_captures", "MOODLE_ENVELOPE_CAPTURES");
    file("campus_map_export", "ISCT_CAMPUS_MAP_EXPORT");
}
