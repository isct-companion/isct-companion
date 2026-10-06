use super::model::*;
use super::parser::{self, DetailPage, ParsedScheduleEntry, SearchPage, SearchRow};
use super::transport::{self, ORIGIN};
use scraper::Html;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use url::Url;

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

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyllabusFilters {
    pub teacher: Option<String>,
    pub name: Option<String>,
    pub code: Option<String>,
    pub groups: Option<Vec<String>>,
    pub department: Option<String>,
}

pub(crate) fn utc_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn valid_scope(value: &str) -> bool {
    let pieces = value.split('/').collect::<Vec<_>>();
    pieces.len() == 2
        && (1..=3).contains(&pieces[0].len())
        && pieces[0].chars().all(|value| value.is_ascii_digit())
        && pieces[1].split('-').count() == 5
        && pieces[1]
            .split('-')
            .all(|part| !part.is_empty() && part.chars().all(|value| value.is_ascii_digit()))
}

pub(crate) fn search_url(query: &str, year: u16, filters: &SyllabusFilters) -> Result<Url, String> {
    let maximum_year = chrono::Datelike::year(&chrono::Utc::now()) + 1;
    if year < 2020 || i32::from(year) > maximum_year {
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
    let scope = filters.department.as_deref().unwrap_or_default().trim();
    if !scope.is_empty() && !valid_scope(scope) {
        return Err("Syllabus department scope is invalid.".into());
    }
    if let Some(groups) = &filters.groups {
        if groups.len() > 6
            || groups
                .iter()
                .any(|group| !["100", "200", "300", "400", "500", "600"].contains(&group.as_str()))
        {
            return Err("Syllabus series filter is invalid.".into());
        }
    }
    let endpoint = if scope.is_empty() {
        format!("{ORIGIN}/search")
    } else {
        format!("{ORIGIN}/courses/{year}/{scope}/search")
    };
    let mut url = Url::parse(&endpoint).map_err(|_| "Syllabus search URL is invalid.")?;
    let mut pairs = url.query_pairs_mut();
    pairs.append_pair("q", query.trim());
    if scope.is_empty() {
        pairs.append_pair("year", &year.to_string());
    }
    for (key, value) in [
        ("teacher", &filters.teacher),
        ("name", &filters.name),
        ("code", &filters.code),
    ] {
        if let Some(value) = value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            pairs.append_pair(key, value);
        }
    }
    if let Some(groups) = &filters.groups {
        let mut seen = HashSet::new();
        for group in groups {
            if seen.insert(group) {
                pairs.append_pair("group", group);
            }
        }
    }
    drop(pairs);
    Ok(url)
}

pub(crate) fn localized_search_url(
    query: &str,
    year: u16,
    filters: &SyllabusFilters,
    language: &str,
) -> Result<Url, String> {
    if !matches!(language, "ja" | "en") {
        return Err("Unsupported syllabus language.".into());
    }
    let mut url = search_url(query, year, filters)?;
    url.query_pairs_mut().append_pair("hl", language);
    Ok(url)
}

fn public_search_url(
    query: &str,
    year: u16,
    filters: &SyllabusFilters,
    language: &str,
) -> Result<Url, String> {
    let mut url = localized_search_url(query, year, filters, language)?;
    if language == "ja" {
        let pairs = url
            .query_pairs()
            .filter(|(key, _)| key != "hl")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        url.set_query(None);
        url.query_pairs_mut().extend_pairs(pairs);
    }
    Ok(url)
}

fn should_skip_search(query: &str, filters: &SyllabusFilters) -> bool {
    let explicit = [
        &filters.teacher,
        &filters.name,
        &filters.code,
        &filters.department,
    ]
    .iter()
    .any(|value| {
        value
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    }) || filters
        .groups
        .as_ref()
        .is_some_and(|groups| !groups.is_empty());
    filters.groups.as_ref().is_some_and(Vec::is_empty)
        || (query.trim().chars().count() < 2 && !explicit)
}

