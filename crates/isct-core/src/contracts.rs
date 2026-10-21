use crate::schedule::ClassGroup;
use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

macro_rules! contract {
    ($name:ident { $($body:tt)* }) => {
        #[derive(Clone, Debug, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(TS))]
        #[serde(rename_all = "camelCase")]
        pub struct $name { $($body)* }
    };
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub enum Quarter {
    #[serde(rename = "1Q")]
    #[cfg_attr(feature = "ts", ts(rename = "1Q"))]
    Q1,
    #[serde(rename = "2Q")]
    #[cfg_attr(feature = "ts", ts(rename = "2Q"))]
    Q2,
    #[serde(rename = "3Q")]
    #[cfg_attr(feature = "ts", ts(rename = "3Q"))]
    Q3,
    #[serde(rename = "4Q")]
    #[cfg_attr(feature = "ts", ts(rename = "4Q"))]
    Q4,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "kebab-case")]
pub enum InstructionLanguage {
    Ja,
    En,
    JaEn,
    Other,
}

contract!(Localized {
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub ja: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub en: Option<String>,
});

contract!(SyncMeta {
    pub id: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))] pub rev: u64,
    pub updated_at: String,
    pub updated_by: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub deleted_at: Option<String>,
});

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "kebab-case")]
pub enum SourceProvider {
    Sample,
    School,
    Lms,
    Kyomu,
    Manual,
    Import,
    Mcp,
    Subscription,
}

contract!(SourceRef {
    pub provider: SourceProvider,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub external_id: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub import_run_id: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub fetched_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub hash: Option<String>,
});

contract!(CreditSplit { pub lecture: f64, pub exercise: f64, pub lab: f64, });
contract!(Meeting {
    pub days: Vec<u8>, pub periods: Vec<u8>, pub start_time: String, pub end_time: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub term_ids: Option<Vec<String>>,
    pub room: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub room_names: Option<Localized>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub valid_from: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub valid_to: Option<String>,
});
contract!(CourseNames { pub title: Localized, pub instructor: Localized, });
// Which LMS section (lesson) one class meeting covered, as the person marked it.
contract!(LessonMark { pub occurrence: String, #[cfg_attr(feature = "ts", ts(type = "number"))] pub section_id: u64 });
contract!(CourseOverrides {
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub title: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub instructor: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub color: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub lesson_log: Option<Vec<LessonMark>>,
});

contract!(Course {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    #[cfg_attr(feature = "ts", ts(type = "\"course\""))] pub kind: String,
    #[cfg_attr(feature = "ts", ts(type = "`${number}-${Quarter}`"))] pub term_id: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_key: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub offering_id: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub names: Option<CourseNames>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "number"))] pub lms_course_id: Option<u64>,
    pub code: String, pub title: String, pub instructor: String, pub credits: f64,
    pub credit_split: CreditSplit, pub instruction_language: InstructionLanguage,
    pub color: String, pub meetings: Vec<Meeting>, pub source: SourceRef,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub overrides: Option<CourseOverrides>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub url: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_seen_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub missing_since: Option<String>,
});

contract!(Membership {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    #[cfg_attr(feature = "ts", ts(type = "\"membership\""))] pub kind: String,
    #[cfg_attr(feature = "ts", ts(type = "`${number}-${Quarter}`"))] pub term_id: String,
    pub course_id: String,
    #[cfg_attr(feature = "ts", ts(type = "\"declared\" | \"registered\" | \"lms-member\" | \"completed\" | \"withdrawn\""))] pub status: String,
    pub evidence: SourceRef,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_key: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_seen_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub missing_since: Option<String>,
});

contract!(Assignment {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    #[cfg_attr(feature = "ts", ts(type = "\"assignment\""))] pub kind: String, pub course_id: String, pub title: String, pub details: String,
    pub due_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_key: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_url: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "number"))] pub lms_cmid: Option<u64>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "number"))] pub lms_course_id: Option<u64>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "\"unknown\" | \"open\" | \"submitted\" | \"graded\""))] pub submission: Option<String>,
    /// The grade as displayed by the LMS.
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub grade: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "\"unknown\" | \"draft\" | \"submitted\" | \"late\" | \"not-submitted\""))] pub submission_status: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub hidden: Option<bool>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub pinned: Option<bool>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub completed_at: Option<String>,
    pub source: SourceRef,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_seen_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub missing_since: Option<String>,
});

// One file of an LMS folder, as its folder page lists it.
contract!(FolderFile { pub name: String, pub url: String });

