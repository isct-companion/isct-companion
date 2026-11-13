//! 教務Web: the Rust readers for its pages and course search, and the trust
//! boundary every read passes before persistence sees it.
use super::SchoolParseError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

/// A compiled constant pattern, built once. `None` only if the literal is
/// invalid, which callers treat as "no match" instead of panicking.
macro_rules! regex {
    ($pattern:literal) => {{
        static PATTERN: std::sync::LazyLock<Option<regex::Regex>> =
            std::sync::LazyLock::new(|| regex::Regex::new($pattern).ok());
        PATTERN.as_ref()
    }};
}

mod html;
mod pages;
pub mod search;

pub use pages::extract_page;

/// The notice of a 教務Web page that says the site is under maintenance now.
pub fn maintenance_notice(html: &str) -> Option<String> {
    html::maintenance_notice(&scraper::Html::parse_document(html))
}

pub const HOST: &str = "kyomu0.gakumu.titech.ac.jp";

const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
const MAX_RECORDS: usize = 5_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KyomuPayload {
    pub status: String,
    pub source_url: String,
    pub kind: String,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KyomuStage {
    Ready,
    Maintenance,
    Unavailable,
    ReconnectRequired,
}

const SHINKOKU: &str = "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/";
const SEISEKI: &str = "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/";

/// The student top, whose side menu links the pages this account has.
pub const STUDENT_TOP: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/";

/// A 教務Web page the app reads: its kind and every address 教務Web
/// publishes it at. The school gives each programme its own copy of some
/// pages (the 学士 grades are `PID2_0`, the 修士 ones `PID2_2`) and sends an
/// account away from a copy that is not its own, so a read asks the
/// account's student menu which one it has ([`Menu::url`]). The list is
/// closed: no other address is ever fetched, and each one's reader was
/// checked against a real page.
pub struct Page {
    pub kind: &'static str,
    /// Directory and file names, in the order a menu that links more than
    /// one is read in.
    dir: &'static str,
    files: &'static [&'static str],
    /// Linked from the student menu. The profile is not; it is one page
    /// for every programme.
    in_menu: bool,
}

impl Page {
    fn paths(&self) -> impl Iterator<Item = String> + '_ {
        self.files.iter().map(|file| format!("{}{file}", self.dir))
    }
}

/// The six pages, in menu order.
pub const PAGES: [Page; 6] = [
    Page {
        kind: "gpa",
        dir: SEISEKI,
        files: &["PID2_7.aspx"],
        in_menu: true,
    },
    Page {
        kind: "registrations",
        dir: SHINKOKU,
        files: &["PID1_0.aspx"],
        in_menu: true,
    },
    Page {
        kind: "declared",
        dir: SHINKOKU,
        // 科目から: 学士, then 修士 (issue #64).
        files: &["PID1_1.aspx", "PID1_2.aspx"],
        in_menu: true,
    },
    Page {
        kind: "profile",
        dir: "/Titech/Student/%E5%AD%A6%E7%94%9F%E6%83%85%E5%A0%B1/",
        files: &["PID4_2.aspx"],
        in_menu: false,
    },
    Page {
        kind: "grades",
        dir: SEISEKI,
        // 成績一覧: 学士, then 修士 (issue #64).
        files: &["PID2_0.aspx", "PID2_2.aspx"],
        in_menu: true,
    },
    Page {
        kind: "requirements",
        dir: SEISEKI,
        files: &["PID2_6.aspx"],
        in_menu: true,
    },
];

fn page(kind: &str) -> Option<&'static Page> {
    PAGES.iter().find(|page| page.kind == kind)
}

/// The page kind an approved path belongs to (case-insensitive, like the
/// school's IIS).
pub fn expected_kind(path: &str) -> Option<&'static str> {
    PAGES
        .iter()
        .find(|page| {
            page.paths()
                .any(|approved| path.eq_ignore_ascii_case(&approved))
        })
        .map(|page| page.kind)
}

/// The first address of one of the six pages: the one a sync key is named
/// after. What a read fetches is the account's own ([`Menu::url`]).
pub fn page_url(kind: &str) -> Option<String> {
    page(kind)
        .and_then(|page| page.paths().next())
        .map(|path| format!("https://{HOST}{path}"))
}

/// What the student menu (`#ctl00_sideMenu1_pnlStudent`) links to on
/// 教務Web, as paths. Only where the links point is read: their ids and
/// words differ between programmes (成績一覧 is `HyperLink39` for a 学士
/// account, `HyperLink8` for a 修士 one), the addresses of each page do not
/// leave [`PAGES`].
#[derive(Clone, Debug, Default)]
pub struct Menu {
    paths: Vec<String>,
}

