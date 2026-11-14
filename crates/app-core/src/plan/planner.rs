//! What is due, in which order, and why due work waits. Pure over the
//! stored facts ([`Inputs`]), the shell's facts ([`Facts`]) and the school
//! sessions' credential facts ([`Credentials`]), so the rules can be read
//! (and tested) without a runner.
use super::{AccountWait, Held, Network, Priority, Scope, View};
use crate::{
    lifecycle::SchoolSite,
    lms_ingest::{current_lms_term, lms_course_term, StoredLmsCourse},
    net::{moodle::TokenFact, Host},
    school_auth::SchoolAuthProfile,
    school_session::{AccountFacts, SignInAdmission, Site},
    sync_state::{self, Dataset, Due, Key, KeyState, Term},
};
use chrono::{DateTime, Datelike, Duration, Utc};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap, HashSet},
    sync::OnceLock,
};

/// A page that became visible renews its keys once they are this old.
pub(crate) const OPEN_THRESHOLD: Duration = Duration::minutes(15);

/// Where a key is read: one lane per host, with that host's key budget.
/// Requests within a key are paced again by `net::Http`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub(crate) enum Lane {
    Moodle,
    Kyomu,
    IsctData,
}

impl Lane {
    pub(crate) const ALL: [Lane; 3] = [Lane::Moodle, Lane::Kyomu, Lane::IsctData];

    pub(crate) fn of(dataset: Dataset) -> Self {
        match dataset {
            Dataset::LmsEnrolment
            | Dataset::LmsDeadlines
            | Dataset::LmsCourse
            | Dataset::LmsAssign
            | Dataset::LmsHistory => Self::Moodle,
            Dataset::KyomuPage | Dataset::CatalogOfficial => Self::Kyomu,
            Dataset::Reference => Self::IsctData,
        }
    }

    /// Keys read at once: Moodle 4, 教務Web 1, Isct-Data 2.
    pub(crate) const fn capacity(self) -> usize {
        match self {
            Self::Moodle => 4,
            Self::Kyomu => 1,
            Self::IsctData => 2,
        }
    }

    pub(crate) const fn host(self) -> Host {
        match self {
            Self::Moodle => Host::Moodle,
            Self::Kyomu => Host::Kyomu,
            Self::IsctData => Host::IsctData,
        }
    }
}

/// The school profile whose session reads a dataset.
pub(crate) fn profile_of(dataset: Dataset) -> Option<SchoolAuthProfile> {
    match Lane::of(dataset) {
        Lane::Moodle => Some(SchoolAuthProfile::Science),
        Lane::Kyomu => Some(SchoolAuthProfile::TokyoTech),
        Lane::IsctData => None,
    }
}

/// One reference file the person follows.
#[derive(Clone, Debug)]
pub(crate) struct ReferenceWatch {
    pub(crate) key: Key,
    pub(crate) index_url: String,
    /// `reference_refresh::reference_due`: the validator and held-version
    /// rules of public files on top of `sync_state::due`.
    pub(crate) due: Due,
    /// When a reference that is fresh now goes stale.
    pub(crate) stale_at: Option<DateTime<Utc>>,
}

/// Stored facts, read once per evaluation on a blocking worker.
#[derive(Clone, Debug, Default)]
pub(crate) struct Inputs {
    pub(crate) states: HashMap<Key, KeyState>,
    pub(crate) courses: Vec<StoredLmsCourse>,
    /// Saved credentials exist, so Core reads the account's data on its own.
    pub(crate) science_configured: bool,
    pub(crate) tokyo_configured: bool,
    pub(crate) allow_cellular: bool,
    /// The academic year the person is viewing.
    pub(crate) year: Option<u16>,
    /// Content is shown in English: the catalog reads 教務's English page
    /// too.
    pub(crate) english: bool,
    pub(crate) references: Vec<ReferenceWatch>,
}

impl Inputs {
    fn configured(&self, profile: SchoolAuthProfile) -> bool {
        match profile {
            SchoolAuthProfile::Science => self.science_configured,
            SchoolAuthProfile::TokyoTech => self.tokyo_configured,
        }
    }

    fn course(&self, id: u64) -> Option<&StoredLmsCourse> {
        self.courses.iter().find(|course| course.id == id)
    }
}