contract!(Material {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    #[cfg_attr(feature = "ts", ts(type = "\"material\""))] pub kind: String, pub course_id: String, pub session: i32, pub title: String,
    #[serde(rename = "type")] #[cfg_attr(feature = "ts", ts(type = "\"pdf\" | \"link\" | \"video\""))] pub material_type: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_key: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "number"))] pub cmid: Option<u64>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "number"))] pub lms_course_id: Option<u64>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "number"))] pub section_id: Option<u64>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub section_number: Option<i32>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub section_title: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub module: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub visible: Option<bool>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub body_html: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub resource_url: Option<String>,
    /// A folder's files, kept from its last successful read.
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub folder_files: Option<Vec<FolderFile>>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub url: Option<String>,
    pub source: SourceRef,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_seen_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub missing_since: Option<String>,
});

contract!(PersonalEvent {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    #[cfg_attr(feature = "ts", ts(type = "\"event\""))] pub kind: String,
    #[cfg_attr(feature = "ts", ts(type = "`${number}-${Quarter}`"))] pub term_id: String,
    pub title: String, pub room: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub date: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub days: Option<Vec<u8>>,
    pub start_time: String, pub end_time: String, pub blocking: bool,
});
contract!(PlanItem {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    #[cfg_attr(feature = "ts", ts(type = "\"plan-item\""))] pub kind: String,
    #[cfg_attr(feature = "ts", ts(type = "`${number}-${Quarter}`"))] pub term_id: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub offering_id: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub course_id: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "\"candidate\" | \"planned\""))] pub state: String,
});
contract!(CalendarException {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub date: String,
    #[serde(rename = "type")] #[cfg_attr(feature = "ts", ts(type = "\"no-class\" | \"replacement\" | \"break\""))] pub exception_type: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub replacement_day: Option<u8>, pub label: String,
});

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
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
contract!(ClassFormat { pub kinds: Vec<String>, });
// The 教務 course search's facts about an offering.
contract!(OfficialListing { pub department: String, });
contract!(Offering {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub year: i32, pub quarters: Vec<Quarter>, pub code: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_key: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub unit_code: Option<String>,
    pub title: Localized, pub instructors: Vec<Localized>,
    pub instruction_language: InstructionLanguage, pub format: ClassFormat,
    pub credits: CreditSplit, pub schedule: Vec<ScheduleEntry>,
    pub intensive: bool, pub syllabus_url: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_seen_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub missing_since: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub kyomu: Option<OfficialListing>,
});

