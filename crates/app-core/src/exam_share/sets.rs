//! The subject page's sittings: documents of one year, occasion, round,
//! variant and set of teachers side by side, so a question and its answer are
//! one row on every shell.
use super::form::occasion_rank;
use super::model::{ExamDocument, ExamSet};
use std::cmp::Reverse;
use std::collections::{BTreeSet, HashSet};

/// Groups `exams` by sitting: the course's own class's sittings first, then
/// newest first, a year's sittings in the forms' occasion order, a sitting
/// with no year last. `ours` holds the IDs of the class's teachers. Several
/// documents of one role in a sitting are numbered in their order.
pub(super) fn group(exams: Vec<ExamDocument>, ours: &HashSet<String>) -> Vec<ExamSet> {
    let mut sets: Vec<ExamSet> = Vec::new();
    for exam in exams {
        let teachers = teacher_ids(&exam);
        let sitting = sets.iter_mut().find(|set| {
            set.academic_year == exam.academic_year
                && set.occasion == exam.occasion
                && set.occasion_number == exam.occasion_number
                && set.variant == exam.variant
                && set
                    .teachers
                    .iter()
                    .map(|t| t.id.as_str())
                    .collect::<BTreeSet<_>>()
                    == teachers
        });
        match sitting {
            Some(set) => set.documents.push(exam),
            None => sets.push(ExamSet {
                academic_year: exam.academic_year,
                occasion: exam.occasion.clone(),
                occasion_number: exam.occasion_number,
                variant: exam.variant.clone(),
                teachers: exam.teachers.clone(),
                mine: exam.teachers.iter().any(|t| ours.contains(&t.id)),
                documents: vec![exam],
            }),
        }
    }
    for set in &mut sets {
        set.documents.sort_by(|a, b| {
            (role_order(a.role.as_deref()), &a.role).cmp(&(role_order(b.role.as_deref()), &b.role))
        });
        number_repeated_roles(&mut set.documents);
    }
    sets.sort_by_key(|set| {
        (
            !set.mine,
            set.academic_year.is_none(),
            Reverse(set.academic_year),
            set.occasion.as_deref().map_or(usize::MAX, occasion_rank),
            set.occasion_number,
            set.variant.clone(),
        )
    });
    sets
}

/// `documents` sorted by role: a role held by several gets 1, 2, … in order.
fn number_repeated_roles(documents: &mut [ExamDocument]) {
    for run in documents.chunk_by_mut(|a, b| a.role == b.role) {
        if run.len() > 1 {
            for (number, document) in (1..).zip(run.iter_mut()) {
                document.number = Some(number);
            }
        }
    }
}

fn teacher_ids(exam: &ExamDocument) -> BTreeSet<&str> {
    exam.teachers.iter().map(|t| t.id.as_str()).collect()
}

fn role_order(role: Option<&str>) -> u8 {
    match role {
        Some("question") => 0,
        Some("both") => 1,
        Some("answer") => 2,
        Some("notes") => 3,
        _ => 4,
    }
}

#[cfg(test)]
#[path = "../tests/exam_share__sets.rs"]
mod tests;
