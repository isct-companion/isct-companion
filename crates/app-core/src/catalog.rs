//! Catalog acquisition: the official 教務 department reads as sync keys, and
//! the stored offerings the finder searches. A search never reads the
//! network.
use crate::course_query::OfficialFilterOption;
use crate::net::{
    kyomu::{self, CatalogRows},
    BoxFuture, KeyOutcome, NetError,
};
use crate::plan::{net_end, session_end, KeyEnd, Lease};
use crate::school_session::{Attempt, Operation, RunError, SchoolSessions, Site};
use crate::sync_state::{self, Commit, Outcome, RunOutcome, RunRecord};
use crate::{data, school_auth::SchoolAuthProfile};

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error(transparent)]
    Data(#[from] data::DataError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    InvalidInput(String),
}

#[cfg(any(test, not(feature = "demo")))]
pub(crate) use adpt_isct::urls::syllabus_url;
pub(crate) use adpt_isct::urls::KYOMU_SEARCH_PATH;
#[cfg(not(feature = "demo"))]
use adpt_isct::urls::SYLLABUS_ORIGIN;

/// The offering a シラバス link names, by its `jwc` query value or its last
/// path segment. Offering IDs contain nine digits.
#[cfg(not(feature = "demo"))]
pub(crate) fn syllabus_offering(url: &str) -> Option<String> {
    let url = url::Url::parse(url)
        .ok()
        .filter(|url| url.origin().ascii_serialization() == SYLLABUS_ORIGIN)?;
    url.query_pairs()
        .find(|(key, _)| key == "jwc")
        .map(|(_, value)| value.into_owned())
        .or_else(|| url.path_segments()?.next_back().map(str::to_string))
        .filter(|id| id.len() == 9 && id.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Stored offerings for one year.
pub(crate) fn offerings(
    store: &data::DataStore,
    year: u16,
) -> Result<Vec<serde_json::Value>, CatalogError> {
    Ok(store
        .catalog_year(year)?
        .into_iter()
        .filter(|row| {
            row.get("year").and_then(serde_json::Value::as_u64) == Some(u64::from(year))
                && row.get("deletedAt").is_none_or(serde_json::Value::is_null)
        })
        .collect())
}

#[cfg(test)]
#[path = "tests/catalog.rs"]
mod tests;

/// Weekly slots of a course row as (day, first period).
fn weekly_slots(row: &serde_json::Value) -> Vec<(u64, u64)> {
    row["schedule"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|slot| slot["kind"] == "weekly")
        .filter_map(|slot| Some((slot["day"].as_u64()?, slot["periods"][0].as_u64()?)))
        .collect()
}

fn has_quarter(row: &serde_json::Value, quarter: u64) -> bool {
    row["quarters"]
        .as_array()
        .is_some_and(|quarters| quarters.iter().any(|q| *q == format!("{quarter}Q")))
}

/// The leading number of a label such as `100番台` or `3限`.
fn leading_number(label: &str) -> Option<u64> {
    let digits: String = label.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Whether a row meets one choice of the official form, read from its label
/// the way the form means it. A choice whose meaning the row cannot show
/// (an unknown family, a category the grid does not list) keeps the row.
pub(crate) fn official_control_match(
    row: &serde_json::Value,
    option: &OfficialFilterOption,
) -> bool {
    const DAYS: &str = "月火水木金土日";
    let label = option.label.as_str();
    let kyomu = &row["kyomu"];
    let schedule_kind = |kind: &str| {
        row["schedule"]
            .as_array()
            .is_some_and(|slots| slots.iter().any(|slot| slot["kind"] == kind))
    };
    match option.family.as_str() {
        "level" => leading_number(label).is_none_or(|level| {
            row["code"]
                .as_str()
                .and_then(|code| code.chars().nth(5))
                .and_then(|digit| digit.to_digit(10))
                .is_some_and(|digit| u64::from(digit) * 100 == level)
        }),
        "quarter" => {
            let digits: Vec<u64> = label
                .chars()
                .filter_map(|c| c.to_digit(10))
                .filter(|digit| (1..=4).contains(digit))
                .map(u64::from)
                .collect();
            let quarters: Vec<u64> = match digits.as_slice() {
                [from, to] if label.contains(['~', '〜', '～', '-']) => (*from..=*to).collect(),
                _ => digits,
            };
            quarters
                .into_iter()
                .any(|quarter| has_quarter(row, quarter))
        }
        "delivery" => match DAYS.chars().position(|day| label == day.to_string()) {
            Some(index) => weekly_slots(row)
                .iter()
                .any(|(day, _)| *day == index as u64 + 1),
            None if label.contains("オンデマンド") => schedule_kind("on-demand"),
            None if label.contains("集中") => schedule_kind("intensive"),
            None if label.contains("申告専用") => schedule_kind("registration-only"),
            None => true,
        },
        "start" => leading_number(label)
            .is_none_or(|start| weekly_slots(row).iter().any(|(_, first)| *first == start)),
        "language" => {
            let lower = label.to_lowercase();
            if label.contains("日本語") || lower.contains("japanese") {
                row["instructionLanguage"] == "ja"
            } else if label.contains("英語") || lower.contains("english") {
                row["instructionLanguage"] == "en"
            } else {
                true
            }
        }
        "requirement" => kyomu["group"] == label,
        "recommended" => kyomu["mark"].as_str().is_some_and(|mark| !mark.is_empty()),
        "category" => kyomu["category"]
            .as_str()
            .filter(|category| !category.is_empty())
            .is_none_or(|category| category.contains(label) || label.contains(category)),
        _ => true,
    }
}

/// `catalog.official/{year}/{prefix}`: the whole department from the
/// official course search, merged into `offerings`, its freshness and its
/// run row in one transaction. `{year}/{prefix}/en` reads the English page
/// and adds only English titles and instructor names to the stored rows. A
/// year the school does not offer yet is recorded as unavailable; stored
/// rows stay.
/// One search of the official catalog (a GET and its ViewState postback,
/// never split), as an operation of the 教務Web session (the English one
/// for English titles).
struct Search<'a> {
    year: u16,
    query: &'a kyomu::CatalogSearch,
}

impl Operation for Search<'_> {
    type Output = KeyOutcome<CatalogRows>;
    type Error = NetError;

    fn attempt(
        &mut self,
        attempt: Attempt,
    ) -> BoxFuture<'_, Result<KeyOutcome<CatalogRows>, NetError>> {
        Box::pin(async move { kyomu::search(&attempt, self.year, self.query).await })
    }
}

