use super::{array, text, ExamView};
use crate::course_materials::CourseKey;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

/// IDs project the authoritative snapshot without copying cached school records.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CourseDetail {
    pub material_ids: Vec<String>,
    pub sections: Vec<i32>,
    pub assignment_ids: Vec<String>,
    pub exam_ids: Vec<String>,
    pub exams: Vec<ExamView>,
    pub offering_id: Option<String>,
    /// The course's T2SCHOLA page, from the course itself or its LMS record.
    pub lms_url: Option<String>,
    /// The course's LMS id, which the course page sends in its `Open` hint.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub lms_course_id: Option<u64>,
    pub syllabus_url: Option<String>,
    /// Where the course's downloaded files live, for opening its materials.
    pub files: Option<CourseKey>,
}

const LMS_HOST: &str = "lms.s.isct.ac.jp";

fn lms_url(workspace: &Value, course: &Value) -> Option<String> {
    let provider = course.pointer("/source/provider").and_then(Value::as_str);
    if let Some(url) =
        text(course, "url").filter(|url| provider == Some("lms") && url.contains(LMS_HOST))
    {
        return Some(url.into());
    }
    lms_record(workspace, course)
        .and_then(|row| text(row, "url"))
        .map(str::to_owned)
}

/// The course's LMS id: its LMS record's, or an LMS-sourced course's own.
fn lms_course_id(workspace: &Value, course: &Value) -> Option<u64> {
    lms_record(workspace, course)
        .and_then(|row| row.get("lmsId").and_then(Value::as_u64))
        .or_else(|| {
            let provider = course.pointer("/source/provider").and_then(Value::as_str);
            let external = course.pointer("/source/externalId").and_then(Value::as_str);
            external
                .filter(|_| provider == Some("lms"))
                .and_then(|id| id.parse().ok())
        })
}

/// The course's LMS record (school records' `lmsCourses`), by offering or by
/// the LMS id an LMS-only course carries.
pub(super) fn lms_record<'a>(workspace: &'a Value, course: &Value) -> Option<&'a Value> {
    let provider = course.pointer("/source/provider").and_then(Value::as_str);
    let offering = text(course, "offeringId");
    let external = course.pointer("/source/externalId").and_then(Value::as_str);
    array(workspace, "lmsCourses").iter().find(|row| {
        (offering.is_some() && text(row, "offeringId") == offering)
            || (provider == Some("lms")
                && row
                    .get("lmsId")
                    .and_then(Value::as_i64)
                    .is_some_and(|id| Some(id.to_string().as_str()) == external))
    })
}

/// The course's シラバス page in the content language, from its offering or
/// a シラバス link the course or its materials carry.
fn syllabus_url(workspace: &Value, course: &Value, materials: &[&Value]) -> Option<String> {
    let linked = || {
        std::iter::once(text(course, "url"))
            .chain(materials.iter().map(|row| text(row, "resourceUrl")))
            .flatten()
            .find_map(crate::catalog::syllabus_offering)
    };
    text(course, "offeringId")
        .map(str::to_owned)
        .or_else(linked)
        .map(|id| crate::catalog::syllabus_url(&id, super::content_language(workspace)))
}

pub fn course_detail(workspace: &Value, course_id: &str) -> CourseDetail {
    let Some(course) = array(workspace, "courses")
        .iter()
        .find(|row| visible(row) && text(row, "id") == Some(course_id))
    else {
        return CourseDetail::default();
    };
    let mut materials: Vec<_> = array(workspace, "materials")
        .iter()
        .filter(|row| {
            visible(row)
                && text(row, "courseId") == Some(course_id)
                && row.get("visible").and_then(Value::as_bool) != Some(false)
                && !matches!(text(row, "module"), Some("assign" | "quiz"))
        })
        .collect();
    let syllabus_url = syllabus_url(workspace, course, &materials);
    let lms_url = lms_url(workspace, course);
    let section = |row: &&Value| {
        row.get("sectionNumber")
            .and_then(Value::as_i64)
            .or_else(|| row.get("session").and_then(Value::as_i64))
            .unwrap_or_default() as i32
    };
    materials.sort_by_key(section);
    let mut sections: Vec<_> = materials.iter().map(section).collect();
    sections.dedup();
    let mut assignments: Vec<_> = array(workspace, "assignments")
        .iter()
        .filter(|row| {
            visible(row)
                && text(row, "courseId") == Some(course_id)
                && row.get("hidden").and_then(Value::as_bool) != Some(true)
        })
        .collect();
    assignments.sort_by_key(|row| text(row, "dueAt").unwrap_or("~"));
    let exams: Vec<_> = super::course_terms(workspace, course)
        .iter()
        .flat_map(|term| super::exam_views(workspace, term))
        .filter(|exam| exam.course_id.as_deref() == Some(course_id))
        .collect();
    let ids = |rows: Vec<&Value>| {
        rows.iter()
            .filter_map(|row| text(row, "id").map(str::to_owned))
            .collect()
    };
    CourseDetail {
        material_ids: ids(materials),
        sections,
        assignment_ids: ids(assignments),
        exam_ids: exams
            .iter()
            .filter(|exam| exam.kind == "exam" && !exam.hidden && exam.status == "scheduled")
            .map(|exam| exam.id.clone())
            .collect(),
        exams,
        offering_id: array(workspace, "offerings")
            .iter()
            .find(|row| visible(row) && text(row, "id") == text(course, "offeringId"))
            .and_then(|row| text(row, "id"))
            .map(str::to_owned),
        lms_url,
        lms_course_id: lms_course_id(workspace, course),
        syllabus_url,
        files: files_of(course),
    }
}

