//! Hidden pull navigation, extraction relays and the apply boundary.
use super::*;

const KYOMU_ROOT: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/";

const KYOMU_PULL_PAGES: [&str; 5] = [
    "Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx",
    "Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx",
    "Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_1.aspx",
    "Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx",
    "Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx",
];

const KYOMU_PULL_KINDS: [&str; 5] = ["gpa", "registrations", "declared", "grades", "requirements"];

const OPEN_TIMEOUT: Duration = Duration::from_secs(150);

const PAGE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub(super) struct Extraction {
    pub(super) browser: BrowserId,
    pub(super) run: String,
    pub(super) course_id: Option<u64>,
    pub(super) full: bool,
    pub(super) baseline: u64,
    pub(super) target: String,
    pub(super) visible: bool,
}

/// A hidden pull navigation. It owns automatic sign-in in its view and is
/// resumed at `target` when that sign-in completes elsewhere.
pub(super) struct HiddenWork {
    pub(super) browser: BrowserId,
    pub(super) run: String,
    pub(super) target: Url,
    pub(super) prefix: Option<String>,
}

impl HiddenWork {
    pub(super) fn reached(&self, url: &Url) -> bool {
        match &self.prefix {
            Some(prefix) => url.as_str().starts_with(prefix.as_str()),
            None => url == &self.target,
        }
    }
}

/// Clears the hidden navigation and its unconsumed extraction on every exit.
pub(super) struct HiddenGuard<'a> {
    pub(super) inner: &'a Inner,
    pub(super) run: String,
}

impl Drop for HiddenGuard<'_> {
    fn drop(&mut self) {
        {
            let mut state = self.inner.state.guard();
            if state
                .hidden
                .as_ref()
                .is_some_and(|work| work.run == self.run)
            {
                state.hidden = None;
            }
            if state
                .extraction
                .as_ref()
                .is_some_and(|extraction| extraction.run == self.run)
            {
                state.extraction = None;
            }
        }
    }
}

/// How a hidden navigation ended when it did not fail.
pub(super) enum HiddenLoad<T> {
    Ready(T),
    /// The view landed on a sign-in page that saved credentials cannot pass.
    SignInRequired,
}

impl SchoolBrowserEngine {
    pub(super) async fn pull_once(
        &self,
        site: SchoolSite,
        course_id: Option<u64>,
        context: &PullContext,
    ) -> Result<Vec<IngestOutcome>, SchoolBrowserError> {
        match site {
            SchoolSite::ScienceLms => {
                let outcome = self
                    .extract_page(
                        site,
                        "lms",
                        "https://lms.s.isct.ac.jp/2025/my/",
                        course_id,
                        course_id.is_none(),
                        OPEN_TIMEOUT,
                        context,
                    )
                    .await?;
                Ok(vec![outcome])
            }
            SchoolSite::Kyomu => {
                let landing = self
                    .navigate_and_wait(
                        site,
                        &format!("{KYOMU_ROOT}Default.aspx"),
                        Some(&format!("{KYOMU_ROOT}Student/")),
                        OPEN_TIMEOUT,
                        context,
                    )
                    .await?;
                if let HiddenLoad::SignInRequired = landing {
                    let outcome = sign_in_required(site, "session");
                    let _ = self.0.events.emit(CoreEvent::SchoolIngest(outcome.clone()));
                    return Ok(vec![outcome]);
                }
                let mut outcomes = Vec::new();
                for (page, kind) in KYOMU_PULL_PAGES.into_iter().zip(KYOMU_PULL_KINDS) {
                    let outcome = self
                        .extract_page(
                            site,
                            kind,
                            &format!("{KYOMU_ROOT}{page}"),
                            None,
                            false,
                            PAGE_TIMEOUT,
                            context,
                        )
                        .await;
                    let outcome = match outcome {
                        Ok(outcome) => outcome,
                        Err(
                            error @ (SchoolBrowserError::Cancelled
                            | SchoolBrowserError::Preempted
                            | SchoolBrowserError::Stopped),
                        ) => return Err(error),
                        Err(error) if outcomes.is_empty() => return Err(error),
                        Err(error) => {
                            outcomes.push(IngestOutcome {
                                site,
                                kind: kind.into(),
                                state: IngestState::Error,
                                changed: 0,
                                message: error.to_string(),
                            });
                            break;
                        }
                    };
                    let stop = outcome.state == IngestState::ReconnectRequired;
                    outcomes.push(outcome);
                    if stop {
                        break;
                    }
                }
                Ok(outcomes)
            }
            _ => Err(SchoolBrowserError::InvalidInput(
                "Only LMS and 教務Web can be pulled.".into(),
            )),
        }
    }

