use super::*;
use crate::exam_share::model::ExamTeacher;

fn document(id: &str, year: u32, occasion: &str, role: &str, teachers: &[&str]) -> ExamDocument {
    ExamDocument {
        id: id.into(),
        subjects: Vec::new(),
        teachers: teachers
            .iter()
            .map(|id| ExamTeacher { id: (*id).into(), name: format!("T{id}") })
            .collect(),
        academic_year: Some(year),
        occasion: Some(occasion.into()),
        occasion_number: 1,
        variant: String::new(),
        role: Some(role.into()),
        note: None,
        file_name: format!("{id}.pdf"),
        file_type: "application/pdf".into(),
    }
}

/// A question and its answer from one sitting share a row, question first,
/// whatever order the service lists them in; another year, occasion or
/// teacher is another row, and rows keep the order the page sorted them in
/// (the class's own teachers first).
#[test]
fn documents_of_one_sitting_share_a_row() {
    let exams = [
        document("a", 2025, "final", "answer", &["t1"]),
        document("b", 2024, "final", "question", &["t1"]),
        document("q", 2025, "final", "question", &["t1"]),
        document("m", 2025, "midterm", "question", &["t1"]),
        document("o", 2025, "final", "question", &["t2"]),
    ];
    let ours = HashSet::from(["t1".to_string()]);
    let sets = group(&exams, &ours);
    let rows: Vec<(Vec<&str>, bool)> = sets
        .iter()
        .map(|set| (set.documents.iter().map(|d| d.id.as_str()).collect(), set.mine))
        .collect();
    assert_eq!(
        rows,
        [
            (vec!["q", "a"], true),
            (vec!["b"], true),
            (vec!["m"], true),
            (vec!["o"], false),
        ]
    );
}
