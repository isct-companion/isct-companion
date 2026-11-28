//! The past-exams forms: the values they offer, read from the service's
//! `client-config`, and how an upload draft stays consistent as it is edited.
//! The service checks every form; nothing here repeats its rules.
use super::{
    ExamAnswerSource as Source, ExamChoices, ExamError, ExamLimits, ExamOccasion as Occasion,
    ExamReportReason, ExamRole as Role, ExamRoleChoice, ExamTeacher, ExamUploadDraft,
    ExamUploadForm, ExamUploadSubject,
};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::Value;
use std::collections::HashSet;

/// The parts of `client-config` the forms read. A list or limit the service
/// stops sending reads as empty, and the version gate never rests on them.
#[derive(Deserialize)]
struct Choices {
    choices: Codes,
}

#[derive(Deserialize)]
struct Limits {
    #[serde(default)]
    limits: ExamLimits,
}

pub(super) fn limits(config: &Value) -> Result<ExamLimits, ExamError> {
    Limits::deserialize(config)
        .map(|rules| rules.limits)
        .map_err(|_| ExamError::Response)
}

/// The codes `client-config` lists for the forms, as sent.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct Codes {
    pub(crate) upload_occasions: Vec<String>,
    pub(crate) request_occasions: Vec<String>,
    pub(crate) numbered_occasions: Vec<String>,
    pub(crate) roles: Vec<String>,
    pub(crate) provenances: Vec<String>,
    pub(crate) answer_sources: Vec<String>,
    pub(crate) wanted_roles: Vec<String>,
    pub(crate) report_reasons: Vec<String>,
}

/// The codes of `codes` this build has words for, in the service's order.
/// The service may add codes at any time; they show once the app knows them.
fn known<T: DeserializeOwned>(codes: &[String]) -> Vec<T> {
    codes
        .iter()
        .filter_map(|code| serde_json::from_value(Value::from(code.as_str())).ok())
        .collect()
}

/// The forms' choices from a `client-config` answer.
pub(super) fn choices(config: &Value) -> Result<ExamChoices, ExamError> {
    let codes = Choices::deserialize(config)
        .map_err(|_| ExamError::Response)?
        .choices;
    build(codes, limits(config)?)
}

/// The choices for `codes`. A list a form needs that holds nothing this
/// build knows makes the forms unusable: this build is too old for the
/// service.
pub(super) fn build(codes: Codes, limits: ExamLimits) -> Result<ExamChoices, ExamError> {
    let occasions: Vec<Occasion> = known(&codes.upload_occasions);
    let sources: Vec<Source> = known(&codes.answer_sources);
    let roles: Vec<Role> = known(&codes.roles);
    let choices = ExamChoices {
        roles: roles
            .into_iter()
            .map(|role| ExamRoleChoice {
                role,
                // The service's config does not say which occasions a role
                // takes; its upload check does (`examUploadModel.ts`).
                occasions: match role {
                    Role::Notes | Role::Other => vec![Occasion::NotApplicable],
                    Role::Question | Role::Answer | Role::Both => occasions.clone(),
                },
                answer_sources: match role {
                    Role::Answer | Role::Both => sources.clone(),
                    Role::Question | Role::Notes | Role::Other => vec![],
                },
            })
            .collect(),
        numbered: known(&codes.numbered_occasions),
        provenances: known(&codes.provenances),
        request_roles: known(&codes.wanted_roles),
        request_occasions: known(&codes.request_occasions),
        report_reasons: codes
            .report_reasons
            .iter()
            .filter_map(|code| ExamReportReason::from_api_code(code))
            .collect(),
        limits,
    };
    if choices.roles.is_empty()
        || occasions.is_empty()
        || sources.is_empty()
        || choices.provenances.is_empty()
        || choices.report_reasons.is_empty()
    {
        return Err(ExamError::Unsupported);
    }
    Ok(choices)
}

/// Bring the draft's dependent fields in line with its role and occasion,
/// as the website's form does: a role the service does not offer becomes
/// its first; an occasion the role does not take is cleared for the person
/// to pick again, unless the role takes only one; a number only for a
/// numbered occasion; a name only for an `other` occasion or role; an answer
/// source only for an answer, `unknown` until picked; no teachers for a
/// material common to all.
pub(super) fn normalize(draft: &mut ExamUploadDraft, choices: &ExamChoices) {
    let allowed = match choices
        .roles
        .iter()
        .find(|choice| choice.role == draft.role)
    {
        Some(allowed) => allowed,
        None => match choices.roles.first() {
            Some(first) => {
                draft.role = first.role;
                first
            }
            None => return,
        },
    };
    if let [only] = allowed.occasions.as_slice() {
        draft.occasion = Some(*only);
    } else if draft
        .occasion
        .is_some_and(|occasion| !allowed.occasions.contains(&occasion))
    {
        draft.occasion = None;
    }
    if !draft
        .occasion
        .is_some_and(|occasion| choices.numbered.contains(&occasion))
    {
        draft.occasion_number = 0;
    }
    if draft.occasion != Some(Occasion::Other) {
        draft.occasion_text.clear();
    }
    if draft.role != Role::Other {
        draft.role_text.clear();
    }
    draft.answer_source = match draft.answer_source {
        Some(source) if allowed.answer_sources.contains(&source) => Some(source),
        _ if allowed.answer_sources.contains(&Source::Unknown) => Some(Source::Unknown),
        _ => allowed.answer_sources.first().copied(),
    };
    if !choices.provenances.contains(&draft.provenance) {
        if let Some(first) = choices.provenances.first() {
            draft.provenance = *first;
        }
    }
    if draft.all_teachers {
        draft.teacher_ids.clear();
        draft.new_teachers.clear();
    }
}

/// A code the API sends, as this build's value; a code it does not know is
/// none.
pub(super) fn parse_code<T: DeserializeOwned>(value: Option<&str>) -> Option<T> {
    value.and_then(|value| serde_json::from_value(Value::from(value)).ok())
}

/// The form's teacher choices and the draft's subjects, after `subjects`
/// changed. A chosen teacher no subject lists any more is dropped.
pub(super) fn settle(form: &mut ExamUploadForm, choices: &ExamChoices) {
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
    let requested = form
        .request
        .as_ref()
        .map(|request| request.subject.code.clone());
    for entry in &mut form.subjects {
        entry.removable = !only && requested.as_ref() != Some(&entry.subject.code);
    }
    normalize(&mut form.draft, choices);
}

pub(super) fn add_subject(
    form: &mut ExamUploadForm,
    subject: ExamUploadSubject,
    choices: &ExamChoices,
) {
    if form
        .subjects
        .iter()
        .any(|entry| entry.subject.code == subject.subject.code)
    {
        return;
    }
    form.subjects.push(subject);
    settle(form, choices);
}

pub(super) fn remove_subject(
    form: &mut ExamUploadForm,
    code: &str,
    choices: &ExamChoices,
) -> Result<(), ExamError> {
    if !form
        .subjects
        .iter()
        .any(|entry| entry.subject.code == code && entry.removable)
    {
        return Err(ExamError::InvalidInput);
    }
    form.subjects.retain(|entry| entry.subject.code != code);
    settle(form, choices);
    Ok(())
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
