//! The subject page's sittings: documents the service pairs (its `pairKey`)
//! side by side, so a question and its answer are one row on every shell.
use super::model::{ExamDocument, ExamSet};
use std::cmp::Reverse;
use std::collections::HashSet;

/// The occasions in the order a year's sittings are shown. Display only: an
/// occasion this build does not know comes last.
const OCCASION_ORDER: [&str; 5] = ["final", "midterm", "quiz", "report", "other"];

/// Whether `exam` belongs to the class taught by `teachers`, as the
/// service's `examTaughtBy` decides it: a class with no teachers listed
/// takes every material, and a material common to all teachers belongs to
/// every class.
fn taught_by(exam: &ExamDocument, teachers: &HashSet<String>) -> bool {
    teachers.is_empty()
        || exam.all_teachers
        || exam.teachers.iter().any(|t| teachers.contains(&t.id))
}

/// Groups `exams`, each with the service's `pairKey`, into sittings: those
/// of the page's class (`class` holds its teachers' IDs) first, then newest first, a year's sittings in occasion order,
/// a sitting with no year last. A document with no key is a sitting of its
/// own. Several documents of one role (and role name) in a sitting are
/// numbered in their order.
pub(super) fn group(
    exams: Vec<(Option<String>, ExamDocument)>,
    class: Option<&HashSet<String>>,
) -> Vec<ExamSet> {
    let mut sets: Vec<(Option<String>, ExamSet)> = Vec::new();
    for (key, exam) in exams {
        let paired = key.as_ref().and_then(|key| {
            sets.iter_mut()
                .find(|(other, _)| other.as_ref() == Some(key))
        });
        match paired {
            Some((_, set)) => set.documents.push(exam),
            None => {
                let set = ExamSet {
                    academic_year: exam.academic_year,
                    occasion: exam.occasion.clone(),
                    occasion_text: exam.occasion_text.clone(),
                    occasion_number: exam.occasion_number,
                    variant: exam.variant.clone(),
                    teachers: exam.teachers.clone(),
                    all_teachers: exam.all_teachers,
                    mine: class.is_some_and(|teachers| taught_by(&exam, teachers)),
                    documents: vec![exam],
                };
                sets.push((key, set));
            }
        }
    }
    let mut sets: Vec<ExamSet> = sets.into_iter().map(|(_, set)| set).collect();
    for set in &mut sets {
        set.documents.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        number_repeated_roles(&mut set.documents);
    }
    sets.sort_by_key(|set| {
        (
            !set.mine,
            set.academic_year.is_none(),
            Reverse(set.academic_year),
            set.occasion
                .as_deref()
                .and_then(|occasion| OCCASION_ORDER.iter().position(|known| *known == occasion))
                .unwrap_or(OCCASION_ORDER.len()),
            set.occasion_text.clone(),
            set.occasion_number,
            set.variant.clone(),
        )
    });
    sets
}

fn sort_key(document: &ExamDocument) -> (u8, Option<&str>, &str) {
    (
        role_order(document.role.as_deref()),
        document.role.as_deref(),
        &document.role_text,
    )
}

/// `documents` sorted by role: a role held by several gets 1, 2, … in order.
fn number_repeated_roles(documents: &mut [ExamDocument]) {
    for run in documents.chunk_by_mut(|a, b| a.role == b.role && a.role_text == b.role_text) {
        if run.len() > 1 {
            for (number, document) in (1..).zip(run.iter_mut()) {
                document.number = Some(number);
            }
        }
    }
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
