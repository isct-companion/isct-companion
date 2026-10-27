//! Safe Back/Forward history and same-site popup return.
use super::*;

#[derive(Default)]
pub(super) struct PageHistory {
    pub(super) pages: Vec<String>,
    pub(super) forward: Vec<String>,
}

#[derive(Clone)]
pub(super) struct PopupReturn {
    pub(super) opener: String,
    pub(super) popup: String,
}

impl PageHistory {
    pub(super) fn push(&mut self, url: String) {
        if self.pages.last() == Some(&url) {
            return;
        }
        self.pages.push(url);
        self.forward.clear();
        if self.pages.len() > 50 {
            self.pages.remove(0);
        }
    }
    pub(super) fn back_target(&mut self, current: Option<&str>) -> Option<String> {
        let index = self
            .pages
            .iter()
            .rposition(|page| Some(page.as_str()) != current)?;
        if let Some(left) = self.pages.drain(index + 1..).next_back() {
            self.forward.push(left);
        }
        self.pages.last().cloned()
    }
    pub(super) fn forward_target(&mut self) -> Option<String> {
        let next = self.forward.pop()?;
        self.pages.push(next.clone());
        Some(next)
    }
}

impl SchoolBrowserEngine {
    pub fn back(&self, site: SchoolSite) -> Result<bool, SchoolBrowserError> {
        let Ok(browser) = self.browser_for(site) else {
            return Ok(false);
        };
        let target = {
            let mut state = self.0.state.guard();
            let current = state.views.get(&site).map(|view| view.url.clone());
            let target = state
                .history
                .entry(site)
                .or_default()
                .back_target(current.as_deref());
            if target.is_some() {
                state.suppress_history.insert(site);
            }
            target
        };
        let Some(target) = target else {
            return Ok(false);
        };
        self.dispatch(browser, BrowserAction::Navigate(interactive_url(&target)?))?;
        Ok(true)
    }

    pub fn forward(&self, site: SchoolSite) -> Result<bool, SchoolBrowserError> {
        let Ok(browser) = self.browser_for(site) else {
            return Ok(false);
        };
        let target = self
            .0
            .state
            .guard()
            .history
            .entry(site)
            .or_default()
            .forward_target();
        let Some(target) = target else {
            return Ok(false);
        };
        self.dispatch(browser, BrowserAction::Navigate(interactive_url(&target)?))?;
        Ok(true)
    }
}

pub(super) fn history_entry(_site: SchoolSite, raw: &str) -> Option<String> {
    let safe = browsing_url(raw)?;
    let url = Url::parse(&safe).ok()?;
    match url.host_str().and_then(SchoolAuthProfile::for_auth_host) {
        Some(profile) if !login_succeeded(&safe, profile) => None,
        _ => Some(safe),
    }
}

pub(super) fn same_page(left: &str, right: &str) -> bool {
    let (Ok(left), Ok(right)) = (Url::parse(left), Url::parse(right)) else {
        return false;
    };
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
        && left.path() == right.path()
}
