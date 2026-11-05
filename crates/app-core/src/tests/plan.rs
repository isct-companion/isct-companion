use super::runner::{ReadRefused, ReadRequest};
use super::planner::{
    self, Candidate, Credential, Credentials, Depth, Facts, Inputs, Lane, ReferenceWatch, Waited,
};
use super::*;
use crate::{
    data::DataStore,
    lifecycle::SchoolSite,
    lms_ingest::StoredLmsCourse,
    net::{
        moodle::{MoodleClient, MoodleFunction, TokenFact, Via},
        Host,
    },
    ports::{SecretError, SecretVault},
    school_auth::SchoolAuthService,
    school_session::{AccountFacts, SchoolSessions, SignInAdmission, Site},
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

#[derive(Default)]
struct MemoryVault(std::sync::Mutex<HashMap<String, String>>);

impl SecretVault for MemoryVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(account.into(), secret.into());
        Ok(())
    }
    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }
    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(account);
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
    fetched: Option<DateTime<Utc>>,
    next_eligible: Option<DateTime<Utc>>,
) -> (Key, KeyState) {
    (
        key(dataset, name),
        KeyState {
            dataset: dataset.name().into(),
            key: name.into(),
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

/// Both profiles signed in; every 教務 page read a day ago; no catalog year.
fn inputs(states: Vec<(Key, KeyState)>) -> Inputs {
    let mut all: HashMap<Key, KeyState> = planner::kyomu_keys()
        .into_iter()
        .map(|page| {
            stored(
                Dataset::KyomuPage,
                &page.key,
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
        ..Facts::default()
    }
}

/// An account whose every site holds a session, its identity valid.
fn account(sites: &[Site]) -> AccountFacts {
    AccountFacts {
        admitted: true,
        identity: true,
        sign_in: SignInAdmission::Allowed,
        sites: sites.to_vec(),
    }
}

/// An account whose sessions are all gone and whose identity the school
/// found invalid: only a sign-in, as `sign_in` admits it, gets a new one.
fn gone(sign_in: SignInAdmission) -> AccountFacts {
    AccountFacts {
        identity: false,
        sign_in,
        ..account(&[])
    }
}

/// Both accounts signed in, the LMS token held.
fn credentials() -> Credentials {
    Credentials {
        version: 1,
        science: account(&[Site::Lms]),
        tokyo_tech: account(&[Site::Kyomu, Site::KyomuEnglish]),
        token: TokenFact::Held {
            unavailable: HashSet::new(),
        },
    }
}

fn plan(inputs: &Inputs, facts: &Facts, opens: &[View], manual: &[Key]) -> Vec<Candidate> {
    plan_with(inputs, facts, &credentials(), opens, manual)
}

fn plan_with(
    inputs: &Inputs,
    facts: &Facts,
    credentials: &Credentials,
    opens: &[View],
    manual: &[Key],
) -> Vec<Candidate> {
    let manual = manual.iter().map(|key| (key.clone(), None)).collect();
    planner::plan(
        inputs,
        facts,
        credentials,
        opens,
        &manual,
        &HashSet::new(),
        now(),
    )
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
        stored(Dataset::LmsEnrolment, "all", hours(1), None),
        stored(Dataset::LmsDeadlines, "all", hours(3), None),
        stored(Dataset::LmsAssign, "101", hours(1), None),
        stored(Dataset::LmsCourse, "102", hours(40 * 24), None),
        stored(
            Dataset::LmsAssign,
            "102",
            hours(40 * 24),
            Some(now() + Duration::minutes(10)),
        ),
        stored(Dataset::LmsCourse, "103", hours(400 * 24), None),
        // Never read: missing.
        stored(Dataset::KyomuPage, "gpa", None, None),
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
    assert_eq!(
        find(&plan, &key(Dataset::LmsCourse, "104")).unwrap().held,
        None
    );
    assert_eq!(
        find(&plan, &key(Dataset::LmsAssign, "104")).unwrap().held,
        Some(Held::Prerequisite)
    );
    // Search is local only, so the catalog covers the course families no
    // department owns (liberal arts, entrepreneurship, cross-departmental).
    let prefixes = planner::department_prefixes();
    for family in ["LA", "ENT", "X", "MTH"] {
        assert!(prefixes.iter().any(|prefix| prefix == family), "{family}");
    }
    // The leased key is single-flight: never planned twice.
    let leased = HashSet::from([key(Dataset::LmsCourse, "101")]);
    let again = planner::plan(
        &inputs,
        &facts(),
        &credentials(),
        &[],
        &HashMap::new(),
        &leased,
        now(),
    );
    assert!(find(&again, &key(Dataset::LmsCourse, "101")).is_none());
}

#[test]
fn an_open_page_renews_after_fifteen_minutes_and_a_refresh_ignores_ttl_and_backoff() {
    let minutes = |minutes: i64| Some(now() - Duration::minutes(minutes));
    let inputs = inputs(vec![
        stored(Dataset::LmsCourse, "101", minutes(20), None),
        stored(Dataset::LmsAssign, "101", minutes(5), None),
        stored(Dataset::LmsCourse, "103", minutes(400 * 24 * 60), None),
        stored(Dataset::LmsAssign, "103", minutes(400 * 24 * 60), None),
        stored(
            Dataset::LmsCourse,
            "104",
            minutes(30),
            Some(now() + Duration::minutes(10)),
        ),
        stored(Dataset::LmsCourse, "102", minutes(60), None),
        stored(
            Dataset::LmsAssign,
            "102",
            minutes(60),
            Some(now() + Duration::minutes(10)),
        ),
        stored(Dataset::LmsEnrolment, "all", minutes(1), None),
        stored(Dataset::LmsDeadlines, "all", minutes(1), None),
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
    assert_eq!(
        priority(&plan_now(&inputs, &facts()), &english),
        Some((Priority::Interactive, Some(Held::Prerequisite)))
    );
    let (state_key, state) = stored(
        Dataset::CatalogOfficial,
        &japanese.key,
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

/// After a reset the open course page asks for its course before the
/// enrolment list that course lands on exists (2026-10-25: three courses
/// failed `parse` and backed off five minutes). The course waits for the
/// list, pulls it forward, is read the moment it lands, and a course the
/// list drops while its read is in flight waits again without a failure.
#[tokio::test]
async fn a_course_page_opened_before_the_enrolment_list_waits_for_it() {
    let opens = [View::Course {
        lms_course_id: 12470,
    }];
    let outline = key(Dataset::LmsCourse, "12470");
    let assign = key(Dataset::LmsAssign, "12470");
    let enrolment = key(Dataset::LmsEnrolment, "all");
    // Nothing stored, not even 教務, on cellular: only the page's reads go.
    let mut empty = inputs(Vec::new());
    empty.states.clear();
    empty.courses.clear();
    let cellular = Facts {
        network: Network::Cellular,
        ..facts()
    };
    let before = plan(&empty, &cellular, &opens, &[]);
    let held = |plan: &[Candidate], wanted: &Key| {
        find(plan, wanted).map(|candidate| (candidate.priority, candidate.held.clone()))
    };
    for waiting in [&outline, &assign] {
        assert_eq!(
            held(&before, waiting),
            Some((Priority::Interactive, Some(Held::Prerequisite)))
        );
    }
    assert_eq!(
        held(&before, &enrolment),
        Some((Priority::Interactive, None))
    );
    let first = before
        .iter()
        .find(|candidate| candidate.lane == Lane::Moodle && candidate.held.is_none())
        .unwrap();
    assert_eq!(first.key, enrolment);

    // The list landed with the course: it is read now, its assignments
    // after its outline.
    let mut listed = empty.clone();
    listed
        .states
        .extend([stored(Dataset::LmsEnrolment, "all", Some(now()), None)]);
    listed.courses.push(course(12470, 2026, 3));
    let after = plan(&listed, &cellular, &opens, &[]);
    assert_eq!(held(&after, &outline), Some((Priority::Interactive, None)));
    assert_eq!(
        held(&after, &assign),
        Some((Priority::Interactive, Some(Held::Prerequisite)))
    );
    assert!(find(&after, &enrolment).is_none());
    // A list without the course: nothing can read it, so nothing waits.
    listed.courses.clear();
    assert!(find(&plan(&listed, &cellular, &opens, &[]), &outline).is_none());

    // A course the list dropped while its outline was read: no failure,
    // no backoff, and the source shows none.
    let (engine, store) = engine("not-enrolled");
    engine
        .settle_for_test(
            Lease::detached(&store, outline.clone()),
            KeyEnd::NotEnrolled {
                source: String::new(),
            },
        )
        .await;
    let (state, run): (Option<KeyState>, (String, Option<String>)) = store
        .write(None, |transaction| {
            let state = sync_state::get(transaction, &outline)?;
            let run = transaction
                .query_row("SELECT outcome, error_kind FROM sync_runs", [], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .map_err(crate::data::DataError::from)?;
            Ok::<_, crate::data::DataError>((state, run))
        })
        .unwrap();
    assert_eq!(state.map_or(0, |state| state.failures), 0);
    assert_eq!(
        (run.0.as_str(), run.1.as_deref()),
        ("waiting", Some("not-enrolled"))
    );
    assert!(engine.snapshot().sources[0].failure.is_none());
}

#[test]
fn held_reasons_follow_the_shell_facts() {
    let mut inputs = inputs(vec![stored(
        Dataset::LmsDeadlines,
        "all",
        Some(now()),
        None,
    )]);
    // 教務 grades were never read.
    inputs.states.remove(&key(Dataset::KyomuPage, "grades"));
    inputs.references = vec![ReferenceWatch {
        key: key(Dataset::Reference, "calendar/2026"),
        index_url: crate::reference_refresh::PUBLIC_INDEX.into(),
        due: Due::Missing,
        stale_at: None,
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
    // In the background every source still reads; only a suspended app
    // holds its work.
    let background = with(&|facts| facts.foreground = false);
    assert_eq!(held(&inputs, &background, &grades), Some(None));
    let suspended = with(&|facts| facts.suspended = true);
    assert_eq!(
        held(&inputs, &suspended, &deadlines),
        Some(Some(Held::Background))
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
    // Without saved credentials, automatic 教務 keys are not wanted.
    let mut signed_out = inputs.clone();
    signed_out.tokyo_configured = false;
    assert_eq!(held(&signed_out, &facts(), &grades), None);
}

/// Whether a due key waits is derived per key from the credentials on its
/// path and their owner's facts now, never stored per account: after the
/// school refused the password, a token or a session that still works keeps
/// its keys reading (`approach/sync-account-hold`).
#[test]
fn a_key_waits_only_when_no_credential_on_its_path_can_serve_it() {
    let deadlines = key(Dataset::LmsDeadlines, "all");
    let enrolment = key(Dataset::LmsEnrolment, "all");
    let grades = key(Dataset::KyomuPage, "grades");
    // Deadlines stale: a light read. The enrolment list asked for by hand:
    // its full read takes every quarter's grid from the session's dashboard.
    let mut inputs = inputs(vec![
        stored(
            Dataset::LmsDeadlines,
            "all",
            Some(now() - Duration::hours(3)),
            None,
        ),
        stored(Dataset::LmsEnrolment, "all", Some(now()), None),
    ]);
    inputs.states.remove(&grades);
    let held = |credentials: &Credentials, wanted: &Key| {
        let plan = plan_with(
            &inputs,
            &facts(),
            credentials,
            &[],
            std::slice::from_ref(&enrolment),
        );
        find(&plan, wanted).map(|candidate| candidate.held.clone())
    };
    let science = |wait| {
        Some(Some(Held::Account {
            profile: SchoolAuthProfile::Science,
            wait,
        }))
    };
    for wanted in [&deadlines, &enrolment, &grades] {
        assert_eq!(held(&credentials(), wanted), Some(None), "{wanted}");
    }

    // The saved password refused, the LMS session and identity gone, the
    // token alive: light LMS reads go on over the token; the full enrolment
    // needs the session and waits for the person. 教務 is another account.
    let refused = Credentials {
        science: gone(SignInAdmission::Person),
        ..credentials()
    };
    assert_eq!(held(&refused, &deadlines), Some(None));
    assert_eq!(held(&refused, &enrolment), science(AccountWait::Person));
    assert_eq!(held(&refused, &grades), Some(None));
    // The token confirmed absent too: nothing serves the deadlines.
    let no_token = Credentials {
        token: TokenFact::Absent { launchable: true },
        ..refused.clone()
    };
    assert_eq!(held(&no_token, &deadlines), science(AccountWait::Person));
    // Not looked for yet (a cold start, a vault read that failed): try-able.
    let unchecked = Credentials {
        token: TokenFact::Unchecked,
        ..refused.clone()
    };
    assert_eq!(held(&unchecked, &deadlines), Some(None));

    // A valid identity snapshot: the site's SSO gets a new session without
    // a password, so nothing waits.
    let silent = Credentials {
        science: AccountFacts {
            identity: true,
            ..gone(SignInAdmission::Person)
        },
        ..no_token.clone()
    };
    for wanted in [&deadlines, &enrolment] {
        assert_eq!(held(&silent, wanted), Some(None), "{wanted}");
    }

    // A pause after a sign-in without a verdict holds only the keys with no
    // usable credential, until it runs out.
    let until = now() + Duration::minutes(5);
    let paused = Credentials {
        science: gone(SignInAdmission::Paused { until }),
        ..credentials()
    };
    assert_eq!(held(&paused, &deadlines), Some(None));
    assert_eq!(
        held(&paused, &enrolment),
        science(AccountWait::Paused { until })
    );
    let over = Credentials {
        science: gone(SignInAdmission::Paused {
            until: now() - Duration::seconds(1),
        }),
        ..credentials()
    };
    assert_eq!(held(&over, &enrolment), Some(None));

    // Admission first: without saved credentials, or with a sign-out
    // pending, even a held token is not used.
    let unadmitted = Credentials {
        science: AccountFacts {
            admitted: false,
            ..account(&[Site::Lms])
        },
        ..credentials()
    };
    assert_eq!(held(&unadmitted, &deadlines), science(AccountWait::Person));
    assert_eq!(held(&unadmitted, &grades), Some(None));
}

/// 教務Web in Japanese and in English are separate sessions: with the
/// Japanese one gone and the password refused, the English catalog still
/// reads once the Japanese rows it merges into exist.
#[test]
fn the_english_catalog_reads_on_its_own_session() {
    let department = planner::department_prefixes()[0].clone();
    let japanese = key(Dataset::CatalogOfficial, &format!("2026/{department}"));
    let english = key(Dataset::CatalogOfficial, &format!("2026/{department}/en"));
    let mut inputs = inputs(Vec::new());
    inputs.year = Some(2026);
    inputs.english = true;
    let english_only = Credentials {
        tokyo_tech: AccountFacts {
            sites: vec![Site::KyomuEnglish],
            ..gone(SignInAdmission::Person)
        },
        ..credentials()
    };
    let finder = [View::Finder { year: 2026 }];
    let held = |inputs: &Inputs, wanted: &Key| {
        let plan = plan_with(inputs, &facts(), &english_only, &finder, &[]);
        find(&plan, wanted).map(|candidate| candidate.held.clone())
    };
    let person = Some(Some(Held::Account {
        profile: SchoolAuthProfile::TokyoTech,
        wait: AccountWait::Person,
    }));
    assert_eq!(held(&inputs, &japanese), person);
    assert_eq!(held(&inputs, &english), Some(Some(Held::Prerequisite)));
    let (state_key, state) = stored(
        Dataset::CatalogOfficial,
        &japanese.key,
        Some(now() - Duration::days(8)),
        None,
    );
    inputs.states.insert(state_key, state);
    assert_eq!(held(&inputs, &english), Some(None));
    assert_eq!(held(&inputs, &japanese), person);
}

/// A key that ended waiting is not leased again at that depth while the
/// credential facts stay at the version its wait was decided under, whatever
/// the derived rule says: its own read found what the facts do not show
/// (here the token answered the enrolment's dashboard block with no grid the
/// parser reads, and the session behind the cookie fallback was refused).
/// That record never holds another key: the deadlines use the same token and
/// go on, and so does the same key asked for in full. Any move of the facts
/// lets it go, and a pause it waited for runs out on its own.
#[test]
fn a_key_that_ended_waiting_waits_until_the_facts_move_and_holds_no_other() {
    let deadlines = key(Dataset::LmsDeadlines, "all");
    let enrolment = key(Dataset::LmsEnrolment, "all");
    let inputs = inputs(vec![
        stored(
            Dataset::LmsDeadlines,
            "all",
            Some(now() - Duration::hours(3)),
            None,
        ),
        stored(
            Dataset::LmsEnrolment,
            "all",
            Some(now() - Duration::days(2)),
            None,
        ),
    ]);
    let until = now() + Duration::minutes(1);
    let wait = |until| Held::Account {
        profile: SchoolAuthProfile::Science,
        wait: AccountWait::Paused { until },
    };
    let waited = |until| Facts {
        waited: HashMap::from([(
            (enrolment.clone(), Depth::Light),
            Waited {
                version: 1,
                held: wait(until),
            },
        )]),
        ..facts()
    };
    let held = |facts: &Facts, credentials: &Credentials, manual: &[Key], wanted: &Key| {
        let plan = plan_with(&inputs, facts, credentials, &[], manual);
        find(&plan, wanted).map(|candidate| (candidate.depth, candidate.held.clone()))
    };
    assert_eq!(
        held(&waited(until), &credentials(), &[], &enrolment),
        Some((Depth::Light, Some(wait(until))))
    );
    assert_eq!(
        held(&waited(until), &credentials(), &[], &deadlines),
        Some((Depth::Light, None))
    );
    assert_eq!(
        held(
            &waited(until),
            &credentials(),
            std::slice::from_ref(&enrolment),
            &enrolment
        ),
        Some((Depth::Full, None))
    );
    // Stamped with a version older than the facts now: it holds nothing.
    let moved = Credentials {
        version: 2,
        ..credentials()
    };
    assert_eq!(
        held(&waited(until), &moved, &[], &enrolment),
        Some((Depth::Light, None))
    );
    let ran_out = waited(now() - Duration::seconds(1));
    assert_eq!(
        held(&ran_out, &credentials(), &[], &enrolment),
        Some((Depth::Light, None))
    );
}

/// A REST function the token cannot serve moves only the keys that call it
/// to the session cookie (`approach/lms-token-function-fallback-evidence`);
/// the token stays good for the others, and a token renewed drops the
/// evidence with the old one.
#[test]
fn a_function_the_token_cannot_serve_moves_only_its_keys_to_the_cookie() {
    let course = key(Dataset::LmsCourse, "101");
    let deadlines = key(Dataset::LmsDeadlines, "all");
    let token = TokenFact::Held {
        unavailable: HashSet::from([MoodleFunction::PageGetPagesByCourses]),
    };
    let lms = Credential::Site(Site::Lms);
    assert_eq!(
        planner::usable_path(&course, Depth::Light, &token),
        vec![lms]
    );
    assert_eq!(
        planner::usable_path(&deadlines, Depth::Light, &token),
        vec![Credential::Token, lms]
    );
    assert_eq!(school::via(&course, Depth::Light, &token), Via::Cookie);
    assert_eq!(school::via(&deadlines, Depth::Light, &token), Via::Token);
    // The full enrolment reads on the session whatever the token knows; a
    // token the LMS offers none of is not launched for again.
    let enrolment = key(Dataset::LmsEnrolment, "all");
    assert_eq!(
        school::via(&enrolment, Depth::Full, &TokenFact::Unchecked),
        Via::Cookie
    );
    assert_eq!(
        school::via(&deadlines, Depth::Light, &TokenFact::Absent { launchable: true }),
        Via::Token
    );
    assert_eq!(
        school::via(&deadlines, Depth::Light, &TokenFact::Absent { launchable: false }),
        Via::Cookie
    );
    // The session gone and unrecoverable: the course waits, the deadlines
    // read over the token.
    let inputs = inputs(vec![
        stored(Dataset::LmsCourse, "101", Some(now() - Duration::days(2)), None),
        stored(
            Dataset::LmsDeadlines,
            "all",
            Some(now() - Duration::hours(3)),
            None,
        ),
    ]);
    let refused = Credentials {
        science: gone(SignInAdmission::Person),
        token,
        ..credentials()
    };
    let plan = plan_with(&inputs, &facts(), &refused, &[], &[]);
    assert!(matches!(
        find(&plan, &course).unwrap().held,
        Some(Held::Account { .. })
    ));
    assert_eq!(find(&plan, &deadlines).unwrap().held, None);
}

fn engine(name: &str) -> (PlanEngine, DataStore) {
    engine_with(name, Arc::new(SchoolAuthService::new(EmptyVault)))
}

fn engine_with(name: &str, auth: Arc<SchoolAuthService>) -> (PlanEngine, DataStore) {
    let path = std::env::temp_dir().join(format!("isct-sync-{name}-{}", uuid::Uuid::new_v4()));
    let store = DataStore::initialize_dir(path).unwrap().0;
    store
        .put_workspace_setting("selectedTerm", serde_json::json!("2026-3Q"))
        .unwrap();
    let engine = PlanEngine::new(store.clone(), auth, Network::Wifi);
    (engine, store)
}

/// An engine whose Tokyo Tech account is set up, its school sessions
/// attached (they never reach the network).
fn tokyo_engine(name: &str) -> (PlanEngine, DataStore, SchoolSessions) {
    let auth = Arc::new(SchoolAuthService::new(MemoryVault::default()));
    auth.promote(
        SchoolAuthProfile::TokyoTech,
        &crate::school_auth::SchoolCredentials {
            username: "student".into(),
            password: "secret".into(),
            otp_secret: String::new(),
            matrix: "A".repeat(70),
        },
    )
    .unwrap();
    let (engine, store) = engine_with(name, auth.clone());
    let sessions = SchoolSessions::new(None, auth.clone());
    engine.attach_school(SchoolReaders {
        sessions: sessions.clone(),
        moodle: Arc::new(MoodleClient::new(auth, sessions.credential_signal())),
    });
    (engine, store, sessions)
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

/// The `sync_runs` row of the last ending: outcome and error kind.
fn last_run(store: &DataStore) -> (String, Option<String>) {
    store
        .write(None, |transaction| {
            transaction
                .query_row(
                    "SELECT outcome, error_kind FROM sync_runs ORDER BY rowid DESC LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(crate::data::DataError::from)
        })
        .unwrap()
}

fn failures(store: &DataStore, key: &Key) -> u32 {
    store
        .write(None, |transaction| sync_state::get(transaction, key))
        .unwrap()
        .map_or(0, |state| state.failures)
}

#[tokio::test]
async fn a_paused_host_or_a_blocked_account_holds_keys_without_failing_them() {
    let (engine, store, _) = tokyo_engine("paused");
    let page = key(Dataset::KyomuPage, "grades");
    let until = Utc::now() + Duration::minutes(5);
    engine
        .settle_for_test(
            Lease::detached(&store, page.clone()),
            KeyEnd::HostPaused {
                host: Host::Kyomu,
                until,
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
    assert_eq!(failures(&store, &page), 0);
    assert_eq!(last_run(&store).1.as_deref(), Some("host-paused"));

    // A read that ended waiting (its sign-in was refused) is no failure of
    // the key, and no key of its path is leased again while the account's
    // credential facts stay as they were; any move of them (a Connect, a
    // Retry, a session got) lets them go at once.
    let (engine, store, sessions) = tokyo_engine("blocked");
    let signal = sessions.credential_signal();
    let refused_at = |version: u64| KeyEnd::Blocked {
        profile: SchoolAuthProfile::TokyoTech,
        wait: AccountWait::Person,
        version,
        source: String::new(),
    };
    let held_by_person = |plan: &[Candidate], wanted: &Key| {
        find(plan, wanted).unwrap().held
            == Some(Held::Account {
                profile: SchoolAuthProfile::TokyoTech,
                wait: AccountWait::Person,
            })
    };
    engine.evaluate_now().await;
    let lease = engine.lease_for_test(Lane::Kyomu).unwrap();
    let refused = lease.key.clone();
    engine
        .settle_for_test(lease, refused_at(signal.version()))
        .await;
    assert_eq!(failures(&store, &refused), 0);
    assert_eq!(last_run(&store).1.as_deref(), Some("sign-in"));
    let plan = engine.evaluate_now().await;
    assert!(held_by_person(&plan, &refused));
    // Its siblings wait only by the facts, which say a session can be had.
    assert!(plan
        .iter()
        .any(|candidate| candidate.key != refused && candidate.held.is_none()));
    signal.changed();
    assert!(!held_by_person(&engine.evaluate_now().await, &refused));

    // Decided before a Connect moved the facts, settled after it: stale,
    // so only its run is kept and the key goes on.
    let lease = engine.lease_for_test(Lane::Kyomu).unwrap();
    let late = lease.key.clone();
    let decided = signal.version();
    signal.changed();
    engine.settle_for_test(lease, refused_at(decided)).await;
    assert_eq!(last_run(&store).1.as_deref(), Some("sign-in"));
    assert!(!held_by_person(&engine.evaluate_now().await, &late));

    // A sign-in that found the shell offline sent nothing: no failure, and
    // the refresh button's press stays for when the network is back.
    engine.refresh_for_test(Scope::Site {
        site: SchoolSite::Kyomu,
    });
    engine.evaluate_now().await;
    let offline = engine.lease_for_test(Lane::Kyomu).unwrap();
    let pressed = offline.key.clone();
    engine
        .settle_for_test(
            offline,
            KeyEnd::Offline {
                source: String::new(),
            },
        )
        .await;
    assert_eq!(failures(&store, &pressed), 0);
    assert_eq!(
        last_run(&store),
        ("waiting".to_string(), Some("offline".to_string()))
    );
    let plan = engine.evaluate_now().await;
    let again = find(&plan, &pressed).unwrap();
    assert_eq!(
        (again.priority, again.held.clone()),
        (Priority::Interactive, None)
    );
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

/// The plan's own deadline includes what is not due yet: a fresh key at the
/// end of its TTL, an open page's key at fifteen minutes, a backoff at its
/// end and a paused host at its end. A key wanted by hand is due now, so it
/// is in the plan, not in the deadline; with every key due or waiting for a
/// fact, there is no time to wake for.
#[test]
fn the_next_deadline_includes_keys_that_are_not_due_yet() {
    let deadlines = key(Dataset::LmsDeadlines, "all");
    let enrolment = key(Dataset::LmsEnrolment, "all");
    let inputs = inputs(vec![
        stored(Dataset::LmsDeadlines, "all", Some(now() - Duration::minutes(10)), None),
        stored(
            Dataset::LmsEnrolment,
            "all",
            Some(now() - Duration::hours(1)),
            Some(now() + Duration::minutes(20)),
        ),
    ]);
    let next = |opens: &[View], facts: &Facts| {
        let plan = plan(&inputs, facts, opens, &[]);
        planner::next_due(&inputs, &plan, opens, &HashMap::new(), now())
    };
    // The deadlines key goes stale at its two-hour TTL; the enrolment list
    // backs off for twenty minutes, sooner.
    assert_eq!(next(&[], &facts()), Some(now() + Duration::minutes(20)));
    // An open page renews the deadlines at fifteen minutes.
    assert_eq!(
        next(&[View::Assignments], &facts()),
        Some(now() - Duration::minutes(10) + planner::OPEN_THRESHOLD)
    );
    let mut paused = facts();
    paused
        .paused
        .insert(Host::Moodle, now() + Duration::minutes(5));
    assert!(next(&[], &paused).unwrap() <= now() + Duration::minutes(20));
    let manual: HashMap<Key, Option<String>> =
        [(deadlines.clone(), None), (enrolment.clone(), None)].into();
    let wanted = planner::plan(
        &inputs,
        &facts(),
        &credentials(),
        &[],
        &manual,
        &HashSet::new(),
        now(),
    );
    assert!(find(&wanted, &deadlines).is_some());
}

/// A one-off read is admitted by the engine's rules: held now (offline, a
/// reset) it is refused at once with the reason, never queued behind a
/// fact. Two equal reads share one run of the work, which the engine owns.
#[tokio::test]
async fn a_one_off_read_is_held_like_a_key_and_joins_its_equal() {
    let (engine, _store) = engine("read");
    let request = |key: &str| ReadRequest {
        key: key.into(),
        lane: Lane::Moodle,
        account: None,
        deadline: std::time::Duration::from_secs(5),
    };
    engine.hint(Hint::Network {
        network: Network::Offline,
    });
    let held = engine
        .read(request("a"), |_| Box::pin(async { 1 }))
        .await;
    assert_eq!(held, Err(ReadRefused::Held(Held::Offline)));

    engine.hint(Hint::Network {
        network: Network::Wifi,
    });
    let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let release = Arc::new(tokio::sync::Notify::new());
    let read = |engine: PlanEngine| {
        let runs = runs.clone();
        let release = release.clone();
        async move {
            engine
                .read(request("b"), move |_| {
                    Box::pin(async move {
                        runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        release.notified().await;
                        7
                    })
                })
                .await
        }
    };
    let first = tokio::spawn(read(engine.clone()));
    let second = tokio::spawn(read(engine.clone()));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    release.notify_one();
    assert_eq!(first.await.unwrap(), Ok(7));
    assert_eq!(second.await.unwrap(), Ok(7));
    assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);

    let _admission = engine.close_admission().await;
    assert_eq!(
        engine.read(request("c"), |_| Box::pin(async { 1 })).await,
        Err(ReadRefused::Held(Held::Resetting))
    );
}

/// An LMS write starts on a key's resource: a read leased before it never
/// commits after it, and the next lease of the key, taken after, does.
#[tokio::test]
async fn a_read_leased_before_a_write_does_not_commit_after_it() {
    let (engine, _store) = engine("resource");
    engine.evaluate_now().await;
    let before = engine.lease_for_test(Lane::IsctData).unwrap();
    let revision = engine.bump_resource(&before.key);
    assert_eq!(revision, 1);
    assert!(before.writer().enter().is_none());
    engine
        .settle_for_test(before, KeyEnd::Stale)
        .await;
    engine.evaluate_now().await;
    let after = engine.lease_for_test(Lane::IsctData).unwrap();
    assert!(after.writer().enter().is_some());
}

/// A duty that is due once, counting its runs.
struct Once {
    work: BackgroundWork,
    due: std::sync::Mutex<bool>,
    runs: std::sync::atomic::AtomicUsize,
}

impl Once {
    fn new(work: BackgroundWork) -> Arc<Self> {
        Arc::new(Self {
            work,
            due: std::sync::Mutex::new(true),
            runs: std::sync::atomic::AtomicUsize::new(0),
        })
    }
    fn runs(&self) -> usize {
        self.runs.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Duty for Once {
    fn name(&self) -> &'static str {
        "once"
    }
    fn work(&self) -> BackgroundWork {
        self.work
    }
    fn next_due(&self) -> Option<DateTime<Utc>> {
        self.due.lock().unwrap().then_some(DateTime::UNIX_EPOCH)
    }
    fn run(self: Arc<Self>) -> crate::net::BoxFuture<'static, ()> {
        Box::pin(async move {
            *self.due.lock().unwrap() = false;
            self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
    }
}

/// The engine runs a due duty without anyone polling it, once. A suspended
/// app runs only what its background opportunity allows: local work in an
/// opportunity without the network, everything in one with it; the
/// opportunity's wait for quiet ends once that work is done.
#[tokio::test]
async fn a_suspended_engine_runs_only_what_its_opportunity_allows() {
    let (engine, _store) = engine("duties");
    engine.hint(Hint::Network {
        network: Network::Offline,
    });
    engine.hint(Hint::Background);
    engine.hint(Hint::Suspend);
    let local = Once::new(BackgroundWork::Local);
    let network = Once::new(BackgroundWork::Network);
    engine.register(local.clone());
    engine.register(network.clone());
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!((local.runs(), network.runs()), (0, 0));

    engine.begin_opportunity("local".into(), false);
    assert!(engine.wait_quiet(std::time::Duration::from_secs(5)).await);
    assert_eq!((local.runs(), network.runs()), (1, 0));
    engine.end_opportunity("local", OpportunityEnd::Finished);

    engine.begin_opportunity("network".into(), true);
    // Android runs a local opportunity beside it; ending one leaves the
    // other's work alone.
    engine.begin_opportunity("local-2".into(), false);
    assert!(engine.wait_quiet(std::time::Duration::from_secs(5)).await);
    assert_eq!((local.runs(), network.runs()), (1, 1));
    assert!(!engine.end_opportunity("network", OpportunityEnd::Expired));
    assert!(!engine.in_front());
    assert!(!engine.end_opportunity("unknown", OpportunityEnd::Expired));
    assert!(engine.end_opportunity("local-2", OpportunityEnd::Finished));
}
