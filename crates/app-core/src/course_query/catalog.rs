use super::IndexedCourse;
use crate::locale::{ContentLanguage, Localized};
#[cfg(not(feature = "demo"))]
use crate::schedule::text_or_empty as text;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FinderDepartment {
    pub id: String,
    /// Its name in the content language.
    pub name: String,
    pub unit_code: String,
    #[serde(default)]
    pub code_prefix: String,
    #[serde(default)]
    pub scope: Option<String>,
    /// Both names, either of which a typed word may name.
    #[serde(skip)]
    pub(crate) names: Localized,
}
/// One department as the syllabus lists it (`departments.json`).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyllabusDepartment {
    pub id: String,
    pub name: Localized,
    pub unit_code: String,
    #[serde(default)]
    pub code_prefix: String,
    #[serde(default)]
    pub scope: Option<String>,
}
impl SyllabusDepartment {
    pub(crate) fn all() -> Result<Vec<Self>, serde_json::Error> {
        serde_json::from_str(include_str!("departments.json"))
    }
    pub(super) fn shown(self, language: ContentLanguage) -> FinderDepartment {
        let name = self
            .name
            .pick(language)
            .map_or_else(|| self.id.clone(), str::to_owned);
        FinderDepartment {
            id: self.id,
            name,
            unit_code: self.unit_code,
            code_prefix: self.code_prefix,
            scope: self.scope,
            names: self.name,
        }
    }
}

pub fn department_name_ja(id: &str) -> Result<Option<String>, serde_json::Error> {
    Ok(SyllabusDepartment::all()?
        .into_iter()
        .find(|department| department.id == id || id.ends_with(&format!(":{}", department.id)))
        .and_then(|department| department.name.ja))
}

/// One year's stored catalog prepared for the finder: the rows it can list,
/// with syllabus links and search text, and the 教務 guide fields of every
/// stored row, listable or not.
pub struct FinderCatalog {
    pub(super) courses: Vec<IndexedCourse>,
    pub(super) guide: Vec<GuideRow>,
    pub(super) language: ContentLanguage,
}
pub(super) struct GuideRow {
    pub(super) year: Option<u64>,
    pub(super) code: Option<String>,
    pub(super) group: String,
    pub(super) mark: Option<String>,
}
impl FinderCatalog {
    pub fn new(offerings: Vec<Value>, language: ContentLanguage) -> Self {
        let guide = offerings
            .iter()
            .filter_map(|r| {
                let group = r.pointer("/kyomu/group")?.as_str()?;
                if group.is_empty() {
                    return None;
                }
                Some(GuideRow {
                    year: r["year"].as_u64(),
                    code: r.get("code").and_then(Value::as_str).map(str::to_string),
                    group: group.into(),
                    mark: r
                        .pointer("/kyomu/mark")
                        .and_then(Value::as_str)
                        .filter(|m| !m.is_empty())
                        .map(str::to_string),
                })
            })
            .collect();
        let courses = offerings
            .into_iter()
            .filter_map(normalize_offering)
            .map(|course| {
                // The demo scene's offerings have no syllabus page.
                #[cfg(not(feature = "demo"))]
                let course = {
                    let mut course = course;
                    let url = crate::catalog::syllabus_url(text(&course, "id"), language.code());
                    course["syllabusUrl"] = url.into();
                    course
                };
                IndexedCourse::new(course, language)
            })
            .collect();
        Self {
            courses,
            guide,
            language,
        }
    }
}

pub fn normalize_offering(mut row: Value) -> Option<Value> {
    if !row.is_object()
        || !row.get("id").is_some_and(Value::is_string)
        || !row.get("code").is_some_and(Value::is_string)
        || !row.get("quarters").is_some_and(Value::is_array)
        || !row.get("title").is_some_and(Value::is_object)
    {
        return None;
    }
    if !row.get("schedule").is_some_and(Value::is_array) {
        row["schedule"] = json!([]);
    }
    if !row.get("credits").is_some_and(Value::is_object) {
        row["credits"] = json!({"lecture":0,"exercise":0,"lab":0});
    }
    if !row.get("instructors").is_some_and(Value::is_array) {
        row["instructors"] = json!([]);
    }
    if !row.pointer("/format/kinds").is_some_and(Value::is_array) {
        row["format"] = json!({"kinds":[]});
    }
    if !row.get("instructionLanguage").is_some_and(Value::is_string) {
        row["instructionLanguage"] = json!("other");
    }
    Some(row)
}