/// What the shell has told Core, and what the runner saw of its reads.
#[derive(Clone, Debug)]
pub(crate) struct Facts {
    pub(crate) network: Network,
    pub(crate) foreground: bool,
    pub(crate) suspended: bool,
    pub(crate) power_saver: bool,
    pub(crate) resetting: bool,
    /// Hosts the circuit breaker paused, until when.
    pub(crate) paused: HashMap<Host, DateTime<Utc>>,
    /// Keys (at a depth) whose read ended waiting, with the credential
    /// facts' version the sessions decided under: that key is not leased
    /// again until the facts move (`hold`). It never holds another key.
    pub(crate) waited: HashMap<(Key, Depth), Waited>,
}

impl Default for Facts {
    fn default() -> Self {
        Self {
            network: Network::Unknown,
            foreground: true,
            suspended: false,
            power_saver: false,
            resetting: false,
            paused: HashMap::new(),
            waited: HashMap::new(),
        }
    }
}

/// A read that ended waiting (`KeyEnd::Blocked`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Waited {
    /// `Credentials::version` when it ended.
    pub(crate) version: u64,
    pub(crate) held: Held,
}

/// One credential a school read can use.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum Credential {
    /// The LMS mobile token: requests carry it alone, no site session.
    Token,
    /// A site session's instance (its cookie jar).
    Site(Site),
}

/// The credential facts of both accounts and the LMS token, read from the
/// school sessions (their owner) when the planner needs them; the planner
/// keeps none of them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Credentials {
    /// `CredentialSignal::version` when they were read.
    pub(crate) version: u64,
    pub(crate) science: AccountFacts,
    pub(crate) tokyo_tech: AccountFacts,
    /// The LMS token of the Science account's current generation.
    pub(crate) token: TokenFact,
}

impl Default for Credentials {
    /// No school sessions: no account is admitted.
    fn default() -> Self {
        Self {
            version: 0,
            science: AccountFacts::default(),
            tokyo_tech: AccountFacts::default(),
            token: TokenFact::Unchecked,
        }
    }
}

impl Credentials {
    fn account(&self, profile: SchoolAuthProfile) -> &AccountFacts {
        match profile {
            SchoolAuthProfile::Science => &self.science,
            SchoolAuthProfile::TokyoTech => &self.tokyo_tech,
        }
    }

    /// Whether `credential` may serve a read: held, or not yet looked for.
    fn usable(&self, account: &AccountFacts, credential: Credential) -> bool {
        match credential {
            Credential::Token => !matches!(self.token, TokenFact::Absent { .. }),
            Credential::Site(site) => account.sites.contains(&site),
        }
    }
}

/// The one table of which credentials a read of `key` at `depth` can use,
/// in the order it tries them; each site session recovers through the
/// identity's SSO, then a sign-in. The executors read with the same table
/// (`school::via`).
pub(crate) fn credential_path(key: &Key, depth: Depth) -> &'static [Credential] {
    const LMS: Credential = Credential::Site(Site::Lms);
    match key.dataset {
        // Every quarter's grid comes from the signed-in dashboard only.
        Dataset::LmsEnrolment if depth == Depth::Full => &[LMS],
        Dataset::LmsEnrolment
        | Dataset::LmsDeadlines
        | Dataset::LmsCourse
        | Dataset::LmsAssign
        | Dataset::LmsHistory => &[Credential::Token, LMS],
        Dataset::KyomuPage => &[Credential::Site(Site::Kyomu)],
        Dataset::CatalogOfficial if japanese_key(key).is_some() => {
            &[Credential::Site(Site::KyomuEnglish)]
        }
        Dataset::CatalogOfficial => &[Credential::Site(Site::Kyomu)],
        Dataset::Reference => &[],
    }
}

/// `credential_path` as the token's facts leave it: without the token when
/// it was found unable to serve one of the key's functions.
pub(crate) fn usable_path(key: &Key, depth: Depth, token: &TokenFact) -> Vec<Credential> {
    let unserved = match token {
        TokenFact::Held { unavailable } => crate::net::moodle::token_functions(key.dataset)
            .iter()
            .any(|function| unavailable.contains(function)),
        TokenFact::Unchecked | TokenFact::Absent { .. } => false,
    };
    credential_path(key, depth)
        .iter()
        .copied()
        .filter(|credential| !(unserved && *credential == Credential::Token))
        .collect()
}

