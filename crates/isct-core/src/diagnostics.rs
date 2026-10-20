//! Bounded, read-only application diagnostics; never part of workspace backups.
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};

const CAPACITY: usize = 2000;
const MESSAGE_LIMIT: usize = 2048;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: u64,
    pub at: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSnapshot {
    pub entries: Vec<LogEntry>,
    pub cursor: u64,
    pub dropped: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DiagnosticsError {
    #[error("Application logs are unavailable.")]
    Unavailable,
    #[error("Application logging is already configured.")]
    AlreadyConfigured,
}

#[derive(Default)]
struct Buffer {
    entries: VecDeque<LogEntry>,
    cursor: u64,
}
impl Buffer {
    fn push(&mut self, level: &str, target: &str, message: &str) {
        self.cursor = self.cursor.saturating_add(1);
        self.entries.push_back(LogEntry {
            id: self.cursor,
            at: chrono::Utc::now().to_rfc3339(),
            level: level.into(),
            target: target.chars().take(128).collect(),
            message: redact(message).chars().take(MESSAGE_LIMIT).collect(),
        });
        while self.entries.len() > CAPACITY {
            self.entries.pop_front();
        }
    }
    fn snapshot(&self, after: Option<u64>) -> LogSnapshot {
        LogSnapshot {
            entries: self
                .entries
                .iter()
                .filter(|entry| after.is_none_or(|id| entry.id > id))
                .cloned()
                .collect(),
            cursor: self.cursor,
            dropped: after.is_some_and(|id| {
                self.entries
                    .front()
                    .is_some_and(|first| id.saturating_add(1) < first.id)
            }),
        }
    }
    /// Plain text a person can attach to a bug report; continuation lines of
    /// multi-line messages are indented so each entry stays distinguishable.
    fn export(&self, about: &str, exported_at: &str) -> String {
        let mut text = format!(
            "{about}\nExported {exported_at} · {} entries\n\n",
            self.entries.len()
        );
        for entry in &self.entries {
            text.push_str(&format!(
                "{} {} [{}] {}\n",
                entry.at,
                entry.level,
                entry.target,
                entry.message.replace('\n', "\n    ")
            ));
        }
        text
    }
}

fn redact(message: &str) -> String {
    static PATTERN: OnceLock<Result<regex::Regex, regex::Error>> = OnceLock::new();
    match PATTERN.get_or_init(|| regex::Regex::new(r#"(?i)((?:password|passwd|secret|otp_secret|token|access_token|refresh_token|sesskey|code)["']?\s*[:=]\s*["']?)[^&\s"',}]+|(bearer\s+)[a-z0-9._~+/=-]+"#)) {
        Ok(pattern) => pattern.replace_all(message, "${1}${2}[redacted]").into_owned(),
        Err(_) => "[Log message unavailable]".into(),
    }
}

fn buffer() -> &'static Mutex<Buffer> {
    static BUFFER: OnceLock<Mutex<Buffer>> = OnceLock::new();
    BUFFER.get_or_init(|| Mutex::new(Buffer::default()))
}

pub fn snapshot(after: Option<u64>) -> Result<LogSnapshot, DiagnosticsError> {
    buffer()
        .lock()
        .map(|buffer| buffer.snapshot(after))
        .map_err(|_| DiagnosticsError::Unavailable)
}

/// The retained, already-redacted log as text; `about` names the app build.
pub fn export_text(about: &str) -> Result<String, DiagnosticsError> {
    buffer()
        .lock()
        .map(|buffer| buffer.export(about, &chrono::Utc::now().to_rfc3339()))
        .map_err(|_| DiagnosticsError::Unavailable)
}

fn write(level: &str, target: &str, message: &str) {
    let message: String = redact(message).chars().take(MESSAGE_LIMIT).collect();
    if let Ok(mut buffer) = buffer().lock() {
        buffer.push(level, target, &message);
    }
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "[{level}] [{target}] {message}");
}

pub fn report(scope: &str, error: &dyn std::fmt::Display) {
    write("ERROR", scope, &error.to_string());
}
pub fn info(scope: &str, message: &str) {
    write("INFO", scope, message);
}

struct AppLogger;
impl log::Log for AppLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info && metadata.target().starts_with("isct")
    }
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            write(
                record.level().as_str(),
                record.target(),
                &record.args().to_string(),
            );
        }
    }
    fn flush(&self) {}
}

/// Shell startup installs the standard Rust logger; Core reports also work in
/// hosts which already own their logger. No platform handles enter this store.
pub fn initialize() -> Result<(), DiagnosticsError> {
    static LOGGER: AppLogger = AppLogger;
    log::set_logger(&LOGGER).map_err(|_| DiagnosticsError::AlreadyConfigured)?;
    log::set_max_level(log::LevelFilter::Info);
    info("app", "Application started");
    Ok(())
}

#[cfg(test)]
#[path = "tests/diagnostics.rs"]
mod tests;
