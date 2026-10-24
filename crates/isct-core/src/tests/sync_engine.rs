use super::planner::{self, Candidate, Depth, Facts, Inputs, Lane, ReferenceWatch};
use super::*;
use crate::{
    data::DataStore,
    lms_ingest::StoredLmsCourse,
    net::Host,
    ports::{SecretError, SecretVault},
    school_auth::SchoolAuthService,
    sync_state::{self, Commit, Dataset, Due, Key, KeyState, Outcome},
};
use chrono::Duration;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[derive(Default)]
struct EmptyVault;

impl SecretVault for EmptyVault {
    fn store(&self, _account: &str, _secret: &str) -> Result<(), SecretError> {
        Ok(())
    }
    fn load(&self, _account: &str) -> Result<Option<String>, SecretError> {
        Ok(None)
    }
    fn delete(&self, _account: &str) -> Result<(), SecretError> {
        Ok(())
    }
}

/// 12:00 in Japan on a weekday of the third quarter.
fn now() -> DateTime<Utc> {
    "2026-10-20T03:00:00Z".parse().unwrap()
}

fn key(dataset: Dataset, key: &str) -> Key {
    Key::new(dataset, key)
}

fn stored(
    dataset: Dataset,
    name: &str,
    account: &str,
    fetched: Option<DateTime<Utc>>,
    next_eligible: Option<DateTime<Utc>>,
) -> (Key, KeyState) {
    (
        key(dataset, name),
        KeyState {
            dataset: dataset.name().into(),
            key: name.into(),
            account: account.into(),
            fetched_at: fetched,
            validator: None,
            outcome: fetched.map(|_| sync_state::Outcome::Complete),
            message: None,
            failures: u32::from(next_eligible.is_some()),
            next_eligible,
            last_error: next_eligible.map(|_| "timeout".into()),
            last_used: None,
        },
    )
}

fn course(id: u64, year: i32, quarter: u8) -> StoredLmsCourse {
    StoredLmsCourse {
        id,
        title: format!("Course {id}"),
        year: Some(year),
        quarter: Some(quarter),
        shown_in: Vec::new(),
    }
}

const SCIENCE: &str = "science:student";
const TOKYO: &str = "tokyotech:student";

/// Both profiles signed in; every 教務 page read a day ago; no catalog year.
fn inputs(states: Vec<(Key, KeyState)>) -> Inputs {
    let mut all: HashMap<Key, KeyState> = planner::kyomu_keys()
        .into_iter()
        .map(|page| {
            stored(
                Dataset::KyomuPage,
                &page.key,
                TOKYO,
                Some(now() - Duration::days(1)),
                None,
            )
        })
        .collect();
    all.extend(states);
    Inputs {
        states: all,
        courses: vec![
            course(101, 2026, 3),
            course(102, 2026, 1),
            course(103, 2025, 1),
            course(104, 2026, 3),
        ],
        science: Some(SCIENCE.into()),
        tokyo: Some(TOKYO.into()),
        science_configured: true,
        tokyo_configured: true,
        allow_cellular: false,
        year: None,
        english: false,
        references: Vec::new(),
    }
}

fn facts() -> Facts {
    Facts {
        network: Network::Wifi,
        school_attached: true,
        signed_in: HashSet::from([SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech]),
        ..Facts::default()
    }
}

fn plan(inputs: &Inputs, facts: &Facts, opens: &[View], manual: &[Key]) -> Vec<Candidate> {
    let manual = manual.iter().map(|key| (key.clone(), None)).collect();
    planner::plan(inputs, facts, opens, &manual, &HashSet::new(), now())
}

fn find<'a>(plan: &'a [Candidate], wanted: &Key) -> Option<&'a Candidate> {
    plan.iter().find(|candidate| candidate.key == *wanted)
}

fn position(plan: &[Candidate], wanted: &Key) -> usize {
    plan.iter()
        .position(|candidate| candidate.key == *wanted)
        .unwrap_or_else(|| panic!("{wanted} not planned"))
}