    pub(super) async fn navigate_and_wait(
        &self,
        site: SchoolSite,
        raw: &str,
        prefix: Option<&str>,
        timeout: Duration,
        context: &PullContext,
    ) -> Result<HiddenLoad<PageLoaded>, SchoolBrowserError> {
        let url = safe_url(raw)?;
        let mut traces = self.hidden_traces()?;
        let browser = self.ensure_browser(site, url.clone())?;
        let run = new_id();
        let baseline = {
            let mut state = self.0.state.guard();
            if state.browsing.is_some() {
                return Err(SchoolBrowserError::Preempted);
            }
            state.hidden = Some(HiddenWork {
                browser,
                run: run.clone(),
                target: url.clone(),
                prefix: prefix.map(str::to_string),
            });
            state
                .views
                .get(&site)
                .map(|view| view.last_page)
                .unwrap_or_default()
        };
        let _hidden = HiddenGuard {
            inner: &self.0,
            run,
        };
        self.dispatch(browser, BrowserAction::Navigate(url))?;
        let mut signing_in = None;
        tokio::time::timeout(timeout, async {
            loop {
                if context.is_cancelled() {
                    return Err(SchoolBrowserError::Cancelled);
                }
                let received = tokio::select! {
                    value = traces.recv() => value,
                    _ = tokio::time::sleep(Duration::from_millis(250)) => continue,
                };
                match received {
                    Ok(Trace::Loaded(event))
                        if event.page.browser == browser
                            && event.page.sequence > baseline
                            && prefix
                                .is_none_or(|prefix| event.url.as_str().starts_with(prefix)) =>
                    {
                        return Ok(HiddenLoad::Ready(event))
                    }
                    Ok(Trace::Loaded(event)) if event.page.browser == browser => {
                        if self.sign_in_landing(&event.url, &mut signing_in) {
                            return Ok(HiddenLoad::SignInRequired);
                        }
                    }
                    Ok(Trace::Session { profile, state })
                        if signing_in == Some(profile)
                            && sign_in_settled_without_session(state) =>
                    {
                        return Ok(HiddenLoad::SignInRequired)
                    }
                    Ok(Trace::Failed(event)) if event.page.browser == browser => {
                        return Err(SchoolBrowserError::Platform(event.message))
                    }
                    Ok(Trace::Browsing(Some(_))) => return Err(SchoolBrowserError::Preempted),
                    Ok(Trace::Closed(id)) if id == browser => return Err(view_closed()),
                    Ok(Trace::Stopped) => return Err(SchoolBrowserError::Stopped),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(SchoolBrowserError::Stopped)
                    }
                }
            }
        })
        .await
        .map_err(|_| {
            SchoolBrowserError::Timeout(
                "The school page did not answer in time. Check the account in Settings.".into(),
            )
        })?
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn extract_page(
        &self,
        site: SchoolSite,
        kind: &str,
        url: &str,
        course_id: Option<u64>,
        full: bool,
        timeout: Duration,
        context: &PullContext,
    ) -> Result<IngestOutcome, SchoolBrowserError> {
        let parsed = safe_url(url)?;
        let mut traces = self.hidden_traces()?;
        let browser = self.ensure_browser(site, parsed.clone())?;
        let run = new_id();
        {
            let mut state = self.0.state.guard();
            // Checked under the same lock that arms the extraction: opening a
            // page clears it, so the visible view is never navigated for a pull.
            if state.browsing.is_some() {
                return Err(SchoolBrowserError::Preempted);
            }
            let baseline = state
                .views
                .get(&site)
                .map(|view| view.last_page)
                .unwrap_or_default();
            state.extraction = Some(Extraction {
                browser,
                run: run.clone(),
                course_id,
                full,
                baseline,
                target: parsed.to_string(),
                visible: false,
            });
            state.hidden = Some(HiddenWork {
                browser,
                run: run.clone(),
                target: parsed.clone(),
                prefix: None,
            });
        }
        let _hidden = HiddenGuard {
            inner: &self.0,
            run: run.clone(),
        };
        self.dispatch(browser, BrowserAction::Navigate(parsed))?;
        let mut signing_in = None;
        let raw = tokio::time::timeout(timeout, async {
            loop {
                if context.is_cancelled() {
                    return Err(SchoolBrowserError::Cancelled);
                }
                let received = tokio::select! {
                    value = traces.recv() => value,
                    _ = tokio::time::sleep(Duration::from_millis(250)) => continue,
                };
                match received {
                    Ok(Trace::Extraction {
                        browser: found,
                        run: found_run,
                        raw,
                    }) if found == browser && found_run == run => {
                        return Ok(HiddenLoad::Ready(raw))
                    }
                    Ok(Trace::Loaded(event)) if event.page.browser == browser => {
                        if self.sign_in_landing(&event.url, &mut signing_in) {
                            return Ok(HiddenLoad::SignInRequired);
                        }
                    }
                    Ok(Trace::Session { profile, state })
                        if signing_in == Some(profile)
                            && sign_in_settled_without_session(state) =>
                    {
                        return Ok(HiddenLoad::SignInRequired)
                    }
                    Ok(Trace::Browsing(Some(_))) => return Err(SchoolBrowserError::Preempted),
                    Ok(Trace::Failed(event)) if event.page.browser == browser => {
                        return Err(SchoolBrowserError::Platform(event.message))
                    }
                    Ok(Trace::Closed(id)) if id == browser => return Err(view_closed()),
                    Ok(Trace::Stopped) => return Err(SchoolBrowserError::Stopped),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(SchoolBrowserError::Stopped)
                    }
                }
            }
        })
        .await
        .map_err(|_| {
            SchoolBrowserError::Timeout(
                "The school page did not answer in time. Check the account in Settings.".into(),
            )
        })??;
        let raw = match raw {
            HiddenLoad::Ready(raw) => raw,
            HiddenLoad::SignInRequired => {
                let outcome = sign_in_required(site, kind);
                let _ = self.0.events.emit(CoreEvent::SchoolIngest(outcome.clone()));
                return Ok(outcome);
            }
        };
        if !context.applying().await {
            return Err(SchoolBrowserError::Cancelled);
        }
        let store = self.0.store.clone();
        let raw_for_apply = raw.clone();
        let applied =
            tokio::task::spawn_blocking(move || apply_raw(&store, site, &raw_for_apply)).await;
        // The write gate covers this page's commit only; the next 教務Web page
        // can be cancelled again.
        context.applied().await;
        let outcome = applied.map_err(|_| SchoolBrowserError::Stopped)?;
        let _ = self.0.events.emit(CoreEvent::SchoolIngest(outcome.clone()));
        Ok(outcome)
    }

    /// Records that a hidden navigation landed on an identity-provider sign-in
    /// page. Returns `true` when saved credentials cannot recover it: the
    /// profile's automatic sign-in is blocked after a failure.
    pub(super) fn sign_in_landing(
        &self,
        url: &Url,
        signing_in: &mut Option<SchoolAuthProfile>,
    ) -> bool {
        let Some(profile) = is_identity_login(url)
            .then(|| url.host_str().and_then(SchoolAuthProfile::for_auth_host))
            .flatten()
        else {
            return false;
        };
        *signing_in = Some(profile);
        self.0
            .state
            .with(|state| state.login.is_none() && state.auto_blocked.contains(&profile))
    }
}

