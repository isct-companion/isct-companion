//! What is due, in which order, and why due work waits. Pure over the
//! stored facts ([`Inputs`]), the shell's facts ([`Facts`]) and the school
//! sessions' credential facts ([`Credentials`]), so the rules can be read
//! (and tested) without a runner.
use super::freshness;
use super::{AccountWait, Held, Network, Priority, Scope, View};
use crate::{
    lifecycle::SchoolSite,
    lms_ingest::{current_lms_term, lms_course_term, StoredLmsCourse},
    net::{moodle::TokenFact, Host},
    school_auth::SchoolAuthProfile,
    school_session::{AccountFacts, SignInAdmission, Site},
    sync_state::{self, Dataset, Due, Key, KeyState, Term},
};
use chrono::{DateTime, Duration, Utc};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap, HashSet},
    sync::OnceLock,
};

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
            Dataset::Reference | Dataset::Announcements => Self::IsctData,
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
    /// The catalog code prefix of the person's own department.
    pub(crate) own_prefix: Option<String>,
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
        Dataset::Reference | Dataset::Announcements => &[],
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

/// Why a key is wanted, leased in this order before priority
/// (`d/cy7j2w`): a refresh press, then what a visible page shows, then
/// what Core keeps current on its own, the catalog last (`d/8j3ytx`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum Tier {
    Manual,
    Shown,
    Automatic,
    Catalog,
}

/// One due key, ready to lease unless `held`.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub(crate) key: Key,
    pub(crate) lane: Lane,
    pub(crate) tier: Tier,
    /// Its place in the list of the page that shows it; within a tier and
    /// priority the page's order wins over the key's name.
    pub(crate) order: usize,
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
    tier: Tier,
    order: usize,
    /// The threshold a visible page renews it at (`freshness::open_threshold`),
    /// the shortest of the pages that show it.
    open: Option<Duration>,
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
        .map(|page| Key::new(Dataset::KyomuPage, page.kind))
        .collect()
}

/// Courses no department owns: liberal arts (`LAS`, `LAH`, `LAE`, `LAW`…),
/// entrepreneurship and cross-departmental courses (`XEG`, `XCO`…). The
/// search matches code prefixes, so one chunk covers each family. `true`:
/// every student searches the family, so the Finder reads it first beside
/// the person's own department; cross-departmental courses wait in the
/// background (`d/8j3ytx`).
const SHARED_PREFIXES: [(&str, bool); 3] = [("LA", true), ("ENT", true), ("X", false)];

/// The catalog code prefix of the department `id` names.
pub(crate) fn department_prefix(id: &str) -> Option<&'static str> {
    crate::course_query::SyllabusDepartment::by_id(id)
        .map(|department| department.code_prefix.as_str())
        .filter(|prefix| !prefix.is_empty())
}

/// Code prefixes of the official course search: every department's, plus
/// the families no department owns. Search is local only, so a family left
/// out here is missing from every search.
pub(crate) fn department_prefixes() -> &'static [String] {
    static PREFIXES: OnceLock<Vec<String>> = OnceLock::new();
    PREFIXES.get_or_init(|| {
        let mut prefixes: Vec<String> = crate::course_query::SyllabusDepartment::all()
            .iter()
            .map(|department| department.code_prefix.clone())
            .filter(|prefix| !prefix.is_empty())
            .chain(SHARED_PREFIXES.iter().map(|(prefix, _)| prefix.to_string()))
            .collect();
        prefixes.sort();
        prefixes.dedup();
        prefixes
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
        | Dataset::Reference
        | Dataset::Announcements => None,
    }
}

