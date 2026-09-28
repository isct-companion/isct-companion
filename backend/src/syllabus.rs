use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use url::Url;

const ORIGIN: &str = "https://syllabus.s.isct.ac.jp";
const MAX_BODY: usize = 3 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyllabusCourse {
    pub external_id: String,
    pub year: u16,
    pub quarter: String,
    pub quarters: Vec<String>,
    pub code: String,
    pub title: String,
    pub instructor: String,
    pub credits: Option<f32>,
    pub language: Option<String>,
    pub meetings: Vec<SyllabusMeeting>,
    pub url: String,
    pub updated: Option<String>,
    pub content: Option<SyllabusContent>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyllabusContent {
    pub schema_version: u8,
    pub source_url: String,
    pub source_language: String,
    pub fetched_at: String,
    pub source_updated: Option<String>,
    pub metadata: Vec<SyllabusMetadata>,
    pub sections: Vec<SyllabusSection>,
    pub lecture_plan: Vec<SyllabusLecture>,
    pub competencies: Vec<SyllabusCompetency>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SyllabusMetadata {
    pub label: String,
    pub value: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct SyllabusSection {
    pub key: String,
    pub heading: String,
    pub blocks: Vec<SyllabusBlock>,
    pub links: Vec<SyllabusLink>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SyllabusBlock {
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cells: Option<Vec<String>>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SyllabusLink {
    pub label: String,
    pub url: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct SyllabusLecture {
    pub label: String,
    pub topic: String,
    pub objective: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct SyllabusCompetency {
    pub label: String,
    pub selected: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyllabusSearch {
    pub items: Vec<SyllabusCourse>,
    pub total: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyllabusMeeting {
    pub days: Vec<u8>,
    pub periods: Vec<u8>,
    pub start_time: String,
    pub end_time: String,
    pub room: String,
}

async fn get(url: Url) -> Result<String, String> {
    if url.scheme() != "https"
        || url.host_str() != Some("syllabus.s.isct.ac.jp")
        || url.username() != ""
        || url.password().is_some()
    {
        return Err("Syllabus request is outside the public syllabus origin.".into());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|_| "Syllabus client is unavailable.")?;
    let mut response = client
        .get(url)
        .header("Accept", "text/html")
        .send()
        .await
        .map_err(|_| "Official syllabus is temporarily unavailable.")?;
    if !response.status().is_success() {
        return Err(format!(
            "Official syllabus returned HTTP {}.",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_BODY as u64)
    {
        return Err("Syllabus response is too large.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Syllabus response could not be read.")?
    {
        if bytes.len() + chunk.len() > MAX_BODY {
            return Err("Syllabus response is too large.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| "Syllabus response was not valid text.".into())
}

fn text(html: &str) -> String {
    static ACTIVE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?is)<(?:script|style|iframe|object)\b[^>]*>.*?</(?:script|style|iframe|object)\s*>",
        )
        .unwrap()
    });
    let html = ACTIVE.replace_all(html, " ");
    static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]+>").unwrap());
    let stripped = TAGS.replace_all(&html, " ");
    static NUMERIC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"&#(?:x([0-9a-fA-F]+)|([0-9]+));").unwrap());
    let decoded = NUMERIC.replace_all(&stripped, |c: &regex::Captures| {
        let number = c
            .get(1)
            .and_then(|v| u32::from_str_radix(v.as_str(), 16).ok())
            .or_else(|| c.get(2).and_then(|v| v.as_str().parse().ok()));
        number
            .and_then(char::from_u32)
            .map(|v| v.to_string())
            .unwrap_or_else(|| c[0].to_owned())
    });
    decoded
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn rich_text(html: &str) -> String {
    // Preserve meaningful line breaks while emitting only plain strings, never source HTML.
    static BR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").unwrap());
    BR.split(html)
        .map(text)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn element_text(element: ElementRef<'_>) -> String {
    element
        .text()
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
}

fn section_key(heading: &str) -> &'static str {
    let labels = [
        ("overview", "Course overview and goals", "授業の目的"),
        ("aims", "Course description and aims", "到達目標"),
        (
            "practicalExperience",
            "Student learning outcomes",
            "実務経験のある教員",
        ),
        ("keywords", "Keywords", "キーワード"),
        ("competencies", "Competencies", "学生が身につける力"),
        ("classFlow", "Class flow", "授業の進め方"),
        (
            "lecturePlan",
            "Course schedule/Objectives",
            "授業計画・課題",
        ),
        ("studyAdvice", "Study advice", "準備学修"),
        ("textbooks", "Textbook(s)", "教科書"),
        ("references", "Reference books", "参考書、講義資料"),
        ("assessment", "Evaluation methods and criteria", "成績評価"),
        ("relatedCourses", "Related courses", "関連する科目"),
        ("prerequisites", "Prerequisites", "履修の条件"),
        ("contact", "Contact information", "連絡先"),
        ("officeHours", "Office hours", "オフィスアワー"),
        ("other", "Other", "その他"),
    ];
    labels
        .iter()
        .find(|(_, en, ja)| heading.starts_with(en) || heading.starts_with(ja))
        .map(|v| v.0)
        .unwrap_or("otherSection")
}

fn utc_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Gregorian civil date from Unix days; no platform-local timezone is involved.
    let z = (seconds / 86400) as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}

fn parse_content(
    url: &Url,
    html: &str,
    pairs: &[(String, String)],
    updated: Option<String>,
) -> Result<SyllabusContent, String> {
    let title_start = Regex::new(r"(?i)<h1\b")
        .unwrap()
        .find(html)
        .ok_or("Syllabus content is missing.")?
        .start();
    let rest = &html[title_start..];
    let body_end = Regex::new(r"(?i)<aside\b|<footer\b")
        .unwrap()
        .find(rest)
        .map(|v| v.start())
        .unwrap_or(rest.len());
    let body = &rest[..body_end];
    let headings = Regex::new(r"(?is)<h3\b[^>]*>(.*?)</h3\s*>").unwrap();
    let matches = headings.captures_iter(body).collect::<Vec<_>>();
    if matches.is_empty() || matches.len() > 64 {
        return Err("Syllabus content sections could not be verified.".into());
    }
    let blocks_re = Regex::new(r"(?is)<(p|li|h4|tr)\b[^>]*>").unwrap();
    let cells_re = Regex::new(r"(?is)<t[dh]\b[^>]*>(.*?)</t[dh]\s*>").unwrap();
    let links_re =
        Regex::new(r#"(?is)<a\b[^>]*href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a\s*>"#).unwrap();
    let mut sections = Vec::new();
    let mut lecture_plan = Vec::new();
    let mut competencies = Vec::new();
    let mut total_text = 0;
    for (index, capture) in matches.iter().enumerate() {
        let heading = text(&capture[1]);
        let key = section_key(&heading);
        let start = capture.get(0).unwrap().end();
        let end = matches
            .get(index + 1)
            .map(|v| v.get(0).unwrap().start())
            .unwrap_or(body.len());
        let raw = &body[start..end];
        let mut blocks = Vec::new();
        let mut remaining = raw;
        while let Some(block) = blocks_re.captures(remaining) {
            let tag = block[1].to_ascii_lowercase();
            let opening_match = block.get(0).unwrap();
            let opening = opening_match.as_str();
            let after_open = &remaining[opening_match.end()..];
            let closing_re = Regex::new(&format!(r"(?i)</{}\s*>", tag)).unwrap();
            let closing = closing_re
                .find(after_open)
                .ok_or("Syllabus section markup is incomplete.")?;
            let block_body = &after_open[..closing.start()];
            remaining = &after_open[closing.end()..];
            let cells = (tag == "tr").then(|| {
                cells_re
                    .captures_iter(block_body)
                    .map(|v| rich_text(&v[1]))
                    .collect::<Vec<_>>()
            });
            let value = rich_text(block_body);
            if key == "lecturePlan"
                && tag == "tr"
                && !block_body.to_ascii_lowercase().contains("<th")
            {
                if let Some(cells) = &cells {
                    if cells.len() != 3 {
                        return Err("Syllabus lesson plan could not be verified.".into());
                    }
                    lecture_plan.push(SyllabusLecture {
                        label: cells[0].clone(),
                        topic: cells[1].clone(),
                        objective: cells[2].clone(),
                    });
                }
            }
            if key == "competencies" && tag == "li" {
                competencies.push(SyllabusCompetency {
                    label: value.clone(),
                    selected: opening.contains("is-check-list-checked"),
                });
            }
            if !value.is_empty() {
                total_text += value.len();
                blocks.push(SyllabusBlock {
                    kind: match tag.as_str() {
                        "li" => "listItem",
                        "h4" => "subheading",
                        "tr" => "tableRow",
                        _ => "paragraph",
                    }
                    .into(),
                    text: value,
                    cells,
                });
            }
        }
        // Preserve unexpected but textual section layouts rather than silently dropping them.
        if blocks.is_empty() {
            let value = rich_text(raw);
            if !value.is_empty() {
                total_text += value.len();
                blocks.push(SyllabusBlock {
                    kind: "paragraph".into(),
                    text: value,
                    cells: None,
                });
            }
        }
        let links = links_re
            .captures_iter(raw)
            .filter_map(|link| {
                let href = text(&link[1]);
                let resolved = url.join(&href).ok()?;
                if !matches!(resolved.scheme(), "http" | "https")
                    || !resolved.username().is_empty()
                    || resolved.password().is_some()
                {
                    return None;
                }
                Some(SyllabusLink {
                    label: text(&link[2]),
                    url: resolved.to_string(),
                })
            })
            .collect::<Vec<_>>();
        if blocks.len() > 300
            || links.len() > 128
            || heading.len() > 2048
            || total_text > 400000
            || blocks.iter().any(|b| b.text.chars().count() > 40000)
        {
            return Err("Syllabus content exceeds supported limits.".into());
        }
        sections.push(SyllabusSection {
            key: key.into(),
            heading,
            blocks,
            links,
        });
    }
    if !sections
        .iter()
        .any(|s| s.key == "overview" && !s.blocks.is_empty())
        || !sections
            .iter()
            .any(|s| s.key == "aims" && !s.blocks.is_empty())
    {
        return Err("Syllabus content structure could not be verified.".into());
    }
    if pairs.len() > 64
        || pairs.iter().any(|(k, v)| k.len() > 2048 || v.len() > 16384)
        || lecture_plan.len() > 150
    {
        return Err("Syllabus metadata exceeds supported limits.".into());
    }
    let source_language = if sections.iter().any(|s| s.heading.starts_with("授業の目的")) {
        "ja"
    } else {
        "en"
    };
    Ok(SyllabusContent {
        schema_version: 1,
        source_url: url.to_string(),
        source_language: source_language.into(),
        fetched_at: utc_now(),
        source_updated: updated,
        metadata: pairs
            .iter()
            .map(|(k, v)| SyllabusMetadata {
                label: k.clone(),
                value: v.clone(),
            })
            .collect(),
        sections,
        lecture_plan,
        competencies,
    })
}
fn quarters(value: &str) -> Vec<String> {
    let range = Regex::new(r"\b([1-4])-([1-4])Q\b").unwrap();
    let mut values = Vec::new();
    for captures in range.captures_iter(value) {
        let start: u8 = captures[1].parse().unwrap_or(1);
        let end: u8 = captures[2].parse().unwrap_or(start);
        for quarter in start..=end {
            values.push(format!("{quarter}Q"));
        }
    }
    for captures in Regex::new(r"\b([1-4]Q)\b").unwrap().captures_iter(value) {
        let value = captures[1].to_string();
        if !values.contains(&value) {
            values.push(value);
        }
    }
    values
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyllabusFilters {
    pub teacher: Option<String>,
    pub name: Option<String>,
    pub code: Option<String>,
    pub groups: Option<Vec<String>>,
    pub department: Option<String>,
}

fn search_url(query: &str, year: u16, filters: &SyllabusFilters) -> Result<Url, String> {
    if !(2020..=2026).contains(&year) {
        return Err("Syllabus academic year is unsupported.".into());
    }
    for value in [
        Some(query),
        filters.teacher.as_deref(),
        filters.name.as_deref(),
        filters.code.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if value.chars().count() > 200 || value.chars().any(char::is_control) {
            return Err("Syllabus search text is invalid or too long.".into());
        }
    }
    let scope = filters.department.as_deref().unwrap_or("").trim();
    if !scope.is_empty() && year != 2026 {
        return Err(
            "Department options are currently verified for academic year 2026 only.".into(),
        );
    }
    if !scope.is_empty()
        && (scope.len() > 100
            || !Regex::new(r"^[0-9]{1,3}/[0-9]+(?:-[0-9]+){4}$")
                .unwrap()
                .is_match(scope))
    {
        return Err("Syllabus department scope is invalid.".into());
    }
    if let Some(groups) = &filters.groups {
        if groups.len() > 6
            || groups
                .iter()
                .any(|g| !["100", "200", "300", "400", "500", "600"].contains(&g.as_str()))
        {
            return Err("Syllabus series filter is invalid.".into());
        }
    }
    let endpoint = if scope.is_empty() {
        format!("{ORIGIN}/search")
    } else {
        format!("{ORIGIN}/courses/{year}/{scope}/search")
    };
    let mut url = Url::parse(&endpoint).unwrap();
    let mut pairs = url.query_pairs_mut();
    pairs.append_pair("q", query.trim()).append_pair("hl", "en");
    if scope.is_empty() {
        pairs.append_pair("year", &year.to_string());
    }
    for (key, value) in [
        ("teacher", &filters.teacher),
        ("name", &filters.name),
        ("code", &filters.code),
    ] {
        if let Some(value) = value {
            if !value.trim().is_empty() {
                pairs.append_pair(key, value.trim());
            }
        }
    }
    if let Some(groups) = &filters.groups {
        let mut seen = std::collections::HashSet::new();
        for group in groups {
            if seen.insert(group) {
                pairs.append_pair("group", group);
            }
        }
    }
    drop(pairs);
    Ok(url)
}

fn localized_search_url(
    query: &str,
    year: u16,
    filters: &SyllabusFilters,
    language: &str,
) -> Result<Url, String> {
    if !["en", "ja"].contains(&language) {
        return Err("Unsupported syllabus language.".into());
    }
    let mut url = search_url(query, year, filters)?;
    let pairs = url
        .query_pairs()
        .filter(|(key, _)| key != "hl")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    url.query_pairs_mut()
        .extend_pairs(pairs)
        .append_pair("hl", language);
    Ok(url)
}

pub async fn search(
    query: &str,
    year: u16,
    limit: usize,
    filters: Option<SyllabusFilters>,
    language: Option<&str>,
) -> Result<SyllabusSearch, String> {
    let filters = filters.unwrap_or_default();
    let language = language.unwrap_or("en");
    let url = localized_search_url(query, year, &filters, language)?;
    let explicit_filter = [
        &filters.teacher,
        &filters.name,
        &filters.code,
        &filters.department,
    ]
    .iter()
    .any(|value| value.as_ref().is_some_and(|s| !s.trim().is_empty()))
        || filters.groups.as_ref().is_some_and(|g| !g.is_empty());
    if filters.groups.as_ref().is_some_and(Vec::is_empty)
        || (query.trim().chars().count() < 2 && !explicit_filter)
    {
        return Ok(SyllabusSearch {
            items: vec![],
            total: 0,
            truncated: false,
        });
    }
    let html = get(url).await?;
    let mut result = parse_search_html(&html, year, limit)?;
    for item in &mut result.items {
        let mut url = Url::parse(&item.url).map_err(|_| "Invalid syllabus URL")?;
        url.set_query(Some(&format!("hl={language}")));
        item.url = url.to_string();
    }
    Ok(result)
}

fn parse_search_html(html: &str, year: u16, limit: usize) -> Result<SyllabusSearch, String> {
    let document = Html::parse_document(html);
    let rows = Selector::parse("tr").map_err(|_| "Syllabus row selector is invalid.")?;
    let cells = Selector::parse("td").map_err(|_| "Syllabus cell selector is invalid.")?;
    let links = Selector::parse("a[href*='/courses/']")
        .map_err(|_| "Syllabus link selector is invalid.")?;
    let identity = Regex::new(r#"/courses/([0-9]{4})/[^?]+/([0-9]+)"#).unwrap();
    let page_text = text(html);
    let amount = Regex::new(r"([0-9]+)\s+items?\s*/\s*out of\s*([0-9]+)")
        .unwrap()
        .captures(&page_text);
    let total = amount
        .as_ref()
        .and_then(|v| v[2].parse().ok())
        .ok_or("Syllabus result count could not be verified.")?;
    let shown = amount.as_ref().and_then(|v| v[1].parse().ok()).unwrap_or(0);
    let mut result = Vec::new();
    for row in document.select(&rows) {
        let Some(anchor) = row.select(&links).next() else {
            continue;
        };
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Some(found) = identity.captures(href) else {
            continue;
        };
        let columns = row.select(&cells).map(element_text).collect::<Vec<_>>();
        if columns.len() < 5 {
            continue;
        }
        let quarters = quarters(&columns[4]);
        let Some(q) = quarters.first().cloned() else {
            continue;
        };
        let Ok(mut detail_url) = Url::parse(ORIGIN).and_then(|base| base.join(href)) else {
            continue;
        };
        detail_url.set_query(Some("hl=en"));
        result.push(SyllabusCourse {
            external_id: found[2].to_string(),
            year: found[1].parse().unwrap_or(year),
            quarter: q,
            quarters,
            code: columns[0].clone(),
            title: columns[1].clone(),
            instructor: columns[2].clone(),
            credits: None,
            language: None,
            meetings: vec![],
            url: detail_url.to_string(),
            updated: columns.get(5).cloned(),
            content: None,
        });
        if result.len() >= limit.clamp(1, 100) {
            break;
        }
    }
    if total > 0 && result.is_empty() {
        return Err("Syllabus results could not be parsed.".into());
    }
    Ok(SyllabusSearch {
        truncated: shown < total || result.len() < total,
        items: result,
        total,
    })
}

pub async fn detail(url: &str) -> Result<SyllabusCourse, String> {
    let parsed = Url::parse(url).map_err(|_| "Syllabus URL is invalid.")?;
    let html = get(parsed.clone()).await?;
    parse_detail_html(&parsed, &html)
}

fn parse_detail_html(parsed: &Url, html: &str) -> Result<SyllabusCourse, String> {
    if html.len() > MAX_BODY {
        return Err("Syllabus response is too large.".into());
    }
    let path = parsed
        .path_segments()
        .and_then(|segments| segments.last())
        .ok_or("Syllabus course identity is missing.")?;
    let year = parsed
        .path_segments()
        .and_then(|mut segments| segments.nth(1))
        .and_then(|value| value.parse().ok())
        .ok_or("Syllabus year is missing.")?;
    let document = Html::parse_document(html);
    let definition_terms =
        Selector::parse("dt").map_err(|_| "Syllabus field selector is invalid.")?;
    let pairs = document
        .select(&definition_terms)
        .filter_map(|term| {
            let value = term
                .next_siblings()
                .filter_map(ElementRef::wrap)
                .find(|element| element.value().name() == "dd")?;
            Some((element_text(term), element_text(value)))
        })
        .collect::<Vec<_>>();
    let field = |labels: &[&str]| {
        pairs
            .iter()
            .find(|(label, _)| labels.iter().any(|wanted| label.contains(wanted)))
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };
    let heading = Selector::parse("h1").map_err(|_| "Syllabus heading selector is invalid.")?;
    let title = document
        .select(&heading)
        .next()
        .map(element_text)
        .unwrap_or_default();
    let code = field(&["Course Code", "科目コード"]);
    let instructor = field(&["Instructor(s)", "担当教員"]);
    let offered = field(&["Offered quarter", "開講クォーター"]);
    let quarters = quarters(&offered);
    let quarter = quarters
        .first()
        .cloned()
        .ok_or("Syllabus quarter is missing.")?;
    let language = field(&["Language", "使用言語"]);
    let credit_raw = field(&["Number of credits", "単位数"]);
    let credits = Regex::new(r"[0-9]+(?:\.[0-9]+)?")
        .unwrap()
        .find_iter(&credit_raw)
        .filter_map(|v| v.as_str().parse::<f32>().ok())
        .reduce(|a, b| a + b);
    let schedule = field(&["Day of week/Period", "曜日・時限"]);
    let meeting_re = Regex::new(r"(?i)(?:(\d+)-(\d+)\s+(Mon|Tue|Wed|Thu|Fri)|(月|火|水|木|金)\s*(\d+)-(\d+))\s*(?:\(([^)]*)\))?").unwrap();
    let days = |name: &str| match name {
        "Mon" | "月" => 1,
        "Tue" | "火" => 2,
        "Wed" | "水" => 3,
        "Thu" | "木" => 4,
        "Fri" | "金" => 5,
        _ => 0,
    };
    let times = |start: u8, end: u8| match (start, end) {
        (1, 2) => ("08:50", "10:30"),
        (3, 4) => ("10:45", "12:25"),
        (5, 6) => ("13:30", "15:10"),
        (7, 8) => ("15:25", "17:05"),
        (9, 10) => ("17:15", "18:55"),
        _ => ("", ""),
    };
    let meetings = meeting_re
        .captures_iter(&schedule)
        .filter_map(|v| {
            let start = v.get(1).or_else(|| v.get(5))?.as_str().parse::<u8>().ok()?;
            let end = v.get(2).or_else(|| v.get(6))?.as_str().parse::<u8>().ok()?;
            let day = days(v.get(3).or_else(|| v.get(4))?.as_str());
            let (a, b) = times(start, end);
            if day == 0 || a.is_empty() {
                return None;
            }
            Some(SyllabusMeeting {
                days: vec![day],
                periods: (start..=end).collect(),
                start_time: a.into(),
                end_time: b.into(),
                room: v.get(7).map(|x| x.as_str()).unwrap_or("").into(),
            })
        })
        .collect();
    if title.is_empty() || code.is_empty() {
        return Err("Syllabus detail page was incomplete.".into());
    }
    let updated = field(&["Syllabus updated", "シラバス更新日"]);
    let updated = (!updated.is_empty()).then_some(updated);
    let content = parse_content(parsed, html, &pairs, updated.clone())?;
    Ok(SyllabusCourse {
        external_id: path.to_string(),
        year,
        quarter,
        quarters,
        code,
        title,
        instructor,
        credits,
        language: (!language.is_empty()).then_some(language),
        meetings,
        url: parsed.to_string(),
        updated,
        content: Some(content),
    })
}

#[cfg(test)]
mod language_tests {
    use super::*;
    #[test]
    fn language_url_preserves_filters_and_validates_locale() {
        let filters = SyllabusFilters::default();
        let url = localized_search_url("parallel", 2026, &filters, "ja").unwrap();
        assert_eq!(
            url.query_pairs()
                .filter(|(k, _)| k == "hl")
                .collect::<Vec<_>>()
                .len(),
            1
        );
        assert_eq!(url.query_pairs().find(|(k, _)| k == "hl").unwrap().1, "ja");
        assert_eq!(
            url.query_pairs().find(|(k, _)| k == "q").unwrap().1,
            "parallel"
        );
        assert!(localized_search_url("parallel", 2026, &filters, "xx").is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_structured_search_transport() {
        let filters = SyllabusFilters {
            teacher: Some("Kobayashi".into()),
            code: Some("CSC.T273".into()),
            groups: Some(vec!["200".into(), "300".into()]),
            department: Some("4/0-904-342300-0-0".into()),
            ..Default::default()
        };
        let url = search_url("", 2026, &filters).unwrap();
        assert_eq!(url.path(), "/courses/2026/4/0-904-342300-0-0/search");
        assert_eq!(
            url.query_pairs()
                .filter(|(k, _)| k == "group")
                .map(|(_, v)| v.to_string())
                .collect::<Vec<_>>(),
            vec!["200", "300"]
        );
        assert!(url
            .query_pairs()
            .any(|(k, v)| k == "teacher" && v == "Kobayashi"));
        let captured = parse_search_html(
            include_str!("../../artifacts/syllabus-research/combined-search.html"),
            2026,
            100,
        )
        .unwrap();
        assert_eq!(captured.total, 1);
        assert_eq!(captured.items[0].code, "CSC.T273");
        assert!(!captured.truncated);
        let invalid = SyllabusFilters {
            department: Some("../search?x=1".into()),
            ..Default::default()
        };
        assert!(search_url("test", 2026, &invalid).is_err());
        let invalid = SyllabusFilters {
            groups: Some(vec!["999".into()]),
            ..Default::default()
        };
        assert!(search_url("test", 2026, &invalid).is_err());
        let global = search_url("a & b", 2025, &SyllabusFilters::default()).unwrap();
        assert_eq!(global.path(), "/search");
        assert!(global.query_pairs().any(|(k, v)| k == "q" && v == "a & b"));
    }
    #[test]
    fn parses_real_multi_course_examples() {
        let fixtures = [
            (
                "social",
                "7/0-907-0-110100-0/202600907",
                include_str!("../../artifacts/syllabus-research/social-en-content.html"),
            ),
            (
                "parallel",
                "4/0-904-342300-0-0/202602440",
                include_str!("../../artifacts/syllabus-research/parallel-en-content.html"),
            ),
            (
                "procedural",
                "4/0-904-342300-0-0/202602417",
                include_str!("../../artifacts/syllabus-research/procedural-en-content.html"),
            ),
            (
                "logic",
                "4/1-22-442222-0-0/202604060",
                include_str!("../../artifacts/syllabus-research/logic-en-content.html"),
            ),
            (
                "assembly",
                "4/0-904-342300-0-0/202602427",
                include_str!("../../artifacts/syllabus-research/assembly-en-content.html"),
            ),
            (
                "programming-one",
                "4/0-904-342200-0-0/202602384",
                include_str!("../../artifacts/syllabus-research/programming-one-en-content.html"),
            ),
        ];
        let mut result = Vec::new();
        for (name, path, html) in fixtures {
            let url = Url::parse(&format!("{ORIGIN}/courses/2026/{path}?hl=en")).unwrap();
            let course = parse_detail_html(&url, html).unwrap();
            assert!(course
                .content
                .as_ref()
                .is_some_and(|c| !c.sections.is_empty()));
            if ["logic", "assembly", "programming-one"].contains(&name) {
                assert_eq!(course.quarter, "3Q");
            }
            result.push(course);
        }
        if let Ok(path) = std::env::var("STWS_PARSED_EXAMPLES_OUTPUT") {
            std::fs::write(path, serde_json::to_string_pretty(&result).unwrap()).unwrap();
        }
    }
    #[test]
    fn parses_verified_search_fixture() {
        let result = parse_search_html(
            include_str!("../../artifacts/syllabus-research/search-excerpt.html"),
            2026,
            12,
        )
        .unwrap();
        assert_eq!(result.total, 113);
        assert!(result.truncated);
        let course = result
            .items
            .iter()
            .find(|item| item.code == "CSC.T354")
            .unwrap();
        assert_eq!(course.external_id, "202602440");
        assert_eq!(course.quarter, "2Q");
        assert!(course
            .url
            .starts_with("https://syllabus.s.isct.ac.jp/courses/2026/"));
    }
    #[test]
    fn parses_verified_detail_fixture() {
        let url = Url::parse(
            "https://syllabus.s.isct.ac.jp/courses/2026/4/0-904-342300-0-0/202602440?hl=en",
        )
        .unwrap();
        let course = parse_detail_html(
            &url,
            include_str!("../../artifacts/syllabus-research/parallel-en-content.html"),
        )
        .unwrap();
        assert_eq!(course.credits, Some(2.0));
        assert_eq!(course.meetings.len(), 2);
        assert_eq!(course.meetings[0].days, vec![2]);
        assert_eq!(course.meetings[1].days, vec![5]);
        assert_eq!(course.meetings[0].periods, vec![3, 4]);
        assert_eq!(course.meetings[0].room, "W9-323");
    }
    #[test]
    fn preserves_multi_quarter_offerings() {
        assert_eq!(quarters("2026 1-2Q / 3-4Q"), vec!["1Q", "2Q", "3Q", "4Q"]);
    }
    #[test]
    fn parses_complete_bilingual_content_without_fabricating_resources() {
        for (language, html) in [
            (
                "en",
                include_str!("../../artifacts/syllabus-research/social-en-content.html"),
            ),
            (
                "ja",
                include_str!("../../artifacts/syllabus-research/social-ja-content.html"),
            ),
        ] {
            let url = Url::parse(&format!(
                "{ORIGIN}/courses/2026/7/0-907-0-110100-0/202600907?hl={language}"
            ))
            .unwrap();
            let result = parse_detail_html(&url, html).unwrap();
            assert_eq!(result.credits, Some(2.0));
            assert!(result.updated.is_some());
            if language == "en" {
                if let Ok(path) = std::env::var("STWS_PARSED_SYLLABUS_OUTPUT") {
                    std::fs::write(path, serde_json::to_string_pretty(&result).unwrap()).unwrap();
                }
            }
            let content = result.content.unwrap();
            assert_eq!(content.source_language, language);
            assert_eq!(content.lecture_plan.len(), 14);
            assert!(content.sections.iter().any(|s| s.key == "contact"));
            assert!(content.sections.iter().any(|s| s.key == "officeHours"));
            assert!(content
                .sections
                .iter()
                .any(|s| s.key == "practicalExperience"));
            assert!(content.sections.iter().all(|s| s.links.is_empty()));
            assert!(content
                .sections
                .iter()
                .find(|s| s.key == "references")
                .unwrap()
                .blocks[0]
                .text
                .contains("ISBN"));
            assert_eq!(content.fetched_at.len(), 20);
            assert!(content.fetched_at.ends_with('Z'));
        }
    }
    #[test]
    fn preserves_checked_competencies_and_absent_optional_sections() {
        let url = Url::parse(&format!(
            "{ORIGIN}/courses/2026/4/0-904-342300-0-0/202602440?hl=en"
        ))
        .unwrap();
        let result = parse_detail_html(
            &url,
            include_str!("../../artifacts/syllabus-research/parallel-en-content.html"),
        )
        .unwrap();
        let content = result.content.unwrap();
        assert_eq!(content.lecture_plan.len(), 14);
        assert_eq!(content.competencies.len(), 5);
        assert_eq!(
            content.competencies.iter().filter(|c| c.selected).count(),
            2
        );
        assert!(!content.sections.iter().any(|s| s.key == "contact"));
        assert!(content.lecture_plan[0].topic.contains("Introduction"));
        assert!(content
            .sections
            .iter()
            .find(|s| s.key == "references")
            .unwrap()
            .blocks[0]
            .text
            .contains("\n"));
    }
    #[test]
    fn rejects_incomplete_content_instead_of_replacing_good_cache() {
        let url = Url::parse(&format!(
            "{ORIGIN}/courses/2026/4/0-904-342300-0-0/202602440?hl=en"
        ))
        .unwrap();
        assert!(parse_detail_html(
            &url,
            include_str!("../../artifacts/syllabus-research/detail-en-excerpt.html")
        )
        .is_err());
    }
    #[test]
    fn emits_plain_text_and_only_actual_safe_links() {
        let url = Url::parse(&format!("{ORIGIN}/courses/2026/4/a/1?hl=en")).unwrap();
        let html = r#"<h1>Example</h1><h3>Course overview and goals</h3><p>A &#x26; B<br>next<script>evil()</script></p><h3>Course description and aims</h3><p><a href="javascript:evil()">bad</a><a href="https://example.org/book">Book</a></p>"#;
        let content = parse_content(&url, html, &[], None).unwrap();
        assert_eq!(content.sections[0].blocks[0].text, "A & B\nnext");
        assert_eq!(content.sections[1].links.len(), 1);
        assert_eq!(content.sections[1].links[0].url, "https://example.org/book");
    }
    #[test]
    fn parses_optional_live_capture() {
        let Ok(path) = std::env::var("STWS_LIVE_SYLLABUS_HTML") else {
            return;
        };
        let html = std::fs::read_to_string(path).unwrap();
        let result = parse_search_html(&html, 2026, 12).unwrap();
        assert!(!result.items.is_empty());
        assert!(result.total >= result.items.len());
    }
}
