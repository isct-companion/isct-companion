use serde::{Deserialize, Serialize};

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
    #[serde(rename = "syllabus")]
    Syllabus,
}

impl SchoolSite {
    pub const fn webview_label(self) -> &'static str {
        match self {
            Self::SciencePortal => "school-web-science-portal",
            Self::ScienceLms => "school-web-science-lms",
            Self::TokyoPortal => "school-web-tokyo-portal",
            Self::TokyoMail => "school-web-tokyo-mail",
            Self::Kyomu => "school-web-kyomu",
            Self::Library => "school-web-library",
            Self::Syllabus => "school-web-syllabus",
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
            Self::Syllabus => "https://syllabus.s.isct.ac.jp/",
        }
    }
}