/// Why `key` at `depth` waits for its account's credentials, if it does.
/// The account's admission comes first: without it no token or cookie is
/// used. Then the key waits only if every credential on its path is known
/// unusable and neither the identity's silent SSO nor a sign-in may get a
/// new session; the sign-in's admission says what it waits for. This key
/// at this depth, if its own read ended waiting under the facts as they
/// still are, waits again (no re-lease until they move); that record never
/// holds another key.
fn credential_hold(
    key: &Key,
    depth: Depth,
    facts: &Facts,
    credentials: &Credentials,
    now: DateTime<Utc>,
) -> Option<Held> {
    let profile = profile_of(key.dataset)?;
    let path = usable_path(key, depth, &credentials.token);
    account_hold(profile, &path, credentials, now).or_else(|| {
        facts
            .waited
            .get(&(key.clone(), depth))
            .filter(|waited| waited.version == credentials.version)
            .filter(|waited| {
                !matches!(
                    waited.held,
                    Held::Account { wait: AccountWait::Paused { until }, .. } if until <= now
                )
            })
            .map(|waited| waited.held.clone())
    })
}

/// Why a read of `profile`'s account over `path` waits for the account,
/// derived from the credential facts alone: no admission, or every
/// credential on the path known unusable while neither the identity's
/// silent SSO nor a sign-in may get a new one.
fn account_hold(
    profile: SchoolAuthProfile,
    path: &[Credential],
    credentials: &Credentials,
    now: DateTime<Utc>,
) -> Option<Held> {
    let account = credentials.account(profile);
    let wait = |wait: AccountWait| Held::Account { profile, wait };
    if !account.admitted {
        return Some(wait(AccountWait::Person));
    }
    let recoverable = path
        .iter()
        .any(|credential| credentials.usable(account, *credential))
        || account.identity;
    if recoverable {
        return None;
    }
    match account.sign_in {
        SignInAdmission::Allowed => None,
        SignInAdmission::Person => Some(wait(AccountWait::Person)),
        SignInAdmission::Paused { until } if until > now => {
            Some(wait(AccountWait::Paused { until }))
        }
        SignInAdmission::Paused { .. } => None,
    }
}

/// Why a one-off read (`runner::ReadRequest`) on `lane` over `path` may
/// not run now: the rules of [`hold`] at Interactive priority, with the
/// account's facts for `path` when `credentials` are the sessions'.
pub(crate) fn request_hold(
    lane: Lane,
    account: Option<(SchoolAuthProfile, &[Credential])>,
    facts: &Facts,
    credentials: Option<&Credentials>,
    now: DateTime<Utc>,
) -> Option<Held> {
    if facts.resetting {
        return Some(Held::Resetting);
    }
    if facts.network == Network::Offline {
        return Some(Held::Offline);
    }
    let host = lane.host();
    if let Some(until) = facts.paused.get(&host).filter(|until| **until > now) {
        return Some(Held::HostPaused {
            host: host.name().into(),
            until: *until,
        });
    }
    if let (Some((profile, path)), Some(credentials)) = (account, credentials) {
        if let Some(held) = account_hold(profile, path, credentials, now) {
            return Some(held);
        }
    }
    facts.suspended.then_some(Held::Background)
}

/// How a key came to be wanted; a stronger rule wins when several apply.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum Rule {
    /// Its dataset's TTL.
    Ttl,
    /// A visible page: the open threshold, backoff still applies.
    Open,
    /// The refresh button: TTL and backoff are ignored.
    Manual,
}

/// The run a key is shown in: one dataset, narrowed to a page by `scope`,
/// and for LMS courses split into the current term and earlier ones.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct Group {
    pub(crate) dataset: Dataset,
    pub(crate) scope: Option<String>,
    pub(crate) term: Option<&'static str>,
}

/// One due key, ready to lease unless `held`.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub(crate) key: Key,
    pub(crate) lane: Lane,
    pub(crate) priority: Priority,
    /// 0 missing, 1 stale; lower is leased first within a priority.
    pub(crate) rank: u8,
    pub(crate) held: Option<Held>,
    pub(crate) group: Group,
    /// The course title, page or department the key reads, for progress.
    pub(crate) title: Option<String>,
    /// The Isct-Data index of a reference key.
    pub(crate) locator: Option<String>,
    pub(crate) depth: Depth,
}

