//! An account's session as the device keeps it: one device-only vault item
//! per account (`school-session-science`, `school-session-tokyo-tech`) with
//! the identity snapshot and each site's jar. Never synchronized. A session
//! read back is only a candidate: the first request finds out whether the
//! school still honours it.

use super::{Site, WebCookie};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const VERSION: u8 = 1;

#[derive(Deserialize, Serialize)]
pub(super) struct Saved {
    version: u8,
    /// The username the session belongs to.
    pub(super) account: String,
    pub(super) identity: Vec<WebCookie>,
    /// Each site's jar, as `jar::Jar::save` wrote it.
    pub(super) sites: BTreeMap<String, String>,
}

impl Saved {
    pub(super) fn new(account: String, identity: Vec<WebCookie>) -> Self {
        Self {
            version: VERSION,
            account,
            identity,
            sites: BTreeMap::new(),
        }
    }

    pub(super) fn with_site(mut self, site: Site, jar: Option<String>) -> Self {
        if let Some(jar) = jar {
            self.sites.insert(site.key().to_owned(), jar);
        }
        self
    }

    pub(super) fn site(&self, site: Site) -> Option<&str> {
        self.sites.get(site.key()).map(String::as_str)
    }

    pub(super) fn encode(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }

    /// A record of another version is not read: the next sign-in replaces it.
    pub(super) fn decode(text: &str) -> Option<Self> {
        serde_json::from_str::<Self>(text)
            .ok()
            .filter(|saved| saved.version == VERSION)
    }
}
