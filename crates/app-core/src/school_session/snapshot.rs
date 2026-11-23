use super::{guard, Account, Inner, SchoolSessionState, SchoolSignInProblem, SignInPresentation};
use crate::{events::CoreEvent, school_auth::SchoolAuthProfile};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A site the school paused for maintenance, while the pause lasts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SchoolMaintenance {
    pub notice: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub until: DateTime<Utc>,
}

/// One account's session as shells render it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SchoolProfileSession {
    pub sign_out_pending: bool,
    pub state: SchoolSessionState,
    pub presentation: SignInPresentation,
    pub problem: Option<SchoolSignInProblem>,
    pub refusal_unsaved: bool,
    /// When Core tries again on its own (`unavailable` only).
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub retry_at: Option<DateTime<Utc>>,
    /// The account's site (LMS, 教務Web) is paused for maintenance.
    pub maintenance: Option<SchoolMaintenance>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SchoolSessionProfiles {
    pub science: SchoolProfileSession,
    pub tokyo_tech: SchoolProfileSession,
}

/// Sent as `school-session` whenever it changes; a shell keeps the higher
/// revision of this and what it read.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SchoolSessionSnapshot {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub revision: u64,
    pub profiles: SchoolSessionProfiles,
}

impl SchoolSessionSnapshot {
    pub fn profile(&self, profile: SchoolAuthProfile) -> &SchoolProfileSession {
        match profile {
            SchoolAuthProfile::Science => &self.profiles.science,
            SchoolAuthProfile::TokyoTech => &self.profiles.tokyo_tech,
        }
    }
}

impl Inner {
    pub(super) fn profile(&self, profile: SchoolAuthProfile) -> SchoolProfileSession {
        let account = self.account(profile);
        let (state, problem, retry_at) = account.identity.view(self.auth.refusal(profile));
        let sign_out_pending = account.owner.sign_out_pending();
        let presentation = SignInPresentation::for_profile(
            state,
            problem.as_ref().map(|problem| problem.kind),
            retry_at.is_some(),
            sign_out_pending,
        );
        SchoolProfileSession {
            sign_out_pending,
            state,
            presentation,
            problem,
            refusal_unsaved: self.auth.refusal_unsaved(profile),
            retry_at,
            maintenance: self.maintenance(account),
        }
    }

    fn profiles(&self) -> SchoolSessionProfiles {
        SchoolSessionProfiles {
            science: self.profile(SchoolAuthProfile::Science),
            tokyo_tech: self.profile(SchoolAuthProfile::TokyoTech),
        }
    }

    /// The account's site paused for maintenance: by the school's notice, or
    /// by the host's breaker after a maintenance answer.
    fn maintenance(&self, account: &Account) -> Option<SchoolMaintenance> {
        let host = account.sites.first()?.site.host();
        let until = self.transport.paused_until(host)?;
        let notice = account
            .sites
            .iter()
            .find_map(|site| guard(&site.state).notice.clone());
        notice.as_ref()?;
        Some(SchoolMaintenance {
            notice,
            until: DateTime::from_timestamp(until, 0).unwrap_or_else(crate::clock::now),
        })
    }

    /// Brings the published snapshot up to date and returns it, sending
    /// `school-session` when what shells render changed. The view is derived
    /// and numbered under the snapshot's lock: a later revision is always a
    /// later view. Events may arrive out of order; shells keep the highest
    /// revision.
    pub(super) fn publish(&self) -> SchoolSessionSnapshot {
        let (snapshot, changed) = {
            let mut published = guard(&self.published);
            let profiles = self.profiles();
            match published.as_mut() {
                Some(current) if current.profiles == profiles => (current.clone(), false),
                Some(current) => {
                    current.revision += 1;
                    current.profiles = profiles;
                    (current.clone(), true)
                }
                None => (
                    published
                        .insert(SchoolSessionSnapshot {
                            revision: 1,
                            profiles,
                        })
                        .clone(),
                    true,
                ),
            }
        };
        if changed {
            if let Some(events) = self.events.get() {
                let _ = events.emit(CoreEvent::SchoolSession(snapshot.clone()));
            }
        }
        snapshot
    }
}