/// Which form of a key a read takes, where its reader has two.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum Depth {
    /// An automatic read: only what changes in use, over the light path.
    Light,
    /// The refresh button or the key's first read: everything the key holds.
    Full,
}

/// One key's attributes before the rules decide whether it is due.
#[derive(Clone, Debug)]
struct Watch {
    priority: Priority,
    rule: Rule,
    term: Term,
    scope: Option<String>,
    title: Option<String>,
    locator: Option<String>,
    due: Option<Due>,
}

/// The six 教務Web pages, in menu order.
pub(crate) fn kyomu_keys() -> Vec<Key> {
    adpt_isct::kyomu::PAGES
        .iter()
        .map(|(_, page)| Key::new(Dataset::KyomuPage, *page))
        .collect()
}

/// Courses no department owns: liberal arts (`LAS`, `LAH`, `LAE`, `LAW`…),
/// entrepreneurship and cross-departmental courses (`XEG`, `XCO`…). The
/// search matches code prefixes, so one chunk covers each family.
const SHARED_PREFIXES: [&str; 3] = ["LA", "ENT", "X"];

/// Code prefixes of the official course search: every department's, plus
/// the families no department owns. Search is local only, so a family left
/// out here is missing from every search.
pub(crate) fn department_prefixes() -> &'static [String] {
    static PREFIXES: OnceLock<Vec<String>> = OnceLock::new();
    PREFIXES.get_or_init(|| {
        serde_json::from_str::<Vec<crate::course_query::FinderDepartment>>(include_str!(
            "../course_query/departments.json"
        ))
        .map(|departments| {
            let mut prefixes: Vec<String> = departments
                .into_iter()
                .map(|department| department.code_prefix)
                .filter(|prefix| !prefix.is_empty())
                .chain(SHARED_PREFIXES.iter().map(|prefix| prefix.to_string()))
                .collect();
            prefixes.sort();
            prefixes.dedup();
            prefixes
        })
        .unwrap_or_default()
    })
}

/// The 教務 search of every department: `{year}/{prefix}` in Japanese, and
/// with `english` also `{year}/{prefix}/en`, the English page that adds
/// English titles and instructor names to the same rows.
pub(crate) fn official_keys(year: u16, english: bool) -> Vec<Key> {
    department_prefixes()
        .iter()
        .flat_map(|prefix| {
            let japanese = Key::new(Dataset::CatalogOfficial, format!("{year}/{prefix}"));
            let english =
                english.then(|| Key::new(Dataset::CatalogOfficial, format!("{year}/{prefix}/en")));
            std::iter::once(japanese).chain(english)
        })
        .collect()
}

/// The Japanese key an English catalog key adds names to.
pub(crate) fn japanese_key(key: &Key) -> Option<Key> {
    (key.dataset == Dataset::CatalogOfficial)
        .then(|| key.key.strip_suffix("/en"))
        .flatten()
        .map(|japanese| Key::new(Dataset::CatalogOfficial, japanese))
}

/// The one table of read order: the key whose stored rows a read of `key`
/// lands on. Until that key is read, `key` waits
/// ([`Held::Prerequisite`]) and the plan wants the prerequisite at `key`'s
/// priority.
pub(crate) fn prerequisite(key: &Key) -> Option<Key> {
    match key.dataset {
        // A course's outline lands on the course row the enrolment list
        // writes.
        Dataset::LmsCourse => Some(Key::lms_enrolment()),
        // A course's assignment states land on the rows its outline creates.
        Dataset::LmsAssign => Some(Key::new(Dataset::LmsCourse, key.key.clone())),
        // English names merge into rows the Japanese page stored.
        Dataset::CatalogOfficial => japanese_key(key),
        Dataset::LmsEnrolment
        | Dataset::LmsDeadlines
        | Dataset::LmsHistory
        | Dataset::KyomuPage
        | Dataset::Reference => None,
    }
}

/// Whether `key` was read.
fn read(key: &Key, inputs: &Inputs) -> bool {
    inputs
        .states
        .get(key)
        .is_some_and(|state| state.fetched_at.is_some())
}

/// `key`'s prerequisite, while it is not read.
fn missing_prerequisite(key: &Key, inputs: &Inputs) -> Option<Key> {
    prerequisite(key).filter(|prerequisite| !read(prerequisite, inputs))
}

