//! Institute of Science Tokyo addresses that more than one part of Core needs.

pub const SYLLABUS_ORIGIN: &str = "https://syllabus.s.isct.ac.jp";

/// The 教務Web course-search page (科目申告 → 開講科目検索).
pub const KYOMU_SEARCH_PATH: &str =
    "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0_1.aspx";

/// The school's シラバス page of one offering, in English when `language` is
/// `en`. The one place a syllabus link is built; shells open it as given.
pub fn syllabus_url(offering_id: &str, language: &str) -> String {
    let url = format!("{SYLLABUS_ORIGIN}/courses/-/-/-/{offering_id}");
    if language == "en" {
        format!("{url}?hl=en")
    } else {
        url
    }
}
