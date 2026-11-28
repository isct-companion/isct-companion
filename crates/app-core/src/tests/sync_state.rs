use super::*;
use crate::data::migrations;

fn database() -> Connection {
    migrations::open_in_memory().unwrap()
}

fn at(hours: i64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-20T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
        + Duration::hours(hours)
}

#[test]
fn a_key_is_fresh_only_after_a_complete_commit() {
    let db = database();
    let course = Key::new(Dataset::LmsCourse, "123");
    let ttl = ttl(&course, Term::Current);
    assert_eq!(
        due(get(&db, &course).unwrap().as_ref(), ttl, at(0)),
        Due::Missing
    );

    // A failure stores no freshness and backs the key off.
    let next = fail(&db, &course, "timeout", at(0)).unwrap();
    assert_eq!(next, at(0) + Duration::minutes(5));
    let state = get(&db, &course).unwrap();
    assert_eq!(due(state.as_ref(), ttl, at(0)), Due::BackingOff(next));
    // Once the backoff passes, missing content is due before stale content.
    assert_eq!(due(state.as_ref(), ttl, at(1)), Due::Missing);
    assert!(Due::Missing < Due::Stale);
    // A key that keeps failing (offline for weeks) waits longer each time,
    // never past six hours, and its count never overflows.
    assert!(backoff(2) > backoff(1));
    assert_eq!(backoff(u32::MAX), Duration::hours(6));

    commit(
        &db,
        &course,
        Commit {
            validator: Some("h1"),
            outcome: Outcome::Complete,
            message: None,
        },
        at(1),
    )
    .unwrap();
    let state = get(&db, &course).unwrap().unwrap();
    assert_eq!(
        (
            state.failures,
            state.next_eligible,
            state.last_error.as_deref()
        ),
        (0, None, None)
    );
    assert_eq!(due(Some(&state), ttl, at(2)), Due::Fresh);
    assert_eq!(due(Some(&state), ttl, at(25)), Due::Stale);
    // A frozen key stays fresh however old it gets, until its content goes.
    assert_eq!(due(Some(&state), None, at(24 * 400)), Due::Fresh);

    // A later failure keeps the last good fetch.
    fail(&db, &course, "http-503", at(30)).unwrap();
    assert_eq!(get(&db, &course).unwrap().unwrap().fetched_at, Some(at(1)));
}

#[test]
fn renewing_needs_stored_content_and_invalidation_beats_freezing() {
    let db = database();
    let file = Key::new(Dataset::Reference, "calendar/2026");
    // Nothing stored: a matching validator must not count as a fetch.
    assert!(!renew(&db, &file, at(0)).unwrap());
    assert!(get(&db, &file).unwrap().is_none());

    commit(
        &db,
        &file,
        Commit {
            validator: Some("index-1"),
            outcome: Outcome::Complete,
            message: None,
        },
        at(0),
    )
    .unwrap();
    assert!(renew(&db, &file, at(5)).unwrap());
    assert_eq!(get(&db, &file).unwrap().unwrap().fetched_at, Some(at(5)));

    // gc deleted the content: the key is missing even though it is frozen.
    invalidate(&db, &file).unwrap();
    let state = get(&db, &file).unwrap().unwrap();
    assert_eq!((state.fetched_at, state.validator.as_deref()), (None, None));
    assert_eq!(due(Some(&state), None, at(6)), Due::Missing);
    assert!(!renew(&db, &file, at(6)).unwrap());
}

#[test]
fn run_sources_never_keep_query_strings() {
    let url = url::Url::parse(
        "https://lms.s.isct.ac.jp/2025/lib/ajax/service.php?sesskey=SECRET&info=core",
    )
    .unwrap();
    assert_eq!(
        source_of_url(&url),
        "lms.s.isct.ac.jp/2025/lib/ajax/service.php"
    );
}

/// A grades change marks the GPA and requirements pages due at once and a
/// day later (`d/azfvr2`, `d/aa17ta`). A read answers only the marks set
/// before it started; a failed read answers none; an unchanged read marks
/// nothing.
#[test]
fn a_change_marks_its_dependents_and_only_a_later_read_answers_them() {
    let db = database();
    let grades = Key::new(Dataset::KyomuPage, "grades");
    let gpa = Key::new(Dataset::KyomuPage, "gpa");
    let run = |key: &Key, started: DateTime<Utc>, outcome: RunOutcome, changed: usize| {
        record_run(
            &db,
            &RunRecord {
                run: "run",
                key,
                started_at: started,
                finished_at: started + Duration::seconds(1),
                outcome,
                changed,
                requests: None,
                bytes: None,
                source: "kyomu",
                error_kind: None,
            },
        )
        .unwrap();
    };
    let marks = |key: &Key| {
        get(&db, key)
            .unwrap()
            .map(|state| state.due_marks)
            .unwrap_or_default()
    };
    run(&grades, at(0), RunOutcome::Complete, 0);
    assert!(marks(&gpa).is_empty());
    run(&grades, at(1), RunOutcome::Complete, 3);
    let changed = at(1) + Duration::seconds(1);
    // An unchanged read keeps the last real change.
    run(&grades, at(2), RunOutcome::Complete, 0);
    assert_eq!(
        get(&db, &grades).unwrap().unwrap().changed_at,
        Some(changed)
    );
    assert_eq!(marks(&gpa), [changed, changed + Duration::days(1)]);
    assert_eq!(
        marks(&Key::new(Dataset::KyomuPage, "requirements")),
        [changed, changed + Duration::days(1)]
    );
    // A GPA read that started before the change answers nothing; a failed
    // one neither; one started after it answers the mark now, not the one
    // a day ahead.
    run(&gpa, at(1) - Duration::seconds(5), RunOutcome::Complete, 0);
    run(&gpa, at(2), RunOutcome::Failed, 0);
    assert_eq!(marks(&gpa).len(), 2);
    run(&gpa, at(2), RunOutcome::Complete, 0);
    assert_eq!(marks(&gpa), [changed + Duration::days(1)]);
    // A mark added by hand joins the set, earliest first.
    mark_due(&db, &gpa, at(3)).unwrap();
    assert_eq!(marks(&gpa), [at(3), changed + Duration::days(1)]);
}
