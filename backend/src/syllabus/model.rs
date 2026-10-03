use serde::{Deserialize, Serialize};

pub type Lang = &'static str;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Localized {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ja: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub en: Option<String>,
}

impl Localized {
    pub(crate) fn one(language: &str, value: String) -> Self {
        match language {
            "ja" => Self {
                ja: Some(value),
                en: None,
            },
            _ => Self {
                ja: None,
                en: Some(value),
            },
        }
    }

    pub(crate) fn pair(ja: String, en: String) -> Self {
        Self {
            ja: Some(ja),
            en: Some(en),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credits {
    pub lecture: f32,
    pub exercise: f32,
    pub lab: f32,
}

impl Credits {
    pub fn total(&self) -> f32 {
        self.lecture + self.exercise + self.lab
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassFormat {
    pub kinds: Vec<FormatKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<DeliveryKind>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormatKind {
    Lecture,
    Exercise,
    Experiment,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeliveryKind {
    FaceToFace,
    Livestream,
    Blended,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScheduleKind {
    Weekly,
    Intensive,
    Seminar,
    Internship,
    OnDemand,
    Tba,
    RegistrationOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleEntry {
    pub kind: ScheduleKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub periods: Option<[u8; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room: Option<Localized>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Offering {
    pub id: String,
    pub year: u16,
    pub quarters: Vec<String>,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_code: Option<String>,
    pub title: Localized,
    pub instructors: Vec<Localized>,
    pub instruction_language: String,
    pub format: ClassFormat,
    pub media: Localized,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_label: Option<String>,
    pub credits: Credits,
    pub schedule: Vec<ScheduleEntry>,
    pub academic_unit: Localized,
    pub intensive: bool,
    pub syllabus_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_updated: Option<String>,
    pub fetched_at: Localized,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_raw: Option<Localized>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_raw: Option<Localized>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSearchItem {
    pub id: String,
    pub year: u16,
    pub quarters: Vec<String>,
    pub code: String,
    pub title: Localized,
    pub instructors: Vec<Localized>,
    pub academic_unit: Localized,
    pub syllabus_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_updated: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSearchResult {
    pub items: Vec<CatalogSearchItem>,
    pub returned: usize,
    pub total: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferingContent {
    pub offering_id: String,
    pub lang: String,
    pub content: SyllabusContent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOfferingResult {
    pub offering: Offering,
    pub content: Vec<OfferingContent>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepartmentSyncResult {
    pub offerings: Vec<SyncOfferingResult>,
    pub listed: usize,
    pub synced: usize,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyllabusMetadata {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyllabusSection {
    pub key: String,
    pub heading: String,
    pub blocks: Vec<SyllabusBlock>,
    pub links: Vec<SyllabusLink>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyllabusBlock {
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cells: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyllabusLink {
    pub label: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyllabusLecture {
    pub label: String,
    pub topic: String,
    pub objective: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyllabusCompetency {
    pub label: String,
    pub selected: bool,
}
