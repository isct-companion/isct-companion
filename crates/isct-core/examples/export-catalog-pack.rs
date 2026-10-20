//! Builds the Isct-Data catalog pack of one academic year from the public
//! シラバス: every unit the year's index page links to, every offering of
//! those units read once in both languages, written as one gzip JSON file,
//! with the `index.json` entry printed on stdout.
//!
//! ```text
//! cargo run -p isct-core --example export-catalog-pack -- \
//!   --year 2026 --work /tmp/isct-catalog \
//!   --out ../Isct-Data/catalog/2026.json.gz
//! ```
//!
//! Resumable: the unit list, each unit's listing and each offering are saved
//! under `--work/{year}` as they complete, and a rerun skips what is saved.
//! Delete that folder to start over. Requests go through the シラバス
//! client's host budget, one offering at a time with `--spacing-ms` (250 by
//! default) between offerings.
//!
//! A detail page the parser refuses (an unsupported language label, a
//! changed layout) is left out and listed under the unit's `skipped`.
//!
//! Exit codes: 0 written; 2 some unit could not be read (rerun to resume;
//! `--allow-incomplete` writes the pack anyway); 3 written, but some unit
//! listing stayed capped at 100 rows even split by course level.
use isct_core::catalog_pack::{CatalogPack, PackScope, PackSource, SCHEMA_VERSION};
use isct_core::syllabus::{self, Offering, SyllabusError};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

struct Options {
    year: u16,
    work: PathBuf,
    out: PathBuf,
    /// The pack's path relative to `index.json`.
    source: String,
    spacing: Duration,
    extra_scopes: Vec<String>,
    allow_incomplete: bool,
}

const USAGE: &str = "usage: export-catalog-pack --year YYYY --work DIR [--out FILE] \
[--source catalog/YYYY.json.gz] [--spacing-ms 250] [--scope N/A-B-C-D-E]... [--allow-incomplete]";

fn options() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let (mut year, mut work, mut out, mut source) = (None, None, None, None);
    let mut spacing = Duration::from_millis(250);
    let mut extra_scopes = Vec::new();
    let mut allow_incomplete = false;
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value\n{USAGE}"));
        match arg.as_str() {
            "--year" => year = Some(value()?.parse::<u16>().map_err(|_| "invalid --year")?),
            "--work" => work = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--source" => source = Some(value()?),
            "--spacing-ms" => {
                spacing = Duration::from_millis(
                    value()?.parse().map_err(|_| "invalid --spacing-ms")?,
                )
            }
            "--scope" => extra_scopes.push(value()?),
            "--allow-incomplete" => allow_incomplete = true,
            _ => return Err(format!("unknown argument {arg}\n{USAGE}")),
        }
    }
    let year = year.ok_or(USAGE)?;
    let work = work.ok_or(USAGE)?;
    Ok(Options {
        out: out.unwrap_or_else(|| work.join(format!("catalog/{year}.json.gz"))),
        source: source.unwrap_or_else(|| format!("catalog/{year}.json.gz")),
        year,
        work,
        spacing,
        extra_scopes,
        allow_incomplete,
    })
}

/// One unit whose listing and offerings are all saved.
#[derive(Serialize, Deserialize)]
struct ScopeRecord {
    scope: String,
    listed: usize,
    truncated: bool,
    ids: Vec<String>,
    /// Detail pages the parser refused: retrying will not help.
    skipped: Vec<String>,
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, Box<dyn Error>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Written beside the target and renamed, so an interrupted run never
/// leaves a half file that a rerun would trust.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, path)?;
    Ok(())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), Box<dyn Error>> {
    write_file(path, &serde_json::to_vec_pretty(value)?)
}