#[test]
fn due_keys_are_planned_missing_first_within_each_tier() {
    let hours = |hours: i64| Some(now() - Duration::hours(hours));
    let inputs = inputs(vec![
        stored(Dataset::LmsEnrolment, "all", SCIENCE, hours(1), None),
        stored(Dataset::LmsDeadlines, "all", SCIENCE, hours(3), None),
        stored(Dataset::LmsAssign, "101", SCIENCE, hours(1), None),
        stored(Dataset::LmsCourse, "102", SCIENCE, hours(40 * 24), None),
        stored(
            Dataset::LmsAssign,
            "102",
            SCIENCE,
            hours(40 * 24),
            Some(now() + Duration::minutes(10)),
        ),
        stored(Dataset::LmsCourse, "103", SCIENCE, hours(400 * 24), None),
        // Stored for another account: missing for this one.
        stored(Dataset::KyomuPage, "gpa", "tokyotech:other", hours(1), None),
    ]);
    let plan = plan(&inputs, &facts(), &[], &[]);
    let at = |dataset, name| position(&plan, &key(dataset, name));

    // Current-term and 教務 keys are Visible, missing before stale.
    assert_eq!(
        find(&plan, &key(Dataset::LmsCourse, "101")).unwrap().rank,
        0
    );
    assert!(at(Dataset::LmsCourse, "101") < at(Dataset::LmsDeadlines, "all"));
    assert!(at(Dataset::KyomuPage, "gpa") < at(Dataset::LmsDeadlines, "all"));
    // A key's first read takes its full form; a stale one the light form.
    assert_eq!(
        find(&plan, &key(Dataset::LmsCourse, "101")).unwrap().depth,
        Depth::Full
    );
    assert_eq!(
        find(&plan, &key(Dataset::LmsDeadlines, "all"))
            .unwrap()
            .depth,
        Depth::Light
    );
    // Earlier terms are Background, after every Visible key; a frozen course
    // is read only when its content is missing.
    for (dataset, name) in [(Dataset::LmsCourse, "102"), (Dataset::LmsAssign, "103")] {
        let candidate = find(&plan, &key(dataset, name)).unwrap();
        assert_eq!(candidate.priority, Priority::Background);
        assert!(at(Dataset::LmsDeadlines, "all") < at(dataset, name));
    }
    assert_eq!(
        find(&plan, &key(Dataset::LmsCourse, "102"))
            .unwrap()
            .group
            .term,
        Some("past")
    );
    // Fresh, frozen and backing-off keys wait.
    for (dataset, name) in [
        (Dataset::LmsEnrolment, "all"),
        (Dataset::LmsAssign, "101"),
        (Dataset::LmsAssign, "102"),
        (Dataset::LmsCourse, "103"),
        (Dataset::KyomuPage, "grades"),
    ] {
        assert!(
            find(&plan, &key(dataset, name)).is_none(),
            "{dataset:?}/{name}"
        );
    }
    // A course whose outline was never stored is read first; its assignment
    // states wait, since they land on the rows the outline creates.
    assert!(find(&plan, &key(Dataset::LmsCourse, "104")).is_some());
    assert!(find(&plan, &key(Dataset::LmsAssign, "104")).is_none());
    // Search is local only, so the catalog covers the course families no
    // department owns (liberal arts, entrepreneurship, cross-departmental).
    let prefixes = planner::department_prefixes();
    for family in ["LA", "ENT", "X", "MTH"] {
        assert!(prefixes.iter().any(|prefix| prefix == family), "{family}");
    }
    // The leased key is single-flight: never planned twice.
    let leased = HashSet::from([key(Dataset::LmsCourse, "101")]);
    let again = planner::plan(&inputs, &facts(), &[], &HashMap::new(), &leased, now());
    assert!(find(&again, &key(Dataset::LmsCourse, "101")).is_none());
}

