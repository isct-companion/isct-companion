//! The course finder's product grammar, shared by all shells.
//! Its pills and syllabus fan-out rules are application-specific; regex and
//! Unicode normalization handle the parsing primitives.
mod contracts;
mod matching;

pub(crate) use catalog::SyllabusDepartment;
pub use catalog::{department_name_ja, normalize_offering, FinderCatalog, FinderDepartment};
pub use conditions::{condition, factors_of, live_text, Condition, Factor, Factors};
pub use contracts::*;
pub use matching::{run, suggest, FinderMatch, IndexedCourse};
pub use personal::{clashes, clashing_courses, total_credits, BusySlot, FinderPersonal};

use conditions::normalized;

use crate::locale::ContentLanguage;
use crate::workspace_changes::WorkspaceChange;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex, PoisonError},
};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::schedule::{array, text_or_empty as text};
pub use crate::study_profile::catalog_course_eligibility;
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FinderContext {
    pub current_quarter: u8,
    #[serde(default)]
    pub recommended: BTreeMap<String, Recommendation>,
    #[serde(default)]
    pub departments: Vec<FinderDepartment>,
    #[serde(default)]
    pub busy: Vec<BusySlot>,
    #[serde(default)]
    pub official_options: Vec<OfficialFilterOption>,
}
pub use adpt_isct::kyomu::search::OfficialFilterOption;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct Recommendation {
    pub group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<String>,
}
mod catalog;
mod personal;

mod conditions;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FinderRequest {
    pub year: u16,
    pub term: String,
    /// Explicitly include courses that cannot be newly added to this term.
    #[serde(default)]
    pub include_ineligible: bool,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub action: Option<FinderAction>,
    /// The first matching row to return.
    #[serde(default)]
    pub offset: u32,
    /// How many matching rows to return from `offset`.
    #[serde(default = "default_page_size")]
    pub limit: u32,
}
fn default_page_size() -> u32 {
    100
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FinderAction {
    Commit { key: String },
    ToggleTime { key: String, append: bool },
    Paste { text: String },
}
/// One page of a search: the conditions as Core read them, what the typed
/// word offers, how many rows match and the rows of this page, and what the
/// timetable highlights. `catalog_version` and `content_revision` name the
/// data it was read from.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FinderPage {
    pub context: FinderContext,
    pub factors: Vec<Factor>,
    pub duplicates: usize,
    pub keys: Vec<String>,
    pub live: String,
    pub suggestions: Vec<Suggestion>,
    /// Matching rows in all.
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<FinderRowView>,
    /// Stored rows of the year's catalog.
    pub stored: usize,
    /// Stored rows this study profile cannot newly plan.
    pub ineligible: usize,
    pub active_slots: Vec<String>,
    /// Codes on the term's timetable.
    pub placed_codes: Vec<String>,
    pub commit_key: Option<String>,
    pub paste_handled: bool,
    pub pasted: Vec<FinderRowView>,
    pub missing_codes: Vec<String>,
    pub ineligible_codes: Vec<String>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub catalog_version: u64,
    pub content_revision: String,
    /// The changes the page reads besides the catalog (`FINDER_READS`); a
    /// change of the shown year's catalog arrives as its own event.
    pub reads: Vec<WorkspaceChange>,
    /// `DataStore::commit_seq` read before the page's data.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub seq: u64,
}

