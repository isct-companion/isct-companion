//! A course's LMS content lesson by lesson, as the desktop Lecture Desk lays
//! it out (`LmsWorkspace.tsx`, `lessons.ts`): each section's own text, its
//! files and links, and the pages, labels and announcements shown inline.
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::course_detail::lms_record;
use super::lms_html::safe_html;
use super::{array, text};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CourseContent {
    pub course_id: String,
    /// Sections with anything in them, in section order.
    pub lessons: Vec<LessonContent>,
    /// The section to open first: the last numbered lesson with material
    /// (teachers post around class, so a cancelled class does not advance
    /// it); in a course without numbered lessons, the last section with any.
    pub current_section: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LessonContent {
    /// The LMS section number (0 is the course's general section).
    pub section: i32,
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

pub fn course_content(workspace: &Value, course_id: &str) -> Option<CourseContent> {
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
    let current = if numbered >= 2 {
        lessons
            .iter()
            .rev()
            .find(|lesson| lesson.number.is_some() && lesson.posted)
            .or_else(|| lessons.iter().find(|lesson| lesson.number.is_some()))
    } else {
        lessons
            .iter()
            .rev()
            .find(|lesson| lesson.posted)
            .or(lessons.first())
    };
    Some(CourseContent {
        course_id: course_id.into(),
        current_section: current.map(|lesson| lesson.section),
        lessons,
    })
}

fn visible(row: &Value) -> bool {
    row.get("deletedAt").is_none_or(Value::is_null)
}