contract!(IntensiveDay { pub date: String, pub periods: Option<(u8, u8)>, });
contract!(IntensiveSchedule {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub course_id: String, pub from: String, pub to: String, pub periods: (u8, u8),
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub days: Option<Vec<IntensiveDay>>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub room: Option<String>,
});
contract!(Plan {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub title: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub course_id: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub room: Option<String>,
    pub start: String, #[cfg_attr(feature = "ts", ts(type = "number"))] pub duration: u64,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub rrule: Option<String>,
    pub exdates: Vec<String>, #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "number"))] pub reminder: Option<u64>,
});
impl Plan {
    /// Every local writer stores a plan the schedule can evaluate: a JST
    /// start and, when it repeats, a supported RRULE.
    pub fn validate(&self) -> Result<(), crate::plans::PlanError> {
        crate::plans::validate_plan_recurrence(&self.start, self.rrule.as_deref())
    }
}
contract!(DateRange { pub start: String, pub end: String, });
contract!(AcademicCalendarDay {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_key: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub academic_year: Option<i32>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub quarter: Option<Quarter>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub teaching_ranges: Option<Vec<DateRange>>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub exam_ranges: Option<Vec<DateRange>>,
    pub date: String, #[serde(rename = "type")] pub day_type: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub replacement_day: Option<u8>, pub label: Localized,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_seen_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub missing_since: Option<String>,
});
contract!(GpaSummary {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub scope: String, #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub gpa: Option<f64>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub gpt: Option<f64>, pub earned_credits: f64, pub fetched_at: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_key: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_seen_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub missing_since: Option<String>,
});
contract!(Department {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub admission_year: i32, pub name: Localized, pub syllabus_scope: String,
    pub unit_code: String, pub guide_pdf: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_key: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_seen_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub missing_since: Option<String>,
});
contract!(RecommendedCourse {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub admission_year: i32, pub department_id: String, pub category: Localized,
    pub level: i32, pub code: String, pub group: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "\"required\" | \"recommended\""))] pub mark: Option<String>,
    pub title: Localized, #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub credits: Option<CreditSplit>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_pages: Option<Vec<u32>>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_kind: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub note: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_key: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_seen_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub missing_since: Option<String>,
});
contract!(ImportantDate {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub academic_year: i32, pub start_date: String, pub end_date: String,
    pub label: Localized, pub category: String, pub source_ids: Vec<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub label_english_source: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_key: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_seen_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub missing_since: Option<String>,
});
// Dated sessions of an alternating course, published in Isct-Data.
contract!(ClassSession {
    pub date: String,
    #[cfg_attr(feature = "ts", ts(type = "\"orientation\" | \"exercise\" | \"lab\" | \"exam\" | \"no-class\""))] pub kind: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub group: Option<ClassGroup>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub number: Option<String>,
});
contract!(ClassSchedule {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub academic_year: i32, pub course_code: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub class_label: Option<Localized>,
    pub offering_ids: Vec<String>, pub quarters: Vec<Quarter>, pub source_ids: Vec<String>,
    pub sessions: Vec<ClassSession>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_key: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_seen_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub missing_since: Option<String>,
});
contract!(Exam {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    /// `class` marks a class row the exam timetable lists in the exam period.
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "\"exam\" | \"class\""))] pub kind: Option<String>,
    pub academic_year: i32,
    #[cfg_attr(feature = "ts", ts(type = "\"undergraduate\" | \"graduate\""))] pub level: String,
    pub quarter: Quarter, pub course_code: String, pub date: String, pub label: Localized,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub course_id: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub class_label: Option<Localized>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub syllabus_course_id: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub periods: Option<(u8, u8)>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub start_time: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub end_time: Option<String>,
    pub room: Option<Localized>, pub rooms: Vec<Localized>,
    #[cfg_attr(feature = "ts", ts(type = "\"scheduled\" | \"cancelled\""))] pub status: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub notes: Option<Localized>,
    pub source_ids: Vec<String>, pub source_pages: Vec<u32>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_key: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_seen_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub missing_since: Option<String>,
});
contract!(ScheduleOverride {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub target_key: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "\"course\" | \"exam\""))] pub target_kind: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub course_id: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub date: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub periods: Option<(u8, u8)>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub start_time: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub end_time: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub room: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub title: Option<String>,
    pub hidden: bool,
    /// The user's call on what this session is: an exam, a cancelled class
    /// (休講) or a class that runs. It outranks every published source.
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "\"exam\" | \"cancelled\" | \"class\""))] pub mark: Option<String>,
});
impl ScheduleOverride {
    /// Corrections must remain usable by agendas and notification planning.
    pub fn validate(&self) -> Result<(), crate::schedule::ScheduleError> {
        if self
            .mark
            .as_deref()
            .is_some_and(|mark| !matches!(mark, "exam" | "cancelled" | "class"))
        {
            return Err(crate::schedule::ScheduleError::InvalidInput(
                "Mark a session as an exam, cancelled or a normal class.".into(),
            ));
        }
        self.periods
            .map(|(first, last)| crate::schedule::validate_period_range([first, last]))
            .transpose()?;
        if let Some(date) = &self.date {
            crate::schedule::parse_civil_date(date)?;
        }
        for clock in [&self.start_time, &self.end_time].into_iter().flatten() {
            if chrono::NaiveTime::parse_from_str(clock, "%H:%M")
                .ok()
                .is_none_or(|value| value.format("%H:%M").to_string() != *clock)
            {
                return Err(crate::schedule::ScheduleError::InvalidInput(
                    "Choose a valid time in HH:MM format.".into(),
                ));
            }
        }
        if self
            .start_time
            .as_ref()
            .zip(self.end_time.as_ref())
            .is_some_and(|(start, end)| start >= end)
        {
            return Err(crate::schedule::ScheduleError::InvalidInput(
                "Exam end time must be after its start time.".into(),
            ));
        }
        Ok(())
    }
}
contract!(Subscription {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub data_url: Option<String>, pub url: String,
    #[cfg_attr(feature = "ts", ts(type = "(typeof REFERENCE_KINDS)[number]"))] pub kind: String, pub enabled: bool,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub index_hash: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub remote_hash: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub source_ts: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub fetched_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_error: Option<String>,
    // Academic or admission year the stored hashes were checked for.
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub year: Option<i32>,
});
contract!(ScheduledTask {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub name: String, #[cfg_attr(feature = "ts", ts(type = "\"school-refresh\" | \"lms-refresh\" | \"reference-refresh\" | \"export\" | \"hook\""))] pub action: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub hook_id: Option<String>, pub schedule: String, pub enabled: bool,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_occurrence_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_run_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_result: Option<String>,
});