/// What a finder page reads besides the catalog: placed and registered
/// courses, earned credits, terms, plans, busy events and settings.
pub const FINDER_READS: &[WorkspaceChange] = &[
    WorkspaceChange::Courses,
    WorkspaceChange::Registrations,
    WorkspaceChange::Grades,
    WorkspaceChange::Finder,
    WorkspaceChange::Calendar,
    WorkspaceChange::Plans,
    WorkspaceChange::Events,
    WorkspaceChange::Settings,
];
#[derive(Debug, thiserror::Error)]
pub enum FinderError {
    #[error("{0}")]
    InvalidInput(String),
    #[error(transparent)]
    Schedule(#[from] crate::schedule::ScheduleError),
    #[error(transparent)]
    Data(#[from] crate::data::DataError),
    #[error("Course finder data is invalid: {0}")]
    Compatibility(#[from] serde_json::Error),
}

/// Core's finder caches. Typing reruns only the filter and suggestions: the
/// prepared catalog is kept per year, catalog version and syllabus language,
/// and the personal context per catalog, workspace content and term.
///
/// Queries run concurrently with each other and with commits. Each entry is
/// built while its lock is held, so queries that miss together build it
/// once and the later ones reuse it. The two locks are never held together,
/// and a build reads only pooled store connections, which never wait.
#[derive(Default)]
pub struct FinderCache {
    catalog: Mutex<Option<CatalogEntry>>,
    personal: Mutex<Option<PersonalEntry>>,
}
type CatalogEntry = ((u16, u64, ContentLanguage), Arc<FinderCatalog>);
type PersonalEntry = (Arc<FinderCatalog>, (String, String), Arc<FinderPersonal>);
impl FinderCache {
    pub fn page(
        &self,
        store: &crate::data::DataStore,
        workspace: &Value,
        content_revision: &str,
        official_options: &Value,
        request: FinderRequest,
    ) -> Result<FinderPage, FinderError> {
        validate(&request)?;
        let (catalog_version, catalog) = self.catalog(store, request.year, workspace)?;
        let personal = self.personal(&catalog, workspace, content_revision, &request)?;
        let options = parse_options(Some(official_options))?;
        page(
            &catalog,
            &personal,
            options,
            request,
            (catalog_version, content_revision),
        )
    }
    fn catalog(
        &self,
        store: &crate::data::DataStore,
        year: u16,
        workspace: &Value,
    ) -> Result<(u64, Arc<FinderCatalog>), FinderError> {
        let mut entry = lock(&self.catalog);
        // Read under the lock, so a query that waited for a build sees that
        // build's version or a newer one. The version is read before the
        // rows: a commit in between moves it again, so newer rows are never
        // kept under an older version.
        let version = store.catalog_version();
        let key = (year, version, ContentLanguage::of(workspace));
        if let Some((_, catalog)) = entry.as_ref().filter(|(k, _)| *k == key) {
            return Ok((version, catalog.clone()));
        }
        let offerings = crate::catalog::offerings(store, year).map_err(|error| match error {
            crate::catalog::CatalogError::Data(error) => FinderError::Data(error),
            crate::catalog::CatalogError::Json(error) => FinderError::Compatibility(error),
            crate::catalog::CatalogError::InvalidInput(message) => {
                FinderError::InvalidInput(message)
            }
        })?;
        let catalog = Arc::new(FinderCatalog::new(offerings, key.2));
        *entry = Some((key, catalog.clone()));
        Ok((version, catalog))
    }
    fn personal(
        &self,
        catalog: &Arc<FinderCatalog>,
        workspace: &Value,
        content_revision: &str,
        request: &FinderRequest,
    ) -> Result<Arc<FinderPersonal>, FinderError> {
        let key = (content_revision.to_string(), request.term.clone());
        let mut entry = lock(&self.personal);
        if let Some((_, _, personal)) = entry
            .as_ref()
            .filter(|(c, k, _)| Arc::ptr_eq(c, catalog) && *k == key)
        {
            return Ok(personal.clone());
        }
        let personal = Arc::new(FinderPersonal::new(
            workspace,
            catalog,
            request.year,
            &request.term,
        )?);
        *entry = Some((catalog.clone(), key, personal.clone()));
        Ok(personal)
    }
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn validate(request: &FinderRequest) -> Result<(), FinderError> {
    crate::schedule::Term::parse(&request.term)?;
    if !(1000..=9999).contains(&request.year)
        || request.keys.len() > 200
        || request.keys.iter().any(|k| k.len() > 2048)
        || request.input.len() > 4096
        || request.limit > 1000
    {
        return Err(FinderError::InvalidInput(
            "Course search exceeds its input limits.".into(),
        ));
    }
    if matches!(&request.action,Some(FinderAction::Commit{key}|FinderAction::ToggleTime{key,..}) if key.len()>2048)
    {
        return Err(FinderError::InvalidInput(
            "Course condition is too long.".into(),
        ));
    }
    Ok(())
}
fn parse_options(options: Option<&Value>) -> Result<Vec<OfficialFilterOption>, FinderError> {
    Ok(options
        .map(|options| {
            serde_json::from_value(options.clone())
                .map_err(|e| FinderError::InvalidInput(e.to_string()))
        })
        .transpose()?
        .unwrap_or_default())
}

/// The finder over offerings prepared for this one call; Core keeps them
/// in a [`FinderCache`] instead. The page names no stored data version.
pub fn finder_page(
    workspace: &Value,
    offerings: Vec<Value>,
    request: FinderRequest,
) -> Result<FinderPage, FinderError> {
    validate(&request)?;
    let catalog = FinderCatalog::new(offerings, ContentLanguage::of(workspace));
    let personal = FinderPersonal::new(workspace, &catalog, request.year, &request.term)?;
    let options = parse_options(workspace.get("kyomuSearchOptions"))?;
    page(&catalog, &personal, options, request, (0, ""))
}

fn page(
    catalog: &FinderCatalog,
    personal: &FinderPersonal,
    official_options: Vec<OfficialFilterOption>,
    request: FinderRequest,
    (catalog_version, content_revision): (u64, &str),
) -> Result<FinderPage, FinderError> {
    let departments = SyllabusDepartment::all()?
        .into_iter()
        .map(|department| department.shown(catalog.language))
        .collect();
    let ctx = FinderContext {
        current_quarter: personal.current_quarter,
        recommended: personal.recommended.clone(),
        departments,
        busy: personal.busy.clone(),
        official_options,
    };
    let mut keys = request.keys;
    match &request.action {
        Some(FinderAction::Commit { key }) => keys.push(key.clone()),
        Some(FinderAction::ToggleTime { key, append }) => {
            if keys.contains(key) {
                keys.retain(|k| k != key);
            } else {
                if !append {
                    keys.retain(|k| condition(k, &ctx).r#type != "time" && k != "集中");
                }
                keys.push(key.clone());
            }
        }
        _ => {}
    }
    let parsed = factors_of(&keys, &ctx);
    if parsed
        .factors
        .iter()
        .filter(|f| f.r#type == "exclude")
        .flat_map(|f| &f.alts)
        .any(|c| c.r#type.starts_with("official-"))
    {
        return Err(FinderError::InvalidInput(
            "Official form filters support inclusion only.".into(),
        ));
    }
    let keys: Vec<_> = parsed
        .factors
        .iter()
        .flat_map(|f| f.alts.iter().map(|c| c.key.clone()))
        .collect();
    let visible = catalog
        .courses
        .iter()
        .zip(&personal.eligible)
        .filter(|(_, eligible)| request.include_ineligible || **eligible)
        .map(|(course, _)| course)
        .collect::<Vec<_>>();
    let row = |c: &IndexedCourse| personal.row(c, Vec::new());
    let suggestions = suggest(&request.input, &keys, &visible, &ctx, &row);
    let live = live_text(&request.input, &ctx);
    let matches = run(&visible, &parsed.factors, &ctx, &live);
    let total = matches.len();
    let offset = (request.offset as usize).min(total);
    let rows = matches
        .into_iter()
        .skip(offset)
        .take(request.limit as usize)
        .map(|m| personal.row(m.course, m.matched))
        .collect();
    let commit_key =
        if request.input.ends_with(char::is_whitespace) && !request.input.trim().is_empty() {
            let canonical = condition(request.input.trim(), &ctx);
            suggestions.first().and_then(|suggestion| match suggestion {
                Suggestion::Condition { key, condition, .. }
                    if condition.r#type != "text" && *key == canonical.key =>
                {
                    Some(key.clone())
                }
                _ => None,
            })
        } else {
            None
        };
    let active_slots = parsed
        .factors
        .iter()
        .filter(|f| f.r#type == "time")
        .flat_map(|f| &f.alts)
        .flat_map(|c| &c.values)
        .filter_map(|v| {
            Some(format!(
                "{}:{}-{}",
                v.get("day")?.as_u64()?,
                v.get("from")?.as_u64()?,
                v.get("to")?.as_u64()?
            ))
        })
        .collect();
    let mut paste_handled = false;
    let mut pasted = Vec::new();
    let mut missing_codes = Vec::new();
    let mut ineligible_codes = Vec::new();
    if let Some(FinderAction::Paste { text: raw }) = &request.action {
        if raw.len() > 64 * 1024 {
            return Err(FinderError::InvalidInput(
                "Pasted course list is too large.".into(),
            ));
        }
        let normalized = normalized(raw);
        let regex = Regex::new(r"(?i)[a-z]{3}\.[a-z][0-9]{3}(?:-[0-9]{2})?")
            .map_err(|e| FinderError::InvalidInput(e.to_string()))?;
        let mut seen = HashSet::new();
        let codes = regex
            .find_iter(&normalized)
            .map(|m| m.as_str().to_uppercase())
            .filter(|code| seen.insert(code.clone()))
            .collect::<Vec<_>>();
        paste_handled = codes.len() >= 2;
        let courses = || catalog.courses.iter();
        for code in codes.into_iter().filter(|_| paste_handled) {
            let quarter = request.term.rsplit('-').next().unwrap_or("");
            let found = courses()
                .find(|c| {
                    text(&c.course, "code") == code
                        && array(&c.course, "quarters").contains(&json!(quarter))
                })
                .or_else(|| courses().find(|c| text(&c.course, "code") == code));
            if let Some(c) = found {
                if personal.ineligible(text(&c.course, "id")) {
                    ineligible_codes.push(code);
                } else if !personal.on_timetable.contains(&code) {
                    pasted.push(personal.row(c, Vec::new()));
                }
            } else {
                missing_codes.push(code);
            }
        }
    }
    Ok(FinderPage {
        context: ctx,
        factors: parsed.factors,
        duplicates: parsed.duplicates,
        keys,
        live,
        suggestions,
        total,
        offset,
        rows,
        stored: catalog.courses.len(),
        ineligible: personal.ineligible,
        active_slots,
        placed_codes: personal.on_timetable.clone(),
        commit_key,
        paste_handled,
        pasted,
        missing_codes,
        ineligible_codes,
        catalog_version,
        content_revision: content_revision.into(),
        reads: FINDER_READS.to_vec(),
        seq: 0,
    })
}

#[cfg(test)]
#[path = "../tests/course_query.rs"]
mod tests;

#[cfg(feature = "ts")]
pub fn declarations(cfg: &ts_rs::Config) -> Vec<String> {
    vec![
        TimeValue::decl(cfg),
        FinderCourse::decl(cfg),
        FinderSchedule::decl(cfg),
        FinderRowView::decl(cfg),
        Suggestion::decl(cfg),
        OfficialSelection::decl(cfg),
        OfficialFilterOption::decl(cfg),
        FinderContext::decl(cfg),
        Recommendation::decl(cfg),
        FinderDepartment::decl(cfg),
        BusySlot::decl(cfg),
        Condition::decl(cfg),
        Factor::decl(cfg),
        Factors::decl(cfg),
        FinderRequest::decl(cfg),
        FinderAction::decl(cfg),
        FinderPage::decl(cfg),
    ]
}