impl Inner {
    pub(super) fn progress_relay(&self, browser: BrowserId, encoded: &str) {
        if encoded.len() > 4 * 1024 {
            return;
        }
        let Some(progress) = URL_SAFE_NO_PAD
            .decode(encoded)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ProgressRelay>(&bytes).ok())
        else {
            return;
        };
        let active = self.state.with(|state| {
            state
                .extraction
                .as_ref()
                .is_some_and(|run| run.browser == browser && run.run == progress.extraction_run)
        });
        if active
            && matches!(
                progress.phase.as_str(),
                "catalog" | "course" | "material" | "assignment"
            )
            && progress.completed <= progress.total
            && progress.total <= 10_000
        {
            let _ = self
                .events
                .emit(CoreEvent::SchoolPullProgress(SchoolPullProgress {
                    site: SchoolSite::ScienceLms,
                    phase: progress.phase,
                    completed: progress.completed,
                    total: progress.total,
                    course_id: progress.course_id,
                    course_title: truncate(progress.course_title, 512),
                    term: truncate(progress.term, 16),
                    item_title: truncate(progress.item_title, 512),
                }));
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProgressRelay {
    #[serde(default, rename = "_isctExtractionRun")]
    extraction_run: String,
    phase: String,
    completed: usize,
    total: usize,
    #[serde(default)]
    course_id: u64,
    #[serde(default)]
    course_title: String,
    #[serde(default)]
    term: String,
    #[serde(default)]
    item_title: String,
}

pub(super) fn extraction_script(setup: &str, script: &str, relay: &str, run: &str) -> String {
    let run = serde_json::to_string(run).unwrap_or_else(|_| "\"\"".into());
    format!("{setup}Promise.resolve({script}).then(v=>{{if(v&&typeof v==='object')v._isctExtractionRun={run};return v}}).then(v=>location.href='{relay}'+btoa(unescape(encodeURIComponent(JSON.stringify(v)))).replace(/\\+/g,'-').replace(/\\//g,'_').replace(/=+$/,''))")
}

pub(super) fn ingestion_script(raw: &str) -> Option<&'static str> {
    let url = Url::parse(raw).ok()?;
    if url.host_str() == Some("lms.s.isct.ac.jp")
        && url.path().starts_with("/2025/")
        && !url.path().contains("/pluginfile.php/")
    {
        return Some(LMS_EXTRACT_SCRIPT);
    }
    (url.host_str() == Some("kyomu0.gakumu.titech.ac.jp")
        && KYOMU_PULL_PAGES
            .iter()
            .any(|page| url.path() == format!("/Titech/{page}")))
    .then_some(KYOMU_EXTRACT_SCRIPT)
}

/// Applies one extracted page through the store's commit path, so a visible
/// page and a hidden pull publish the same workspace change.
pub(super) fn apply_raw(store: &DataStore, site: SchoolSite, raw: &str) -> IngestOutcome {
    let source = Some(WorkspaceChangeSource::SchoolPull);
    let applied = if site == SchoolSite::ScienceLms {
        match parse_lms_payload(raw) {
            Ok((LmsStage::Ready, Some(payload))) => store
                .write(source, |transaction| {
                    apply_lms_payload(transaction, &payload)
                })
                .map(|changed| ("lms".to_string(), changed))
                .map_err(|error| ("lms".to_string(), Some(error.to_string()))),
            Ok(_) => Err(("lms".into(), None)),
            Err(error) => Err((
                "lms".into(),
                Some(
                    serde_json::from_str::<serde_json::Value>(raw)
                        .ok()
                        .and_then(|value| value.get("error")?.as_str().map(str::to_string))
                        .map(|reason| truncate(reason, 1_024))
                        .map(|reason| format!("{error}: {reason}"))
                        .unwrap_or_else(|| error.to_string()),
                ),
            )),
        }
    } else {
        match parse_kyomu_payload(raw) {
            Ok((KyomuStage::Ready, Some(payload))) => {
                let kind = payload.kind.clone();
                store
                    .write(source, |transaction| {
                        apply_kyomu_payload(transaction, &payload)
                    })
                    .map(|changed| (kind.clone(), changed))
                    .map_err(|error| (kind, Some(error.to_string())))
            }
            Ok((KyomuStage::Unavailable, Some(payload))) => {
                let kind = payload.kind.clone();
                let message = payload.message.clone();
                return match store.write(source, |transaction| {
                    record_kyomu_unavailable(transaction, &payload)
                }) {
                    Ok(_) => IngestOutcome {
                        site,
                        kind,
                        state: IngestState::Unavailable,
                        changed: 0,
                        message,
                    },
                    Err(error) => IngestOutcome {
                        site,
                        kind,
                        state: IngestState::Error,
                        changed: 0,
                        message: error.to_string(),
                    },
                };
            }
            Ok((_, payload)) => Err((
                payload
                    .map(|value| value.kind)
                    .unwrap_or_else(|| "session".into()),
                None,
            )),
            Err(error) => Err(("unknown".into(), Some(error.to_string()))),
        }
    };
    match applied {
        Ok((kind, changed)) => IngestOutcome {
            site,
            kind,
            state: IngestState::Ok,
            changed,
            message: format!("{changed} rows changed"),
        },
        // No message: the page asked for sign-in instead of returning data.
        Err((kind, None)) => sign_in_required(site, &kind),
        Err((kind, Some(message))) => IngestOutcome {
            site,
            kind,
            state: IngestState::Error,
            changed: 0,
            message,
        },
    }
}

pub(super) fn sign_in_required(site: SchoolSite, kind: &str) -> IngestOutcome {
    IngestOutcome {
        site,
        kind: kind.into(),
        state: IngestState::ReconnectRequired,
        changed: 0,
        message: "Sign-in required".into(),
    }
}

#[cfg(test)]
#[path = "../../tests/school_browser__engine__extraction.rs"]
mod tests;