#[test]
fn an_open_page_renews_after_fifteen_minutes_and_a_refresh_ignores_ttl_and_backoff() {
    let minutes = |minutes: i64| Some(now() - Duration::minutes(minutes));
    let inputs = inputs(vec![
        stored(Dataset::LmsCourse, "101", SCIENCE, minutes(20), None),
        stored(Dataset::LmsAssign, "101", SCIENCE, minutes(5), None),
        stored(
            Dataset::LmsCourse,
            "103",
            SCIENCE,
            minutes(400 * 24 * 60),
            None,
        ),
        stored(
            Dataset::LmsAssign,
            "103",
            SCIENCE,
            minutes(400 * 24 * 60),
            None,
        ),
        stored(
            Dataset::LmsCourse,
            "104",
            SCIENCE,
            minutes(30),
            Some(now() + Duration::minutes(10)),
        ),
        stored(Dataset::LmsCourse, "102", SCIENCE, minutes(60), None),
        stored(
            Dataset::LmsAssign,
            "102",
            SCIENCE,
            minutes(60),
            Some(now() + Duration::minutes(10)),
        ),
        stored(Dataset::LmsEnrolment, "all", SCIENCE, minutes(1), None),
        stored(Dataset::LmsDeadlines, "all", SCIENCE, minutes(1), None),
    ]);
    let opens = [
        View::Course { lms_course_id: 101 },
        View::Course { lms_course_id: 103 },
        View::Course { lms_course_id: 104 },
    ];
    let plan = plan(
        &inputs,
        &facts(),
        &opens,
        &[
            key(Dataset::LmsAssign, "102"),
            key(Dataset::LmsEnrolment, "all"),
        ],
    );
    let course = find(&plan, &key(Dataset::LmsCourse, "101")).unwrap();
    assert_eq!(
        (course.priority, course.rank, course.depth),
        (Priority::Interactive, 1, Depth::Light)
    );
    assert_eq!(course.group.scope.as_deref(), Some("101"));
    // Read 5 minutes ago: within the open threshold.
    assert!(find(&plan, &key(Dataset::LmsAssign, "101")).is_none());
    // A frozen course stays frozen when opened.
    assert!(find(&plan, &key(Dataset::LmsCourse, "103")).is_none());
    // An open page's key that failed recently: shown as waiting.
    assert!(matches!(
        find(&plan, &key(Dataset::LmsCourse, "104")).unwrap().held,
        Some(Held::Backoff { .. })
    ));
    // The refresh button reads a backing-off and a fresh key now.
    for wanted in [
        key(Dataset::LmsAssign, "102"),
        key(Dataset::LmsEnrolment, "all"),
    ] {
        let candidate = find(&plan, &wanted).unwrap();
        assert_eq!(
            (candidate.priority, candidate.held.clone(), candidate.depth),
            (Priority::Interactive, None, Depth::Full)
        );
    }
}

