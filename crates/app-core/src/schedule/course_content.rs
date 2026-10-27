//! A course's LMS content lesson by lesson, as the desktop Lecture Desk lays
//! it out (`LmsWorkspace.tsx`, `lessons.ts`): each section's own text, its
//! files and links, and the pages, labels and announcements shown inline.
use super::SchoolReads;
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::course_detail::lms_record;
use super::lms_html::safe_html;
use super::occurrence::course_timeline;
use super::{array, text};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CourseContent {
    pub course_id: String,
    /// Sections with anything in them, in section order.
    pub lessons: Vec<LessonContent>,
    /// Where the course is, as the desktop rail marks it (`markedLesson` in
    /// `lessons.ts`): the lesson the person last marked for a class, or the
    /// one after it once a later class has begun. Without a mark, the last
    /// numbered lesson with material (teachers post around class, so a
    /// cancelled class does not advance it); in a course without numbered
    /// lessons, the last section with any.
    pub current_section: Option<i32>,
    /// The class a "this class" mark is recorded against
    /// (`CourseTimeline.class_at`): the one in progress or held earlier
    /// today, else the next one. `None` when the course has no meetings.
    pub class_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LessonContent {
    /// The LMS section number (0 is the course's general section).
    pub section: i32,
    /// The LMS section id, which a lesson mark (`RecordLesson`) names.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub section_id: Option<u64>,
    pub title: Option<String>,
    /// The lesson number read from the title (第3回, Lecture 3, #3), when the
    /// course numbers at least two of its sections.
    pub number: Option<u32>,
    /// The section's own text (授業計画 and the like), made safe to render.
    pub summary_html: String,
    pub items: Vec<LessonItem>,
    /// Whether the section has material besides assignments.
    pub posted: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LessonItem {
    pub material_id: String,
    pub kind: LessonItemKind,
    pub title: String,
    /// The text shown in place, for pages, labels and announcements.
    pub body_html: Option<String>,
    /// The LMS page or file Core resolves and downloads.
    pub source: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LessonItemKind {
    File,
    Link,
    Folder,
    /// A page: shown in place under its title.
    Page,
    /// A label: text the teacher put between items, shown in place.
    Label,
    /// Announcements and discussions, shown in place.
    Forum,
    /// Anything else LMS offers (quizzes' neighbours, tools): its page.
    Other,
}

/// `第3回`, `Lecture 3`, `Week 3`, `#3.`: the rules of the desktop rail
/// (`lessons.ts`), on NFKC text so full-width digits count.
pub fn lesson_number(title: &str) -> Option<u32> {
    use unicode_normalization::UnicodeNormalization;
    static LESSON: std::sync::LazyLock<Result<regex::Regex, regex::Error>> =
        std::sync::LazyLock::new(|| {
            regex::Regex::new(
                r"(?i)第\s*([0-9]+)\s*回|(?:lecture|lesson|class|week|session|day)\s*#?\s*([0-9]+)\b|^\s*#?\s*([0-9]+)\s*(?:[.:、]|$)",
            )
        });
    let normalized: String = title.nfkc().collect();
    let captures = LESSON.as_ref().ok()?.captures(&normalized)?;
    (1..=3)
        .find_map(|group| captures.get(group))
        .and_then(|digits| digits.as_str().parse().ok())
}

fn section_of(row: &Value) -> i32 {
    row.get("sectionNumber")
        .and_then(Value::as_i64)
        .or_else(|| row.get("session").and_then(Value::as_i64))
        .unwrap_or_default() as i32
}

fn kind_of(module: Option<&str>) -> LessonItemKind {
    match module {
        Some("resource") => LessonItemKind::File,
        Some("url") => LessonItemKind::Link,
        Some("folder") => LessonItemKind::Folder,
        Some("page") => LessonItemKind::Page,
        Some("label") => LessonItemKind::Label,
        Some("forum") => LessonItemKind::Forum,
        None => LessonItemKind::File,
        Some(_) => LessonItemKind::Other,
    }
}

pub fn course_content(
    workspace: &Value,
    school: &SchoolReads,
    course_id: &str,
    now: DateTime<FixedOffset>,
) -> Option<CourseContent> {
    let course = array(workspace, "courses")
        .iter()
        .find(|row| visible(row) && text(row, "id") == Some(course_id))?;
    let rows: Vec<&Value> = array(workspace, "materials")
        .iter()
        .filter(|row| {
            visible(row)
                && text(row, "courseId") == Some(course_id)
                && row.get("visible").and_then(Value::as_bool) != Some(false)
        })
        .collect();
    // Sections from the LMS record, which also carries each section's text;
    // a course without one keeps the sections its materials name.
    let mut lessons: Vec<(Option<u64>, LessonContent)> = Vec::new();
    for section in lms_record(workspace, course)
        .and_then(|record| record.get("sections"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|section| section.get("visible").and_then(Value::as_bool) != Some(false))
    {
        let number = section
            .get("number")
            .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
            .unwrap_or_default() as i32;
        let id = section
            .get("id")
            .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()));
        lessons.push((
            id,
            LessonContent {
                section: number,
                section_id: id,
                title: text(section, "title").map(str::to_owned),
                number: None,
                summary_html: text(section, "bodyHtml")
                    .map(|html| safe_html(html, false))
                    .unwrap_or_default(),
                items: Vec::new(),
                posted: false,
            },
        ));
    }
    for row in &rows {
        let section = section_of(row);
        let section_id = row.get("sectionId").and_then(Value::as_u64);
        let index = lessons
            .iter()
            .position(|(id, lesson)| match (section_id, id) {
                (Some(section_id), Some(id)) => section_id == *id,
                _ => lesson.section == section,
            })
            .unwrap_or_else(|| {
                lessons.push((
                    section_id,
                    LessonContent {
                        section,
                        section_id,
                        title: text(row, "sectionTitle").map(str::to_owned),
                        number: None,
                        summary_html: String::new(),
                        items: Vec::new(),
                        posted: false,
                    },
                ));
                lessons.len() - 1
            });
        let lesson = &mut lessons[index].1;
        let module = text(row, "module");
        // Assignments and quizzes have pages of their own (AssignmentDetail).
        if matches!(module, Some("assign" | "quiz")) {
            continue;
        }
        lesson.posted = true;
        let kind = kind_of(module);
        let inline = matches!(
            kind,
            LessonItemKind::Page | LessonItemKind::Label | LessonItemKind::Forum
        );
        lesson.items.push(LessonItem {
            material_id: text(row, "id").unwrap_or_default().into(),
            kind,
            title: text(row, "title").unwrap_or_default().into(),
            body_html: inline
                .then(|| text(row, "bodyHtml").map(|html| safe_html(html, true)))
                .flatten()
                .filter(|html| !html.is_empty()),
            source: text(row, "url")
                .or_else(|| text(row, "resourceUrl"))
                .map(str::to_owned),
        });
    }
    let assignment_sections: Vec<i32> = array(workspace, "assignments")
        .iter()
        .filter(|row| visible(row) && text(row, "courseId") == Some(course_id))
        .filter_map(|row| row.get("lmsCmid").and_then(Value::as_u64))
        .filter_map(|cmid| {
            rows.iter()
                .find(|row| row.get("cmid").and_then(Value::as_u64) == Some(cmid))
                .map(|row| section_of(row))
        })
        .collect();
    let mut lessons: Vec<LessonContent> = lessons
        .into_iter()
        .map(|(_, lesson)| lesson)
        .filter(|lesson| {
            !lesson.summary_html.is_empty()
                || !lesson.items.is_empty()
                || assignment_sections.contains(&lesson.section)
        })
        .collect();
    lessons.sort_by_key(|lesson| lesson.section);
    let numbered = lessons
        .iter()
        .filter(|lesson| lesson.title.as_deref().and_then(lesson_number).is_some())
        .count();
    if numbered >= 2 {
        for lesson in &mut lessons {
            lesson.number = lesson.title.as_deref().and_then(lesson_number);
        }
    }
    // The rail: numbered lessons in a lesson course, else every section.
    let rail: Vec<&LessonContent> = lessons
        .iter()
        .filter(|lesson| numbered < 2 || lesson.number.is_some())
        .collect();
    // The class times only place a mark; content reads without them, so a
    // timetable Core cannot resolve leaves the page without marks.
    let timeline = course_timeline(workspace, school, course_id, now, 14, 14).ok();
    let last_started = timeline
        .as_ref()
        .and_then(|timeline| timeline.last_started.as_deref())
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok());
    let marked = course
        .pointer("/overrides/lessonLog")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|mark| {
            let occurrence = DateTime::parse_from_rfc3339(text(mark, "occurrence")?).ok()?;
            let section_id = mark.get("sectionId").and_then(Value::as_u64)?;
            let index = rail
                .iter()
                .position(|lesson| lesson.section_id == Some(section_id))?;
            Some((occurrence, index))
        })
        .max_by_key(|(occurrence, _)| *occurrence)
        .and_then(|(occurrence, index)| {
            let moved_on = last_started.is_some_and(|started| occurrence < started);
            rail.get(index + usize::from(moved_on))
                .or_else(|| rail.get(index))
                .copied()
        });
    let current = marked.or_else(|| {
        if numbered >= 2 {
            rail.iter()
                .rev()
                .find(|lesson| lesson.posted)
                .or(rail.first())
                .copied()
        } else {
            lessons
                .iter()
                .rev()
                .find(|lesson| lesson.posted)
                .or(lessons.first())
        }
    });
    Some(CourseContent {
        course_id: course_id.into(),
        current_section: current.map(|lesson| lesson.section),
        class_at: timeline.and_then(|timeline| timeline.class_at),
        lessons,
    })
}

fn visible(row: &Value) -> bool {
    row.get("deletedAt").is_none_or(Value::is_null)
}