pub(super) fn files_of(course: &Value) -> Option<CourseKey> {
    text(course, "termId").map(|term| CourseKey {
        term: crate::storage::folder_name(term),
        course: crate::storage::folder_name(
            text(course, "code")
                .filter(|code| !code.is_empty())
                .or_else(|| text(course, "id"))
                .unwrap_or_default(),
        ),
    })
}

/// One assignment as the course page shows it: state, the teacher's text and
/// the files it links, read from LMS without opening the LMS page.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AssignmentDetail {
    pub assignment_id: String,
    pub course_id: String,
    pub title: String,
    pub due_at: Option<String>,
    pub submission: Option<String>,
    pub submission_status: Option<String>,
    pub grade: Option<String>,
    /// The LMS page, where the person reviews and submits.
    pub source_url: Option<String>,
    /// The description as LMS shows it, without Moodle's own date lines,
    /// which the page states already.
    pub body_html: String,
    /// Notes the assignment row carries besides the LMS text.
    pub details: String,
    pub attachments: Vec<AssignmentAttachment>,
    pub section_number: Option<i32>,
    pub section_title: Option<String>,
    /// Where opened attachments are kept.
    pub files: Option<CourseKey>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AssignmentAttachment {
    pub name: String,
    pub url: String,
}

/// The rules the desktop assignment card applied in the shell
/// (`LmsWorkspace.tsx`): its text is the LMS module row with the same cmid.
pub fn assignment_detail(workspace: &Value, assignment_id: &str) -> Option<AssignmentDetail> {
    let assignment = array(workspace, "assignments")
        .iter()
        .find(|row| visible(row) && text(row, "id") == Some(assignment_id))?;
    let course_id = text(assignment, "courseId").unwrap_or_default();
    let cmid = assignment.get("lmsCmid").and_then(Value::as_u64);
    let source = cmid.and_then(|cmid| {
        array(workspace, "materials")
            .iter()
            .find(|row| visible(row) && row.get("cmid").and_then(Value::as_u64) == Some(cmid))
    });
    let raw = source
        .and_then(|row| text(row, "bodyHtml"))
        .unwrap_or_default();
    let course = array(workspace, "courses")
        .iter()
        .find(|row| visible(row) && text(row, "id") == Some(course_id));
    let owned = |key: &str| text(assignment, key).map(str::to_owned);
    Some(AssignmentDetail {
        assignment_id: assignment_id.into(),
        course_id: course_id.into(),
        title: text(assignment, "title").unwrap_or_default().into(),
        due_at: owned("dueAt"),
        submission: owned("submission"),
        submission_status: owned("submissionStatus"),
        grade: owned("grade"),
        source_url: owned("sourceUrl"),
        attachments: attachments(raw),
        body_html: super::lms_html::safe_html(raw, true),
        details: text(assignment, "details").unwrap_or_default().into(),
        section_number: source
            .and_then(|row| row.get("sectionNumber").or_else(|| row.get("session")))
            .and_then(Value::as_i64)
            .and_then(|number| i32::try_from(number).ok()),
        section_title: source
            .and_then(|row| text(row, "sectionTitle"))
            .map(str::to_owned),
        files: course.and_then(files_of),
    })
}

const LMS_FILE_HOST: &str = "lms.s.isct.ac.jp";

/// LMS files the description links (`pluginfile.php`), once each, named by
/// their file name.
fn attachments(html: &str) -> Vec<AssignmentAttachment> {
    let Ok(links) = scraper::Selector::parse(r#"a[href*="/pluginfile.php/"]"#) else {
        return Vec::new();
    };
    let page = scraper::Html::parse_fragment(html);
    let mut found: Vec<AssignmentAttachment> = Vec::new();
    for link in page.select(&links) {
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        let Ok(url) = url::Url::parse(href) else {
            continue;
        };
        if url.scheme() != "https"
            || url.host_str() != Some(LMS_FILE_HOST)
            || found.iter().any(|known| known.url == href)
        {
            continue;
        }
        let file = url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .map(|segment| {
                percent_encoding::percent_decode_str(segment)
                    .decode_utf8_lossy()
                    .into_owned()
            })
            .filter(|name| !name.is_empty());
        let label = link.text().collect::<String>().trim().to_owned();
        found.push(AssignmentAttachment {
            name: file
                .or((!label.is_empty()).then_some(label))
                .unwrap_or_else(|| "attachment".into()),
            url: href.into(),
        });
    }
    found
}

// Course details retain cached rows until tombstoned; exam freshness is checked separately.
fn visible(row: &Value) -> bool {
    row.get("deletedAt").is_none_or(Value::is_null)
}