/// A year's whole catalog: its official departments.
fn catalog_keys(year: u16, english: bool) -> Vec<(Key, Option<String>)> {
    official_keys(year, english)
        .into_iter()
        .map(|key| (key, Some(year.to_string())))
        .collect()
}

fn lms_all() -> [Key; 2] {
    [Key::lms_enrolment(), Key::lms_deadlines()]
}

/// The timetable grid: LMS quarter grids and 教務 registrations.
fn timetable_keys() -> Vec<Key> {
    vec![
        Key::lms_enrolment(),
        Key::new(Dataset::KyomuPage, "registrations"),
    ]
}

fn course_keys(id: u64) -> [Key; 2] {
    [
        Key::new(Dataset::LmsCourse, id.to_string()),
        Key::lms_assignments(id),
    ]
}

fn japan_month(now: DateTime<Utc>) -> (i32, u32) {
    let today = crate::schedule::japan_today(now);
    (today.year(), today.month())
}

/// The course and assignment keys of every current-term course.
pub(crate) fn current_course_keys(inputs: &Inputs, now: DateTime<Utc>) -> Vec<Key> {
    let current = current_lms_term(now);
    inputs
        .courses
        .iter()
        .filter(|course| lms_course_term(course, current) == Term::Current)
        .flat_map(|course| course_keys(course.id))
        .collect()
}

/// The keys a refresh button covers.
pub(crate) fn scope_keys(
    scope: &Scope,
    inputs: &Inputs,
    now: DateTime<Utc>,
) -> Vec<(Key, Option<String>)> {
    let unscoped = |keys: Vec<Key>| keys.into_iter().map(|key| (key, None)).collect::<Vec<_>>();
    match scope {
        Scope::Everything => {
            let mut keys = lms_all().to_vec();
            keys.extend(current_course_keys(inputs, now));
            keys.extend(kyomu_keys());
            keys.extend(inputs.references.iter().map(|watch| watch.key.clone()));
            unscoped(keys)
        }
        Scope::Site {
            site: SchoolSite::ScienceLms,
        } => {
            let mut keys = lms_all().to_vec();
            keys.extend(current_course_keys(inputs, now));
            unscoped(keys)
        }
        Scope::Site {
            site: SchoolSite::Kyomu,
        } => unscoped(kyomu_keys()),
        Scope::Site { .. } => Vec::new(),
        Scope::Course { lms_course_id } => course_view_keys(*lms_course_id),
        Scope::Timetable => unscoped(timetable_keys()),
        Scope::References => unscoped(
            inputs
                .references
                .iter()
                .map(|watch| watch.key.clone())
                .collect(),
        ),
        Scope::Catalog { year } => catalog_keys(*year, inputs.english),
    }
}

/// One course page: its LMS keys.
fn course_view_keys(id: u64) -> Vec<(Key, Option<String>)> {
    if id == 0 {
        return Vec::new();
    }
    course_keys(id)
        .into_iter()
        .map(|key| (key, Some(id.to_string())))
        .collect()
}

/// The keys a visible page depends on, their priority and rule.
fn view_keys(
    view: &View,
    inputs: &Inputs,
    now: DateTime<Utc>,
) -> (Vec<(Key, Option<String>)>, Priority, Rule) {
    let unscoped = |keys: Vec<Key>| keys.into_iter().map(|key| (key, None)).collect::<Vec<_>>();
    match view {
        View::Today => (unscoped(lms_all().to_vec()), Priority::Visible, Rule::Open),
        View::Assignments => {
            let mut keys = vec![Key::lms_deadlines()];
            keys.extend(
                current_course_keys(inputs, now)
                    .into_iter()
                    .filter(|key| key.dataset == Dataset::LmsAssign),
            );
            (unscoped(keys), Priority::Visible, Rule::Open)
        }
        View::Timetable => (unscoped(timetable_keys()), Priority::Visible, Rule::Open),
        View::Course { lms_course_id } => (
            course_view_keys(*lms_course_id),
            Priority::Interactive,
            Rule::Open,
        ),
        // The catalog changes little in a year: opening the finder raises
        // its departments, but each is still read only once its week is up.
        // A department never read is the one the finder is waiting for
        // (`first_read`).
        View::Finder { year } => (
            catalog_keys(*year, inputs.english),
            Priority::Visible,
            Rule::Ttl,
        ),
        View::Grades => (
            unscoped(
                ["gpa", "grades", "requirements"]
                    .into_iter()
                    .map(|page| Key::new(Dataset::KyomuPage, page))
                    .collect(),
            ),
            Priority::Visible,
            Rule::Open,
        ),
    }
}

