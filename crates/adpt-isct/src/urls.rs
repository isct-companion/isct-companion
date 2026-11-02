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

/// The registrable domains the Science Tokyo account's websites keep their
/// data under (cookies, storage, caches), by the cookie domains measured on
/// Extic, the LMS and the portal.
pub const SCIENCE_DOMAINS: &[&str] = &["isct.ac.jp", "ex-tic.com"];

/// The registrable domain of the Tokyo Tech account's websites (GetAccess,
/// 教務Web, the portal, mail, the library, TSUBAME).
pub const TOKYO_TECH_DOMAINS: &[&str] = &["titech.ac.jp"];

/// Each school site's entry page: a GET that sends no form and changes
/// nothing, so opening it again is always safe. A site opens here when it
/// has no page to show, and a sign-in recovery opens it when the visit kept
/// no safe page of its own.
pub mod entry {
    pub const SCIENCE_PORTAL: &str = "https://isct.ex-tic.com/idm/user/portal/";
    pub const SCIENCE_LMS: &str = "https://lms.s.isct.ac.jp/2025/";
    pub const TOKYO_PORTAL: &str =
        "https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList?lang=ja";
    pub const TOKYO_MAIL: &str = "https://mailv4.m.titech.ac.jp/cgi-bin/loginweb";
    pub const KYOMU: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx";
    pub const LIBRARY: &str = "https://request.libra.titech.ac.jp/portal/user.php?plang=jpn";
    /// The Tokyo Tech portal SSO entry; it signs in with the same GetAccess
    /// login as 教務Web instead of a mailed one-time code.
    pub const TSUBAME: &str = "https://portal.t4.gsic.titech.ac.jp/ptl/ga/sso/";
    pub const SYLLABUS: &str = "https://syllabus.s.isct.ac.jp/";
    /// 学修案内等一覧, where the student site's 学修案内 (理工学系) links;
    /// the guide PDFs live under www.titech.ac.jp/guide/.
    pub const STUDY_GUIDE: &str = "https://www.titech.ac.jp/student/students/life/resources";
}