/// The open finder reads a department it never stored before other due
/// work and on any network; a stored one, however old, keeps the view's
/// priority. English names wait for the Japanese rows they merge into.
#[test]
fn finder_catalog_keys_read_new_departments_first_and_english_after_japanese() {
    let department = planner::department_prefixes()[0].clone();
    let japanese = key(Dataset::CatalogOfficial, &format!("2026/{department}"));
    let english = key(Dataset::CatalogOfficial, &format!("2026/{department}/en"));
    let mut inputs = inputs(Vec::new());
    inputs.year = Some(2026);
    let finder = [View::Finder { year: 2026 }];
    let plan_now = |inputs: &Inputs, facts: &Facts| plan(inputs, facts, &finder, &[]);
    let priority = |plan: &[Candidate], wanted: &Key| {
        find(plan, wanted).map(|candidate| (candidate.priority, candidate.held.clone()))
    };
    // Nothing stored: the shown year's departments come first, even on
    // cellular; without the finder they are background work that waits.
    let cellular = Facts {
        network: Network::Cellular,
        ..facts()
    };
    assert_eq!(
        priority(&plan_now(&inputs, &cellular), &japanese),
        Some((Priority::Interactive, None))
    );
    assert_eq!(
        priority(&plan(&inputs, &cellular, &[], &[]), &japanese),
        Some((Priority::Background, Some(Held::Cellular)))
    );
    // Japanese content: the English page is never read.
    assert!(find(&plan_now(&inputs, &facts()), &english).is_none());
    // English content: only once the Japanese rows it names are stored.
    inputs.english = true;
    assert!(find(&plan_now(&inputs, &facts()), &english).is_none());
    let (state_key, state) = stored(
        Dataset::CatalogOfficial,
        &japanese.key,
        "",
        Some(now() - Duration::days(8)),
        None,
    );
    inputs.states.insert(state_key, state);
    let plan = plan_now(&inputs, &facts());
    let english_read = find(&plan, &english).cloned().unwrap();
    assert_eq!(english_read.lane, Lane::Kyomu);
    assert_eq!(english_read.priority, Priority::Interactive);
    // A week-old stored department is renewed at the view's priority.
    assert_eq!(priority(&plan, &japanese), Some((Priority::Visible, None)));
    assert_eq!(planner::japanese_key(&english), Some(japanese.clone()));
    assert_eq!(planner::japanese_key(&japanese), None);
}

#[test]
fn held_reasons_follow_the_shell_facts() {
    let mut inputs = inputs(vec![stored(
        Dataset::LmsDeadlines,
        "all",
        SCIENCE,
        Some(now()),
        None,
    )]);
    // 教務 grades were never read.
    inputs.states.remove(&key(Dataset::KyomuPage, "grades"));
    inputs.references = vec![ReferenceWatch {
        key: key(Dataset::Reference, "calendar/2026"),
        index_url: crate::reference_refresh::PUBLIC_INDEX.into(),
        due: Due::Missing,
    }];
    let deadlines = key(Dataset::LmsDeadlines, "all");
    let grades = key(Dataset::KyomuPage, "grades");
    let calendar = key(Dataset::Reference, "calendar/2026");
    // The refresh button wants the deadlines; 教務 and the calendar are
    // automatic.
    let held = |inputs: &Inputs, facts: &Facts, wanted: &Key| {
        let plan = plan(inputs, facts, &[], std::slice::from_ref(&deadlines));
        find(&plan, wanted).map(|candidate| candidate.held.clone())
    };
    let with = |change: &dyn Fn(&mut Facts)| {
        let mut facts = facts();
        change(&mut facts);
        facts
    };
    for wanted in [&deadlines, &grades, &calendar] {
        assert_eq!(held(&inputs, &facts(), wanted), Some(None));
        let offline = with(&|facts| facts.network = Network::Offline);
        assert_eq!(held(&inputs, &offline, wanted), Some(Some(Held::Offline)));
        let resetting = with(&|facts| facts.resetting = true);
        assert_eq!(
            held(&inputs, &resetting, wanted),
            Some(Some(Held::Resetting))
        );
    }
    // Cellular and power saver hold background work alike, reference data
    // included; what the person asked for still runs.
    let cellular = with(&|facts| facts.network = Network::Cellular);
    assert_eq!(held(&inputs, &cellular, &deadlines), Some(None));
    assert_eq!(
        held(&inputs, &cellular, &grades),
        Some(Some(Held::Cellular))
    );
    assert_eq!(
        held(&inputs, &cellular, &calendar),
        Some(Some(Held::Cellular))
    );
    let mut allowed = inputs.clone();
    allowed.allow_cellular = true;
    assert_eq!(held(&allowed, &cellular, &calendar), Some(None));
    let saver = with(&|facts| facts.power_saver = true);
    assert_eq!(held(&inputs, &saver, &deadlines), Some(None));
    assert_eq!(
        held(&inputs, &saver, &calendar),
        Some(Some(Held::PowerSaver))
    );
    // 教務 reads only in the foreground; token deadlines and the token
    // dashboard/enrolment read can finish without a resident web session.
    let background = with(&|facts| facts.foreground = false);
    assert_eq!(
        held(&inputs, &background, &grades),
        Some(Some(Held::Background))
    );
    assert_eq!(held(&inputs, &background, &deadlines), Some(None));
    assert_eq!(
        held(&inputs, &background, &key(Dataset::LmsEnrolment, "all")),
        Some(None)
    );
    // A paused host holds its keys only.
    let paused = with(&|facts| {
        facts
            .paused
            .insert(Host::Kyomu, now() + Duration::minutes(5));
    });
    assert!(matches!(
        held(&inputs, &paused, &grades),
        Some(Some(Held::HostPaused { host, .. })) if host == Host::Kyomu.name()
    ));
    assert_eq!(held(&inputs, &paused, &deadlines), Some(None));
    // Without a session or saved credentials, automatic 教務 keys are not
    // wanted and an asked-for LMS key waits for sign-in.
    let mut signed_out = inputs.clone();
    signed_out.science_configured = false;
    let nobody = with(&|facts| facts.signed_in.clear());
    assert_eq!(held(&signed_out, &nobody, &grades), None);
    assert_eq!(
        held(&signed_out, &nobody, &deadlines),
        Some(Some(Held::SignInRequired {
            profile: SchoolAuthProfile::Science
        }))
    );
    let detached = with(&|facts| facts.school_attached = false);
    assert_eq!(
        held(&inputs, &detached, &deadlines),
        Some(Some(Held::SignInRequired {
            profile: SchoolAuthProfile::Science
        }))
    );
}