contract!(AssignmentMark {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub source_key: String, pub hidden: bool,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub pinned: Option<bool>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub completed_at: Option<String>,
});
contract!(CourseOverride {
    #[serde(flatten)] #[cfg_attr(feature = "ts", ts(flatten))] pub meta: SyncMeta,
    pub source_key: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub title: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub instructor: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub color: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub lesson_log: Option<Vec<LessonMark>>,
});

contract!(AgentWrite { pub at: String, pub tool: String, });
contract!(AgentAccess { pub enabled: bool, #[serde(default)] pub last_writes: Vec<AgentWrite>, });
contract!(Notifications {
    pub classes: bool, pub assignments: bool,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub exams: Option<bool>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub exam_lead_minutes: Option<u32>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub class_lead_minutes: Option<u32>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub assignment_lead_minutes: Option<Vec<u32>>,
    #[cfg_attr(feature = "ts", ts(type = "NotificationPermission"))] pub local_permission: String,
});
contract!(CloudSettings {
    #[cfg_attr(feature = "ts", ts(type = "\"none\" | \"google-drive\" | \"onedrive\" | \"icloud\""))] pub provider: String,
    pub client_id: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub redirect_uri: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "\"keep-current\" | \"use-latest\" | \"resolve-manually\""))] pub conflict: String,
});
contract!(SubscriptionRefreshSettings {
    #[cfg_attr(feature = "ts", ts(type = "\"daily\""))] pub frequency: String,
    pub allow_cellular: bool,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_checked_at: Option<String>,
});
contract!(SiteCustomization { pub css: String, pub js: String, pub enabled: bool, });
contract!(SiteCustomizations { pub portal: SiteCustomization, pub lms: SiteCustomization, pub syllabus: SiteCustomization, });
contract!(CustomizationHook { pub id: String, pub name: String, pub action: String, pub schedule: String, pub script: String, pub enabled: bool, });
contract!(Customization {
    pub background: String, pub sites: SiteCustomizations, pub hooks: Vec<CustomizationHook>,
});
contract!(SiteConsents { pub portal: bool, pub lms: bool, pub syllabus: bool, });
contract!(Settings {
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub department_id: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub lms_bilingual_filter: Option<bool>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub auto_download_limit_mb: Option<u32>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub auto_clean_days: Option<u32>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub agent_access: Option<AgentAccess>,
    #[cfg_attr(feature = "ts", ts(type = "\"system\" | \"light\" | \"dark\""))] pub appearance: String,
    #[cfg_attr(feature = "ts", ts(type = "\"comfortable\" | \"compact\""))] pub density: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "\"small\" | \"medium\" | \"large\""))] pub text_size: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub automatic_app_updates: Option<bool>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub restore_school_page: Option<bool>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub active_school_site: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub school_web_zoom: Option<f64>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub show_grades: Option<bool>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub site_consents: Option<SiteConsents>,
    #[cfg_attr(feature = "ts", ts(type = "\"en\" | \"ja\" | \"zh\""))] pub locale: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "\"auto\" | \"ja\" | \"en\""))] pub content_language: Option<String>,
    pub notifications: Notifications, pub cloud: CloudSettings,
    pub subscription_refresh: SubscriptionRefreshSettings,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub customization: Option<Customization>,
    /// When the user acknowledged the one-time notice on data accuracy.
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub data_notice_acknowledged_at: Option<String>,
});
contract!(GradePathEntry { pub grade: String, pub academic_year: i32, });
contract!(QuarterStarts {
    #[serde(rename = "1Q")] #[cfg_attr(feature = "ts", ts(rename = "1Q"))] pub q1: String,
    #[serde(rename = "2Q")] #[cfg_attr(feature = "ts", ts(rename = "2Q"))] pub q2: String,
    #[serde(rename = "3Q")] #[cfg_attr(feature = "ts", ts(rename = "3Q"))] pub q3: String,
    #[serde(rename = "4Q")] #[cfg_attr(feature = "ts", ts(rename = "4Q"))] pub q4: String,
});
contract!(AcademicYearConfig { pub academic_year: i32, pub quarter_starts: QuarterStarts, });
contract!(AcademicProfile {
    pub name: String, #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub admission_year: Option<i32>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "StudyEnrollment[]"))]
    pub study_enrollments: Option<Vec<crate::study_profile::StudyEnrollment>>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "TeachingPeriod[] | null"))]
    pub teaching_periods: Option<Vec<crate::schedule::TeachingPeriod>>,
    pub grade_path: Vec<GradePathEntry>, pub years: Vec<AcademicYearConfig>,
});
contract!(ImportStatus {
    #[cfg_attr(feature = "ts", ts(type = "\"not-configured\" | \"connected\" | \"reconnect-required\" | \"error\""))] pub school: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_imported_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub message: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "\"fresh\" | \"cached\" | \"reconnect-required\" | \"error\""))] pub last_result: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub timetable_reviewed_at: Option<String>,
});
contract!(SyncStatus {
    #[cfg_attr(feature = "ts", ts(type = "\"idle\" | \"needs-configuration\" | \"syncing\" | \"error\""))] pub state: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub last_synced_at: Option<String>,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub message: Option<String>,
});
contract!(WorkspaceLog { pub at: String, pub source: String, pub phase: String, pub state: String, pub message: String, pub retryable: bool, });
contract!(Workspace {
    #[cfg_attr(feature = "ts", ts(type = "3"))] pub schema_version: u8, pub device_id: String,
    #[cfg_attr(feature = "ts", ts(type = "`${number}-${Quarter}`"))] pub selected_term: String,
    /// Derived by Rust from profile overrides, then the subscribed academic
    /// calendar. If neither has a usable boundary, the selected term is kept;
    /// no month-based academic-calendar assumptions are made here.
    #[cfg_attr(feature = "ts", ts(type = "`${number}-${Quarter}`"))] pub current_term: String,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "Array<`${number}-${Quarter}`>"))] pub terms: Option<Vec<String>>,
    pub courses: Vec<Course>, pub memberships: Vec<Membership>, pub assignments: Vec<Assignment>,
    pub materials: Vec<Material>, pub events: Vec<PersonalEvent>, pub plan_items: Vec<PlanItem>,
    pub exceptions: Vec<CalendarException>, pub offerings: Vec<Offering>,
    pub intensive_schedules: Vec<IntensiveSchedule>, pub plans: Vec<Plan>,
    pub academic_calendar: Vec<AcademicCalendarDay>, pub gpa_summary: Vec<GpaSummary>,
    pub departments: Vec<Department>, pub recommended_courses: Vec<RecommendedCourse>,
    pub important_dates: Vec<ImportantDate>, pub exams: Vec<Exam>,
    pub class_schedules: Vec<ClassSchedule>,
    /// The red/white group chosen for each semester ("2026-3Q4Q") of
    /// alternating 物理学演習・実験 classes.
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional, type = "Partial<Record<string, ClassGroup>>"))] pub class_groups: Option<std::collections::BTreeMap<String, ClassGroup>>,
    pub schedule_overrides: Vec<ScheduleOverride>,
    pub subscriptions: Vec<Subscription>, pub scheduled_tasks: Vec<ScheduledTask>,
    pub settings: Settings, pub academic_profile: AcademicProfile,
    #[serde(default)] #[cfg_attr(feature = "ts", ts(optional))] pub logs: Option<Vec<WorkspaceLog>>,
    pub import_status: ImportStatus, pub sync_status: SyncStatus,
});

contract!(IntegerPolicy { pub default: u32, pub min: u32, pub max: u32, });
contract!(CourseFilePolicy { pub auto_download_limit_mb: IntegerPolicy, pub retention_days: IntegerPolicy, });

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "kebab-case")]
pub enum SchoolPageAvailabilityState {
    Available,
    Unavailable,
}

/// Latest authoritative capability observation for one school page, as the
/// store reads it from the page's `kyomu.page` freshness key.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SchoolPageAvailability {
    pub state: SchoolPageAvailabilityState,
    pub checked_at: String,
    pub message: String,
}

#[cfg(feature = "ts")]
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
        LessonMark::decl(),
        CourseOverrides::decl(),
        Course::decl(),
        Membership::decl(),
        Assignment::decl(),
        FolderFile::decl(),
        Material::decl(),
        PersonalEvent::decl(),
        PlanItem::decl(),
        CalendarException::decl(),
        ScheduleEntry::decl(),
        ClassFormat::decl(),
        OfficialListing::decl(),
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
        ClassSession::decl(),
        ClassSchedule::decl(),
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
        IntegerPolicy::decl(),
        CourseFilePolicy::decl(),
    ]
}