impl Menu {
    /// The menu of the page at `url`. A page without one is not a student
    /// page of a signed-in account, or the menu's markup changed.
    pub fn read(url: &str, html: &str) -> Result<Self, SchoolParseError> {
        let base = Url::parse(url).map_err(|_| "the student menu's address is invalid")?;
        let document = scraper::Html::parse_document(html);
        let panel = scraper::Selector::parse("#ctl00_sideMenu1_pnlStudent")
            .map_err(|_| "bad student menu selector")?;
        let links = scraper::Selector::parse("a[href]").map_err(|_| "bad student menu selector")?;
        let panel = document
            .select(&panel)
            .next()
            .ok_or("教務Web's student menu is missing")?;
        let paths = panel
            .select(&links)
            .filter_map(|link| base.join(link.value().attr("href")?.trim()).ok())
            .filter(|url| url.host_str() == Some(HOST))
            .map(|url| url.path().to_string())
            .collect();
        Ok(Self { paths })
    }

    fn links(&self, path: &str) -> bool {
        self.paths
            .iter()
            .any(|linked| linked.eq_ignore_ascii_case(path))
    }

    /// The address of `kind` this account has: the first of its addresses
    /// the menu links. A menu linking none of them is an error that names
    /// the menu's other links in the page's directory, so a programme with
    /// an address of its own shows it in the log instead of being guessed.
    pub fn url(&self, kind: &str) -> Result<String, SchoolParseError> {
        let page = page(kind).ok_or("unknown 教務Web page")?;
        let found = match page.in_menu {
            false => page.paths().next(),
            true => page.paths().find(|path| self.links(path)),
        };
        if let Some(path) = found {
            return Ok(format!("https://{HOST}{path}"));
        }
        let known: Vec<String> = PAGES.iter().flat_map(Page::paths).collect();
        let mut others: Vec<&str> = self
            .paths
            .iter()
            .filter(|path| {
                path.to_ascii_lowercase()
                    .starts_with(&page.dir.to_ascii_lowercase())
                    && !known.iter().any(|known| known.eq_ignore_ascii_case(path))
            })
            .map(String::as_str)
            .collect();
        others.sort_unstable();
        others.dedup();
        others.truncate(8);
        Err(format!(
            "教務Web's student menu links no known address of the {kind} page (other links there: {})",
            if others.is_empty() { "none".into() } else { others.join(", ") }
        )
        .into())
    }
}