pub async fn search(
    query: &str,
    year: u16,
    limit: usize,
    filters: Option<SyllabusFilters>,
    language: Option<&str>,
) -> Result<SyllabusSearch, String> {
    let filters = filters.unwrap_or_default();
    if should_skip_search(query, &filters) {
        return Ok(SyllabusSearch {
            items: Vec::new(),
            total: 0,
            truncated: false,
        });
    }
    let language = language.unwrap_or("en");
    let page = transport::get(public_search_url(query, year, &filters, language)?).await?;
    let parsed = parser::parse_search(&page.html, year, limit)?;
    Ok(legacy_search(parsed, language))
}

pub async fn search_bilingual(
    query: &str,
    year: u16,
    limit: usize,
    filters: Option<SyllabusFilters>,
) -> Result<CatalogSearchResult, String> {
    let filters = filters.unwrap_or_default();
    if should_skip_search(query, &filters) {
        return Ok(CatalogSearchResult {
            items: Vec::new(),
            returned: 0,
            total: 0,
            truncated: false,
        });
    }
    let (en, ja) = tokio_join_search(query, year, limit, &filters).await?;
    Ok(merge_search(en, ja))
}

/// Lists a department plus the liberal-arts series searched by the planner.
/// Persistence and freshness checks deliberately remain with the database
/// command layer; this function only performs bounded, read-only public GETs.
const LIST_ALL: &str = ".";

pub async fn department_catalog(
    year: u16,
    scope_path: &str,
    quarter: Option<&str>,
) -> Result<CatalogSearchResult, String> {
    if quarter.is_some_and(|value| !["1Q", "2Q", "3Q", "4Q"].contains(&value)) {
        return Err("Syllabus quarter filter is invalid.".into());
    }
    let base_filters = SyllabusFilters {
        department: Some(scope_path.to_string()),
        ..Default::default()
    };
    // This also validates the scope before any request is made.
    search_url("", year, &base_filters)?;
    // The syllabus refuses an empty keyword; every course code contains a dot,
    // so "." lists the whole department.
    let first = search_bilingual(LIST_ALL, year, 100, Some(base_filters.clone())).await?;
    let mut batches = vec![first];
    if batches[0].truncated {
        for group in ["100", "200", "300", "400", "500", "600"] {
            let filters = SyllabusFilters {
                groups: Some(vec![group.into()]),
                ..base_filters.clone()
            };
            batches.push(search_bilingual(LIST_ALL, year, 100, Some(filters)).await?);
        }
    }
    let truncated = batches.iter().any(|batch| batch.truncated);
    let mut unique = BTreeMap::new();
    for item in batches.into_iter().flat_map(|batch| batch.items) {
        if quarter.is_none_or(|quarter| item.quarters.iter().any(|value| value == quarter)) {
            unique.entry(item.id.clone()).or_insert(item);
        }
    }
    let items = unique.into_values().collect::<Vec<_>>();
    Ok(CatalogSearchResult {
        returned: items.len(),
        total: items.len(),
        items,
        truncated,
    })
}

