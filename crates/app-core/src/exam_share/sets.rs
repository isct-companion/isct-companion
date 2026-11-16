//! The subject page's sittings: documents of one year, occasion, round,
//! variant and set of teachers side by side, so a question and its answer are
//! one row on every shell.
use super::model::{ExamDocument, ExamSet};
use std::collections::{BTreeSet, HashSet};

/// Groups `exams` by sitting in the order they come. `ours` holds the IDs of
/// the course's own class's teachers.
pub(super) fn group(exams: &[ExamDocument], ours: &HashSet<String>) -> Vec<ExamSet> {
    let mut sets: Vec<ExamSet> = Vec::new();
    for exam in exams {
        let teachers = teacher_ids(exam);
        let sitting = sets.iter_mut().find(|set| {
            set.academic_year == exam.academic_year
                && set.occasion == exam.occasion
                && set.occasion_number == exam.occasion_number
                && set.variant == exam.variant
                && set.teachers.iter().map(|t| t.id.as_str()).collect::<BTreeSet<_>>() == teachers
        });
        match sitting {
            Some(set) => set.documents.push(exam.clone()),
            None => sets.push(ExamSet {
                academic_year: exam.academic_year,
                occasion: exam.occasion.clone(),
                occasion_number: exam.occasion_number,
                variant: exam.variant.clone(),
                teachers: exam.teachers.clone(),
                mine: exam.teachers.iter().any(|t| ours.contains(&t.id)),
                documents: vec![exam.clone()],
            }),
        }
    }
    for set in &mut sets {
        set.documents.sort_by_key(|document| role_order(document.role.as_deref()));
    }
    sets
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
