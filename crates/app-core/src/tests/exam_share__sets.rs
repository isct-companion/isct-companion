use super::*;
use crate::exam_share::model::ExamTeacher;

fn document(id: &str, year: u32, occasion: &str, role: &str, teachers: &[&str]) -> ExamDocument {
    ExamDocument {
        id: id.into(),
        subjects: Vec::new(),
        teachers: teachers
            .iter()
            .map(|id| ExamTeacher {
                id: (*id).into(),
                name: format!("T{id}"),
            })
            .collect(),
        academic_year: Some(year),
        occasion: Some(occasion.into()),
        occasion_number: 1,
        variant: String::new(),
        role: Some(role.into()),
        note: None,
        file_name: format!("{id}.pdf"),
        file_type: "application/pdf".into(),
        number: None,
    }
}

/// A question and its answer from one sitting share a row, question first,
/// whatever order the service lists them in; another year, occasion or
/// teacher is another row. The class's own sittings come first, then newest
/// first, a year's sittings in the forms' occasion order. Several documents
/// of one role in a sitting (scanned pages) are numbered so every shell can
/// tell them apart; a role held once is not.
#[test]
fn documents_of_one_sitting_share_a_row() {
    let exams = [
        document("o", 2026, "final", "question", &["t2"]),
        document("b", 2024, "final", "question", &["t1"]),
        document("a", 2025, "final", "answer", &["t1"]),
        document("q", 2025, "final", "question", &["t1"]),
        document("p", 2025, "final", "question", &["t1"]),
        document("m", 2025, "midterm", "question", &["t1"]),
    ];
    let ours = HashSet::from(["t1".to_string()]);
    let sets = group(exams.into(), &ours);
    let rows: Vec<(Vec<(&str, Option<u32>)>, bool)> = sets
        .iter()
        .map(|set| {
            (
                set.documents
                    .iter()
                    .map(|d| (d.id.as_str(), d.number))
                    .collect(),
                set.mine,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (vec![("q", Some(1)), ("p", Some(2)), ("a", None)], true),
            (vec![("m", None)], true),
            (vec![("b", None)], true),
            (vec![("o", None)], false),
        ]
    );
}
