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

const ME: &str = "science:24B00001";

#[test]
fn backoff_climbs_by_three_from_five_minutes_and_stops_at_six_hours() {
    let minutes: Vec<i64> = (1..=6).map(|n| backoff(n).num_minutes()).collect();
    assert_eq!(minutes, [5, 15, 45, 135, 360, 360]);
    assert_eq!(backoff(u32::MAX).num_minutes(), 360);
}

#[test]
fn a_key_is_fresh_only_after_a_complete_commit_for_the_same_account() {
    let db = database();
    let course = Key::new(Dataset::LmsCourse, "123");
    let ttl = ttl(Dataset::LmsCourse, Term::Current);
    assert_eq!(
        due(get(&db, &course).unwrap().as_ref(), ttl, ME, at(0)),
        Due::Missing
    );

    // A failure stores no freshness and backs the key off.
    let next = fail(&db, &course, ME, "timeout", at(0)).unwrap();
    assert_eq!(next, at(0) + Duration::minutes(5));
    let state = get(&db, &course).unwrap();
    assert_eq!(due(state.as_ref(), ttl, ME, at(0)), Due::BackingOff(next));
    // Once the backoff passes, missing content is due before stale content.
    assert_eq!(due(state.as_ref(), ttl, ME, at(1)), Due::Missing);
    assert!(Due::Missing < Due::Stale);

    commit(
        &db,
        &course,
        Commit {
            account: ME,
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
    assert_eq!(due(Some(&state), ttl, ME, at(2)), Due::Fresh);
    assert_eq!(due(Some(&state), ttl, ME, at(25)), Due::Stale);
    // Another account's content is not this account's.
    assert_eq!(due(Some(&state), ttl, "science:other", at(2)), Due::Missing);
    // A frozen key stays fresh however old it gets, until its content goes.
    assert_eq!(due(Some(&state), None, ME, at(24 * 400)), Due::Fresh);

    // A later failure keeps the last good fetch.
    fail(&db, &course, ME, "http-503", at(30)).unwrap();
    assert_eq!(get(&db, &course).unwrap().unwrap().fetched_at, Some(at(1)));
}

#[test]
fn renewing_needs_stored_content_and_invalidation_beats_freezing() {
    let db = database();
    let file = Key::new(Dataset::Reference, "calendar/2026");
    // Nothing stored: a matching validator must not count as a fetch.
    assert!(!renew(&db, &file, "", at(0)).unwrap());
    assert!(get(&db, &file).unwrap().is_none());

    commit(
        &db,
        &file,
        Commit {
            account: "",
            validator: Some("index-1"),
            outcome: Outcome::Complete,
            message: None,
        },
        at(0),
    )
    .unwrap();
    assert!(renew(&db, &file, "", at(5)).unwrap());
    assert_eq!(get(&db, &file).unwrap().unwrap().fetched_at, Some(at(5)));

    // gc deleted the content: the key is missing even though it is frozen.
    invalidate(&db, &file).unwrap();
    let state = get(&db, &file).unwrap().unwrap();
    assert_eq!((state.fetched_at, state.validator.as_deref()), (None, None));
    assert_eq!(due(Some(&state), None, "", at(6)), Due::Missing);
    assert!(!renew(&db, &file, "", at(6)).unwrap());
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