/// Validates the bounded DTO produced by [`extract_page`].
///
/// The reader only reads the fetched HTML. Trust starts here: source
/// identity, completeness, and collection limits are checked before
/// persistence sees it.
pub fn parse_payload(raw: &str) -> Result<(KyomuStage, Option<KyomuPayload>), SchoolParseError> {
    if raw.len() > MAX_PAYLOAD_BYTES {
        return Err("kyomu payload exceeds the size limit".into());
    }
    let payload: KyomuPayload =
        serde_json::from_str(raw).map_err(|_| "kyomu payload is not valid JSON")?;
    let url = Url::parse(&payload.source_url).map_err(|_| "kyomu source URL is invalid")?;
    if payload.status == "maintenance" {
        if url.scheme() != "https"
            || url.host_str() != Some(HOST)
            || payload.message.trim().is_empty()
            || payload.message.len() > 8192
        {
            return Err("kyomu maintenance notice is invalid".into());
        }
        return Ok((KyomuStage::Maintenance, Some(payload)));
    }
    if payload.status == "login" {
        let known_login_host = matches!(
            url.host_str(),
            Some(HOST | "portal.titech.ac.jp" | "rp.nap.gsic.titech.ac.jp")
        );
        if url.scheme() == "https" && known_login_host {
            return Ok((KyomuStage::ReconnectRequired, None));
        }
    }
    if url.scheme() != "https"
        || url.host_str() != Some(HOST)
        || expected_kind(url.path()) != Some(payload.kind.as_str())
    {
        return Err("kyomu source page is not approved".into());
    }
    if payload.status == "unavailable" {
        if payload.message.trim().is_empty() {
            return Err("kyomu unavailable result has no reason".into());
        }
        return Ok((KyomuStage::Unavailable, Some(payload)));
    }
    // The extractor names the selector that rejected the page. Keep it: the
    // generic text alone cannot tell a changed layout from a closed window.
    if payload.status == "error" && !payload.message.trim().is_empty() {
        let reason: String = payload.message.trim().chars().take(256).collect();
        return Err(format!("kyomu extraction was incomplete: {reason}").into());
    }
    if payload.status != "ok"
        || !payload.data.is_object()
        || payload.data.get("complete").and_then(Value::as_bool) != Some(true)
    {
        return Err("kyomu extraction was incomplete".into());
    }
    let shape = payload
        .data
        .get("shape")
        .and_then(Value::as_str)
        .ok_or_else(|| SchoolParseError::from("kyomu payload shape is missing"))?;
    let empty = payload
        .data
        .get("empty")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let array = |key: &str| {
        payload
            .data
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| SchoolParseError::from("kyomu payload collection is missing"))
    };
    let count = match payload.kind.as_str() {
        "registrations" if shape == "registration-page" => {
            if !payload.data.get("state").is_some_and(Value::is_object) {
                return Err("kyomu registration state is missing".into());
            }
            array("courses")?.len()
        }
        "profile" if shape == "academic-profile" => {
            for field in ["department", "programme", "admittedOn"] {
                if payload
                    .data
                    .get(field)
                    .and_then(Value::as_str)
                    .is_none_or(|v| v.is_empty() || v.len() > 200)
                {
                    return Err("kyomu academic profile is incomplete".into());
                }
            }
            chrono::NaiveDate::parse_from_str(
                payload.data["admittedOn"].as_str().unwrap_or(""),
                "%Y-%m-%d",
            )
            .map_err(|_| SchoolParseError::from("kyomu admission date is invalid"))?;
            1
        }
        "declared" if shape == "declared-course-tables" => array("courses")?.len(),
        "grades" if matches!(shape, "undergraduate-grouped" | "graduate-flat") => {
            let groups = array("groups")?;
            if shape == "graduate-flat" && !groups.is_empty() {
                let sections = groups
                    .iter()
                    .filter_map(|group| group.get("section").and_then(Value::as_str))
                    .collect::<std::collections::HashSet<_>>();
                if groups.len() != 2
                    || sections.len() != 2
                    || !sections.contains("graduate")
                    || !sections.contains("undergraduate")
                {
                    return Err("kyomu graduate grade sections are incomplete".into());
                }
            }
            let mut count = 0;
            for group in groups {
                if group
                    .get("category")
                    .and_then(Value::as_str)
                    .is_none_or(|value| value.is_empty())
                {
                    return Err("kyomu grade group has no category".into());
                }
                let rows = group
                    .get("rows")
                    .and_then(Value::as_array)
                    .ok_or_else(|| SchoolParseError::from("kyomu grade rows are missing"))?;
                if rows.iter().any(|row| {
                    row.get("title")
                        .and_then(Value::as_str)
                        .is_none_or(|value| value.is_empty())
                }) {
                    return Err("kyomu grade row has no course title".into());
                }
                count += rows.len();
            }
            count
        }
        "requirements" if shape == "requirements-tables" => {
            let tables = array("tables")?;
            let mut count = 0;
            for table in tables {
                let headers = table
                    .get("headers")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        SchoolParseError::from("kyomu requirement headers are missing")
                    })?;
                let rows = table
                    .get("rows")
                    .and_then(Value::as_array)
                    .ok_or_else(|| SchoolParseError::from("kyomu requirement rows are missing"))?;
                if headers.len() < 2
                    || rows.iter().any(|row| {
                        row.get("label")
                            .and_then(Value::as_str)
                            .is_none_or(|value| value.is_empty())
                            || row.get("values").and_then(Value::as_array).is_none()
                    })
                {
                    return Err("kyomu requirement table is incomplete".into());
                }
                count += rows.len();
            }
            count
        }
        "gpa" if shape == "gpa-summary" => {
            let overall = array("overall")?;
            let terms = array("terms")?;
            let categories = array("categories")?;
            if !empty && overall.is_empty() {
                return Err("kyomu GPA summary is missing".into());
            }
            overall.len() + terms.len() + categories.len()
        }
        "registrations" | "declared" | "grades" | "requirements" | "gpa" => {
            return Err("kyomu payload shape is not supported".into())
        }
        _ => return Err("kyomu payload kind is invalid".into()),
    };
    if count == 0 && !empty {
        return Err("kyomu empty result is not explicit".into());
    }
    if count > MAX_RECORDS {
        return Err("kyomu payload exceeds the record limit".into());
    }
    Ok((KyomuStage::Ready, Some(payload)))
}

#[cfg(test)]
#[path = "tests/kyomu.rs"]
mod tests;
