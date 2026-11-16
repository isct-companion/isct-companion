use super::*;
use crate::exam_share::model::{ExamClass, ExamSubject};

fn teacher(id: &str, name: &str) -> ExamTeacher {
    ExamTeacher {
        id: id.into(),
        name: name.into(),
    }
}

fn document(
    id: &str,
    year: u32,
    occasion: &str,
    role: &str,
    teachers: &[ExamTeacher],
) -> ExamDocument {
    ExamDocument {
        id: id.into(),
        subjects: Vec::new(),
        teachers: teachers.to_vec(),
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

/// The timetable spells names with a full-width space and in another case;
/// a question and its answer of one sitting become one set, newest first,
/// question before answer, and a sitting by another teacher stays apart.
#[test]
fn arranges_sets_and_matches_own_teachers_by_name() {
    let kobayashi = teacher("t1", "小林 隆志");
    let moriguchi = teacher("t2", "森口 草介");
    let other = teacher("t3", "宮崎 純");
    let pair = [kobayashi.clone(), moriguchi.clone()];
    let mut page = ExamSubjectPage {
        subject: ExamSubject {
            code: "CSC.T273".into(),
            name: "オブジェクト指向プログラミング".into(),
            name_en: None,
            exam_count: None,
        },
        rows: vec![ExamClass {
            class_ids: vec![1],
            labels: Vec::new(),
            taught_in: String::new(),
            teachers: vec![teacher("t4", "Partachi Profir Petru")],
        }],
        exams: vec![
            document("a23", 2023, "final", "answer", &pair),
            document("q24", 2024, "final", "question", &pair),
            document("q23", 2023, "final", "question", &pair),
            document("m23", 2023, "midterm", "question", &[other]),
        ],
        my_teachers: Vec::new(),
        sets: Vec::new(),
    };

    arrange(&mut page, Some("小林\u{3000}隆志 / PARTACHI PROFIR PETRU"));

    assert_eq!(page.my_teachers, ["t1", "t4"]);
    let shape: Vec<_> = page
        .sets
        .iter()
        .map(|set| {
            (
                set.academic_year,
                set.occasion.as_deref(),
                set.mine,
                set.documents
                    .iter()
                    .map(|d| d.id.as_str())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            (Some(2024), Some("final"), true, vec!["q24"]),
            (Some(2023), Some("midterm"), false, vec!["m23"]),
            (Some(2023), Some("final"), true, vec!["q23", "a23"]),
        ]
    );

    arrange(&mut page, None);
    assert!(page.my_teachers.is_empty());
    assert!(page.sets.iter().all(|set| !set.mine));
}