/// One department: the search for its code prefix, and when that is over
/// the school's cap, the same search one quarter at a time, merged. Between
/// searches the read gives the lane to one that needs it more
/// (`d/3mjnxv`); nothing is stored until every search answered.
async fn read_department(
    sessions: &SchoolSessions,
    site: Site,
    permit: &crate::school_session::Permit,
    lease: &Lease,
    year: u16,
    prefix: &str,
) -> Result<KeyOutcome<CatalogRows>, RunError<NetError>> {
    let run = |query: kyomu::CatalogSearch| async move {
        sessions
            .run(
                site,
                permit,
                lease.deadline(),
                &mut Search {
                    year,
                    query: &query,
                },
            )
            .await
    };
    let whole = kyomu::CatalogSearch {
        code: prefix.into(),
        ..kyomu::CatalogSearch::default()
    };
    let first = match run(whole.clone()).await? {
        KeyOutcome::Complete(rows) if rows.truncated => rows,
        other => return Ok(other),
    };
    let mut seen = std::collections::HashSet::new();
    let mut merged = CatalogRows {
        courses: Vec::new(),
        search_options: first.search_options,
        truncated: false,
    };
    for quarter in 1..=4 {
        lease.checkpoint().await;
        if lease.is_cancelled() {
            return Err(RunError::Cancelled);
        }
        let rows = match run(kyomu::CatalogSearch {
            quarter,
            ..whole.clone()
        })
        .await?
        {
            KeyOutcome::Complete(rows) => rows,
            other => return Ok(other),
        };
        merged.truncated |= rows.truncated;
        // A course of several quarters is listed once per quarter.
        for course in rows.courses {
            let id = course
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            if id.is_none_or(|id| seen.insert(id)) {
                merged.courses.push(course);
            }
        }
    }
    Ok(KeyOutcome::Complete(merged))
}