/// A finder shows only what is stored: on a year with nothing stored it is
/// empty until its departments land, so each department never read is read
/// as if the person asked for it, before other due work and on any network.
/// A stored department, however old, keeps the view's own priority.
fn first_read(view: &View, key: &Key, inputs: &Inputs) -> bool {
    matches!(view, View::Finder { .. })
        && inputs
            .states
            .get(key)
            .is_none_or(|state| state.fetched_at.is_none())
}

/// The base attributes of `key`, or `None` when nothing can read it (a
/// course no longer listed, a reference kind that is off).
fn watch_for(key: &Key, inputs: &Inputs, now: DateTime<Utc>) -> Option<Watch> {
    let mut watch = Watch {
        priority: Priority::Background,
        rule: Rule::Ttl,
        term: Term::Current,
        scope: match key.dataset {
            Dataset::CatalogOfficial => key.key.split('/').next().map(str::to_owned),
            _ => None,
        },
        title: None,
        locator: None,
        due: None,
    };
    match key.dataset {
        Dataset::LmsCourse | Dataset::LmsAssign => {
            let id: u64 = key.key.parse().ok().filter(|id| *id > 0)?;
            match inputs.course(id) {
                Some(course) => {
                    watch.term = lms_course_term(course, current_lms_term(now));
                    watch.title = Some(course.title.clone());
                }
                // The stored enrolment list lacks it: not (or no longer) the
                // person's course, so the ingest would refuse its rows. It is
                // wanted again once a list that holds it lands.
                None if read(&Key::lms_enrolment(), inputs) => {
                    return None;
                }
                None => {}
            }
        }
        Dataset::LmsHistory => {
            let (year, month) = key.key.split_once('-')?;
            let month = (year.parse::<i32>().ok()?, month.parse::<u32>().ok()?);
            if month < japan_month(now) {
                watch.term = Term::Frozen;
            }
        }
        Dataset::KyomuPage => watch.title = Some(key.key.clone()),
        Dataset::CatalogOfficial => watch.title = key.key.split('/').nth(1).map(str::to_owned),
        Dataset::Reference => {
            let reference = inputs.references.iter().find(|watch| watch.key == *key)?;
            watch.locator = Some(reference.index_url.clone());
            watch.due = Some(reference.due);
            watch.title = Some(key.key.clone());
        }
        Dataset::LmsEnrolment | Dataset::LmsDeadlines => {}
    }
    Some(watch)
}

/// Keys Core keeps current without being asked, with their priority: an
/// account's once its credentials are saved.
fn automatic_keys(inputs: &Inputs, now: DateTime<Utc>) -> Vec<(Key, Priority)> {
    let mut keys = Vec::new();
    if inputs.configured(SchoolAuthProfile::Science) {
        keys.extend(lms_all().map(|key| (key, Priority::Visible)));
        let current = current_lms_term(now);
        for course in &inputs.courses {
            let priority = if lms_course_term(course, current) == Term::Current {
                Priority::Visible
            } else {
                Priority::Background
            };
            keys.extend(course_keys(course.id).map(|key| (key, priority)));
        }
        let today = crate::schedule::japan_today(now);
        let mut years: Vec<i32> = inputs
            .courses
            .iter()
            .filter_map(|course| course.year)
            .collect();
        years.sort_unstable_by(|a, b| b.cmp(a));
        years.dedup();
        for (year, month) in crate::net::moodle::months_to_read(&years, today) {
            keys.push((
                Key::new(Dataset::LmsHistory, format!("{year:04}-{month:02}")),
                Priority::Background,
            ));
        }
    }
    if inputs.configured(SchoolAuthProfile::TokyoTech) {
        keys.extend(kyomu_keys().into_iter().map(|key| (key, Priority::Visible)));
        if let Some(year) = inputs.year {
            keys.extend(
                official_keys(year, inputs.english)
                    .into_iter()
                    .map(|key| (key, Priority::Background)),
            );
        }
    }
    // The campus map is read ahead of the other files: the map buttons
    // appear only once it is stored.
    keys.extend(inputs.references.iter().map(|watch| {
        let priority = if crate::reference_refresh::raised(&watch.key) {
            Priority::Visible
        } else {
            Priority::Background
        };
        (watch.key.clone(), priority)
    }));
    keys
}

