use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum SchoolSite {
    #[serde(rename = "science-portal", alias = "portal")]
    SciencePortal,
    #[serde(rename = "science-lms", alias = "lms")]
    ScienceLms,
    #[serde(rename = "tokyo-portal")]
    TokyoPortal,
    #[serde(rename = "tokyo-mail")]
    TokyoMail,
    #[serde(rename = "kyomu")]
    Kyomu,
    #[serde(rename = "library")]
    Library,
    #[serde(rename = "tsubame")]
    Tsubame,
    #[serde(rename = "syllabus")]
    Syllabus,
    #[serde(rename = "study-guide")]
    StudyGuide,
}

impl SchoolSite {
    pub fn for_url(raw: &str) -> Option<Self> {
        match Url::parse(raw).ok()?.host_str()? {
            "lms.s.isct.ac.jp" | "t2schola.titech.ac.jp" => Some(Self::ScienceLms),
            "syllabus.s.isct.ac.jp" => Some(Self::Syllabus),
            "isct.ex-tic.com" | "portal.isct.ac.jp" => Some(Self::SciencePortal),
            "portal.titech.ac.jp" | "portal.nap.gsic.titech.ac.jp" | "rp.nap.gsic.titech.ac.jp" => {
                Some(Self::TokyoPortal)
            }
            "mailv4.m.titech.ac.jp" => Some(Self::TokyoMail),
            "kyomu0.gakumu.titech.ac.jp" => Some(Self::Kyomu),
            "request.libra.titech.ac.jp" => Some(Self::Library),
            "portal.t4.gsic.titech.ac.jp" => Some(Self::Tsubame),
            "www.titech.ac.jp" => Some(Self::StudyGuide),
            _ => None,
        }
    }

    pub const fn webview_label(self) -> &'static str {
        match self {
            Self::SciencePortal => "school-web-science-portal",
            Self::ScienceLms => "school-web-science-lms",
            Self::TokyoPortal => "school-web-tokyo-portal",
            Self::TokyoMail => "school-web-tokyo-mail",
            Self::Kyomu => "school-web-kyomu",
            Self::Library => "school-web-library",
            Self::Tsubame => "school-web-tsubame",
            Self::Syllabus => "school-web-syllabus",
            Self::StudyGuide => "school-web-study-guide",
        }
    }

    pub const fn start_url(self) -> &'static str {
        match self {
            Self::SciencePortal => "https://isct.ex-tic.com/idm/user/portal/",
            Self::ScienceLms => "https://lms.s.isct.ac.jp/2025/",
            Self::TokyoPortal => {
                "https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList?lang=ja"
            }
            Self::TokyoMail => "https://mailv4.m.titech.ac.jp/cgi-bin/loginweb",
            Self::Kyomu => "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx",
            Self::Library => "https://request.libra.titech.ac.jp/portal/user.php?plang=jpn",
            // The Tokyo Tech portal SSO entry; it signs in with the same
            // GetAccess login as 教務Web instead of a mailed one-time code.
            Self::Tsubame => "https://portal.t4.gsic.titech.ac.jp/ptl/ga/sso/",
            Self::Syllabus => "https://syllabus.s.isct.ac.jp/",
            // 学修案内等一覧, where the student site's 学修案内 (理工学系) links;
            // the guide PDFs live under www.titech.ac.jp/guide/.
            Self::StudyGuide => "https://www.titech.ac.jp/student/students/life/resources",
        }
    }
}
