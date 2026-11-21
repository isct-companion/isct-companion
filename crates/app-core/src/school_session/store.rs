//! An account's session as the device keeps it: one device-only vault record
//! per account (`school-session-science`, `school-session-tokyo-tech`) with
//! the identity snapshot and each site's jar, spread over several items where
//! one is too small for it (`secret_parts`). Never synchronized. A session
//! read back is only a candidate: the first request finds out whether the
//! school still honours it.

use super::{jar::Jar, Inner, Site, Snapshot, WebCookie};
use crate::school_auth::SchoolAuthProfile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

const VERSION: u8 = 2;

#[derive(Deserialize, Serialize)]
pub(super) struct Saved {
    version: u8,
    pub(super) identity: Vec<WebCookie>,
    /// Each site's jar, as `jar::Jar::save` wrote it.
    pub(super) sites: BTreeMap<String, String>,
}

impl Saved {
    pub(super) fn new(identity: Vec<WebCookie>) -> Self {
        Self {
            version: VERSION,
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

impl Inner {
    /// Reads the account's vault items once: its identity, the sites' jars
    /// and the refusals kept on this device. The read holds a permit until
    /// what it read is installed, so a barrier forgets and deletes only
    /// after it; the blocking read holds one of its own to its end, since a
    /// shell's vault is free to write while it reads.
    pub(super) async fn restore(&self, profile: SchoolAuthProfile) {
        let account = self.account(profile);
        let first = AtomicBool::new(false);
        account
            .restored
            .get_or_init(|| async {
                first.store(true, Ordering::SeqCst);
                // A barrier is running: it deletes the item anyway.
                let Some(permit) = account.owner.permit() else {
                    return;
                };
                let Some(blocking) = account.owner.permit_of(permit.generation()) else {
                    return;
                };
                let auth = self.auth.clone();
                let loaded = tokio::task::spawn_blocking(move || {
                    let _permit = blocking;
                    if let Err(error) = auth.load_refusals() {
                        log::warn!(target: "app_core::school_session", "refusals not read: {error}");
                    }
                    let saved = auth.load_session(profile).ok().flatten()?;
                    Saved::decode(&saved)
                })
                .await
                .ok()
                .flatten();
                let Some(saved) = loaded else { return };
                let snapshot = Snapshot {
                    cookies: saved.identity.clone(),
                };
                let Some(generation) = account.identity.restore(snapshot) else {
                    return;
                };
                for site in &account.sites {
                    if let Some(jar) = saved.site(site.site).and_then(Jar::load) {
                        site.install_restored(jar, generation);
                    }
                }
                log::info!(target: "app_core::school_session", "restored the saved session profile={profile:?}");
            })
            .await;
        // The jars and refusals are known now, no longer unchecked.
        if first.load(Ordering::SeqCst) {
            self.changed();
        }
        self.publish();
    }

    /// Saves the identity and every site's jar of the account, for an
    /// operation of account generation `generation`. The write holds a
    /// permit of that generation until it lands, however the caller ends: a
    /// barrier waits for it, so it never lands after the barrier deleted
    /// the item, and an operation of an older generation writes nothing.
    pub(super) async fn persist(&self, profile: SchoolAuthProfile, generation: u64) {
        let account = self.account(profile);
        let Some(permit) = account.owner.permit_of(generation) else {
            return;
        };
        let Some(snapshot) = account.identity.persisted() else {
            return;
        };
        let mut saved = Saved::new(snapshot.cookies.clone());
        for site in &account.sites {
            saved = saved.with_site(site.site, site.saved_jar());
        }
        let Some(text) = saved.encode() else { return };
        let auth = self.auth.clone();
        let stored = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            auth.store_session(profile, &text)
        })
        .await;
        if let Ok(Err(error)) = stored {
            log::warn!(target: "app_core::school_session", "session not saved profile={profile:?}: {error}");
        }
    }
}