fn want(
    watched: &mut BTreeMap<Key, Watch>,
    key: Key,
    priority: Priority,
    rule: Rule,
    scope: Option<String>,
    inputs: &Inputs,
    now: DateTime<Utc>,
) {
    if let Some(existing) = watched.get_mut(&key) {
        existing.priority = existing.priority.max(priority);
        existing.rule = existing.rule.max(rule);
        if scope.is_some() {
            existing.scope = scope;
        }
        return;
    }
    if let Some(mut watch) = watch_for(&key, inputs, now) {
        watch.priority = priority;
        watch.rule = rule;
        if scope.is_some() {
            watch.scope = scope;
        }
        watched.insert(key, watch);
    }
}

/// Whether `key` is due under its rule: `Some(rank)` (0 missing, 1 stale),
/// with a backoff hold for a visible page's key that failed recently.
fn evaluate(
    key: &Key,
    watch: &Watch,
    state: Option<&KeyState>,
    now: DateTime<Utc>,
) -> Option<(u8, Option<Held>)> {
    let ttl = sync_state::ttl(key.dataset, watch.term);
    let fetched = state.and_then(|state| state.fetched_at);
    if watch.rule == Rule::Manual {
        return Some((u8::from(fetched.is_some()), None));
    }
    let due = watch
        .due
        .unwrap_or_else(|| sync_state::due(state, ttl, now));
    match due {
        Due::Missing => Some((0, None)),
        Due::Stale => Some((1, None)),
        Due::BackingOff(until) if watch.rule == Rule::Open => {
            Some((u8::from(fetched.is_some()), Some(Held::Backoff { until })))
        }
        Due::BackingOff(_) => None,
        Due::Fresh => {
            let threshold = ttl.map(|ttl| ttl.min(OPEN_THRESHOLD));
            match (watch.rule, threshold, fetched) {
                (Rule::Open, Some(threshold), Some(fetched)) if now >= fetched + threshold => {
                    Some((1, None))
                }
                _ => None,
            }
        }
    }
}

/// Why a due key may not be leased now, most fundamental reason first.
/// `plan` and the runner's lease both ask it, the lease with the facts as
/// they are at that moment.
pub(crate) fn hold(
    key: &Key,
    priority: Priority,
    depth: Depth,
    inputs: &Inputs,
    facts: &Facts,
    credentials: &Credentials,
    now: DateTime<Utc>,
) -> Option<Held> {
    let dataset = key.dataset;
    if facts.resetting {
        return Some(Held::Resetting);
    }
    if facts.network == Network::Offline {
        return Some(Held::Offline);
    }
    let host = Lane::of(dataset).host();
    if let Some(until) = facts.paused.get(&host).filter(|until| **until > now) {
        return Some(Held::HostPaused {
            host: host.name().into(),
            until: *until,
        });
    }
    if let Some(held) = credential_hold(key, depth, facts, credentials, now) {
        return Some(held);
    }
    if facts.suspended {
        return Some(Held::Background);
    }
    if priority < Priority::Interactive {
        // The campus map uses cellular too, as its owner asked.
        if facts.network == Network::Cellular
            && !inputs.allow_cellular
            && !crate::reference_refresh::raised(key)
        {
            return Some(Held::Cellular);
        }
        if facts.power_saver {
            return Some(Held::PowerSaver);
        }
    }
    // Last: every reason above holds the prerequisite as well.
    missing_prerequisite(key, inputs).map(|_| Held::Prerequisite)
}

