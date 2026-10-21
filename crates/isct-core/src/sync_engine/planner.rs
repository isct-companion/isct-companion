//! What is due, in which order, and why due work waits. Pure over the
//! stored facts ([`Inputs`]) and the shell's facts ([`Facts`]), so the rules
//! can be read (and tested) without a runner.
use super::{Held, Network, Priority, Scope, View};
use crate::{
    lifecycle::SchoolSite,
    lms_ingest::{current_lms_term, lms_course_term, StoredLmsCourse},
    net::Host,
    school_auth::SchoolAuthProfile,
    sync_state::{self, Dataset, Due, Key, KeyState, Term},
};
use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, Utc};
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
}

/// Stored facts, read once per evaluation on a blocking worker.
#[derive(Clone, Debug, Default)]
pub(crate) struct Inputs {
    pub(crate) states: HashMap<Key, KeyState>,
    pub(crate) courses: Vec<StoredLmsCourse>,
    /// `SchoolAuthService::sync_account`; `None` when it could not be read.
    pub(crate) science: Option<String>,
    pub(crate) tokyo: Option<String>,
    /// Saved credentials exist, so a read may sign in on its own.
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
    fn account(&self, dataset: Dataset) -> Option<String> {
        match profile_of(dataset) {
            // 教務 course search rows are public; the session only reads them.
            Some(_) if dataset == Dataset::CatalogOfficial => Some(String::new()),
            Some(SchoolAuthProfile::Science) => self.science.clone(),
            Some(SchoolAuthProfile::TokyoTech) => self.tokyo.clone(),
            None => Some(String::new()),
        }
    }

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

/// What the shell has told Core.
#[derive(Clone, Debug)]
pub(crate) struct Facts {
    pub(crate) network: Network,
    pub(crate) foreground: bool,
    pub(crate) suspended: bool,
    pub(crate) power_saver: bool,
    pub(crate) resetting: bool,
    /// A school browser (sign-in and HTTP transport) is attached.
    pub(crate) school_attached: bool,
    /// Profiles with a session seen active since launch. Automatic school
    /// keys wait for one; a lost session removes it again.
    pub(crate) signed_in: HashSet<SchoolAuthProfile>,
    /// Hosts the circuit breaker paused, until when.
    pub(crate) paused: HashMap<Host, DateTime<Utc>>,
}

impl Default for Facts {
    fn default() -> Self {
        Self {
            network: Network::Unknown,
            foreground: true,
            suspended: false,
            power_saver: false,
            resetting: false,
            school_attached: false,
            signed_in: HashSet::new(),
            paused: HashMap::new(),
        }
    }
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
    pub(crate) account: String,
    pub(crate) held: Option<Held>,
    pub(crate) group: Group,
    /// The course title, page or department the key reads, for progress.
    pub(crate) title: Option<String>,
    /// The Isct-Data index of a reference key.
    pub(crate) locator: Option<String>,
}

/// One key's attributes before the rules decide whether it is due.
#[derive(Clone, Debug)]
struct Watch {
    priority: Priority,
    rule: Rule,
    term: Term,
    account: String,
    scope: Option<String>,
    title: Option<String>,
    locator: Option<String>,
    due: Option<Due>,
}

/// The six 教務Web pages, in menu order.
pub(crate) fn kyomu_keys() -> Vec<Key> {
    crate::school_parser::kyomu::PAGES
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

/// A year's whole catalog: its official departments.
fn catalog_keys(year: u16, english: bool) -> Vec<(Key, Option<String>)> {
    official_keys(year, english)
        .into_iter()
        .map(|key| (key, Some(year.to_string())))
        .collect()
}

fn lms_all() -> [Key; 2] {
    [
        Key::new(Dataset::LmsEnrolment, "all"),
        Key::new(Dataset::LmsDeadlines, "all"),
    ]
}

fn course_keys(id: u64) -> [Key; 2] {
    [
        Key::new(Dataset::LmsCourse, id.to_string()),
        Key::new(Dataset::LmsAssign, id.to_string()),
    ]
}

fn japan_today(now: DateTime<Utc>) -> NaiveDate {
    FixedOffset::east_opt(9 * 3600)
        .map(|offset| now.with_timezone(&offset).date_naive())
        .unwrap_or_else(|| now.date_naive())
}

fn japan_month(now: DateTime<Utc>) -> (i32, u32) {
    let today = japan_today(now);
    (today.year(), today.month())
}

/// The course and assignment keys of every current-term course.
pub(crate) fn current_course_keys(inputs: &Inputs, now: DateTime<Utc>) -> Vec<Key> {
    let current = current_lms_term(None, now);
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
            let mut keys = vec![Key::new(Dataset::LmsDeadlines, "all")];
            keys.extend(
                current_course_keys(inputs, now)
                    .into_iter()
                    .filter(|key| key.dataset == Dataset::LmsAssign),
            );
            (unscoped(keys), Priority::Visible, Rule::Open)
        }
        View::Timetable => (
            unscoped(vec![
                Key::new(Dataset::LmsEnrolment, "all"),
                Key::new(Dataset::KyomuPage, "registrations"),
            ]),
            Priority::Visible,
            Rule::Open,
        ),
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

/// The base attributes of `key`, or `None` when nothing can read it (an
/// account that could not be read, a reference kind that is off).
fn watch_for(key: &Key, inputs: &Inputs, now: DateTime<Utc>) -> Option<Watch> {
    let account = inputs.account(key.dataset)?;
    let mut watch = Watch {
        priority: Priority::Background,
        rule: Rule::Ttl,
        term: Term::Current,
        account,
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
            if let Some(course) = inputs.course(id) {
                watch.term = lms_course_term(course, current_lms_term(None, now));
                watch.title = Some(course.title.clone());
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

/// Keys Core keeps current without being asked, with their priority.
fn automatic_keys(inputs: &Inputs, facts: &Facts, now: DateTime<Utc>) -> Vec<(Key, Priority)> {
    let mut keys = Vec::new();
    if facts.signed_in.contains(&SchoolAuthProfile::Science) {
        keys.extend(lms_all().map(|key| (key, Priority::Visible)));
        let current = current_lms_term(None, now);
        for course in &inputs.courses {
            let priority = if lms_course_term(course, current) == Term::Current {
                Priority::Visible
            } else {
                Priority::Background
            };
            keys.extend(course_keys(course.id).map(|key| (key, priority)));
        }
        let today = japan_today(now);
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
    if facts.signed_in.contains(&SchoolAuthProfile::TokyoTech) {
        keys.extend(kyomu_keys().into_iter().map(|key| (key, Priority::Visible)));
        if let Some(year) = inputs.year {
            keys.extend(
                official_keys(year, inputs.english)
                    .into_iter()
                    .map(|key| (key, Priority::Background)),
            );
        }
    }
    keys.extend(
        inputs
            .references
            .iter()
            .map(|watch| (watch.key.clone(), Priority::Background)),
    );
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
    let fetched = state
        .filter(|state| state.account == watch.account)
        .and_then(|state| state.fetched_at);
    if watch.rule == Rule::Manual {
        return Some((u8::from(fetched.is_some()), None));
    }
    let due = watch
        .due
        .unwrap_or_else(|| sync_state::due(state, ttl, &watch.account, now));
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

/// 教務 reads only with the cookie session of a foreground app; LMS reads
/// that need the dashboard cookie wait too unless the person asked.
fn needs_foreground(dataset: Dataset, priority: Priority) -> bool {
    match dataset {
        Dataset::KyomuPage | Dataset::CatalogOfficial => true,
        Dataset::LmsEnrolment | Dataset::LmsCourse | Dataset::LmsHistory => {
            priority < Priority::Interactive
        }
        _ => false,
    }
}

/// Why a due key may not be leased now, most fundamental reason first.
pub(crate) fn hold(
    dataset: Dataset,
    priority: Priority,
    inputs: &Inputs,
    facts: &Facts,
    now: DateTime<Utc>,
) -> Option<Held> {
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
    if let Some(profile) = profile_of(dataset) {
        if !facts.school_attached
            || !(facts.signed_in.contains(&profile) || inputs.configured(profile))
        {
            return Some(Held::SignInRequired { profile });
        }
    }
    if facts.suspended || (!facts.foreground && needs_foreground(dataset, priority)) {
        return Some(Held::Background);
    }
    if priority < Priority::Interactive {
        if facts.network == Network::Cellular && !inputs.allow_cellular {
            return Some(Held::Cellular);
        }
        if facts.power_saver {
            return Some(Held::PowerSaver);
        }
    }
    None
}

/// Every due key that is not leased, in lease order: Interactive first,
/// missing before stale, then dataset and key order.
pub(crate) fn plan(
    inputs: &Inputs,
    facts: &Facts,
    opens: &[View],
    manual: &HashMap<Key, Option<String>>,
    leased: &HashSet<Key>,
    now: DateTime<Utc>,
) -> Vec<Candidate> {
    let mut watched = BTreeMap::new();
    for (key, priority) in automatic_keys(inputs, facts, now) {
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
    let mut candidates: Vec<Candidate> = watched
        .into_iter()
        .filter(|(key, _)| !leased.contains(key))
        // A course's assignment states land on the rows its outline creates;
        // read them once that outline is stored, not before.
        .filter(|(key, _)| {
            key.dataset != Dataset::LmsAssign
                || inputs
                    .states
                    .get(&Key::new(Dataset::LmsCourse, key.key.clone()))
                    .is_some_and(|state| state.fetched_at.is_some())
        })
        // English names merge into rows the Japanese page stored.
        .filter(|(key, _)| {
            japanese_key(key).is_none_or(|japanese| {
                inputs
                    .states
                    .get(&japanese)
                    .is_some_and(|state| state.fetched_at.is_some())
            })
        })
        .filter_map(|(key, watch)| {
            let (rank, backoff) = evaluate(&key, &watch, inputs.states.get(&key), now)?;
            let held = hold(key.dataset, watch.priority, inputs, facts, now).or(backoff);
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
                account: watch.account,
                held,
                group,
                title: watch.title,
                locator: watch.locator,
                key,
            })
        })
        .collect();
    candidates.sort_by(|a, b| {
        (Reverse(a.priority), a.rank, &a.key).cmp(&(Reverse(b.priority), b.rank, &b.key))
    });
    candidates
}