/// Three attempts, 5 s and 20 s apart; a request the client refuses and a
/// page the parser refuses are not retried.
async fn with_retry<T, F, Fut>(mut attempt: F) -> Result<T, SyllabusError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, SyllabusError>>,
{
    let mut wait = Duration::from_secs(5);
    let mut tries = 1;
    loop {
        match attempt().await {
            Err(error @ (SyllabusError::InvalidInput(_) | SyllabusError::SiteChanged(_))) => {
                return Err(error)
            }
            Err(error) if tries < 3 => {
                eprintln!("  retrying in {}s: {error}", wait.as_secs());
                tokio::time::sleep(wait).await;
                wait *= 4;
                tries += 1;
            }
            result => return result,
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<ExitCode, Box<dyn Error>> {
    let options = options()?;
    let year = options.year;
    let root = options.work.join(year.to_string());
    let offerings_dir = root.join("offerings");
    let scopes_dir = root.join("scopes");

    // The unit list is saved first, so a rerun reads the same units.
    let scopes_file = root.join("scopes.json");
    let mut scopes: Vec<String> = match read_json(&scopes_file)? {
        Some(scopes) => scopes,
        None => {
            let scopes = with_retry(|| syllabus::year_scopes(year)).await?;
            write_json(&scopes_file, &scopes)?;
            scopes
        }
    };
    for scope in &options.extra_scopes {
        if !scopes.contains(scope) {
            scopes.push(scope.clone());
        }
    }

    let mut records = Vec::new();
    let mut incomplete = Vec::new();
    for (index, scope) in scopes.iter().enumerate() {
        let record_file = scopes_dir.join(format!("{}.json", scope.replace('/', "_")));
        if let Some(record) = read_json::<ScopeRecord>(&record_file)? {
            records.push(record);
            continue;
        }
        eprintln!("[{}/{}] {scope}", index + 1, scopes.len());
        let listing = match with_retry(|| syllabus::department_catalog(year, scope, None)).await {
            Ok(listing) => listing,
            Err(error) => {
                eprintln!("  listing failed: {error}");
                incomplete.push(scope.clone());
                continue;
            }
        };
        let mut complete = true;
        let mut ids = Vec::new();
        let mut skipped = Vec::new();
        for item in &listing.items {
            let file = offerings_dir.join(format!("{}.json", item.id));
            if file.exists() {
                ids.push(item.id.clone());
                continue;
            }
            tokio::time::sleep(options.spacing).await;
            match with_retry(|| syllabus::sync_offering(&item.syllabus_url)).await {
                Ok(synced) => {
                    let mut offering = synced.offering;
                    if offering.id != item.id || offering.year != year {
                        eprintln!("  {}: the detail page is {} of {}", item.id, offering.id, offering.year);
                        complete = false;
                        continue;
                    }
                    // Parser notes are diagnostics, not catalog data.
                    offering.warnings.clear();
                    write_json(&file, &offering)?;
                    ids.push(item.id.clone());
                }
                Err(SyllabusError::SiteChanged(reason)) => {
                    eprintln!("  {}: skipped, {reason}", item.id);
                    skipped.push(item.id.clone());
                }
                Err(error) => {
                    eprintln!("  {}: {error}", item.id);
                    complete = false;
                }
            }
        }
        if !complete {
            incomplete.push(scope.clone());
            continue;
        }
        if listing.truncated {
            eprintln!("  warning: still capped at 100 rows after the level split");
        }
        let record = ScopeRecord {
            scope: scope.clone(),
            listed: listing.items.len(),
            truncated: listing.truncated,
            ids,
            skipped,
        };
        write_json(&record_file, &record)?;
        records.push(record);
    }
    if !incomplete.is_empty() {
        eprintln!(
            "{} of {} units are incomplete: {}",
            incomplete.len(),
            scopes.len(),
            incomplete.join(", ")
        );
        if !options.allow_incomplete {
            eprintln!("rerun the same command to resume them");
            return Ok(ExitCode::from(2));
        }
    }

    // Units overlap (a school and its departments); each offering once.
    let mut offerings: BTreeMap<String, Offering> = BTreeMap::new();
    for id in records.iter().flat_map(|record| &record.ids) {
        if offerings.contains_key(id) {
            continue;
        }
        let offering: Offering = read_json(&offerings_dir.join(format!("{id}.json")))?
            .ok_or_else(|| format!("offering {id} is listed but not saved"))?;
        offerings.insert(id.clone(), offering);
    }
    let truncated: Vec<&str> = records
        .iter()
        .filter(|record| record.truncated)
        .map(|record| record.scope.as_str())
        .collect();
    let pack = CatalogPack {
        schema_version: SCHEMA_VERSION,
        academic_year: year,
        generated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        source: PackSource {
            title: format!("{year}年度 Science Tokyo シラバス"),
            url: format!("https://syllabus.s.isct.ac.jp/courses/{year}"),
        },
        scopes: records
            .iter()
            .map(|record| PackScope {
                scope: record.scope.clone(),
                listed: record.listed,
                truncated: record.truncated,
                skipped: record.skipped.clone(),
            })
            .collect(),
        offerings: offerings.into_values().collect(),
    };
    let (bytes, hash) = pack.encode()?;
    write_file(&options.out, &bytes)?;
    let entry = serde_json::json!({
        "catalogs": {
            year.to_string(): {
                "source": options.source,
                "hash": hash,
                "ts": pack.generated_at,
                "count": pack.offerings.len(),
            }
        }
    });
    println!("{}", serde_json::to_string_pretty(&entry)?);
    eprintln!(
        "wrote {} ({} bytes): {} offerings from {} units",
        options.out.display(),
        bytes.len(),
        pack.offerings.len(),
        pack.scopes.len()
    );
    if !truncated.is_empty() {
        eprintln!("capped units (rows may be missing): {}", truncated.join(", "));
        return Ok(ExitCode::from(3));
    }
    Ok(ExitCode::SUCCESS)
}