/// Every key some rule wants, with its attributes: automatic keys, open
/// pages, refresh presses and the prerequisites they pull forward.
fn watched(
    inputs: &Inputs,
    opens: &[View],
    manual: &HashMap<Key, Option<String>>,
    now: DateTime<Utc>,
) -> BTreeMap<Key, Watch> {
    let mut watched = BTreeMap::new();
    for (key, priority) in automatic_keys(inputs, now) {
        want(&mut watched, key, priority, Rule::Ttl, None, inputs, now);
    }
    for view in opens {
        let (keys, priority, rule) = view_keys(view, inputs, now);
        for (key, scope) in keys {
            let priority = if first_read(view, &key, inputs) {
                Priority::Interactive
            } else {
                priority
            };
            want(&mut watched, key, priority, rule, scope, inputs, now);
        }
    }
    for (key, scope) in manual {
        want(
            &mut watched,
            key.clone(),
            Priority::Interactive,
            Rule::Manual,
            scope.clone(),
            inputs,
            now,
        );
    }
    // A key waiting for its prerequisite pulls that forward to its own
    // priority: an open course page reads the enrolment list first, on any
    // network the page itself may use.
    let mut waiting: Vec<(Key, Priority)> = watched
        .iter()
        .filter_map(|(key, watch)| Some((missing_prerequisite(key, inputs)?, watch.priority)))
        .collect();
    while let Some((key, priority)) = waiting.pop() {
        if let Some(prerequisite) = missing_prerequisite(&key, inputs) {
            waiting.push((prerequisite, priority));
        }
        want(&mut watched, key, priority, Rule::Ttl, None, inputs, now);
    }
    watched
}

/// Every due key that is not leased, in lease order: Interactive first,
/// missing before stale, then dataset and key order.
pub(crate) fn plan(
    inputs: &Inputs,
    facts: &Facts,
    credentials: &Credentials,
    opens: &[View],
    manual: &HashMap<Key, Option<String>>,
    leased: &HashSet<Key>,
    now: DateTime<Utc>,
) -> Vec<Candidate> {
    let watched = watched(inputs, opens, manual, now);
    let mut candidates: Vec<Candidate> = watched
        .into_iter()
        .filter(|(key, _)| !leased.contains(key))
        .filter_map(|(key, watch)| {
            let (rank, backoff) = evaluate(&key, &watch, inputs.states.get(&key), now)?;
            let depth = if watch.rule == Rule::Manual || rank == 0 {
                Depth::Full
            } else {
                Depth::Light
            };
            let held = hold(
                &key,
                watch.priority,
                depth,
                inputs,
                facts,
                credentials,
                now,
            )
            .or(backoff);
            let group = Group {
                dataset: key.dataset,
                scope: watch.scope.clone(),
                term: matches!(key.dataset, Dataset::LmsCourse | Dataset::LmsAssign).then_some(
                    if watch.term == Term::Current {
                        "current"
                    } else {
                        "past"
                    },
                ),
            };
            Some(Candidate {
                lane: Lane::of(key.dataset),
                priority: watch.priority,
                rank,
                held,
                group,
                title: watch.title,
                locator: watch.locator,
                depth,
                key,
            })
        })
        .collect();
    candidates.sort_by(|a, b| {
        (Reverse(a.priority), a.rank, &a.key).cmp(&(Reverse(b.priority), b.rank, &b.key))
    });
    candidates
}

/// When the plan next changes by time alone: the earliest moment a wanted
/// key that is fresh now goes stale (its TTL, or an open page's threshold),
/// a backoff ends, or a hold with an end lifts (a paused host, an
/// account's pause). Keys due now are in the plan already; a hold that
/// waits for a fact (the network, the person) has no time.
pub(crate) fn next_due(
    inputs: &Inputs,
    plan: &[Candidate],
    opens: &[View],
    manual: &HashMap<Key, Option<String>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let watched = watched(inputs, opens, manual, now);
    let fresh = watched.iter().filter_map(|(key, watch)| {
        if watch.rule == Rule::Manual {
            return None;
        }
        let state = inputs.states.get(key);
        if let Some(until) = state
            .and_then(|state| state.next_eligible)
            .filter(|until| *until > now)
        {
            return Some(until);
        }
        if key.dataset == Dataset::Reference {
            return inputs
                .references
                .iter()
                .find(|reference| reference.key == *key)
                .and_then(|reference| reference.stale_at);
        }
        let fetched = state.and_then(|state| state.fetched_at)?;
        let ttl = sync_state::ttl(key.dataset, watch.term)?;
        let window = if watch.rule == Rule::Open {
            ttl.min(OPEN_THRESHOLD)
        } else {
            ttl
        };
        Some(fetched + window).filter(|at| *at > now)
    });
    let held = plan.iter().filter_map(|candidate| match &candidate.held {
        Some(Held::Backoff { until })
        | Some(Held::HostPaused { until, .. })
        | Some(Held::Account {
            wait: AccountWait::Paused { until },
            ..
        }) => Some(*until),
        _ => None,
    });
    fresh.chain(held).min()
}
