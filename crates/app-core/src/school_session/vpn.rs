//! The campus VPN's APM session, reached from the Tokyo Tech identity.
//!
//! The APM sends a browser to GetAccess's proxy, which knows the account from
//! the portal session and asks for the portal password once more, then hands
//! the browser back to the APM. The walk ends on the APM holding its session
//! cookie, which is all openconnect needs. The jar is this walk's own and is
//! dropped with it; nothing of the session is logged or kept.

use super::{
    jar::Jar,
    walk::{Page, Walk, WalkError},
    SchoolSessions, WebStop,
};
use crate::{
    net::{Host, NetError},
    school_auth::SchoolAuthProfile,
    vpn::f5,
};
use adpt_isct::login::{self as pages, TokyoPage, VpnPage};
use std::sync::{Mutex, PoisonError};
use url::Url;
use vpn_protocol::SessionCookie;

/// The SSL-VPN entry the portal's resource menu links to.
pub const APM_ENTRY: &str = "https://apm.nap.gsic.titech.ac.jp/";
/// Hand-offs and the proxy's password page together (the APM's policy page,
/// the proxy's form, the way back); more means the way in changed.
const MAX_STEPS: usize = 6;

/// A signed-in APM session for the helper. The cookie is a secret.
#[derive(Debug)]
pub struct CampusVpnLogin {
    pub cookie: SessionCookie,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CampusVpnError {
    #[error("no Tokyo Tech account is saved")]
    NeedsAccount,
    #[error("the school refused the saved Tokyo Tech password")]
    Rejected,
    #[error("the portal wants the person on its own page")]
    NeedsWeb,
    #[error("the campus VPN could not be reached: {0}")]
    Unreachable(String),
    #[error("the campus VPN refused access")]
    Denied,
    #[error("the way into the campus VPN changed: {0}")]
    Changed(&'static str),
    /// The walk met GetAccess's sign-in: the identity no longer holds.
    #[error("the portal asked to sign in again")]
    SignIn,
}

impl CampusVpnError {
    /// The stable name shells key their text on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NeedsAccount => "vpn_needs_account",
            Self::Rejected => "vpn_login_rejected",
            Self::NeedsWeb => "vpn_needs_web",
            Self::Unreachable(_) => "vpn_unreachable",
            Self::Denied => "vpn_access_denied",
            Self::Changed(_) | Self::SignIn => "vpn_login_changed",
        }
    }
}

impl From<WebStop> for CampusVpnError {
    fn from(stop: WebStop) -> Self {
        match stop {
            WebStop::Rejected => Self::Rejected,
            WebStop::NeedsWeb => Self::NeedsWeb,
            WebStop::NotConfigured => Self::NeedsAccount,
            WebStop::Unreachable | WebStop::Storage | WebStop::Stale | WebStop::Unsupported => {
                Self::Unreachable("the Tokyo Tech sign-in is not available".into())
            }
        }
    }
}

impl From<WalkError> for CampusVpnError {
    fn from(error: WalkError) -> Self {
        match error {
            WalkError::Off(what) => Self::Changed(what),
            WalkError::Http(status) => Self::Unreachable(NetError::Http(status).to_string()),
            WalkError::Net(error) => Self::Unreachable(error.to_string()),
        }
    }
}

impl SchoolSessions {
    /// A fresh APM session from the Tokyo Tech identity, signing the identity
    /// in with the saved credentials when it has none or the portal no longer
    /// takes it. Each call is a new session: the APM's sessions are one-time.
    pub async fn campus_vpn_login(&self) -> Result<CampusVpnLogin, CampusVpnError> {
        let profile = SchoolAuthProfile::TokyoTech;
        let prepared = self.prepare_web(profile, false).await?;
        match self.walk_to_apm(profile).await {
            Err(CampusVpnError::SignIn) if !prepared.signed_in => {
                self.sign_in_web(profile, prepared.identity, false).await?;
                self.walk_to_apm(profile).await
            }
            ended => ended,
        }
    }

