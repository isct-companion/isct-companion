use serde::{Deserialize, Serialize};
use ts_rs::TS;

macro_rules! contract {
    ($name:ident { $($body:tt)* }) => {
        #[derive(Clone, Debug, Serialize, Deserialize, TS)]
        #[serde(rename_all = "camelCase")]
        pub struct $name { $($body)* }
    };
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub enum Quarter {
    #[serde(rename = "1Q")]
    #[ts(rename = "1Q")]
    Q1,
    #[serde(rename = "2Q")]
    #[ts(rename = "2Q")]
    Q2,
    #[serde(rename = "3Q")]
    #[ts(rename = "3Q")]
    Q3,
    #[serde(rename = "4Q")]
    #[ts(rename = "4Q")]
    Q4,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum InstructionLanguage {
    Ja,
    En,
    JaEn,
    Other,
}

contract!(Localized {
    #[serde(default)] #[ts(optional)] pub ja: Option<String>,
    #[serde(default)] #[ts(optional)] pub en: Option<String>,
});

contract!(SyncMeta {
    pub id: String,
    #[ts(type = "number")] pub rev: u64,
    pub updated_at: String,
    pub updated_by: String,
    #[serde(default)] #[ts(optional)] pub deleted_at: Option<String>,
});

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum SourceProvider {
    Sample,
    School,
    Syllabus,
    Lms,
    Kyomu,
    Manual,
    Import,
    Mcp,
    Subscription,
}

contract!(SourceRef {
    pub provider: SourceProvider,
    #[serde(default)] #[ts(optional)] pub external_id: Option<String>,
    #[serde(default)] #[ts(optional)] pub import_run_id: Option<String>,
    #[serde(default)] #[ts(optional)] pub fetched_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub hash: Option<String>,
});

contract!(CreditSplit { pub lecture: f64, pub exercise: f64, pub lab: f64, });
contract!(Meeting {
    pub days: Vec<u8>, pub periods: Vec<u8>, pub start_time: String, pub end_time: String,
    pub room: String,
    #[serde(default)] #[ts(optional)] pub room_names: Option<Localized>,
    #[serde(default)] #[ts(optional)] pub valid_from: Option<String>,
    #[serde(default)] #[ts(optional)] pub valid_to: Option<String>,
});
contract!(CourseNames { pub title: Localized, pub instructor: Localized, });
contract!(CourseOverrides {
    #[serde(default)] #[ts(optional)] pub title: Option<String>,
    #[serde(default)] #[ts(optional)] pub instructor: Option<String>,
    #[serde(default)] #[ts(optional)] pub color: Option<String>,
});

contract!(Course {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    #[ts(type = "\"course\"")] pub kind: String,
    #[ts(type = "`${number}-${Quarter}`")] pub term_id: String,
    #[serde(default)] #[ts(optional)] pub source_key: Option<String>,
    #[serde(default)] #[ts(optional)] pub offering_id: Option<String>,
    #[serde(default)] #[ts(optional)] pub names: Option<CourseNames>,
    pub code: String, pub title: String, pub instructor: String, pub credits: f64,
    pub credit_split: CreditSplit, pub instruction_language: InstructionLanguage,
    pub color: String, pub meetings: Vec<Meeting>, pub source: SourceRef,
    #[serde(default)] #[ts(optional)] pub overrides: Option<CourseOverrides>,
    #[serde(default)] #[ts(optional)] pub url: Option<String>,
    #[serde(default)] #[ts(optional)] pub last_seen_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub missing_since: Option<String>,
});

contract!(Membership {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    #[ts(type = "\"membership\"")] pub kind: String,
    #[ts(type = "`${number}-${Quarter}`")] pub term_id: String,
    pub course_id: String,
    #[ts(type = "\"declared\" | \"registered\" | \"lms-member\" | \"completed\" | \"withdrawn\"")] pub status: String,
    pub evidence: SourceRef,
    #[serde(default)] #[ts(optional)] pub source_key: Option<String>,
    #[serde(default)] #[ts(optional)] pub last_seen_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub missing_since: Option<String>,
});

contract!(Assignment {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    #[ts(type = "\"assignment\"")] pub kind: String, pub course_id: String, pub title: String, pub details: String,
    pub due_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub source_key: Option<String>,
    #[serde(default)] #[ts(optional)] pub source_url: Option<String>,
    #[serde(default)] #[ts(optional, type = "number")] pub lms_cmid: Option<u64>,
    #[serde(default)] #[ts(optional, type = "number")] pub lms_course_id: Option<u64>,
    #[serde(default)] #[ts(optional, type = "\"unknown\" | \"open\" | \"submitted\" | \"graded\"")] pub submission: Option<String>,
    #[serde(default)] #[ts(optional, type = "\"unknown\" | \"draft\" | \"submitted\" | \"late\" | \"not-submitted\"")] pub submission_status: Option<String>,
    #[serde(default)] #[ts(optional)] pub hidden: Option<bool>,
    #[serde(default)] #[ts(optional)] pub completed_at: Option<String>,
    pub source: SourceRef,
    #[serde(default)] #[ts(optional)] pub last_seen_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub missing_since: Option<String>,
});

contract!(Material {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    #[ts(type = "\"material\"")] pub kind: String, pub course_id: String, pub session: i32, pub title: String,
    #[serde(rename = "type")] #[ts(type = "\"pdf\" | \"link\" | \"video\"")] pub material_type: String,
    #[serde(default)] #[ts(optional)] pub source_key: Option<String>,
    #[serde(default)] #[ts(optional, type = "number")] pub cmid: Option<u64>,
    #[serde(default)] #[ts(optional, type = "number")] pub lms_course_id: Option<u64>,
    #[serde(default)] #[ts(optional, type = "number")] pub section_id: Option<u64>,
    #[serde(default)] #[ts(optional)] pub section_number: Option<i32>,
    #[serde(default)] #[ts(optional)] pub section_title: Option<String>,
    #[serde(default)] #[ts(optional)] pub module: Option<String>,
    #[serde(default)] #[ts(optional)] pub visible: Option<bool>,
    #[serde(default)] #[ts(optional)] pub body_html: Option<String>,
    #[serde(default)] #[ts(optional)] pub resource_url: Option<String>,
    #[serde(default)] #[ts(optional)] pub url: Option<String>,
    pub source: SourceRef,
    #[serde(default)] #[ts(optional)] pub last_seen_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub missing_since: Option<String>,
});

contract!(PersonalEvent {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    #[ts(type = "\"event\"")] pub kind: String,
    #[ts(type = "`${number}-${Quarter}`")] pub term_id: String,
    pub title: String, pub room: String,
    #[serde(default)] #[ts(optional)] pub date: Option<String>,
    #[serde(default)] #[ts(optional)] pub days: Option<Vec<u8>>,
    pub start_time: String, pub end_time: String, pub blocking: bool,
});
contract!(PlanItem {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    #[ts(type = "\"plan-item\"")] pub kind: String,
    #[ts(type = "`${number}-${Quarter}`")] pub term_id: String,
    #[serde(default)] #[ts(optional)] pub offering_id: Option<String>,
    #[serde(default)] #[ts(optional)] pub course_id: Option<String>,
    #[ts(type = "\"candidate\" | \"planned\"")] pub state: String,
});
contract!(CalendarException {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub date: String,
    #[serde(rename = "type")] #[ts(type = "\"no-class\" | \"replacement\" | \"break\"")] pub exception_type: String,
    #[serde(default)] #[ts(optional)] pub replacement_day: Option<u8>, pub label: String,
});

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ScheduleEntry {
    Weekly {
        day: u8,
        periods: (u8, u8),
        #[serde(default)]
        room: Option<Localized>,
    },
    Intensive {
        #[serde(default)]
        room: Option<Localized>,
    },
    Seminar {
        #[serde(default)]
        room: Option<Localized>,
    },
    Internship {
        #[serde(default)]
        room: Option<Localized>,
    },
    OnDemand {
        #[serde(default)]
        room: Option<Localized>,
    },
    Tba {
        #[serde(default)]
        room: Option<Localized>,
    },
    RegistrationOnly {
        #[serde(default)]
        room: Option<Localized>,
    },
}
contract!(ClassFormat {
    pub kinds: Vec<String>, #[serde(default)] #[ts(optional)] pub delivery: Option<String>,
});
contract!(Offering {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub year: i32, pub quarters: Vec<Quarter>, pub code: String,
    #[serde(default)] #[ts(optional)] pub source_key: Option<String>,
    #[serde(default)] #[ts(optional)] pub class_code: Option<String>,
    #[serde(default)] #[ts(optional)] pub unit_code: Option<String>,
    pub title: Localized, pub instructors: Vec<Localized>,
    pub instruction_language: InstructionLanguage, pub format: ClassFormat,
    pub media: Localized,
    #[serde(default)] #[ts(optional)] pub class_label: Option<String>,
    pub credits: CreditSplit, pub schedule: Vec<ScheduleEntry>, pub academic_unit: Localized,
    pub intensive: bool, pub syllabus_url: String,
    #[serde(default)] #[ts(optional)] pub source_updated: Option<String>,
    pub fetched_at: Localized,
    #[serde(default)] #[ts(optional)] pub last_seen_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub missing_since: Option<String>,
});

contract!(IntensiveDay { pub date: String, pub periods: Option<(u8, u8)>, });
contract!(IntensiveSchedule {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub course_id: String, pub from: String, pub to: String, pub periods: (u8, u8),
    #[serde(default)] #[ts(optional)] pub days: Option<Vec<IntensiveDay>>,
    #[serde(default)] #[ts(optional)] pub room: Option<String>,
});
contract!(Plan {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub title: String,
    #[serde(default)] #[ts(optional)] pub course_id: Option<String>,
    #[serde(default)] #[ts(optional)] pub room: Option<String>,
    pub start: String, #[ts(type = "number")] pub duration: u64,
    #[serde(default)] #[ts(optional)] pub rrule: Option<String>,
    pub exdates: Vec<String>, #[serde(default)] #[ts(optional, type = "number")] pub reminder: Option<u64>,
});
contract!(DateRange { pub start: String, pub end: String, });
contract!(AcademicCalendarDay {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    #[serde(default)] #[ts(optional)] pub source_key: Option<String>,
    #[serde(default)] #[ts(optional)] pub academic_year: Option<i32>,
    #[serde(default)] #[ts(optional)] pub quarter: Option<Quarter>,
    #[serde(default)] #[ts(optional)] pub teaching_ranges: Option<Vec<DateRange>>,
    #[serde(default)] #[ts(optional)] pub exam_ranges: Option<Vec<DateRange>>,
    pub date: String, #[serde(rename = "type")] pub day_type: String,
    #[serde(default)] #[ts(optional)] pub replacement_day: Option<u8>, pub label: Localized,
    #[serde(default)] #[ts(optional)] pub last_seen_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub missing_since: Option<String>,
});
contract!(GpaSummary {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub scope: String, #[serde(default)] #[ts(optional)] pub gpa: Option<f64>,
    #[serde(default)] #[ts(optional)] pub gpt: Option<f64>, pub earned_credits: f64, pub fetched_at: String,
    #[serde(default)] #[ts(optional)] pub source_key: Option<String>,
    #[serde(default)] #[ts(optional)] pub last_seen_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub missing_since: Option<String>,
});
contract!(Department {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub admission_year: i32, pub name: Localized, pub syllabus_scope: String,
    pub unit_code: String, pub guide_pdf: String,
    #[serde(default)] #[ts(optional)] pub source_key: Option<String>,
    #[serde(default)] #[ts(optional)] pub last_seen_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub missing_since: Option<String>,
});
contract!(RecommendedCourse {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub admission_year: i32, pub department_id: String, pub category: Localized,
    pub level: i32, pub code: String, pub group: String,
    #[serde(default)] #[ts(optional, type = "\"required\" | \"recommended\"")] pub mark: Option<String>,
    pub title: Localized, #[serde(default)] #[ts(optional)] pub credits: Option<CreditSplit>,
    #[serde(default)] #[ts(optional)] pub source_pages: Option<Vec<u32>>,
    #[serde(default)] #[ts(optional)] pub source_kind: Option<String>,
    #[serde(default)] #[ts(optional)] pub note: Option<String>,
    #[serde(default)] #[ts(optional)] pub source_key: Option<String>,
    #[serde(default)] #[ts(optional)] pub last_seen_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub missing_since: Option<String>,
});
contract!(ImportantDate {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub academic_year: i32, pub start_date: String, pub end_date: String,
    pub label: Localized, pub category: String, pub source_ids: Vec<String>,
    #[serde(default)] #[ts(optional)] pub label_english_source: Option<String>,
    #[serde(default)] #[ts(optional)] pub source_key: Option<String>,
    #[serde(default)] #[ts(optional)] pub last_seen_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub missing_since: Option<String>,
});
contract!(Exam {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub academic_year: i32,
    #[ts(type = "\"undergraduate\" | \"graduate\"")] pub level: String,
    pub quarter: Quarter, pub course_code: String, pub date: String, pub label: Localized,
    #[serde(default)] #[ts(optional)] pub course_id: Option<String>,
    #[serde(default)] #[ts(optional)] pub class_label: Option<Localized>,
    #[serde(default)] #[ts(optional)] pub syllabus_course_id: Option<String>,
    #[serde(default)] #[ts(optional)] pub periods: Option<(u8, u8)>,
    #[serde(default)] #[ts(optional)] pub start_time: Option<String>,
    #[serde(default)] #[ts(optional)] pub end_time: Option<String>,
    pub room: Option<Localized>, pub rooms: Vec<Localized>,
    #[ts(type = "\"scheduled\" | \"cancelled\"")] pub status: String,
    #[serde(default)] #[ts(optional)] pub notes: Option<Localized>,
    pub source_ids: Vec<String>, pub source_pages: Vec<u32>,
    #[serde(default)] #[ts(optional)] pub source_key: Option<String>,
    #[serde(default)] #[ts(optional)] pub last_seen_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub missing_since: Option<String>,
});
contract!(ScheduleOverride {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub target_key: String,
    #[serde(default)] #[ts(optional, type = "\"course\" | \"exam\"")] pub target_kind: Option<String>,
    #[serde(default)] #[ts(optional)] pub course_id: Option<String>,
    #[serde(default)] #[ts(optional)] pub date: Option<String>,
    #[serde(default)] #[ts(optional)] pub periods: Option<(u8, u8)>,
    #[serde(default)] #[ts(optional)] pub start_time: Option<String>,
    #[serde(default)] #[ts(optional)] pub end_time: Option<String>,
    #[serde(default)] #[ts(optional)] pub room: Option<String>,
    #[serde(default)] #[ts(optional)] pub title: Option<String>,
    pub hidden: bool,
});
contract!(Subscription {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    #[serde(default)] #[ts(optional)] pub data_url: Option<String>, pub url: String,
    #[ts(type = "\"calendar\" | \"departments\" | \"important-dates\" | \"exams\"")] pub kind: String, pub enabled: bool,
    #[serde(default)] #[ts(optional)] pub index_hash: Option<String>,
    #[serde(default)] #[ts(optional)] pub remote_hash: Option<String>,
    #[serde(default)] #[ts(optional)] pub source_ts: Option<String>,
    #[serde(default)] #[ts(optional)] pub fetched_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub last_error: Option<String>,
    // Academic or admission year the stored hashes were checked for.
    #[serde(default)] #[ts(optional)] pub year: Option<i32>,
});
contract!(ScheduledTask {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub name: String, #[ts(type = "\"school-refresh\" | \"lms-refresh\" | \"reference-refresh\" | \"export\" | \"hook\"")] pub action: String,
    #[serde(default)] #[ts(optional)] pub hook_id: Option<String>, pub schedule: String, pub enabled: bool,
    #[serde(default)] #[ts(optional)] pub last_occurrence_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub last_run_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub last_result: Option<String>,
});

contract!(AssignmentMark {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub source_key: String, pub hidden: bool,
    #[serde(default)] #[ts(optional)] pub completed_at: Option<String>,
});
contract!(CourseOverride {
    #[serde(flatten)] #[ts(flatten)] pub meta: SyncMeta,
    pub source_key: String,
    #[serde(default)] #[ts(optional)] pub title: Option<String>,
    #[serde(default)] #[ts(optional)] pub instructor: Option<String>,
    #[serde(default)] #[ts(optional)] pub color: Option<String>,
});

contract!(AgentWrite { pub at: String, pub tool: String, });
contract!(AgentAccess { pub enabled: bool, #[serde(default)] pub last_writes: Vec<AgentWrite>, });
contract!(Notifications {
    pub classes: bool, pub assignments: bool,
    #[serde(default)] #[ts(optional)] pub class_lead_minutes: Option<u32>,
    #[ts(type = "NotificationPermission")] pub local_permission: String,
});
contract!(CloudSettings {
    #[ts(type = "\"none\" | \"google-drive\" | \"onedrive\"")] pub provider: String,
    pub client_id: String,
    #[serde(default)] #[ts(optional)] pub redirect_uri: Option<String>,
    #[ts(type = "\"keep-current\" | \"use-latest\" | \"resolve-manually\"")] pub conflict: String,
});
contract!(SubscriptionRefreshSettings {
    #[ts(type = "\"daily\"")] pub frequency: String,
    pub allow_cellular: bool,
    #[serde(default)] #[ts(optional)] pub last_checked_at: Option<String>,
});
contract!(SiteCustomization { pub css: String, pub js: String, pub enabled: bool, });
contract!(SiteCustomizations { pub portal: SiteCustomization, pub lms: SiteCustomization, pub syllabus: SiteCustomization, });
contract!(CustomizationHook { pub id: String, pub name: String, pub action: String, pub schedule: String, pub script: String, pub enabled: bool, });
contract!(Customization {
    pub background: String, pub sites: SiteCustomizations, pub hooks: Vec<CustomizationHook>,
});
contract!(SiteConsents { pub portal: bool, pub lms: bool, pub syllabus: bool, });
contract!(Settings {
    #[serde(default)] #[ts(optional)] pub department_id: Option<String>,
    #[serde(default)] #[ts(optional)] pub lms_bilingual_filter: Option<bool>,
    #[serde(default)] #[ts(optional)] pub auto_download_limit_mb: Option<u32>,
    #[serde(default)] #[ts(optional)] pub auto_clean_days: Option<u32>,
    #[serde(default)] #[ts(optional)] pub agent_access: Option<AgentAccess>,
    #[ts(type = "\"system\" | \"light\" | \"dark\"")] pub appearance: String,
    #[ts(type = "\"comfortable\" | \"compact\"")] pub density: String,
    #[serde(default)] #[ts(optional, type = "\"small\" | \"medium\" | \"large\"")] pub text_size: Option<String>,
    #[serde(default)] #[ts(optional)] pub restore_school_page: Option<bool>,
    #[serde(default)] #[ts(optional)] pub active_school_site: Option<String>,
    #[serde(default)] #[ts(optional)] pub school_service_width: Option<f64>,
    #[serde(default)] #[ts(optional)] pub school_web_zoom: Option<f64>,
    #[serde(default)] #[ts(optional)] pub show_grades: Option<bool>,
    #[serde(default)] #[ts(optional)] pub site_consents: Option<SiteConsents>,
    #[ts(type = "\"en\" | \"ja\" | \"zh\"")] pub locale: String,
    pub notifications: Notifications, pub cloud: CloudSettings,
    pub subscription_refresh: SubscriptionRefreshSettings,
    #[serde(default)] #[ts(optional)] pub customization: Option<Customization>,
});
contract!(GradePathEntry { pub grade: String, pub academic_year: i32, });
contract!(QuarterStarts {
    #[serde(rename = "1Q")] #[ts(rename = "1Q")] pub q1: String,
    #[serde(rename = "2Q")] #[ts(rename = "2Q")] pub q2: String,
    #[serde(rename = "3Q")] #[ts(rename = "3Q")] pub q3: String,
    #[serde(rename = "4Q")] #[ts(rename = "4Q")] pub q4: String,
});
contract!(AcademicYearConfig { pub academic_year: i32, pub quarter_starts: QuarterStarts, });
contract!(AcademicProfile {
    pub name: String, #[serde(default)] #[ts(optional)] pub admission_year: Option<i32>,
    pub grade_path: Vec<GradePathEntry>, pub years: Vec<AcademicYearConfig>,
});
contract!(ImportStatus {
    #[ts(type = "\"not-configured\" | \"connected\" | \"reconnect-required\" | \"error\"")] pub school: String,
    #[serde(default)] #[ts(optional)] pub last_imported_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub message: Option<String>,
    #[serde(default)] #[ts(optional, type = "\"fresh\" | \"cached\" | \"reconnect-required\" | \"error\"")] pub last_result: Option<String>,
    #[serde(default)] #[ts(optional)] pub timetable_reviewed_at: Option<String>,
});
contract!(SyncStatus {
    #[ts(type = "\"idle\" | \"needs-configuration\" | \"syncing\" | \"error\"")] pub state: String,
    #[serde(default)] #[ts(optional)] pub last_synced_at: Option<String>,
    #[serde(default)] #[ts(optional)] pub message: Option<String>,
});
contract!(WorkspaceLog { pub at: String, pub source: String, pub phase: String, pub state: String, pub message: String, pub retryable: bool, });
contract!(Workspace {
    #[ts(type = "3")] pub schema_version: u8, pub device_id: String,
    #[ts(type = "`${number}-${Quarter}`")] pub selected_term: String,
    #[serde(default)] #[ts(optional, type = "Array<`${number}-${Quarter}`>")] pub terms: Option<Vec<String>>,
    pub courses: Vec<Course>, pub memberships: Vec<Membership>, pub assignments: Vec<Assignment>,
    pub materials: Vec<Material>, pub events: Vec<PersonalEvent>, pub plan_items: Vec<PlanItem>,
    pub exceptions: Vec<CalendarException>, pub offerings: Vec<Offering>,
    pub intensive_schedules: Vec<IntensiveSchedule>, pub plans: Vec<Plan>,
    pub academic_calendar: Vec<AcademicCalendarDay>, pub gpa_summary: Vec<GpaSummary>,
    pub departments: Vec<Department>, pub recommended_courses: Vec<RecommendedCourse>,
    pub important_dates: Vec<ImportantDate>, pub exams: Vec<Exam>,
    pub schedule_overrides: Vec<ScheduleOverride>,
    pub subscriptions: Vec<Subscription>, pub scheduled_tasks: Vec<ScheduledTask>,
    pub settings: Settings, pub academic_profile: AcademicProfile,
    #[serde(default)] #[ts(optional)] pub logs: Option<Vec<WorkspaceLog>>,
    pub import_status: ImportStatus, pub sync_status: SyncStatus,
});

contract!(CatalogSearchItem {
    pub id: String, pub year: i32, pub code: String, pub title: Localized,
    pub instructors: Vec<Localized>, pub quarters: Vec<Quarter>, pub syllabus_url: String,
});

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum LifetimePolicy {
    Session,
    UntilDisabled,
    ExpiresAt {
        #[serde(rename = "expiresAt")]
        #[ts(rename = "expiresAt")]
        expires_at: String,
    },
}
contract!(RefreshPolicy { #[ts(type = "number")] pub stale_after_seconds: u64, pub refresh_mode: String, });
contract!(EvictionPolicy {
    #[ts(type = "number")] pub evict_after_unused_seconds: u64,
    #[ts(type = "number | null")] pub max_bytes: Option<u64>,
});
contract!(DataPolicy {
    pub lms: RefreshPolicy, pub timetable: RefreshPolicy, pub kyomu: RefreshPolicy,
    pub syllabus: RefreshPolicy, pub course_materials: EvictionPolicy,
});
contract!(IntegerPolicy { pub default: u32, pub min: u32, pub max: u32, });
contract!(CourseFilePolicy { pub auto_download_limit_mb: IntegerPolicy, pub retention_days: IntegerPolicy, });

pub const LMS_STALE_SECONDS: u64 = 12 * 60 * 60;
pub const TIMETABLE_STALE_SECONDS: u64 = 30 * 24 * 60 * 60;
pub const KYOMU_STALE_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const SYLLABUS_STALE_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const COURSE_MATERIAL_UNUSED_SECONDS: u64 = 30 * 24 * 60 * 60;

pub fn data_policy() -> DataPolicy {
    DataPolicy {
        lms: RefreshPolicy {
            stale_after_seconds: LMS_STALE_SECONDS,
            refresh_mode: "on-access".into(),
        },
        timetable: RefreshPolicy {
            stale_after_seconds: TIMETABLE_STALE_SECONDS,
            refresh_mode: "manual-review".into(),
        },
        kyomu: RefreshPolicy {
            stale_after_seconds: KYOMU_STALE_SECONDS,
            refresh_mode: "on-access".into(),
        },
        syllabus: RefreshPolicy {
            stale_after_seconds: SYLLABUS_STALE_SECONDS,
            refresh_mode: "on-access".into(),
        },
        course_materials: EvictionPolicy {
            evict_after_unused_seconds: COURSE_MATERIAL_UNUSED_SECONDS,
            max_bytes: None,
        },
    }
}

pub fn course_file_policy() -> CourseFilePolicy {
    CourseFilePolicy {
        auto_download_limit_mb: IntegerPolicy {
            default: 20,
            min: 1,
            max: 1024,
        },
        retention_days: IntegerPolicy {
            default: 30,
            min: 1,
            max: 3650,
        },
    }
}

pub fn declarations() -> Vec<String> {
    vec![
        Quarter::decl(),
        InstructionLanguage::decl(),
        Localized::decl(),
        SyncMeta::decl(),
        SourceProvider::decl(),
        SourceRef::decl(),
        CreditSplit::decl(),
        Meeting::decl(),
        CourseNames::decl(),
        CourseOverrides::decl(),
        Course::decl(),
        Membership::decl(),
        Assignment::decl(),
        Material::decl(),
        PersonalEvent::decl(),
        PlanItem::decl(),
        CalendarException::decl(),
        ScheduleEntry::decl(),
        ClassFormat::decl(),
        Offering::decl(),
        IntensiveDay::decl(),
        IntensiveSchedule::decl(),
        Plan::decl(),
        DateRange::decl(),
        AcademicCalendarDay::decl(),
        GpaSummary::decl(),
        Department::decl(),
        RecommendedCourse::decl(),
        ImportantDate::decl(),
        Exam::decl(),
        ScheduleOverride::decl(),
        Subscription::decl(),
        ScheduledTask::decl(),
        AssignmentMark::decl(),
        CourseOverride::decl(),
        AgentWrite::decl(),
        AgentAccess::decl(),
        Notifications::decl(),
        CloudSettings::decl(),
        SubscriptionRefreshSettings::decl(),
        SiteCustomization::decl(),
        SiteCustomizations::decl(),
        CustomizationHook::decl(),
        Customization::decl(),
        SiteConsents::decl(),
        Settings::decl(),
        GradePathEntry::decl(),
        QuarterStarts::decl(),
        AcademicYearConfig::decl(),
        AcademicProfile::decl(),
        ImportStatus::decl(),
        SyncStatus::decl(),
        WorkspaceLog::decl(),
        Workspace::decl(),
        CatalogSearchItem::decl(),
        LifetimePolicy::decl(),
        RefreshPolicy::decl(),
        EvictionPolicy::decl(),
        DataPolicy::decl(),
        IntegerPolicy::decl(),
        CourseFilePolicy::decl(),
    ]
}
