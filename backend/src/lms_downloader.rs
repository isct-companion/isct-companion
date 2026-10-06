//! Course files are downloaded by WebKit itself, from a hidden LMS page that
//! shares the school session: the same network stack, cookies and redirects
//! the LMS page uses to load its own images and attachments. A same-origin
//! `<a download>` turns any response (inline PNG included) into a download,
//! which `on_download` writes straight to the path we choose.
use std::{path::PathBuf, sync::Mutex, time::Duration};
use tauri::{
    webview::{DownloadEvent, PageLoadEvent, WebviewBuilder},
    AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl,
};
use tokio::sync::oneshot;

const LABEL: &str = "lms-files";
/// Any same-origin document will do: each download probes the session itself.
/// This static page loads in ~50 ms; the dashboard takes ~9 s before scripts.
const HOME: &str = "https://lms.s.isct.ac.jp/2025/robots.txt";
/// The page reports a probe result by navigating here; the navigation is cancelled.
const RELAY: &str = "https://isctcompanion.invalid/file";

pub enum Fetched {
    Saved,
    TooLarge(u64),
}

struct Job {
    url: String,
    target: PathBuf,
    done: oneshot::Sender<Result<Fetched, String>>,
}

#[derive(Default)]
pub struct Downloader {
    job: Mutex<Option<Job>>,
    page: Mutex<Option<Url>>,
    /// One download at a time: the page holds a single job.
    turn: tokio::sync::Mutex<()>,
}

fn on_lms(page: &Option<Url>) -> bool {
    page.as_ref().is_some_and(|url| {
        url.host_str() == Some("lms.s.isct.ac.jp")
            && url.path().starts_with("/2025/")
            && !url.path().starts_with("/2025/login/")
    })
}

impl Downloader {
    /// Download `url` to `target`, or report its size when it exceeds `limit`.
    /// WebKit writes `<target>.part`, renamed once complete, so a half-written
    /// file is never previewed.
    pub async fn fetch(
        &self,
        app: &AppHandle,
        url: &Url,
        target: PathBuf,
        limit: Option<u64>,
    ) -> Result<Fetched, String> {
        let _turn = self.turn.lock().await;
        // Another request may have fetched the same file while this one waited.
        if target.is_file() {
            return Ok(Fetched::Saved);
        }
        let view = self.ready(app).await?;
        let mut part = target.clone().into_os_string();
        part.push(".part");
        let part = PathBuf::from(part);
        let (done, result) = oneshot::channel();
        *self.job.lock().map_err(|_| "Downloader is unavailable")? = Some(Job {
            url: url.to_string(),
            target: part.clone(),
            done,
        });
        view.eval(&probe_script(url, limit))
            .map_err(|e| e.to_string())?;
        let outcome = match tokio::time::timeout(Duration::from_secs(180), result).await {
            Ok(Ok(outcome)) => outcome,
            _ => {
                self.job.lock().ok().and_then(|mut job| job.take());
                Err("File download timed out".into())
            }
        };
        match outcome {
            Ok(Fetched::Saved) => std::fs::rename(&part, &target)
                .map(|_| Fetched::Saved)
                .map_err(|e| e.to_string()),
            other => {
                let _ = std::fs::remove_file(&part);
                other
            }
        }
    }