fn engine(name: &str) -> (SyncEngine, DataStore) {
    let path = std::env::temp_dir().join(format!("isct-sync-{name}-{}", uuid::Uuid::new_v4()));
    let store = DataStore::initialize_dir(path).unwrap().0;
    store
        .put_workspace_setting("selectedTerm", serde_json::json!("2026-3Q"))
        .unwrap();
    let engine = SyncEngine::new(
        store.clone(),
        Arc::new(SchoolAuthService::new(EmptyVault)),
        Network::Wifi,
    );
    (engine, store)
}

fn reference_runs(snapshot: &SyncSnapshot) -> Vec<&RunView> {
    snapshot
        .runs
        .iter()
        .filter(|run| run.dataset == "reference")
        .collect()
}

#[tokio::test]
async fn runs_count_real_keys_and_a_refresh_joins_the_lease_in_flight() {
    let (engine, store) = engine("counts");
    // Four followed reference files, all missing.
    let plan = engine.evaluate_now().await;
    let references: Vec<_> = plan
        .iter()
        .filter(|candidate| candidate.key.dataset == Dataset::Reference)
        .collect();
    assert_eq!(references.len(), 4);
    let snapshot = engine.snapshot();
    let run = reference_runs(&snapshot)[0].clone();
    assert_eq!((run.done, run.total), (0, Some(4)));
    assert_eq!(run.label.key, "sync.run.reference");

    let first = engine.lease_for_test(Lane::IsctData).unwrap();
    assert_eq!(first.run, run.run);
    let current = engine.snapshot();
    assert!(reference_runs(&current)[0].current.is_some());
    assert!(current.sources[2].running);

    // A refresh press while the key is read joins it: not planned again.
    engine.refresh_for_test(Scope::References);
    let plan = engine.evaluate_now().await;
    assert!(plan.iter().all(|candidate| candidate.key != first.key));
    let pressed = plan
        .iter()
        .filter(|candidate| candidate.key.dataset == Dataset::Reference)
        .count();
    assert_eq!(pressed, 3);

    let leased = first.key.clone();
    engine
        .settle_for_test(first, KeyEnd::failed("network", ""))
        .await;
    let snapshot = engine.snapshot();
    let run = reference_runs(&snapshot)[0];
    assert_eq!((run.done, run.total), (0, Some(4)));
    let failure = run.failure.clone().unwrap();
    assert_eq!((failure.kind.as_str(), failure.keys_failed), ("network", 1));
    assert!(failure.retry_at.is_some());
    assert_eq!(snapshot.sources[2].failure.as_ref().unwrap().keys_failed, 1);
    // The failure backed the key off and left one run row.
    let state = store
        .write(None, |transaction| sync_state::get(transaction, &leased))
        .unwrap()
        .unwrap();
    assert_eq!(
        (state.failures, state.last_error.as_deref()),
        (1, Some("network"))
    );

    // A second press within 30 s joins the first, though the read failed.
    engine.refresh_for_test(Scope::References);
    let plan = engine.evaluate_now().await;
    assert!(plan.iter().all(|candidate| candidate.key != leased));
}

