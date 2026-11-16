//! The subject page's read model: which teachers are the person's own, and
//! documents of one sitting (year, occasion, round, variant) side by side, so
//! a question and its answer are one row on every shell.
use super::model::{ExamDocument, ExamSet, ExamSubjectPage, ExamTeacher};
use std::collections::BTreeSet;

/// Fills `my_teachers` and `sets`. `instructor` is the timetable course's
/// instructor text as the syllabus gives it ("小林 隆志 / PARTACHI ..."); the
/// remote side has its own teacher IDs, so the match is by name.
pub(super) fn arrange(page: &mut ExamSubjectPage, instructor: Option<&str>) {
    let wanted: BTreeSet<String> = instructor
        .into_iter()
        .flat_map(|text| text.split(['/', ',', '、', ';', '\n']))
        .map(name_key)
        .filter(|key| !key.is_empty())
        .collect();
    let known = page
        .rows
        .iter()
        .flat_map(|row| &row.teachers)
        .chain(page.exams.iter().flat_map(|exam| &exam.teachers));
    let mut mine: Vec<String> = known
        .filter(|teacher| wanted.contains(&name_key(&teacher.name)))
        .map(|teacher| teacher.id.clone())
        .collect();
    mine.sort_unstable();
    mine.dedup();

    let mut sets: Vec<ExamSet> = Vec::new();
    for exam in &page.exams {
        let same = |set: &&mut ExamSet| {
            set.academic_year == exam.academic_year
                && set.occasion == exam.occasion
                && set.occasion_number == exam.occasion_number
                && set.variant == exam.variant
                && same_teachers(&set.teachers, &exam.teachers)
        };
        match sets.iter_mut().find(same) {
            Some(set) => set.documents.push(exam.clone()),
            None => sets.push(ExamSet {
                academic_year: exam.academic_year,
                occasion: exam.occasion.clone(),
                occasion_number: exam.occasion_number,
                variant: exam.variant.clone(),
                teachers: exam.teachers.clone(),
                mine: exam.teachers.iter().any(|t| mine.contains(&t.id)),
                documents: vec![exam.clone()],
            }),
        }
    }
    for set in &mut sets {
        set.documents.sort_by_key(role_order);
    }
    // Newest first; within a year the usual order of a term's sittings.
    sets.sort_by(|a, b| {
        b.academic_year
            .cmp(&a.academic_year)
            .then_with(|| occasion_order(a).cmp(&occasion_order(b)))
            .then_with(|| a.occasion_number.cmp(&b.occasion_number))
            .then_with(|| a.variant.cmp(&b.variant))
    });
    page.my_teachers = mine;
    page.sets = sets;
}

/// Names compare without spaces (half or full width) or case, since the
/// syllabus and the remote list space Japanese and romanised names differently.
fn name_key(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{3000}')
        .flat_map(char::to_lowercase)
        .collect()
}

fn same_teachers(a: &[ExamTeacher], b: &[ExamTeacher]) -> bool {
    let ids = |list: &[ExamTeacher]| list.iter().map(|t| t.id.clone()).collect::<BTreeSet<_>>();
    ids(a) == ids(b)
}

fn role_order(document: &ExamDocument) -> u8 {
    match document.role.as_deref() {
        Some("question") => 0,
        Some("both") => 1,
        Some("answer") => 2,
        Some("notes") => 3,
        _ => 4,
    }
}

fn occasion_order(set: &ExamSet) -> u8 {
    match set.occasion.as_deref() {
        Some("quiz") => 0,
        Some("check") => 1,
        Some("midterm") => 2,
        Some("final") => 3,
        Some("makeup") => 4,
        _ => 5,
    }
}

#[cfg(test)]
#[path = "../tests/exam_share__sets.rs"]
mod tests;
