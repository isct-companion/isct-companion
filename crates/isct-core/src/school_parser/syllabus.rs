use crate::syllabus::model::*;
use crate::syllabus::transport::{MAX_BODY, ORIGIN};
use crate::syllabus::SyllabusError;
use chrono::NaiveDate;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use std::collections::HashMap;
use std::sync::LazyLock;
use url::Url;

#[derive(Clone, Debug)]
pub(crate) struct SearchRow {
    pub id: String,
    pub year: u16,
    pub quarters: Vec<String>,
    pub code: String,
    pub title: String,
    pub instructors: Vec<String>,
    pub academic_unit: String,
    pub syllabus_url: String,
    pub source_updated: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct SearchPage {
    pub rows: Vec<SearchRow>,
    pub total: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ParsedScheduleEntry {
    pub kind: ScheduleKind,
    pub day: Option<u8>,
    pub periods: Option<[u8; 2]>,
    pub room: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct DetailPage {
    pub id: String,
    pub year: u16,
    pub title: String,
    pub instructors: Vec<String>,
    pub academic_unit: String,
    pub code: String,
    pub quarters: Vec<String>,
    pub instruction_language: String,
    pub format: Option<ClassFormat>,
    pub format_raw: String,
    pub media: String,
    pub class_label: Option<String>,
    pub credits: Credits,
    pub schedule: Option<Vec<ParsedScheduleEntry>>,
    pub schedule_raw: String,
    pub source_updated: Option<String>,
    pub unit_code: Option<String>,
    pub syllabus_url: String,
    pub fetched_at: String,
    pub content: SyllabusContent,
    pub warnings: Vec<String>,
}

fn selector(value: &str) -> Result<Selector, SyllabusError> {
    Selector::parse(value)
        .map_err(|_| SyllabusError::from("Internal syllabus selector is invalid."))
}

type CachedPattern = LazyLock<Result<Regex, regex::Error>>;

fn pattern(value: &'static CachedPattern) -> Result<&'static Regex, SyllabusError> {
    value
        .as_ref()
        .map_err(|_| SyllabusError::from("Internal syllabus pattern is invalid."))
}

fn normalized_lines(value: &str) -> String {
    value
        .split('\0')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn rich_element_text(element: ElementRef<'_>) -> String {
    let mut value = String::new();
    for node in element.descendants() {
        if let Some(descendant) = ElementRef::wrap(node) {
            if descendant.value().name() == "br" {
                value.push('\0');
            }
            continue;
        }
        let scraper::node::Node::Text(text) = node.value() else {
            continue;
        };
        let is_active_content = node
            .ancestors()
            .filter_map(ElementRef::wrap)
            .any(|ancestor| {
                matches!(
                    ancestor.value().name(),
                    "script" | "style" | "iframe" | "object"
                )
            });
        if !is_active_content {
            value.push_str(text);
        }
    }
    normalized_lines(&value)
}

fn element_text(element: ElementRef<'_>) -> String {
    rich_element_text(element).replace('\n', " ")
}

fn class(element: ElementRef<'_>, wanted: &str) -> bool {
    element.value().classes().any(|value| value == wanted)
}

fn metadata_key(label: &str) -> Option<&'static str> {
    match label {
        "Academic unit or major" | "開講元" => Some("academicUnit"),
        "Instructor(s)" | "担当教員" => Some("instructors"),
        "Class Format" | "授業形態" => Some("format"),
        "Media-enhanced courses" | "メディア利用科目" => Some("media"),
        "Day of week/Period (Classrooms)" | "曜日・時限 (講義室)" => Some("schedule"),
        "Class" | "クラス" => Some("class"),
        "Course Code" | "科目コード" => Some("code"),
        "Number of credits" | "単位数" => Some("credits"),
        "Course offered" | "開講時期" => Some("year"),
        "Offered quarter" | "開講クォーター" => Some("quarter"),
        "Syllabus updated" | "シラバス更新日" => Some("updated"),
        "Language" | "使用言語" => Some("language"),
        _ => None,
    }
}

fn section_key(heading: &str, unknown_index: usize) -> String {
    let exact = match heading {
        "Course overview and goals" | "授業の目的（ねらい）、概要" => Some("overview"),
        "Course description and aims" | "到達目標" => Some("aims"),
        "Student learning outcomes" | "実務経験のある教員等による授業科目等" => {
            Some("practicalExperience")
        }
        "Keywords" | "キーワード" => Some("keywords"),
        "Competencies" | "学生が身につける力" => Some("competencies"),
        "Class flow" | "授業の進め方" => Some("classFlow"),
        "Course schedule/Objectives" | "授業計画・課題" => Some("lecturePlan"),
        "Study advice (preparation and review)" | "準備学修(事前学修・復習)等についての指示" => {
            Some("studyAdvice")
        }
        "Textbook(s)" | "教科書" => Some("textbooks"),
        "Reference books, course materials, etc." | "参考書、講義資料等" => {
            Some("references")
        }
        "Evaluation methods and criteria" | "成績評価の方法及び基準" => {
            Some("assessment")
        }
        "Related courses" | "関連する科目" => Some("relatedCourses"),
        "Prerequisites" | "履修の条件・注意事項" => Some("prerequisites"),
        "Office hours" | "オフィスアワー" => Some("officeHours"),
        "Other" | "その他" => Some("other"),
        _ => None,
    };
    exact
        .or_else(|| {
            (heading.starts_with("Contact information (e-mail and phone)")
                || heading.starts_with("連絡先 (メール、電話番号)"))
            .then_some("contact")
        })
        .map(str::to_string)
        .unwrap_or_else(|| format!("section-{unknown_index}"))
}

pub(crate) fn quarters(value: &str) -> Vec<String> {
    static RANGE: CachedPattern = LazyLock::new(|| Regex::new(r"([1-4])\s*[-～~]\s*([1-4])Q"));
    static SINGLE: CachedPattern = LazyLock::new(|| Regex::new(r"([1-4])Q"));
    let (Ok(range), Ok(single)) = (pattern(&RANGE), pattern(&SINGLE)) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for captures in range.captures_iter(value) {
        let start = captures[1].parse::<u8>().unwrap_or(1);
        let end = captures[2].parse::<u8>().unwrap_or(start);
        for quarter in start..=end {
            let value = format!("{quarter}Q");
            if !result.contains(&value) {
                result.push(value);
            }
        }
    }
    for captures in single.captures_iter(value) {
        let quarter = format!("{}Q", &captures[1]);
        if !result.contains(&quarter) {
            result.push(quarter);
        }
    }
    result
}

fn parse_date(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value == "-" {
        return None;
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%b %e, %Y") {
        return Some(date.format("%Y-%m-%d").to_string());
    }
    static JA: CachedPattern =
        LazyLock::new(|| Regex::new(r"^(\d{4})年\s*(\d{1,2})月\s*(\d{1,2})日$"));
    static SLASH: CachedPattern = LazyLock::new(|| Regex::new(r"^(\d{4})/(\d{1,2})/(\d{1,2})$"));
    let ja = pattern(&JA).ok()?;
    let slash = pattern(&SLASH).ok()?;
    let captures = ja.captures(value).or_else(|| slash.captures(value))?;
    NaiveDate::from_ymd_opt(
        captures[1].parse().ok()?,
        captures[2].parse().ok()?,
        captures[3].parse().ok()?,
    )
    .map(|date| date.format("%Y-%m-%d").to_string())
}

fn course_identity(url: &Url) -> Option<(u16, String)> {
    let segments = url.path_segments()?.collect::<Vec<_>>();
    let courses = segments.iter().position(|segment| *segment == "courses")?;
    let id = segments.last()?.to_string();
    if id.len() != 9 || !id.chars().all(|value| value.is_ascii_digit()) {
        return None;
    }
    // `courses/{year}/…/{id}`, or the id-only form `courses/-/-/-/{id}`
    // whose id starts with the academic year.
    let year = segments
        .get(courses + 1)
        .and_then(|segment| segment.parse().ok())
        .or_else(|| id[..4].parse().ok())?;
    Some((year, id))
}

fn canonical_url(url: &Url) -> String {
    let mut canonical = url.clone();
    canonical.set_query(None);
    canonical.set_fragment(None);
    canonical.to_string()
}

pub(crate) fn parse_search(
    html: &str,
    requested_year: u16,
    limit: usize,
) -> Result<SearchPage, SyllabusError> {
    if html.len() > MAX_BODY {
        return Err("Syllabus response is too large.".into());
    }
    let document = Html::parse_document(html);
    let table_selector = selector("table.c-table.is-table-style1")?;
    let row_selector = selector("tbody tr")?;
    let cell_selector = selector("td")?;
    let link_selector = selector("td:nth-child(2) a[href*='/courses/']")?;
    let count_selector = selector("p.c-item-amount__number")?;
    let table = document
        .select(&table_selector)
        .next()
        .ok_or("Syllabus result table could not be verified.")?;
    let count = document
        .select(&count_selector)
        .next()
        .map(element_text)
        .ok_or("Syllabus result count could not be verified.")?;
    static COUNT_EN: CachedPattern =
        LazyLock::new(|| Regex::new(r"(\d+)\s+items?\s*/\s*out of\s*(\d+)"));
    static COUNT_JA: CachedPattern =
        LazyLock::new(|| Regex::new(r"(\d+)\s*件\s*/\s*全\s*(\d+)\s*件"));
    let count_en = pattern(&COUNT_EN)?;
    let count_ja = pattern(&COUNT_JA)?;
    let captures = count_en
        .captures(&count)
        .or_else(|| count_ja.captures(&count))
        .ok_or("Syllabus result count could not be verified.")?;
    let returned = captures[1]
        .parse::<usize>()
        .map_err(|_| "Syllabus result count could not be verified.")?;
    let total = captures[2]
        .parse::<usize>()
        .map_err(|_| "Syllabus result count could not be verified.")?;
    let mut rows = Vec::new();
    for row in table.select(&row_selector) {
        let cells = row.select(&cell_selector).collect::<Vec<_>>();
        if cells.len() != 6 {
            continue;
        }
        let Some(anchor) = row.select(&link_selector).next() else {
            continue;
        };
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Ok(detail_url) = Url::parse(ORIGIN).and_then(|origin| origin.join(href)) else {
            continue;
        };
        let Some((year, id)) = course_identity(&detail_url) else {
            continue;
        };
        if year != requested_year {
            continue;
        }
        let offered = element_text(cells[4]);
        let parsed_quarters = quarters(&offered);
        if parsed_quarters.is_empty() {
            continue;
        }
        rows.push(SearchRow {
            id,
            year,
            quarters: parsed_quarters,
            code: element_text(cells[0]),
            title: element_text(anchor),
            instructors: element_text(cells[2])
                .split(" / ")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect(),
            academic_unit: element_text(cells[3]),
            syllabus_url: canonical_url(&detail_url),
            source_updated: parse_date(&element_text(cells[5])),
        });
        if rows.len() >= limit.clamp(1, 100) {
            break;
        }
    }
    if total > 0 && rows.is_empty() {
        return Err("Syllabus results could not be parsed.".into());
    }
    Ok(SearchPage {
        truncated: returned < total || rows.len() < total,
        rows,
        total,
    })
}

fn split_trailing_room(part: &str) -> (&str, Option<String>) {
    let trimmed = part.trim();
    if !trimmed.ends_with(')') {
        return (trimmed, None);
    }
    let mut depth = 0usize;
    for (index, character) in trimmed.char_indices().rev() {
        match character {
            ')' => depth += 1,
            '(' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return (
                        trimmed[..index].trim(),
                        Some(trimmed[index + 1..trimmed.len() - 1].trim().to_string()),
                    );
                }
            }
            _ => {}
        }
    }
    (trimmed, None)
}

fn parse_schedule(value: &str) -> Result<Vec<ParsedScheduleEntry>, SyllabusError> {
    let value = value.trim();
    if value == "-" {
        return Ok(Vec::new());
    }
    static EN_WEEKLY: CachedPattern =
        LazyLock::new(|| Regex::new(r"^(\d{1,2})-(\d{1,2})\s+(Mon|Tue|Wed|Thu|Fri|Sat|Sun)$"));
    static JA_WEEKLY: CachedPattern =
        LazyLock::new(|| Regex::new(r"^(月|火|水|木|金|土|日)\s*(\d{1,2})-(\d{1,2})$"));
    let en_weekly = pattern(&EN_WEEKLY)?;
    let ja_weekly = pattern(&JA_WEEKLY)?;
    let day = |value: &str| match value {
        "Mon" | "月" => Some(1),
        "Tue" | "火" => Some(2),
        "Wed" | "水" => Some(3),
        "Thu" | "木" => Some(4),
        "Fri" | "金" => Some(5),
        "Sat" | "土" => Some(6),
        "Sun" | "日" => Some(7),
        _ => None,
    };
    let mut result = Vec::new();
    for raw_part in value.split(" / ") {
        let (part, room) = split_trailing_room(raw_part);
        if let Some(captures) = en_weekly.captures(part) {
            let start = captures[1].parse::<u8>().map_err(|_| "invalid period")?;
            let end = captures[2].parse::<u8>().map_err(|_| "invalid period")?;
            if crate::schedule::validate_period_range([start, end]).is_err() {
                return Err(format!("Unsupported syllabus schedule: {raw_part}").into());
            }
            result.push(ParsedScheduleEntry {
                kind: ScheduleKind::Weekly,
                day: day(&captures[3]),
                periods: Some([start, end]),
                room,
            });
            continue;
        }
        if let Some(captures) = ja_weekly.captures(part) {
            let start = captures[2].parse::<u8>().map_err(|_| "invalid period")?;
            let end = captures[3].parse::<u8>().map_err(|_| "invalid period")?;
            if crate::schedule::validate_period_range([start, end]).is_err() {
                return Err(format!("Unsupported syllabus schedule: {raw_part}").into());
            }
            result.push(ParsedScheduleEntry {
                kind: ScheduleKind::Weekly,
                day: day(&captures[1]),
                periods: Some([start, end]),
                room,
            });
            continue;
        }
        let kind = match part {
            "Intensive" | "集中講義等" => ScheduleKind::Intensive,
            "Seminar" | "特課PJ" => ScheduleKind::Seminar,
            "Internship" | "インターンシップ" => ScheduleKind::Internship,
            "On-demand" | "オンデマンド" => ScheduleKind::OnDemand,
            "To be announced" | "未定" => ScheduleKind::Tba,
            "Only Registration" | "申告専用" => ScheduleKind::RegistrationOnly,
            _ => return Err(format!("Unsupported syllabus schedule: {raw_part}").into()),
        };
        result.push(ParsedScheduleEntry {
            kind,
            day: None,
            periods: None,
            room,
        });
    }
    Ok(result)
}

fn parse_format(value: &str) -> Result<ClassFormat, SyllabusError> {
    let value = value.trim();
    if value == "-" || value.is_empty() {
        return Ok(ClassFormat::default());
    }
    let (kinds, delivery) = split_trailing_room(value);
    let kinds = kinds
        .split('/')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| match value {
            "Lecture" | "講義" => Ok(FormatKind::Lecture),
            "Exercise" | "演習" => Ok(FormatKind::Exercise),
            "Experiment" | "実験等" => Ok(FormatKind::Experiment),
            _ => Err(format!("Unsupported class format: {value}")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let delivery = delivery
        .map(|value| match value.as_str() {
            "Face-to-face" | "対面型" => Ok(DeliveryKind::FaceToFace),
            "Livestream" | "ライブ型" => Ok(DeliveryKind::Livestream),
            "Blended" | "ブレンド型" => Ok(DeliveryKind::Blended),
            "Others" | "その他" => Ok(DeliveryKind::Other),
            _ => Err(format!("Unsupported class delivery: {value}")),
        })
        .transpose()?;
    if kinds.is_empty() {
        return Err(format!("Unsupported class format: {value}").into());
    }
    Ok(ClassFormat { kinds, delivery })
}

fn parse_credits(document: &Html) -> Result<Credits, SyllabusError> {
    let values = document
        .select(&selector("dd.is-dl-number-of-credits > span")?)
        .map(element_text)
        .map(|value| {
            value
                .parse::<f32>()
                .map_err(|_| "Syllabus credits were invalid.")
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != 3
        || values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err("Syllabus credits could not be verified.".into());
    }
    Ok(Credits {
        lecture: values[0],
        exercise: values[1],
        lab: values[2],
    })
}

fn safe_link(base: &Url, href: &str) -> Option<String> {
    let url = base.join(href).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

fn section_elements<'a>(heading: ElementRef<'a>) -> Vec<ElementRef<'a>> {
    let mut result = Vec::new();
    let mut sibling = heading.next_sibling();
    while let Some(node) = sibling {
        sibling = node.next_sibling();
        let Some(element) = ElementRef::wrap(node) else {
            continue;
        };
        if element.value().name() == "h2"
            || (element.value().name() == "h3" && class(element, "c-h3"))
        {
            break;
        }
        result.push(element);
        result.extend(element.descendent_elements().skip(1));
    }
    result
}

pub(crate) fn parse_content(
    url: &Url,
    document: &Html,
    metadata: &[(String, String)],
    updated: Option<String>,
    language: &str,
) -> Result<SyllabusContent, SyllabusError> {
    let heading_selector = selector("h3.c-h3")?;
    let table_cell_selector = selector("th, td")?;
    let headings = document.select(&heading_selector).collect::<Vec<_>>();
    if headings.len() > 64 {
        return Err("Syllabus content sections exceed supported limits.".into());
    }
    let mut sections = Vec::new();
    let mut total_text = 0usize;
    for (index, heading) in headings.into_iter().enumerate() {
        let heading_text = element_text(heading);
        let key = section_key(&heading_text, index + 1);
        let elements = section_elements(heading);
        let mut blocks = Vec::new();
        let mut links = Vec::new();
        for element in elements {
            let name = element.value().name();
            let is_list_item = name == "li"
                && element
                    .ancestors()
                    .filter_map(ElementRef::wrap)
                    .any(|ancestor| {
                        class(ancestor, "c-ul")
                            || class(ancestor, "c-summary-group")
                            || class(ancestor, "c-check-list")
                    });
            let kind = if name == "p" && class(element, "c-p") {
                Some("paragraph")
            } else if is_list_item {
                Some("listItem")
            } else if name == "h4" {
                Some("subheading")
            } else if name == "tr" {
                Some("tableRow")
            } else {
                None
            };
            if let Some(kind) = kind {
                let cells = (name == "tr").then(|| {
                    element
                        .select(&table_cell_selector)
                        .map(rich_element_text)
                        .collect::<Vec<_>>()
                });
                let text = rich_element_text(element);
                if !text.is_empty() {
                    total_text += text.chars().count();
                    blocks.push(SyllabusBlock {
                        kind: kind.into(),
                        text,
                        cells,
                    });
                }
            }
            if name == "a" {
                if let Some(url) = element
                    .value()
                    .attr("href")
                    .and_then(|href| safe_link(url, href))
                {
                    links.push(SyllabusLink {
                        label: element_text(element),
                        url,
                    });
                }
            }
        }
        if blocks.len() > 300 || links.len() > 128 || total_text > 400_000 {
            return Err("Syllabus content exceeds supported limits.".into());
        }
        sections.push(SyllabusSection {
            key,
            heading: heading_text,
            blocks,
            links,
        });
    }

    let lecture_cell_selector = selector("td")?;
    let lecture_plan = document
        .select(&selector("table#lecture_plans tbody tr")?)
        .filter_map(|row| {
            let cells = row
                .select(&lecture_cell_selector)
                .map(rich_element_text)
                .collect::<Vec<_>>();
            (cells.len() == 3).then(|| SyllabusLecture {
                label: cells[0].clone(),
                topic: cells[1].clone(),
                objective: cells[2].clone(),
            })
        })
        .collect::<Vec<_>>();
    if lecture_plan.len() > 150 {
        return Err("Syllabus lesson plan exceeds supported limits.".into());
    }
    let competencies = document
        .select(&selector("ul.c-check-list > li")?)
        .map(|item| SyllabusCompetency {
            label: rich_element_text(item),
            selected: class(item, "is-check-list-checked"),
        })
        .collect::<Vec<_>>();
    Ok(SyllabusContent {
        schema_version: 1,
        source_url: if language == "en" {
            format!("{}?hl=en", canonical_url(url))
        } else {
            canonical_url(url)
        },
        source_language: language.into(),
        fetched_at: crate::syllabus::utc_now(),
        source_updated: updated,
        metadata: metadata
            .iter()
            .map(|(label, value)| SyllabusMetadata {
                label: label.clone(),
                value: value.clone(),
            })
            .collect(),
        sections,
        lecture_plan,
        competencies,
    })
}

pub(crate) fn parse_detail(
    url: &Url,
    html: &str,
    language: &str,
) -> Result<DetailPage, SyllabusError> {
    if html.len() > MAX_BODY {
        return Err("Syllabus response is too large.".into());
    }
    if !matches!(language, "ja" | "en") {
        return Err("Unsupported syllabus language.".into());
    }
    let (url_year, id) = course_identity(url).ok_or("Syllabus course identity is missing.")?;
    let document = Html::parse_document(html);
    let title = document
        .select(&selector("h1.c-h1")?)
        .next()
        .map(element_text)
        .filter(|value| !value.is_empty())
        .ok_or("Syllabus detail heading could not be verified.")?;
    let item_selector = selector("dl.c-dl-2col > div.c-dl-2col__item")?;
    let dt_selector = selector("dt")?;
    let dd_selector = selector("dd")?;
    let mut metadata = Vec::new();
    let mut fields = HashMap::new();
    for item in document.select(&item_selector) {
        let Some(term) = item.select(&dt_selector).next() else {
            continue;
        };
        let Some(definition) = item.select(&dd_selector).next() else {
            continue;
        };
        let label = element_text(term);
        let value = rich_element_text(definition);
        if let Some(key) = metadata_key(&label) {
            if fields.insert(key, value.clone()).is_some() {
                return Err(format!("Syllabus metadata field {key} was duplicated.").into());
            }
        }
        metadata.push((label, value));
    }
    const REQUIRED: [&str; 12] = [
        "academicUnit",
        "instructors",
        "format",
        "media",
        "schedule",
        "class",
        "code",
        "credits",
        "year",
        "quarter",
        "updated",
        "language",
    ];
    if REQUIRED.iter().any(|key| !fields.contains_key(key)) {
        return Err("Syllabus detail metadata could not be verified.".into());
    }
    let raw_year = fields["year"].trim();
    let year_value = raw_year
        .strip_suffix("年度")
        .or_else(|| raw_year.strip_suffix('年'))
        .unwrap_or(raw_year);
    let year = year_value
        .parse::<u16>()
        .map_err(|_| "Syllabus offered year was invalid.")?;
    if year != url_year {
        return Err("Syllabus offered year did not match its URL.".into());
    }
    let parsed_quarters = quarters(&fields["quarter"]);
    if parsed_quarters.is_empty() {
        return Err("Syllabus quarter could not be verified.".into());
    }
    let instruction_language = match fields["language"].as_str() {
        "Japanese" | "日本語" => "ja",
        "English" | "英語" => "en",
        _ => return Err("Syllabus instruction language was unsupported.".into()),
    }
    .to_string();
    let schedule = parse_schedule(&fields["schedule"]);
    let format = parse_format(&fields["format"]);
    let mut warnings = Vec::new();
    if let Err(error) = &schedule {
        warnings.push(error.to_string());
    }
    if let Err(error) = &format {
        warnings.push(error.to_string());
    }
    let source_updated = parse_date(&fields["updated"]);
    if fields["updated"].trim() != "-" && source_updated.is_none() {
        warnings.push("Syllabus updated date was not recognized.".into());
    }
    let class_label = (fields["class"].trim() != "-").then(|| fields["class"].trim().to_string());
    let path_segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();
    let unit_code = path_segments
        .iter()
        .flat_map(|segment| segment.split('-'))
        .find(|part| part.len() == 6 && part.chars().all(|value| value.is_ascii_digit()))
        .map(str::to_string);
    let fetched_at = crate::syllabus::utc_now();
    let content = parse_content(url, &document, &metadata, source_updated.clone(), language)?;
    Ok(DetailPage {
        id,
        year,
        title,
        instructors: fields["instructors"]
            .split(" / ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        academic_unit: fields["academicUnit"].clone(),
        code: fields["code"].clone(),
        quarters: parsed_quarters,
        instruction_language,
        format: format.ok(),
        format_raw: fields["format"].clone(),
        media: fields["media"].clone(),
        class_label,
        credits: parse_credits(&document)?,
        schedule: schedule.ok(),
        schedule_raw: fields["schedule"].clone(),
        source_updated,
        unit_code,
        syllabus_url: canonical_url(url),
        fetched_at,
        content,
        warnings,
    })
}

#[cfg(test)]
mod course_identity_tests {
    use super::course_identity;
    use url::Url;

    #[test]
    fn identity_comes_from_the_path_or_the_id_prefix() {
        let full =
            Url::parse("https://syllabus.s.isct.ac.jp/courses/2026/4/0-904-342300-0-0/202602421")
                .unwrap();
        assert_eq!(course_identity(&full), Some((2026, "202602421".into())));
        let short = Url::parse("https://syllabus.s.isct.ac.jp/courses/-/-/-/202608712").unwrap();
        assert_eq!(course_identity(&short), Some((2026, "202608712".into())));
        let bad = Url::parse("https://syllabus.s.isct.ac.jp/courses/-/-/-/abc").unwrap();
        assert_eq!(course_identity(&bad), None);
    }
}