pub(crate) async fn read_official(sessions: &SchoolSessions, lease: &Lease) -> KeyEnd {
    let source = format!("{}{}", adpt_isct::kyomu::HOST, KYOMU_SEARCH_PATH);
    let (department, english) = match lease.key.key.strip_suffix("/en") {
        Some(department) => (department, true),
        None => (lease.key.key.as_str(), false),
    };
    let Some((year, prefix)) = department
        .split_once('/')
        .and_then(|(year, prefix)| Some((year.parse::<u16>().ok()?, prefix.to_string())))
    else {
        return KeyEnd::failed("invalid-key", source);
    };
    let site = if english {
        Site::KyomuEnglish
    } else {
        Site::Kyomu
    };
    let Some(permit) = sessions.permit(SchoolAuthProfile::TokyoTech) else {
        return crate::plan::denied_read(sessions, SchoolAuthProfile::TokyoTech, source);
    };
    let read = read_department(sessions, site, &permit, lease, year, &prefix).await;
    let (rows, outcome) = match read {
        Ok(KeyOutcome::Complete(rows)) => (Some(rows), Outcome::Complete),
        Ok(KeyOutcome::Unavailable(_)) => (None, Outcome::Unavailable),
        Err(error) => {
            return session_end(
                error,
                SchoolAuthProfile::TokyoTech,
                source.clone(),
                |error| {
                    log::warn!(target: "app_core::catalog", "official catalog {} failed: {error}", lease.key);
                    net_end(error, source.clone())
                },
            )
        }
    };
    if lease.is_cancelled() || permit.is_cancelled() {
        return KeyEnd::Cancelled { source };
    }
    // Rows past the school's cap were not read even by quarter: the key is
    // not complete and must not be marked fresh.
    if rows.as_ref().is_some_and(|rows| rows.truncated) {
        log::warn!(target: "app_core::catalog", "official catalog {} reached the search cap", lease.key);
        return KeyEnd::failed("truncated", source);
    }
    let writer = lease.writer();
    let (key, run, started, record_source) = (
        lease.key.clone(),
        lease.run.clone(),
        lease.started,
        source.clone(),
    );
    let applied = tokio::task::spawn_blocking(move || {
        let now = crate::clock::now();
        let committed = writer.catalog_write(year, |transaction| {
            let changed = match &rows {
                Some(rows) if english => {
                    data::DataStore::catalog_merge_english_in(transaction, &rows.courses)?
                }
                Some(rows) => {
                    data::DataStore::catalog_upsert_offerings_in(transaction, &rows.courses)?
                }
                None => 0,
            };
            sync_state::commit(
                transaction,
                &key,
                Commit {
                    validator: None,
                    outcome,
                    message: None,
                },
                now,
            )?;
            sync_state::record_run(
                transaction,
                &RunRecord {
                    run: &run,
                    key: &key,
                    started_at: started,
                    finished_at: now,
                    outcome: if outcome == Outcome::Unavailable {
                        RunOutcome::Unavailable
                    } else {
                        RunOutcome::Complete
                    },
                    changed,
                    requests: None,
                    bytes: None,
                    source: &record_source,
                    error_kind: None,
                },
            )?;
            Ok::<_, data::DataError>(changed)
        });
        // The form's filter choices are their own key, `{year}/options`.
        if let (Ok(_), Some(rows), false) = (&committed, &rows, english) {
            if let (Some(_gate), Ok(options)) =
                (writer.enter(), serde_json::to_value(&rows.search_options))
            {
                if let Err(error) = writer
                    .store()
                    .record_kyomu_search_options(year, &options, now)
                {
                    crate::diagnostics::report("catalog", &error);
                }
            }
        }
        committed
    })
    .await;
    match applied {
        Ok(Ok(changed)) => KeyEnd::Done { changed },
        Ok(Err(gated)) => KeyEnd::from_gated(gated, "storage", &source),
        Err(_) => KeyEnd::failed("stopped", source),
    }
}