/// Fetches every detail page returned by `department_catalog`. Callers that
/// need incremental UI progress should call `department_catalog` and then
/// `sync_offering` themselves, upserting each completed pair transactionally.
pub async fn sync_department(
    year: u16,
    scope_path: &str,
    quarter: Option<&str>,
) -> Result<DepartmentSyncResult, String> {
    let catalog = department_catalog(year, scope_path, quarter).await?;
    let listed = catalog.items.len();
    let mut offerings = Vec::with_capacity(listed);
    let mut warnings = Vec::new();
    for pair in catalog.items.chunks(2) {
        let mut handles = Vec::with_capacity(pair.len());
        for item in pair {
            let locator = item.syllabus_url.clone();
            handles.push(tauri::async_runtime::spawn(async move {
                sync_offering(&locator).await
            }));
        }
        for (item, handle) in pair.iter().zip(handles) {
            match handle.await {
                Ok(Ok(offering)) => offerings.push(offering),
                Ok(Err(error)) => warnings.push(format!("{}: {error}", item.id)),
                Err(_) => {
                    warnings.push(format!("{}: syllabus worker stopped unexpectedly", item.id))
                }
            }
        }
        if offerings.len() < listed {
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
    }
    let synced = offerings.len();
    Ok(DepartmentSyncResult {
        offerings,
        listed,
        synced,
        truncated: catalog.truncated,
        warnings,
    })
}

async fn tokio_join_search(
    query: &str,
    year: u16,
    limit: usize,
    filters: &SyllabusFilters,
) -> Result<(SearchPage, SearchPage), String> {
    let en_url = public_search_url(query, year, filters, "en")?;
    let ja_url = public_search_url(query, year, filters, "ja")?;
    // Search is intentionally serialized. Detail/department callers can bound
    // their own work at two, while this low-volume public endpoint never bursts.
    let en = transport::get(en_url).await?;
    let ja = transport::get(ja_url).await?;
    Ok((
        parser::parse_search(&en.html, year, limit)?,
        parser::parse_search(&ja.html, year, limit)?,
    ))
}

fn merge_people(ja: &[String], en: &[String]) -> Vec<Localized> {
    let length = ja.len().max(en.len());
    (0..length)
        .map(|index| Localized {
            ja: ja.get(index).cloned(),
            en: en.get(index).cloned(),
        })
        .collect()
}

fn merge_search(en: SearchPage, ja: SearchPage) -> CatalogSearchResult {
    let total = en.total.max(ja.total);
    let truncated = en.truncated || ja.truncated || en.total != ja.total;
    let mut rows = BTreeMap::<String, (Option<SearchRow>, Option<SearchRow>)>::new();
    for row in en.rows {
        let id = row.id.clone();
        rows.entry(id).or_default().0 = Some(row);
    }
    for row in ja.rows {
        let id = row.id.clone();
        rows.entry(id).or_default().1 = Some(row);
    }
    let items = rows
        .into_iter()
        .filter_map(|(id, (en, ja))| {
            let base = en.as_ref().or(ja.as_ref())?;
            let mut warnings = Vec::new();
            if let (Some(en), Some(ja)) = (&en, &ja) {
                if en.code != ja.code || en.year != ja.year || en.quarters != ja.quarters {
                    warnings.push(
                        "Language-neutral search fields disagreed; English was retained.".into(),
                    );
                }
            }
            Some(CatalogSearchItem {
                id,
                year: en.as_ref().map_or(base.year, |row| row.year),
                quarters: en
                    .as_ref()
                    .map_or_else(|| base.quarters.clone(), |row| row.quarters.clone()),
                code: en
                    .as_ref()
                    .map_or_else(|| base.code.clone(), |row| row.code.clone()),
                title: Localized {
                    ja: ja.as_ref().map(|row| row.title.clone()),
                    en: en.as_ref().map(|row| row.title.clone()),
                },
                instructors: merge_people(
                    ja.as_ref().map_or(&[], |row| row.instructors.as_slice()),
                    en.as_ref().map_or(&[], |row| row.instructors.as_slice()),
                ),
                academic_unit: Localized {
                    ja: ja.as_ref().map(|row| row.academic_unit.clone()),
                    en: en.as_ref().map(|row| row.academic_unit.clone()),
                },
                syllabus_url: base.syllabus_url.clone(),
                source_updated: en
                    .as_ref()
                    .and_then(|row| row.source_updated.clone())
                    .or_else(|| ja.as_ref().and_then(|row| row.source_updated.clone())),
                warnings,
            })
        })
        .collect::<Vec<_>>();
    CatalogSearchResult {
        returned: items.len(),
        items,
        total,
        truncated,
    }
}

fn offering_url(locator: &str) -> Result<Url, String> {
    if locator.len() == 9 && locator.chars().all(|value| value.is_ascii_digit()) {
        // The canonical detail path `courses?jwc={id}` redirects to; a query
        // URL would lose the id once the language is set.
        return Url::parse(&format!("{ORIGIN}/courses/-/-/-/{locator}"))
            .map_err(|_| "Syllabus URL is invalid.".to_string());
    }
    let mut url = Url::parse(locator).map_err(|_| "Syllabus URL is invalid.")?;
    url.set_fragment(None);
    url.set_query(None);
    Ok(url)
}

fn localized_detail_url(url: &Url, language: &str) -> Url {
    let mut result = url.clone();
    result.set_query(None);
    if language == "en" {
        result.query_pairs_mut().append_pair("hl", "en");
    }
    result
}

pub async fn sync_offering(locator: &str) -> Result<SyncOfferingResult, String> {
    let initial = offering_url(locator)?;
    let en_page = transport::get(localized_detail_url(&initial, "en")).await?;
    let canonical = en_page.url.clone();
    let ja_page = transport::get(localized_detail_url(&canonical, "ja")).await?;
    let en = parser::parse_detail(&en_page.url, &en_page.html, "en")?;
    let ja = parser::parse_detail(&ja_page.url, &ja_page.html, "ja")?;
    merge_detail(ja, en)
}

/// Course Finder only needs language-neutral list metadata. Fetch one detail
/// page now; opening the syllabus later still performs the full bilingual sync.
pub async fn sync_offering_summary(locator: &str) -> Result<SyncOfferingResult, String> {
    let initial = offering_url(locator)?;
    let page = transport::get(localized_detail_url(&initial, "en")).await?;
    let parsed = parser::parse_detail(&page.url, &page.html, "en")?;
    single_detail(parsed)
}

pub async fn detail(url: &str) -> Result<SyllabusCourse, String> {
    let requested = Url::parse(url).map_err(|_| "Syllabus URL is invalid.")?;
    let language = requested
        .query_pairs()
        .find(|(key, _)| key == "hl")
        .map_or("ja", |(_, value)| if value == "en" { "en" } else { "ja" });
    let page = transport::get(localized_detail_url(&requested, language)).await?;
    parse_detail_html_with_language(&page.url, &page.html, language)
}

fn neutral_schedule(entry: &ParsedScheduleEntry) -> (&ScheduleKind, Option<u8>, Option<[u8; 2]>) {
    (&entry.kind, entry.day, entry.periods)
}

fn merge_schedules(
    ja: &[ParsedScheduleEntry],
    en: &[ParsedScheduleEntry],
) -> Result<Vec<ScheduleEntry>, ()> {
    if ja.len() != en.len()
        || ja
            .iter()
            .zip(en)
            .any(|(ja, en)| neutral_schedule(ja) != neutral_schedule(en))
    {
        return Err(());
    }
    Ok(ja
        .iter()
        .zip(en)
        .map(|(ja, en)| ScheduleEntry {
            kind: en.kind.clone(),
            day: en.day,
            periods: en.periods,
            room: match (&ja.room, &en.room) {
                (None, None) => None,
                (ja, en) => Some(Localized {
                    ja: ja.clone(),
                    en: en.clone(),
                }),
            },
        })
        .collect())
}

fn class_code(code: &str, class_label: Option<&str>) -> Option<String> {
    code.rsplit_once('-')
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            class_label
                .map(|value| {
                    value.trim_matches(|character| matches!(character, '【' | '】' | '[' | ']'))
                })
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn single_detail(page: DetailPage) -> Result<SyncOfferingResult, String> {
    let DetailPage {
        id,
        year,
        title,
        instructors,
        academic_unit,
        code,
        quarters,
        instruction_language,
        format,
        format_raw,
        media,
        class_label,
        credits,
        schedule,
        schedule_raw,
        source_updated,
        unit_code,
        syllabus_url,
        fetched_at,
        content,
        warnings,
        ..
    } = page;
    let schedule_missing = schedule.is_none();
    let schedule = schedule
        .unwrap_or_default()
        .into_iter()
        .map(|entry| ScheduleEntry {
            kind: entry.kind,
            day: entry.day,
            periods: entry.periods,
            room: entry.room.map(|room| Localized::one("en", room)),
        })
        .collect::<Vec<_>>();
    let format_missing = format.is_none();
    let offering = Offering {
        id: id.clone(),
        year,
        quarters,
        code: code.clone(),
        class_code: class_code(&code, class_label.as_deref()),
        unit_code,
        title: Localized::one("en", title),
        instructors: instructors
            .into_iter()
            .map(|name| Localized::one("en", name))
            .collect(),
        instruction_language,
        format: format.unwrap_or_default(),
        media: Localized::one("en", media),
        class_label,
        credits,
        intensive: schedule
            .iter()
            .any(|entry| entry.kind == ScheduleKind::Intensive),
        schedule,
        academic_unit: Localized::one("en", academic_unit),
        syllabus_url,
        source_updated,
        fetched_at: Localized::one("en", fetched_at),
        schedule_raw: schedule_missing.then(|| Localized::one("en", schedule_raw)),
        format_raw: format_missing.then(|| Localized::one("en", format_raw)),
        warnings: warnings.clone(),
    };
    Ok(SyncOfferingResult {
        offering,
        content: vec![OfferingContent {
            offering_id: id,
            lang: "en".into(),
            content,
        }],
        warnings,
    })
}

fn merge_detail(ja: DetailPage, en: DetailPage) -> Result<SyncOfferingResult, String> {
    if ja.id != en.id {
        return Err("Japanese and English syllabus pages identified different offerings.".into());
    }
    let mut warnings = ja.warnings.clone();
    warnings.extend(en.warnings.clone());
    for (field, equal) in [
        ("year", ja.year == en.year),
        ("code", ja.code == en.code),
        ("quarters", ja.quarters == en.quarters),
        ("credits", ja.credits == en.credits),
        (
            "instructionLanguage",
            ja.instruction_language == en.instruction_language,
        ),
    ] {
        if !equal {
            warnings.push(format!(
                "{field} disagreed between languages; English was retained."
            ));
        }
    }
    let schedule = match (&ja.schedule, &en.schedule) {
        (Some(ja), Some(en)) => merge_schedules(ja, en).unwrap_or_else(|()| {
            warnings.push("schedule disagreed between languages; English was retained.".into());
            en.iter()
                .map(|entry| ScheduleEntry {
                    kind: entry.kind.clone(),
                    day: entry.day,
                    periods: entry.periods,
                    room: entry.room.clone().map(|room| Localized::one("en", room)),
                })
                .collect()
        }),
        (_, Some(en)) => en
            .iter()
            .map(|entry| ScheduleEntry {
                kind: entry.kind.clone(),
                day: entry.day,
                periods: entry.periods,
                room: entry.room.clone().map(|room| Localized::one("en", room)),
            })
            .collect(),
        _ => Vec::new(),
    };
    if ja.format != en.format {
        warnings.push("format disagreed between languages; English was retained.".into());
    }
    let schedule_raw = (ja.schedule.is_none() || en.schedule.is_none())
        .then(|| Localized::pair(ja.schedule_raw.clone(), en.schedule_raw.clone()));
    let format_raw = (ja.format.is_none() || en.format.is_none())
        .then(|| Localized::pair(ja.format_raw.clone(), en.format_raw.clone()));
    let content = vec![
        OfferingContent {
            offering_id: en.id.clone(),
            lang: "ja".into(),
            content: ja.content,
        },
        OfferingContent {
            offering_id: en.id.clone(),
            lang: "en".into(),
            content: en.content,
        },
    ];
    let offering = Offering {
        id: en.id,
        year: en.year,
        quarters: en.quarters,
        code: en.code.clone(),
        class_code: class_code(&en.code, en.class_label.as_deref()),
        unit_code: en.unit_code.or(ja.unit_code),
        title: Localized::pair(ja.title, en.title),
        instructors: merge_people(&ja.instructors, &en.instructors),
        instruction_language: en.instruction_language,
        format: en.format.unwrap_or_default(),
        media: Localized::pair(ja.media, en.media),
        class_label: en.class_label.or(ja.class_label),
        credits: en.credits,
        intensive: schedule
            .iter()
            .any(|entry| entry.kind == ScheduleKind::Intensive),
        schedule,
        academic_unit: Localized::pair(ja.academic_unit, en.academic_unit),
        syllabus_url: en.syllabus_url,
        source_updated: en.source_updated.or(ja.source_updated),
        fetched_at: Localized::pair(ja.fetched_at, en.fetched_at),
        schedule_raw,
        format_raw,
        warnings: warnings.clone(),
    };
    Ok(SyncOfferingResult {
        offering,
        content,
        warnings,
    })
}

fn legacy_search(parsed: SearchPage, language: &str) -> SyllabusSearch {
    SyllabusSearch {
        items: parsed
            .rows
            .into_iter()
            .map(|row| SyllabusCourse {
                external_id: row.id,
                year: row.year,
                quarter: row.quarters.first().cloned().unwrap_or_default(),
                quarters: row.quarters,
                code: row.code,
                title: row.title,
                instructor: row.instructors.join(" / "),
                credits: None,
                language: None,
                meetings: Vec::new(),
                url: if language == "en" {
                    format!("{}?hl=en", row.syllabus_url)
                } else {
                    row.syllabus_url
                },
                updated: row.source_updated,
                content: None,
            })
            .collect(),
        total: parsed.total,
        truncated: parsed.truncated,
    }
}

pub(crate) fn parse_search_html(
    html: &str,
    year: u16,
    limit: usize,
) -> Result<SyllabusSearch, String> {
    parser::parse_search_legacy(html, year, limit).map(|page| legacy_search(page, "en"))
}

fn period_times(periods: [u8; 2]) -> (&'static str, &'static str) {
    match periods {
        [1, 2] => ("08:50", "10:30"),
        [3, 4] => ("10:45", "12:25"),
        [5, 6] => ("13:30", "15:10"),
        [7, 8] => ("15:25", "17:05"),
        [9, 10] => ("17:15", "18:55"),
        _ => ("", ""),
    }
}

fn legacy_detail(detail: DetailPage) -> SyllabusCourse {
    let meetings = detail
        .schedule
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| {
            let day = entry.day?;
            let periods = entry.periods?;
            let (start, end) = period_times(periods);
            Some(SyllabusMeeting {
                days: vec![day],
                periods: (periods[0]..=periods[1]).collect(),
                start_time: start.into(),
                end_time: end.into(),
                room: entry.room.clone().unwrap_or_default(),
            })
        })
        .collect();
    SyllabusCourse {
        external_id: detail.id,
        year: detail.year,
        quarter: detail.quarters.first().cloned().unwrap_or_default(),
        quarters: detail.quarters,
        code: detail.code,
        title: detail.title,
        instructor: detail.instructors.join(" / "),
        credits: Some(detail.credits.total()),
        language: Some(detail.instruction_language),
        meetings,
        url: detail.syllabus_url,
        updated: detail.source_updated,
        content: Some(detail.content),
    }
}

fn language_from_url(url: &Url) -> &str {
    if url
        .query_pairs()
        .any(|(key, value)| key == "hl" && value == "en")
    {
        "en"
    } else {
        "ja"
    }
}

pub(crate) fn parse_detail_html(url: &Url, html: &str) -> Result<SyllabusCourse, String> {
    let course = parse_detail_html_with_language(url, html, language_from_url(url))?;
    if course
        .content
        .as_ref()
        .is_some_and(|content| content.sections.is_empty())
    {
        return Err("Legacy syllabus detail was incomplete.".into());
    }
    Ok(course)
}

fn parse_detail_html_with_language(
    url: &Url,
    html: &str,
    language: &str,
) -> Result<SyllabusCourse, String> {
    parser::parse_detail(url, html, language).map(legacy_detail)
}

pub(crate) fn parse_content(
    url: &Url,
    html: &str,
    pairs: &[(String, String)],
    updated: Option<String>,
) -> Result<SyllabusContent, String> {
    let document = Html::parse_document(html);
    parser::parse_content(
        url,
        &document,
        pairs,
        updated,
        language_from_url(url),
        false,
    )
}

pub(crate) fn quarters(value: &str) -> Vec<String> {
    parser::quarters(value)
}

#[cfg(test)]
mod offering_locator_tests {
    #[test]
    fn a_bare_offering_id_uses_the_canonical_detail_path() {
        let url = super::offering_url("202608712").unwrap();
        assert_eq!(
            url.as_str(),
            "https://syllabus.s.isct.ac.jp/courses/-/-/-/202608712"
        );
        // Setting the language must keep the id.
        assert!(super::localized_detail_url(&url, "en")
            .as_str()
            .ends_with("/202608712?hl=en"));
    }

    #[test]
    fn finder_summary_keeps_filter_metadata_without_claiming_bilingual_content() {
        let url = url::Url::parse(
            "https://syllabus.s.isct.ac.jp/courses/2026/4/0-904-342300-0-0/202602440?hl=en",
        )
        .unwrap();
        let page = super::parser::parse_detail(
            &url,
            include_str!("../../../artifacts/syllabus-research/parallel-en-content.html"),
            "en",
        )
        .unwrap();
        let summary = super::single_detail(page).unwrap();
        assert_eq!(summary.offering.id, "202602440");
        assert_eq!(summary.offering.credits.total(), 2.0);
        assert_eq!(summary.offering.schedule.len(), 2);
        assert!(summary.offering.fetched_at.ja.is_none());
        assert_eq!(summary.content.len(), 1);
        assert_eq!(summary.content[0].lang, "en");
    }
}
