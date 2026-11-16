//! The rules the past-exams forms follow, in one place: which values each
//! form offers and how an upload draft stays consistent as it is edited.
use super::{
    ExamAnswerSource as Source, ExamChoices, ExamError, ExamLimits, ExamOccasion as Occasion,
    ExamProvenance, ExamReportReason, ExamRole as Role, ExamRoleChoice, ExamTeacher,
    ExamUploadDraft, ExamUploadForm, ExamUploadSubject,
};
use std::collections::HashSet;

/// The occasions of an exam material, the default first.
const EXAM_OCCASIONS: [Occasion; 10] = [
    Occasion::Unknown,
    Occasion::Final,
    Occasion::Midterm,
    Occasion::Makeup,
    Occasion::Quiz,
    Occasion::Check,
    Occasion::Report,
    Occasion::Exercise,
    Occasion::Textbook,
    Occasion::Mock,
];

pub(super) fn choices() -> ExamChoices {
    let role = |role| ExamRoleChoice {
        role,
        occasions: match role {
            Role::Notes | Role::Other => vec![Occasion::NotApplicable],
            Role::Question | Role::Answer | Role::Both => EXAM_OCCASIONS.to_vec(),
        },
        answer_sources: match role {
            Role::Answer | Role::Both => vec![Source::Unknown, Source::Official, Source::Student],
            Role::Question | Role::Notes | Role::Other => vec![],
        },
    };
    ExamChoices {
        roles: [Role::Question, Role::Answer, Role::Both, Role::Notes, Role::Other]
            .into_iter()
            .map(role)
            .collect(),
        numbered: vec![Occasion::Quiz, Occasion::Check, Occasion::Report],
        provenances: vec![ExamProvenance::Original, ExamProvenance::Reconstructed],
        request_roles: vec![Role::Question, Role::Answer],
        request_occasions: EXAM_OCCASIONS[1..].to_vec(),
        report_reasons: vec![
            ExamReportReason::Copyright,
            ExamReportReason::Inappropriate,
            ExamReportReason::WrongAnswer,
            ExamReportReason::BrokenFile,
            ExamReportReason::Other,
        ],
        limits: limits(),
    }
}

/// Bring the role's dependent fields in line with what the role allows:
/// a value the role does not offer becomes its default.
pub(super) fn normalize(draft: &mut ExamUploadDraft) {
    let choices = choices();
    let Some(allowed) = choices.roles.iter().find(|choice| choice.role == draft.role) else {
        return;
    };
    if !allowed.occasions.contains(&draft.occasion) {
        draft.occasion = allowed.occasions[0];
    }
    if !choices.numbered.contains(&draft.occasion) {
        draft.occasion_number = 0;
    }
    draft.answer_source = match draft.answer_source {
        Some(source) if allowed.answer_sources.contains(&source) => Some(source),
        _ => allowed.answer_sources.first().copied(),
    };
}

/// The API's own word for an occasion, as requests carry it.
pub(super) fn occasion(value: Option<&str>) -> Occasion {
    value
        .and_then(|value| serde_json::from_value(serde_json::Value::from(value)).ok())
        .filter(|occasion| EXAM_OCCASIONS.contains(occasion))
        .unwrap_or(Occasion::Unknown)
}

pub(super) fn role(value: Option<&str>) -> Role {
    value
        .and_then(|value| serde_json::from_value(serde_json::Value::from(value)).ok())
        .unwrap_or(Role::Question)
}

/// The form's teacher choices and the draft's subjects, after `subjects`
/// changed. A chosen teacher no subject lists any more is dropped.
pub(super) fn settle(form: &mut ExamUploadForm) {
    let mut seen = HashSet::new();
    form.teachers = form
        .subjects
        .iter()
        .flat_map(|entry| &entry.teachers)
        .filter(|teacher| seen.insert(teacher.id.clone()))
        .cloned()
        .collect();
    form.draft.subject_codes = form
        .subjects
        .iter()
        .map(|entry| entry.subject.code.clone())
        .collect();
    form.draft.teacher_ids.retain(|id| seen.contains(id));
    let only = form.subjects.len() == 1;
    let requested = form.request.as_ref().map(|request| request.subject.code.clone());
    for entry in &mut form.subjects {
        entry.removable = !only && requested.as_ref() != Some(&entry.subject.code);
    }
    normalize(&mut form.draft);
}

pub(super) fn add_subject(
    form: &mut ExamUploadForm,
    subject: ExamUploadSubject,
) -> Result<(), ExamError> {
    if form
        .subjects
        .iter()
        .any(|entry| entry.subject.code == subject.subject.code)
    {
        return Ok(());
    }
    if form.subjects.len() >= limits().subjects as usize {
        return Err(ExamError::InvalidInput);
    }
    form.subjects.push(subject);
    settle(form);
    Ok(())
}

pub(super) fn remove_subject(form: &mut ExamUploadForm, code: &str) -> Result<(), ExamError> {
    if !form
        .subjects
        .iter()
        .any(|entry| entry.subject.code == code && entry.removable)
    {
        return Err(ExamError::InvalidInput);
    }
    form.subjects.retain(|entry| entry.subject.code != code);
    settle(form);
    Ok(())
}

/// The App API's limits; the latest year is next year, so it moves.
pub(super) fn limits() -> ExamLimits {
    ExamLimits {
        subjects: 10,
        teachers: 40,
        occasion_number: 30,
        variant: 100,
        note: 40,
        request_description: 1000,
        report_detail: 900,
        query: 100,
        teacher_name: 100,
        earliest_year: 1950,
        latest_year: chrono::Datelike::year(&chrono::Utc::now()) as u32 + 1,
    }
}

/// Whether `year` is one an upload or a request may name.
pub(super) fn valid_year(year: u32) -> bool {
    let limits = limits();
    (limits.earliest_year..=limits.latest_year).contains(&year)
}

/// Teachers once each, in the order the materials first name them.
pub(super) fn distinct_teachers<'a>(
    teachers: impl Iterator<Item = &'a ExamTeacher>,
) -> Vec<ExamTeacher> {
    let mut seen = HashSet::new();
    teachers
        .filter(|teacher| seen.insert(teacher.id.clone()))
        .cloned()
        .collect()
}

/// Where an occasion comes in the order the forms offer them; a word the
/// forms do not know comes last.
pub(super) fn occasion_rank(value: &str) -> usize {
    EXAM_OCCASIONS
        .iter()
        .position(|occasion| {
            serde_json::to_value(occasion).ok().as_ref().and_then(|v| v.as_str()) == Some(value)
        })
        .unwrap_or(EXAM_OCCASIONS.len())
}
