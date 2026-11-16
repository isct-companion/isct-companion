use super::*;

/// A subject view in the shape of the App API (`GET /api/subjects/:code/view`):
/// two classes of LAS.B103 in 2026 3Q with different teachers.
fn page() -> RemotePage {
    let exam = |id: &str, teachers: Value| {
        json!({"id":id,"subjects":[],"teachers":teachers,"academicYear":2025,"occasion":"final",
            "occasionNumber":1,"variant":"","role":"question","note":null,"fileName":"a.pdf",
            "fileType":"application/pdf"})
    };
    serde_json::from_value(json!({
        "subject": {"code":"LAS.B103","name":"Biology","nameEn":null,"examCount":3},
        "rows": [
            {"classIds":[202600336],"labels":["A"],"taughtIn":"3Q","teachers":[{"id":"t-a","name":"A"}]},
            {"classIds":[202608082],"labels":["K"],"taughtIn":"3Q","teachers":[{"id":"t-k","name":"K"}]}
        ],
        "exams": [
            exam("unknown", json!([])),
            exam("other", json!([{"id":"t-k","name":"K"}])),
            exam("ours", json!([{"id":"t-x","name":"X"},{"id":"t-a","name":"A"}]))
        ]
    }))
    .unwrap()
}

#[test]
fn a_course_class_puts_its_teachers_materials_first_and_keeps_the_rest() {
    let ids = |page: &ExamSubjectPage| page.exams.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
    let ours = page().present(Some(202600336)).unwrap();
    assert_eq!(ids(&ours), ["ours", "unknown", "other"]);
    assert!(ours.sets[0].mine && !ours.sets[1].mine);
    assert_eq!(ours.web_url, "https://sciencetokyoexam.com/search?course=LAS.B103");
    // Reopening passes the class back to keep this order.
    assert_eq!(ours.class_id, Some(202600336));
    // A class the latest-year rows do not list: the plain subject order.
    let plain = page().present(Some(202400001)).unwrap();
    assert_eq!(plain.class_id, None);
    assert_eq!(ids(&plain), ["unknown", "other", "ours"]);
    assert!(plain.sets.iter().all(|set| !set.mine));
}
