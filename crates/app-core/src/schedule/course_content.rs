//! A course's LMS content lesson by lesson, as every shell lays it out (the
//! desktop Lecture Desk, the mobile course page): each section's own text,
//! its files and links, the pages, labels and announcements shown inline, and
//! the lesson rail with its labels and where the course is.
use super::SchoolReads;
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::course_detail::lms_record;
use super::lms_html::{image_only, lesson_summary, mark_schedule, safe_html};
use super::occurrence::course_timeline;
use super::{array, text};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CourseContent {
    pub course_id: String,
    /// Sections with anything in them, and every lesson of a course that
    /// numbers its lessons, in section order.
    pub lessons: Vec<LessonContent>,
    /// The course's assignments set outside any section (their LMS activity
    /// is not among the materials).
    pub other_assignment_ids: Vec<String>,
    /// Where the course is, as the lesson rail marks it: the lesson the
    /// person last marked for a class, or the one after it once a later
    /// class has begun. Without a mark, the last numbered lesson with
    /// material (teachers post around class, so a cancelled class does not
    /// advance it); in a course without numbered lessons, the last section
    /// with any.
    pub current_section: Option<i32>,
    /// The class a "this class" mark is recorded against: the one in
    /// progress or held earlier today, else the next one. `None` when the
    /// course has no meetings.
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
    /// course numbers at least two of its sections. Set exactly on the
    /// lessons of the rail.
    pub number: Option<u32>,
    /// What the rail calls the section, shown as is (shells only truncate it
    /// to fit): a lesson of the course's main series by its number, one of
    /// another series by that series' own word and number (演習 3), a series
    /// whose numbers skip or repeat by its order; any other section by its
    /// own title.
    pub label: String,
    /// The section's own text (授業計画 and the like), made safe to render,
    /// with its lesson schedule tables marked against the current lesson:
    /// earlier rows `lmsRowPast`, its row `lmsRowNow`, cancelled rows
    /// `lmsRowOff`.
    pub summary_html: String,
    /// The first line of that text without a short leading label
    /// (授業計画：, Course schedule:), which heads a folded lesson.
    pub summary: String,
    pub items: Vec<LessonItem>,
    /// The assignments set in this section, which have pages of their own
    /// (`AssignmentDetail`).
    pub assignment_ids: Vec<String>,
    /// Whether the section has material besides assignments and labels: a
    /// label alone ("posted after class") is not the lesson's material.
    pub posted: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LessonItem {
    pub material_id: String,
    pub kind: LessonItemKind,
    pub title: String,
    /// The text shown in place, for pages, labels and announcements, with
    /// schedule tables marked as in `LessonContent.summary_html`.
    pub body_html: Option<String>,
    /// The LMS page or file Core resolves and downloads.
    pub source: Option<String>,
    /// A label of LMS pictures only, which shells lay side by side with the
    /// labels like it next to it rather than one block each.
    pub image_only: bool,
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

/// A numbered section title: its lesson number and the series it counts.
pub(super) struct LessonTitle {
    pub(super) number: u32,
    /// The title up to its number with the number taken out, without
    /// whitespace or case: 第回, 演習第回, lecture. Lectures and recitations
    /// numbered side by side (第3回, 演習第3回) are two series, told apart
    /// by the teacher's own words rather than by a list of them.
    series: String,
    /// How the series names itself: the teacher's text before the number
    /// (演習 in 演習第3回), else the words around it (Week in Week 3).
    name: String,
}

/// `第3回`, `Lecture 3`, `Week 3`, `#3.`, on NFKC text so full-width digits
/// count.
pub(super) fn lesson_title(title: &str) -> Option<LessonTitle> {
    use unicode_normalization::UnicodeNormalization;
    static LESSON: std::sync::LazyLock<Result<regex::Regex, regex::Error>> =
        std::sync::LazyLock::new(|| {
            regex::Regex::new(
                r"(?i)第\s*([0-9]+)\s*回|(?:lecture|lesson|class|week|session|day)\s*#?\s*([0-9]+)\b|^\s*#?\s*([0-9]+)\s*(?:[.:、]|$)",
            )
        });
    let normalized: String = title.nfkc().collect();
    let captures = LESSON.as_ref().ok()?.captures(&normalized)?;
    let whole = captures.get(0)?;
    let digits = (1..=3).find_map(|group| captures.get(group))?;
    let prefix = normalized.get(..whole.start())?.trim();
    let around = format!(
        "{}{}",
        normalized.get(whole.start()..digits.start())?,
        normalized.get(digits.end()..whole.end())?
    );
    let name = match prefix {
        "" => around.trim(),
        prefix => prefix,
    };
    Some(LessonTitle {
        number: digits.as_str().parse().ok()?,
        series: format!("{prefix}{around}")
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect(),
        name: name.to_owned(),
    })
}

/// The rail's labels, section by section (`None` off the rail). The series
/// with the most lessons (the first to appear on a tie) is the course's own
/// count and shows bare numbers; any other shows its name and number as the
/// teacher wrote them (演習 3), never translated. A series whose numbers do
/// not count up by one (7, 9, 8, 11, 11) is numbered by its order instead.
fn rail_labels(titles: &[Option<LessonTitle>]) -> Vec<Option<String>> {
    let mut series: Vec<(&str, Vec<&LessonTitle>, Vec<usize>)> = Vec::new();
    for (index, title) in titles.iter().enumerate() {
        let Some(title) = title else { continue };
        match series.iter_mut().find(|(key, ..)| *key == title.series) {
            Some((_, members, indices)) => {
                members.push(title);
                indices.push(index);
            }
            None => series.push((title.series.as_str(), vec![title], vec![index])),
        }
    }
    let largest = series.iter().map(|(_, members, _)| members.len()).max();
    let main = series
        .iter()
        .position(|(_, members, _)| Some(members.len()) == largest);
    let mut labels = vec![None; titles.len()];
    for (rank, (_, members, indices)) in series.iter().enumerate() {
        let counts = members
            .windows(2)
            .all(|pair| pair[0].number.checked_add(1) == Some(pair[1].number));
        for (order, (title, index)) in members.iter().zip(indices).enumerate() {
            let number = if counts {
                title.number
            } else {
                u32::try_from(order + 1).unwrap_or(u32::MAX)
            };
            labels[*index] = Some(if Some(rank) == main || title.name.is_empty() {
                number.to_string()
            } else {
                format!("{} {number}", title.name)
            });
        }
    }
    labels
}

fn section_of(row: &Value) -> i32 {
    row.get("sectionNumber")
        .and_then(Value::as_i64)
        .or_else(|| row.get("session").and_then(Value::as_i64))
        .unwrap_or_default() as i32
}

/// The course's syllabus link among its materials, which the course page
/// offers on its own rather than as course content.
fn syllabus(row: &Value) -> bool {
    let title = text(row, "title").unwrap_or_default().trim().to_lowercase();
    title == "シラバス"
        || title == "syllabus"
        || text(row, "resourceUrl")
            .is_some_and(|url| url.starts_with("https://syllabus.s.isct.ac.jp/"))
}

/// The section a material row sits in: the one with its section id, else
/// its section number; a section the LMS record lacks is added from the row.
fn place(lessons: &mut Vec<(Option<u64>, LessonContent)>, row: &Value) -> usize {
    let section = section_of(row);
    let section_id = row.get("sectionId").and_then(Value::as_u64);
    if let Some(index) = lessons
        .iter()
        .position(|(id, lesson)| match (section_id, id) {
            (Some(section_id), Some(id)) => section_id == *id,
            _ => lesson.section == section,
        })
    {
        return index;
    }
    lessons.push((
        section_id,
        LessonContent {
            section,
            section_id,
            title: text(row, "sectionTitle").map(str::to_owned),
            number: None,
            label: String::new(),
            summary_html: String::new(),
            summary: String::new(),
            items: Vec::new(),
            assignment_ids: Vec::new(),
            posted: false,
        },
    ));
    lessons.len() - 1
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
                && !syllabus(row)
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
                label: String::new(),
                summary_html: text(section, "bodyHtml")
                    .map(|html| safe_html(html, false))
                    .unwrap_or_default(),
                summary: text(section, "bodyHtml")
                    .map(lesson_summary)
                    .unwrap_or_default(),
                items: Vec::new(),
                assignment_ids: Vec::new(),
                posted: false,
            },
        ));
    }
    for row in &rows {
        let index = place(&mut lessons, row);
        let lesson = &mut lessons[index].1;
        let module = text(row, "module");
        // Assignments and quizzes have pages of their own (AssignmentDetail).
        if matches!(module, Some("assign" | "quiz")) {
            continue;
        }
        let kind = kind_of(module);
        lesson.posted |= kind != LessonItemKind::Label;
        let inline = matches!(
            kind,
            LessonItemKind::Page | LessonItemKind::Label | LessonItemKind::Forum
        );
        let body = text(row, "bodyHtml");
        lesson.items.push(LessonItem {
            material_id: text(row, "id").unwrap_or_default().into(),
            kind,
            title: text(row, "title").unwrap_or_default().into(),
            body_html: inline
                .then(|| body.map(|html| safe_html(html, true)))
                .flatten()
                .filter(|html| !html.is_empty()),
            source: text(row, "url")
                .or_else(|| text(row, "resourceUrl"))
                .map(str::to_owned),
            image_only: kind == LessonItemKind::Label && body.is_some_and(image_only),
        });
    }
    // Each assignment sits where its LMS activity does.
    let mut other_assignment_ids = Vec::new();
    for assignment in array(workspace, "assignments").iter().filter(|row| {
        visible(row)
            && text(row, "courseId") == Some(course_id)
            && row.get("hidden").and_then(Value::as_bool) != Some(true)
    }) {
        let Some(id) = text(assignment, "id") else {
            continue;
        };
        let source = assignment
            .get("lmsCmid")
            .and_then(Value::as_u64)
            .and_then(|cmid| {
                rows.iter()
                    .find(|row| row.get("cmid").and_then(Value::as_u64) == Some(cmid))
            });
        match source {
            Some(row) => {
                let index = place(&mut lessons, row);
                lessons[index].1.assignment_ids.push(id.into());
            }
            None => other_assignment_ids.push(id.into()),
        }
    }
    let mut lessons: Vec<LessonContent> = lessons.into_iter().map(|(_, lesson)| lesson).collect();
    lessons.sort_by_key(|lesson| lesson.section);
    // Lessons are numbered and labelled over every visible section, so a
    // lesson with nothing posted yet keeps its place in the count.
    let titles: Vec<Option<LessonTitle>> = lessons
        .iter()
        .map(|lesson| lesson.title.as_deref().and_then(lesson_title))
        .collect();
    let lesson_course = titles.iter().flatten().count() >= 2;
    let labels = if lesson_course {
        rail_labels(&titles)
    } else {
        vec![None; titles.len()]
    };
    for ((lesson, title), label) in lessons.iter_mut().zip(&titles).zip(labels) {
        lesson.number = label.as_ref().and(title.as_ref()).map(|title| title.number);
        lesson.label = label.unwrap_or_else(|| {
            lesson
                .title
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map_or_else(|| lesson.section.to_string(), str::to_owned)
        });
    }
    lessons.retain(|lesson| {
        lesson.number.is_some()
            || !lesson.summary_html.is_empty()
            || !lesson.items.is_empty()
            || !lesson.assignment_ids.is_empty()
    });
    // The rail: numbered lessons in a lesson course, else every section.
    let rail: Vec<&LessonContent> = lessons
        .iter()
        .filter(|lesson| !lesson_course || lesson.number.is_some())
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
        if lesson_course {
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
    let current_section = current.map(|lesson| lesson.section);
    // Schedule tables are marked against the lesson the course is at.
    let current_number = current.and_then(|lesson| lesson.number);
    for lesson in &mut lessons {
        lesson.summary_html = mark_schedule(&lesson.summary_html, current_number);
        for html in lesson
            .items
            .iter_mut()
            .filter_map(|item| item.body_html.as_mut())
        {
            *html = mark_schedule(html, current_number);
        }
    }
    Some(CourseContent {
        course_id: course_id.into(),
        current_section,
        class_at: timeline.and_then(|timeline| timeline.class_at),
        lessons,
        other_assignment_ids,
    })
}

use super::not_deleted as visible;