#[tokio::test]
async fn a_paused_host_holds_its_keys_without_failing_them_but_a_refused_sign_in_backs_off() {
    let (engine, store) = engine("paused");
    engine.set_signed_in(SchoolAuthProfile::TokyoTech);
    let page = key(Dataset::KyomuPage, "grades");
    let until = Utc::now() + Duration::minutes(5);
    engine
        .settle_for_test(
            Lease::detached(&store, page.clone(), TOKYO),
            KeyEnd::HostPaused {
                host: Host::Kyomu,
                until,
                notice: Some("閉局中".into()),
                source: String::new(),
            },
        )
        .await;
    let plan = engine.evaluate_now().await;
    let candidate = find(&plan, &page).unwrap();
    assert_eq!(
        candidate.held,
        Some(Held::HostPaused {
            host: Host::Kyomu.name().into(),
            until
        })
    );
    let snapshot = engine.snapshot();
    assert_eq!(snapshot.sources[1].maintenance.as_deref(), Some("閉局中"));
    let (failures, error_kind): (u32, Option<String>) = store
        .write(None, |transaction| {
            let failures = sync_state::get(transaction, &page)?.map_or(0, |state| state.failures);
            let kind = transaction
                .query_row("SELECT error_kind FROM sync_runs", [], |row| row.get(0))
                .map_err(crate::data::DataError::from)?;
            Ok::<_, crate::data::DataError>((failures, kind))
        })
        .unwrap();
    assert_eq!((failures, error_kind.as_deref()), (0, Some("host-paused")));

    // The reader already ran the saved sign-in and the school still refused:
    // the key backs off instead of being read again on every evaluation.
    let refused = key(Dataset::KyomuPage, "gpa");
    engine
        .settle_for_test(
            Lease::detached(&store, refused.clone(), TOKYO),
            KeyEnd::SignInRequired {
                profile: SchoolAuthProfile::TokyoTech,
                source: String::new(),
            },
        )
        .await;
    let state = store
        .write(None, |transaction| sync_state::get(transaction, &refused))
        .unwrap()
        .unwrap();
    assert_eq!(state.failures, 1);
    assert!(state.next_eligible.is_some_and(|until| until > Utc::now()));
}