    async fn walk_to_apm(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<CampusVpnLogin, CampusVpnError> {
        let (snapshot, _) = self
            .0
            .account(profile)
            .identity
            .snapshot()
            .ok_or(CampusVpnError::SignIn)?;
        let jar = Mutex::new(snapshot.jar());
        // The APM refuses any agent but a browser's.
        let walk = Walk::new(self.0.transport.as_ref(), &jar).browser();
        let entry = Url::parse(APM_ENTRY).map_err(|_| WalkError::Off("bad APM address"))?;
        let menu = Url::parse(adpt_isct::urls::entry::TOKYO_PORTAL)
            .map_err(|_| WalkError::Off("bad portal address"))?;
        // The APM lets in only a browser coming from the portal's menu.
        let mut page = walk.get(entry.clone(), Some(menu)).await?;
        let mut password_sent = false;
        for _ in 0..MAX_STEPS {
            page = match pages::vpn(&page.html, page.url.as_str()) {
                VpnPage::Password(_) if password_sent => return Err(CampusVpnError::Rejected),
                VpnPage::Password(mut form) => {
                    let password = self.portal_password(profile).await?;
                    form.set("password", &password);
                    password_sent = true;
                    log::info!(target: "app_core::school_session", "step: campus VPN password");
                    submit(&walk, &form, &page).await?
                }
                VpnPage::Relay(form) => submit(&walk, &form, &page).await?,
                VpnPage::Other if Host::of(&page.url) == Some(Host::CampusVpn) => {
                    return session(&jar, &page, &entry);
                }
                VpnPage::Other => {
                    return Err(match pages::tokyo(&page.html, page.url.as_str()) {
                        TokyoPage::Other => CampusVpnError::Changed("an unknown page"),
                        _ => CampusVpnError::SignIn,
                    })
                }
            };
        }
        Err(CampusVpnError::Changed("too many steps"))
    }

    /// The saved Tokyo Tech password, read when the proxy asks for it.
    async fn portal_password(&self, profile: SchoolAuthProfile) -> Result<String, CampusVpnError> {
        let auth = self.0.auth.clone();
        match tokio::task::spawn_blocking(move || auth.sign_in_credentials(profile)).await {
            Ok(Ok(Some(saved))) if !saved.credentials.password.is_empty() => {
                Ok(saved.credentials.password)
            }
            Ok(Ok(_)) => Err(CampusVpnError::NeedsAccount),
            Ok(Err(error)) => Err(CampusVpnError::Unreachable(format!(
                "the saved credentials could not be read: {error}"
            ))),
            Err(_) => Err(CampusVpnError::Unreachable("cancelled".into())),
        }
    }
}

/// Posts a form met on the way and follows where it leads. Its action must
/// stay on the school's hosts (the walk refuses anything else).
async fn submit(walk: &Walk<'_>, form: &pages::Form, page: &Page) -> Result<Page, WalkError> {
    let action =
        Url::parse(&form.action).map_err(|_| WalkError::Off("a form named no address"))?;
    let step = walk.post(&action, form.fields(), &page.url, false).await?;
    walk.follow(step, &action).await
}

/// The APM page the walk ended on, read as a session: a logout or hang-up
/// page is the APM refusing the access policy.
fn session(jar: &Mutex<Jar>, page: &Page, apm: &Url) -> Result<CampusVpnLogin, CampusVpnError> {
    let path = page.url.path().to_ascii_lowercase();
    if path.contains("logout") || path.contains("hangup") {
        return Err(CampusVpnError::Denied);
    }
    let header = jar
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .header(apm)
        .unwrap_or_default();
    let value = |name: &str| {
        header
            .split("; ")
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, value)| *key == name && !value.is_empty())
            .map(|(_, value)| value.to_owned())
    };
    let mrh = value("MRHSession").ok_or(CampusVpnError::Denied)?;
    let f5_st = value("F5_ST");
    let cookie = f5::session_cookie(&mrh, f5_st.as_deref())
        .map_err(|_| CampusVpnError::Changed("an unreadable session cookie"))?;
    Ok(CampusVpnLogin { cookie })
}

#[cfg(test)]
#[path = "../tests/school_session__vpn.rs"]
mod tests;
