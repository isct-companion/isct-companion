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

    /// The site's entry page (`adpt_isct::urls::entry`): a GET that sends
    /// no form and changes nothing, so it is always safe to open again.
    pub const fn start_url(self) -> &'static str {
        use adpt_isct::urls::entry;
        match self {
            Self::SciencePortal => entry::SCIENCE_PORTAL,
            Self::ScienceLms => entry::SCIENCE_LMS,
            Self::TokyoPortal => entry::TOKYO_PORTAL,
            Self::TokyoMail => entry::TOKYO_MAIL,
            Self::Kyomu => entry::KYOMU,
            Self::Library => entry::LIBRARY,
            Self::Tsubame => entry::TSUBAME,
            Self::Syllabus => entry::SYLLABUS,
            Self::StudyGuide => entry::STUDY_GUIDE,
        }
    }
}