    /// The hidden page, once it shows a signed-in LMS page. An expired session
    /// is renewed with the saved credentials, then the page reloads.
    async fn ready(&self, app: &AppHandle) -> Result<Webview, String> {
        let view = match app.get_webview(LABEL) {
            Some(view) => view,
            None => create(app)?,
        };
        for attempt in 0..2 {
            for _ in 0..100 {
                if self.page.lock().is_ok_and(|page| on_lms(&page)) {
                    return Ok(view);
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            if attempt == 0 {
                let school = app.state::<crate::school_web::SchoolWebController>();
                let _ = school.request_session(app, crate::school_auth::SchoolAuthProfile::Science);
                tokio::time::sleep(Duration::from_secs(5)).await;
                view.navigate(Url::parse(HOME).expect("valid LMS URL"))
                    .map_err(|e| e.to_string())?;
            }
        }
        Err("LMS sign-in required".into())
    }

    fn finish(&self, outcome: Result<Fetched, String>) {
        if let Some(job) = self.job.lock().ok().and_then(|mut job| job.take()) {
            let _ = job.done.send(outcome);
        }
    }
}

fn create(app: &AppHandle) -> Result<Webview, String> {
    let main = app.get_window("main").ok_or("Main window is unavailable")?;
    let builder = WebviewBuilder::new(
        LABEL,
        WebviewUrl::External(Url::parse(HOME).expect("valid LMS URL")),
    );
    #[cfg(target_os = "macos")]
    let builder = builder.data_store_identifier(crate::school_web::SCHOOL_DATA_STORE);
    #[cfg(target_os = "windows")]
    let builder = builder.data_directory(
        app.path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("shared-webview-data"),
    );
    let handle = app.clone();
    let for_navigation = app.clone();
    let for_download = app.clone();
    let builder = builder
        .on_page_load(move |_, payload| {
            if payload.event() == PageLoadEvent::Finished {
                if let Ok(mut page) = handle.state::<Downloader>().page.lock() {
                    *page = Some(payload.url().clone());
                }
            }
        })
        .on_navigation(move |url| {
            if !url.as_str().starts_with(RELAY) {
                return url.scheme() == "https";
            }
            let query = |key: &str| {
                url.query_pairs()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.into_owned())
            };
            let downloader = for_navigation.state::<Downloader>();
            match query("state").as_deref() {
                Some("large") => downloader.finish(Ok(Fetched::TooLarge(
                    query("value").and_then(|v| v.parse().ok()).unwrap_or(0),
                ))),
                Some("signin") => {
                    if let Ok(mut page) = downloader.page.lock() {
                        *page = None;
                    }
                    downloader.finish(Err("LMS sign-in required".into()));
                }
                _ => downloader.finish(Err(format!(
                    "File download failed: {}",
                    query("value").unwrap_or_default()
                ))),
            }
            false
        })
        .on_download(move |_, event| {
            let downloader = for_download.state::<Downloader>();
            match event {
                DownloadEvent::Requested { url, destination } => {
                    let target = downloader.job.lock().ok().and_then(|job| {
                        job.as_ref()
                            .filter(|job| job.url == url.as_str())
                            .map(|job| job.target.clone())
                    });
                    match target {
                        Some(target) => {
                            // WebKit will not overwrite an existing file.
                            let _ = std::fs::remove_file(&target);
                            *destination = target;
                            true
                        }
                        None => false,
                    }
                }
                DownloadEvent::Finished { url, success, .. } => {
                    let ours =
                        downloader.job.lock().ok().is_some_and(|job| {
                            job.as_ref().is_some_and(|job| job.url == url.as_str())
                        });
                    if ours {
                        downloader.finish(if success {
                            Ok(Fetched::Saved)
                        } else {
                            Err("File download failed".into())
                        });
                    }
                    true
                }
                _ => true,
            }
        });
    let view = main
        .add_child(
            builder,
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|e| format!("Downloader could not be created: {e}"))?;
    view.hide().map_err(|e| e.to_string())?;
    Ok(view)
}

/// Probe first (one byte, same session): a redirect off LMS or an HTML page
/// means sign-in; the size decides the limit. Then let WebKit download it.
fn probe_script(url: &Url, limit: Option<u64>) -> String {
    let json = |value: &str| serde_json::to_string(value).expect("string serializes");
    format!(
        r#"(async () => {{
  const url = {url}, limit = {limit}, relay = {relay};
  const report = (state, value = "") => {{ location.href = relay + "?state=" + state + "&value=" + encodeURIComponent(value); }};
  try {{
    const abort = new AbortController();
    const response = await fetch(url, {{ credentials: "include", headers: {{ Range: "bytes=0-0" }}, signal: abort.signal }});
    abort.abort();
    const html = (response.headers.get("content-type") || "").includes("text/html") && !/\.html?$/i.test(new URL(url).pathname);
    if (new URL(response.url).origin !== location.origin || html) return report("signin");
    if (!response.ok) return report("error", "HTTP " + response.status);
    const total = Number((response.headers.get("content-range") || "").split("/")[1] || response.headers.get("content-length") || 0);
    if (limit && total > limit) return report("large", total);
    const link = Object.assign(document.createElement("a"), {{ href: url, download: "" }});
    document.body.append(link); link.click(); link.remove();
  }} catch (error) {{ report("error", String(error)); }}
}})();"#,
        url = json(url.as_str()),
        limit = limit.map_or("0".to_string(), |limit| limit.to_string()),
        relay = json(RELAY),
    )
}