/// The key a read of `key` also reads, in the same operation: the declared
/// page is read with the registered list beside it, whose scope alone may
/// drop declared courses (`d/aevz1j`). A plan with both due reads the list
/// there, once ([`read_once`]).
pub(crate) fn read_beside(key: &Key) -> Option<Key> {
    (key.dataset == Dataset::KyomuPage && key.key == "declared")
        .then(|| Key::new(Dataset::KyomuPage, "registrations"))
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
            keys.push(crate::announcements::key());
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
                .chain(std::iter::once(crate::announcements::key()))
                .collect(),
        ),
        Scope::Catalog { year } => catalog_keys(*year, inputs.english),
        Scope::LmsHistory { year, month }
            if (1000..=9999).contains(year) && (1..=12).contains(month) =>
        {
            unscoped(vec![Key::new(
                Dataset::LmsHistory,
                format!("{year:04}-{month:02}"),
            )])
        }
        Scope::LmsHistory { .. } => Vec::new(),
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
        // The catalog changes little in a year and may be slow
        // (`d/8j3ytx`): opening the finder keeps its departments in the
        // background, read once their week is up. The few departments never
        // read that the person searches first are the ones the finder waits
        // for (`first_read`).
        View::Finder { year } => (
            catalog_keys(*year, inputs.english),
            Priority::Background,
            Rule::Ttl,
        ),
        // Grades answer what is new; the GPA and requirements follow them
        // (`sync_state::dependents`).
        View::Grades => (
            unscoped(
                ["grades", "gpa", "requirements"]
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
/// empty until its departments land. The person's own department and the
/// shared liberal-arts and entrepreneurship families, never read, are read
/// as if the person asked for them, on any network, though still after
/// what other visible pages need (`Tier::Catalog`); every other department
/// waits in the background. A stored department, however old, keeps the
/// view's own priority.
fn first_read(view: &View, key: &Key, inputs: &Inputs) -> bool {
    matches!(view, View::Finder { .. })
        && searched_first(key, inputs)
        && inputs
            .states
            .get(key)
            .is_none_or(|state| state.fetched_at.is_none())
}

/// Whether a catalog key reads the person's own department or a family
/// every student searches.
fn searched_first(key: &Key, inputs: &Inputs) -> bool {
    key.dataset == Dataset::CatalogOfficial
        && key.key.split('/').nth(1).is_some_and(|prefix| {
            SHARED_PREFIXES.contains(&(prefix, true))
                || inputs.own_prefix.as_deref() == Some(prefix)
        })
}

/// The base attributes of `key`, or `None` when nothing can read it (a
/// course no longer listed, a reference kind that is off).
fn watch_for(key: &Key, inputs: &Inputs, now: DateTime<Utc>) -> Option<Watch> {
    // The demo scene's school rows are seeded and never read again, so no
    // automatic, page or manual read reaches a school account.
    #[cfg(feature = "demo")]
    if profile_of(key.dataset).is_some() {
        return None;
    }
    let mut watch = Watch {
        priority: Priority::Background,
        rule: Rule::Ttl,
        tier: if key.dataset == Dataset::CatalogOfficial {
            Tier::Catalog
        } else {
            Tier::Automatic
        },
        order: usize::MAX,
        open: None,
        term: Term::Current,
        scope: match key.dataset {
            Dataset::CatalogOfficial => key.key.split('/').next().map(str::to_owned),
            _ => None,
        },
        title: None,
        locator: None,
        due: None,
    };
    let course = match key.dataset {
        Dataset::LmsCourse | Dataset::LmsAssign => {
            let id: u64 = key.key.parse().ok().filter(|id| *id > 0)?;
            inputs.course(id)
        }
        _ => None,
    };
    watch.term = freshness::term(key, course, read(&Key::lms_enrolment(), inputs), now)?;
    match key.dataset {
        Dataset::LmsCourse | Dataset::LmsAssign => {
            watch.title = course.map(|course| course.title.clone());
        }
        Dataset::LmsHistory => {}
        Dataset::KyomuPage => watch.title = Some(key.key.clone()),
        Dataset::CatalogOfficial => watch.title = key.key.split('/').nth(1).map(str::to_owned),
        Dataset::Reference => {
            let reference = inputs.references.iter().find(|watch| watch.key == *key)?;
            watch.locator = Some(reference.index_url.clone());
            watch.due = Some(reference.due);
            watch.title = Some(key.key.clone());
        }
        Dataset::LmsEnrolment | Dataset::LmsDeadlines | Dataset::Announcements => {}
    }
    Some(watch)
}

/// Keys Core keeps current without being asked, with their priority: an
/// account's once its credentials are saved, the reference files, and the
/// announcements while the app is in the foreground: they are only ever
/// shown there, so a background opportunity never reads them.
fn automatic_keys(inputs: &Inputs, foreground: bool, now: DateTime<Utc>) -> Vec<(Key, Priority)> {
    let mut keys = Vec::new();
    if foreground {
        keys.push((crate::announcements::key(), Priority::Visible));
    }
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

/// Why one rule wants a key.
struct Wanted {
    priority: Priority,
    rule: Rule,
    /// `None` keeps the key's own tier (automatic, or the catalog).
    tier: Option<Tier>,
    order: usize,
    open: Option<Duration>,
    scope: Option<String>,
}

impl Wanted {
    fn automatic(priority: Priority) -> Self {
        Self {
            priority,
            rule: Rule::Ttl,
            tier: None,
            order: usize::MAX,
            open: None,
            scope: None,
        }
    }
}

/// Adds `key` as `wanted`; several rules wanting one key keep the strongest
/// of each attribute.
fn want(
    watched: &mut BTreeMap<Key, Watch>,
    key: Key,
    wanted: Wanted,
    inputs: &Inputs,
    now: DateTime<Utc>,
) {
    let merge = |watch: &mut Watch, wanted: Wanted| {
        watch.priority = watch.priority.max(wanted.priority);
        watch.rule = watch.rule.max(wanted.rule);
        // The catalog stays last however it is wanted, but for a press.
        if let Some(tier) = wanted.tier {
            if watch.tier != Tier::Catalog || tier == Tier::Manual {
                watch.tier = watch.tier.min(tier);
            }
        }
        watch.order = watch.order.min(wanted.order);
        watch.open = match (watch.open, wanted.open) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if wanted.scope.is_some() {
            watch.scope = wanted.scope;
        }
    };
    if let Some(existing) = watched.get_mut(&key) {
        merge(existing, wanted);
        return;
    }
    if let Some(mut watch) = watch_for(&key, inputs, now) {
        watch.priority = Priority::Background;
        watch.rule = Rule::Ttl;
        merge(&mut watch, wanted);
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
    let ttl = freshness::ttl(key, watch.term, freshness::busy(state, now));
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
            // Something it depends on changed (`d/azfvr2`).
            let marked = state
                .and_then(|state| state.due_marks.first())
                .is_some_and(|at| *at <= now);
            let renewed = match (open_window(watch, ttl), fetched) {
                (Some(threshold), Some(fetched)) => now >= fetched + threshold,
                _ => false,
            };
            (marked || renewed).then_some((1, None))
        }
    }
}

/// How old a key on a visible page may get: the page's threshold, never
/// beyond its TTL; `None` for a key no page renews, or one never renewed.
fn open_window(watch: &Watch, ttl: Option<Duration>) -> Option<Duration> {
    let ttl = ttl?;
    watch.open.map(|open| open.min(ttl))
}

pub(super) fn can_run_in_background(dataset: Dataset) -> bool {
    dataset != Dataset::Announcements
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
    if facts.suspended || (!facts.foreground && !can_run_in_background(dataset)) {
        return Some(Held::Background);
    }
    if priority < Priority::Interactive {
        // The campus map and the announcements use cellular too, as their
        // owner asked.
        if facts.network == Network::Cellular
            && !inputs.allow_cellular
            && !crate::reference_refresh::raised(key)
            && dataset != Dataset::Announcements
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
    foreground: bool,
    opens: &[View],
    manual: &HashMap<Key, Option<String>>,
    now: DateTime<Utc>,
) -> BTreeMap<Key, Watch> {
    let mut watched = BTreeMap::new();
    for (key, priority) in automatic_keys(inputs, foreground, now) {
        want(&mut watched, key, Wanted::automatic(priority), inputs, now);
    }
    for view in opens {
        let (keys, priority, rule) = view_keys(view, inputs, now);
        for (order, (key, scope)) in keys.into_iter().enumerate() {
            let priority = if first_read(view, &key, inputs) {
                Priority::Interactive
            } else {
                priority
            };
            let open = (rule == Rule::Open).then(|| {
                let busy = freshness::busy(inputs.states.get(&key), now);
                freshness::open_threshold(view, &key, busy)
            });
            let wanted = Wanted {
                priority,
                rule,
                tier: Some(Tier::Shown),
                order,
                open,
                scope,
            };
            want(&mut watched, key, wanted, inputs, now);
        }
    }
    for (key, scope) in manual {
        let wanted = Wanted {
            priority: Priority::Interactive,
            rule: Rule::Manual,
            tier: Some(Tier::Manual),
            order: 0,
            open: None,
            scope: scope.clone(),
        };
        want(&mut watched, key.clone(), wanted, inputs, now);
    }
    // A key waiting for its prerequisite pulls that forward to its own
    // priority and place: an open course page reads the enrolment list
    // first, on any network the page itself may use.
    let mut waiting: Vec<(Key, Priority, Tier, usize)> = watched
        .iter()
        .filter_map(|(key, watch)| {
            Some((
                missing_prerequisite(key, inputs)?,
                watch.priority,
                watch.tier,
                watch.order,
            ))
        })
        .collect();
    while let Some((key, priority, tier, order)) = waiting.pop() {
        if let Some(prerequisite) = missing_prerequisite(&key, inputs) {
            waiting.push((prerequisite, priority, tier, order));
        }
        let wanted = Wanted {
            priority,
            rule: Rule::Ttl,
            tier: Some(tier),
            order,
            open: None,
            scope: None,
        };
        want(&mut watched, key, wanted, inputs, now);
    }
    watched
}

/// Every due key that is not leased, in lease order: by tier (a press,
/// visible pages, automatic, the catalog), then Interactive first, the
/// page's own order (what it reads first is what the next read relies on,
/// `d/cy7j2w`), missing before stale, then dataset and key order.
/// A key another due key's read reads beside it is left to that read
/// ([`read_once`]).
pub(crate) fn plan(
    inputs: &Inputs,
    facts: &Facts,
    credentials: &Credentials,
    opens: &[View],
    manual: &HashMap<Key, Option<String>>,
    leased: &HashSet<Key>,
    now: DateTime<Utc>,
) -> Vec<Candidate> {
    let watched = watched(inputs, facts.foreground, opens, manual, now);
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
            let held =
                hold(&key, watch.priority, depth, inputs, facts, credentials, now).or(backoff);
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
                tier: watch.tier,
                order: watch.order,
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
    read_once(&mut candidates);
    candidates.sort_by(|a, b| {
        (a.tier, Reverse(a.priority), a.order, a.rank, &a.key).cmp(&(
            b.tier,
            Reverse(b.priority),
            b.order,
            b.rank,
            &b.key,
        ))
    });
    candidates
}

/// Leaves a due key to a due key whose read reads it beside its own
/// ([`read_beside`]), so it is fetched once; that read takes the earlier
/// place of the two. Only an unheld host wanted at least as urgently (tier,
/// then priority) takes it: a visible page waits for one request, not for
/// the host's page too (`d/3mjnxv`). Whether the key was read is its own
/// committed state; the runner answers what was wanted of it when the
/// host's read committed it.
fn read_once(candidates: &mut Vec<Candidate>) {
    let urgency = |candidate: &Candidate| (candidate.tier, Reverse(candidate.priority));
    let mut index = 0;
    while index < candidates.len() {
        let inner = &candidates[index];
        let Some(host) = candidates.iter().position(|host| {
            host.held.is_none()
                && read_beside(&host.key).as_ref() == Some(&inner.key)
                && urgency(host) <= urgency(inner)
        }) else {
            index += 1;
            continue;
        };
        if urgency(&candidates[host]) == urgency(inner) {
            let (order, rank) = (inner.order, inner.rank);
            let host = &mut candidates[host];
            (host.order, host.rank) = (host.order.min(order), host.rank.min(rank));
        }
        candidates.remove(index);
    }
}

/// When the plan next changes by time alone: the earliest moment a wanted
/// key that is fresh now goes stale (its TTL, or an open page's threshold),
/// a backoff ends, or a hold with an end lifts (a paused host, an
/// account's pause). Keys due now are in the plan already; a hold that
/// waits for a fact (the network, the person) has no time.
pub(crate) fn next_due(
    inputs: &Inputs,
    foreground: bool,
    plan: &[Candidate],
    opens: &[View],
    manual: &HashMap<Key, Option<String>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let watched = watched(inputs, foreground, opens, manual, now);
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
        let marked = state
            .and_then(|state| state.due_marks.iter().find(|at| **at > now))
            .copied();
        let ttl = freshness::ttl(key, watch.term, freshness::busy(state, now));
        let window = open_window(watch, ttl).or(ttl);
        let renewed = window.map(|window| fetched + window).filter(|at| *at > now);
        // A busy key slowing down again makes nothing due: no wake for it.
        [marked, renewed].into_iter().flatten().min()
    });
    let held = plan
        .iter()
        .filter(|candidate| foreground || can_run_in_background(candidate.key.dataset))
        .filter_map(|candidate| match &candidate.held {
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