#[tokio::test]
async fn closed_admission_refuses_leases_and_commits_of_the_old_generation() {
    let (engine, store) = engine("admission");
    engine.evaluate_now().await;
    let lease = engine.lease_for_test(Lane::IsctData).unwrap();
    let writer = lease.writer();
    assert_eq!(engine.generation(), 0);

    let admission = engine.close_admission().await;
    assert_eq!(engine.generation(), 1);
    assert!(lease.is_cancelled());
    assert!(engine.lease_for_test(Lane::IsctData).is_none());
    let plan = engine.evaluate_now().await;
    assert!(plan
        .iter()
        .all(|candidate| candidate.held == Some(Held::Resetting)));
    assert!(engine.snapshot().resetting);

    // The old lease's commit is refused inside the gate and writes nothing.
    let refused = writer.write(None, |transaction| {
        sync_state::commit(
            transaction,
            &lease.key,
            Commit {
                account: "",
                validator: None,
                outcome: Outcome::Complete,
                message: None,
            },
            Utc::now(),
        )
    });
    assert!(matches!(refused, Err(Gated::Stale)));
    assert!(lease
        .ending(&KeyEnd::failed("network", ""))
        .write()
        .is_none());
    assert!(store.freshness(None).unwrap().is_empty());
    assert!(engine.wait_idle(std::time::Duration::from_millis(10)).await);

    drop(admission);
    assert!(!engine.snapshot().resetting);
    engine.evaluate_now().await;
    let fresh = engine.lease_for_test(Lane::IsctData).unwrap();
    assert!(fresh
        .writer()
        .write(None, |transaction| {
            sync_state::commit(
                transaction,
                &fresh.key,
                Commit {
                    account: "",
                    validator: None,
                    outcome: Outcome::Complete,
                    message: None,
                },
                Utc::now(),
            )
        })
        .is_ok());
}

#[tokio::test]
async fn wait_idle_waits_for_a_commit_in_progress() {
    let (engine, _store) = engine("idle");
    engine.evaluate_now().await;
    let lease = engine.lease_for_test(Lane::IsctData).unwrap();
    let writer = lease.writer();
    let committing = writer.enter().unwrap();
    let _admission = engine.close_admission().await;
    // A commit that entered before the bump finishes; the reset waits.
    assert!(!engine.wait_idle(std::time::Duration::from_millis(30)).await);
    drop(committing);
    assert!(engine.wait_idle(std::time::Duration::from_millis(30)).await);
    // Nothing of the old generation enters afterwards.
    assert!(writer.enter().is_none());
}

#[test]
fn snapshot_wire_names_match_the_shell_contract() {
    let snapshot = SyncSnapshot {
        generation: 2,
        resetting: false,
        runs: vec![RunView {
            run: "r".into(),
            dataset: "lms.course".into(),
            label: Label {
                key: "sync.run.lms-course".into(),
                args: serde_json::json!({"term": "current"}),
            },
            priority: Priority::Interactive,
            done: 1,
            total: Some(3),
            current: None,
            scope: Some("12345".into()),
            held: Some(Held::HostPaused {
                host: "lms.s.isct.ac.jp".into(),
                until: now(),
            }),
            failure: Some(SyncFailure {
                kind: "timeout".into(),
                keys_failed: 1,
                retry_at: None,
            }),
        }],
        sources: Vec::new(),
    };
    let wire = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(wire["runs"][0]["priority"], "interactive");
    assert_eq!(wire["runs"][0]["held"]["kind"], "host-paused");
    assert_eq!(wire["runs"][0]["failure"]["keysFailed"], 1);
    // The shapes `apps/desktop/src/platform/sync.ts` sends.
    let hint: Hint = serde_json::from_value(serde_json::json!({
        "kind": "open", "view": {"kind": "course", "lmsCourseId": 12345}
    }))
    .unwrap();
    assert_eq!(
        hint,
        Hint::Open {
            view: View::Course {
                lms_course_id: 12345
            }
        }
    );
    let hint: Hint = serde_json::from_value(serde_json::json!({
        "kind": "refresh", "scope": {"kind": "course", "lmsCourseId": 12345}
    }))
    .unwrap();
    assert_eq!(
        hint,
        Hint::Refresh {
            scope: Scope::Course {
                lms_course_id: 12345
            }
        }
    );
    let hint: Hint = serde_json::from_value(
        serde_json::json!({"kind": "refresh", "scope": {"kind": "timetable"}}),
    )
    .unwrap();
    assert_eq!(
        hint,
        Hint::Refresh {
            scope: Scope::Timetable
        }
    );
    let event = crate::events::CoreEvent::SyncSnapshot(snapshot);
    assert_eq!(event.name(), "sync-snapshot");
    assert_eq!(event.payload(), wire);
}
