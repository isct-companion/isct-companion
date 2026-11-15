//! An account sign-in over HTTP, with no web view: Extic (Science Tokyo ID,
//! password, App one-time password) and GetAccess (Tokyo Tech account,
//! password, then the Matrix challenge).
//!
//! Only the identity provider's own sign-in page gets the password and the
//! second factor, and only at its own form address. The flow never trusts a
//! provider's "carry on" answer: it ends by opening the target (the LMS
//! dashboard, a 教務Web page), and only that page proves the sign-in. A page
//! asking again for a factor already sent ends the flow instead of sending
//! it twice: a wrong password or one-time password must never be retried
//! until the account locks. A maintenance page of the target ends it with
//! no verdict.
//!
//! The attempt's jar is thrown away afterwards. What it keeps is the
//! identity snapshot: every `Set-Cookie` the identity hosts sent, with the
//! URL it came from, the last value of each cookie.

use super::{
    jar::Jar,
    walk::{is_lms_acs, is_rejection, Page, Step, Walk, WalkError},
    Snapshot, Transport, WebCookie, OTP_MARGIN,
};
use crate::{
    net::{Host, NetError},
    school_auth::{upcoming_otps, OtpWindow, SchoolAuthError, SchoolCredentials},
    school_session::SchoolSignInProblemKind as Kind,
};
use adpt_isct::login::{self, Form, SciencePage, TokyoPage};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

/// The LMS dashboard: opening it starts the SAML hand-off that ends in a
/// Moodle session, and it is the page that proves an LMS session.
pub(crate) const LMS_HOME: &str = "https://lms.s.isct.ac.jp/2025/my/";
/// GetAccess's entry for a 教務Web resource; the agreement page follows.
const GETACCESS_START: &str = "https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList?lang=ja";
/// 教務Web's sign-in entry: the page that starts the GetAccess ticket round
/// trip. A deep page opened without it only lands on the portal menu.
pub(crate) const KYOMU_ENTRY: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx";
/// Where 教務Web lands a signed-in student.
pub(crate) const KYOMU_STUDENT: &str = "/Titech/Student/";
/// The second-factor page Extic shows after the password.
const EXTIC_SECOND_FACTOR: &str = "/auth/session/second_factor";

/// How one sign-in ended.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum LoginEnd {
    SignedIn(Snapshot),
    /// The school turned the credentials down.
    Rejected(Kind, String),
    /// The school wants a person on its own page.
    NeedsWeb(Kind, String),
    /// The site showed its maintenance page, with this notice: no verdict
    /// on the credentials.
    Maintenance(String),
    /// No verdict: the network, a server error, no answer in time.
    Unavailable(Kind, String),
}

#[derive(Clone, Debug)]
enum Fail {
    Rejected(Kind, &'static str),
    NeedsWeb(Kind, &'static str),
    Maintenance(String),
    Walk(WalkError),
}

impl From<WalkError> for Fail {
    fn from(error: WalkError) -> Self {
        Self::Walk(error)
    }
}

impl From<NetError> for Fail {
    fn from(error: NetError) -> Self {
        Self::Walk(WalkError::Net(error))
    }
}

/// Signs in to Extic and finishes on the LMS dashboard. `submitted` turns
/// true just before the password leaves the device: a sign-in stopped after
/// that has been seen by the school.
pub(super) async fn science(
    transport: &dyn Transport,
    credentials: &SchoolCredentials,
    submitted: &AtomicBool,
) -> LoginEnd {
    let jar = Mutex::new(Jar::default());
    let captured = Mutex::new(Vec::new());
    let walk = Walk::new(transport, &jar).capturing(&[Host::ScienceAccount], &captured);
    let ended = science_flow(&walk, credentials, submitted).await;
    finish(ended, submitted, &captured)
}

/// Signs in through GetAccess, answers the Matrix challenge, and finishes on
/// a 教務Web page.
pub(super) async fn tokyo(
    transport: &dyn Transport,
    credentials: &SchoolCredentials,
    submitted: &AtomicBool,
) -> LoginEnd {
    let jar = Mutex::new(Jar::default());
    let captured = Mutex::new(Vec::new());
    let walk =
        Walk::new(transport, &jar).capturing(&[Host::GetAccess, Host::GetAccessRouter], &captured);
    let ended = tokyo_flow(&walk, &jar, credentials, submitted).await;
    finish(ended, submitted, &captured)
}

fn finish(
    ended: Result<(), Fail>,
    submitted: &AtomicBool,
    captured: &Mutex<Vec<WebCookie>>,
) -> LoginEnd {
    match ended {
        Ok(()) => {
            let lines = captured
                .lock()
                .map(|lines| lines.clone())
                .unwrap_or_default();
            LoginEnd::SignedIn(Snapshot::from_responses(lines, chrono::Utc::now()))
        }
        Err(Fail::Rejected(kind, what)) => LoginEnd::Rejected(kind, what.into()),
        Err(Fail::NeedsWeb(kind, what)) => LoginEnd::NeedsWeb(kind, what.into()),
        Err(Fail::Maintenance(notice)) => LoginEnd::Maintenance(notice),
        Err(Fail::Walk(WalkError::Http(status))) if status < 500 => LoginEnd::NeedsWeb(
            Kind::UnknownPage,
            format!("the school answered HTTP {status}"),
        ),
        Err(Fail::Walk(WalkError::Off(what))) => LoginEnd::NeedsWeb(Kind::UnknownPage, what.into()),
        Err(Fail::Walk(error)) => {
            // After the password went out, a missing answer is the school's
            // to give: it counts as submitted, never as a rejection.
            let kind = if submitted.load(Ordering::SeqCst) {
                Kind::Unanswered
            } else {
                Kind::Network
            };
            LoginEnd::Unavailable(kind, error.to_string())
        }
    }
}

async fn science_flow(
    walk: &Walk<'_>,
    credentials: &SchoolCredentials,
    submitted: &AtomicBool,
) -> Result<(), Fail> {
    if credentials.username.is_empty() || credentials.password.is_empty() {
        return Err(Fail::Rejected(
            Kind::InvalidCredentials,
            "the Science Tokyo ID or password is missing",
        ));
    }
    // An empty OTP setup (a Connect before App OTP is set up) goes on: the
    // school's pages decide whether an emailed code or the App OTP is next.
    let lms = url(LMS_HOME)?;
    let mut page = walk.get(lms.clone(), None).await?;
    let mut credentials_sent = false;
    let mut otp_sent = false;
    for _ in 0..8 {
        maintenance(&page)?;
        match login::science(&page.html, page.url.as_str()) {
            SciencePage::Credentials(mut form) => {
                if credentials_sent {
                    // Back on the password form after sending it: the
                    // school turned it down. Never send it twice.
                    return Err(Fail::Rejected(
                        Kind::InvalidCredentials,
                        "the school did not accept the credentials",
                    ));
                }
                let action = extic_action(&form)?;
                credentials_sent = true;
                log::info!(target: "app_core::school_session", "step: Extic ID and password");
                form.set("identifier", &credentials.username);
                form.set("password", &credentials.password);
                submitted.store(true, Ordering::SeqCst);
                // Extic answers a refused password with 401, the same answer
                // a nameless ID gets: the credentials' verdict.
                let step = match walk.post(&action, form.fields(), &page.url, true).await {
                    Err(WalkError::Http(401)) => {
                        return Err(Fail::Rejected(
                            Kind::InvalidCredentials,
                            "the school did not accept the credentials",
                        ))
                    }
                    other => other?,
                };
                let referer = page.url.clone();
                page = walk.follow(step, &referer).await?;
                maintenance(&page)?;
                // A script-answered success names no next page; the second
                // factor's page is fixed.
                if matches!(
                    login::science(&page.html, page.url.as_str()),
                    SciencePage::Other
                ) {
                    page = walk
                        .get(second_factor(&page.url), Some(page.url.clone()))
                        .await?;
                }
            }
            SciencePage::Totp(mut form) => {
                // Back on the code form after sending one: the school turned
                // it down. A factor goes out once per sign-in.
                if otp_sent {
                    return Err(invalid_otp());
                }
                if credentials.otp_secret.is_empty() {
                    return Err(Fail::Rejected(
                        Kind::OtpNeeded,
                        "the school asks for the App one-time password and none was entered",
                    ));
                }
                let action = extic_action(&form)?;
                let (window, _) = fresh_otp(&credentials.otp_secret).await.map_err(|_| {
                    Fail::Rejected(Kind::InvalidOtp, "the saved OTP setup is not usable")
                })?;
                otp_sent = true;
                log::info!(target: "app_core::school_session", "step: Extic one-time password");
                form.set("totp", &window.code);
                // Extic answers a refused code with 401: the verdict.
                submitted.store(true, Ordering::SeqCst);
                let step = match walk.post(&action, form.fields(), &page.url, true).await {
                    Err(WalkError::Http(401)) => return Err(invalid_otp()),
                    other => other?,
                };
                let referer = page.url.clone();
                page = walk.follow(step, &referer).await?;
            }
            SciencePage::Saml(form) => {
                page = saml(walk, &form, &page).await?;
            }
            SciencePage::EmailOtp => {
                return Err(Fail::NeedsWeb(
                    Kind::EmailOtp,
                    "the school sends the one-time password by email",
                ))
            }
            SciencePage::Other => break,
        }
    }
    // The target is the only proof: nothing here knows what a success
    // response looks like.
    let page = arrive(walk, lms).await?;
    maintenance(&page)?;
    if page.url.host_str() != Some(Host::Moodle.name()) {
        return Err(Fail::NeedsWeb(
            Kind::UnknownPage,
            "the sign-in did not end on the LMS",
        ));
    }
    // Moodle shows the log-out action only to a signed-in user; a guest
    // session has a MoodleSession cookie too, so the page decides.
    if !page.html.contains("login/logout.php") {
        return Err(Fail::NeedsWeb(
            Kind::UnknownPage,
            "the LMS answered as a guest",
        ));
    }
    Ok(())
}

async fn tokyo_flow(
    walk: &Walk<'_>,
    jar: &Mutex<Jar>,
    credentials: &SchoolCredentials,
    submitted: &AtomicBool,
) -> Result<(), Fail> {
    if credentials.username.is_empty() || credentials.password.is_empty() {
        return Err(Fail::Rejected(
            Kind::InvalidCredentials,
            "the Tokyo Tech account or password is missing",
        ));
    }
    let mut page = walk.get(url(GETACCESS_START)?, None).await?;
    let mut password_sent = false;
    let mut matrix_sent = false;
    let mut method_sent = false;
    for _ in 0..8 {
        maintenance(&page)?;
        match login::tokyo(&page.html, page.url.as_str()) {
            TokyoPage::Agreement { password_url } => {
                if password_sent {
                    return Err(Fail::NeedsWeb(
                        Kind::UnknownPage,
                        "the portal showed the agreement again",
                    ));
                }
                log::info!(target: "app_core::school_session", "step: GetAccess agreement");
                let next = Url::parse(&password_url).map_err(|_| {
                    Fail::NeedsWeb(Kind::UnknownPage, "the agreement names no usable login")
                })?;
                if Host::of(&next) != Some(Host::GetAccess) {
                    return Err(Fail::NeedsWeb(
                        Kind::UnknownPage,
                        "the agreement points away from GetAccess",
                    ));
                }
                page = walk.get(next, Some(page.url.clone())).await?;
            }
            TokyoPage::Credentials(mut form) => {
                if password_sent {
                    return Err(Fail::Rejected(
                        Kind::InvalidCredentials,
                        "the school did not accept the credentials",
                    ));
                }
                let action = getaccess_action(&form)?;
                password_sent = true;
                log::info!(target: "app_core::school_session", "step: GetAccess account and password");
                // The portal rejects any lowercase letter in the name.
                form.set("usr_name", &credentials.username.to_ascii_uppercase());
                form.set("usr_password", &credentials.password);
                submitted.store(true, Ordering::SeqCst);
                page =
                    tokyo_step(walk, &form, &action, &page.url, Kind::InvalidCredentials).await?;
            }
            TokyoPage::Matrix(mut challenge) => {
                if matrix_sent {
                    return Err(invalid_matrix());
                }
                let action = getaccess_action(&challenge.form)?;
                matrix_sent = true;
                if !challenge.answer(&credentials.matrix) {
                    return Err(Fail::Rejected(
                        Kind::InvalidMatrix,
                        "the saved Matrix is not 7 rows of 10 letters",
                    ));
                }
                log::info!(
                    target: "app_core::school_session",
                    "step: Matrix challenge ({} cells)",
                    challenge.cells.len()
                );
                submitted.store(true, Ordering::SeqCst);
                page = tokyo_step(
                    walk,
                    &challenge.form,
                    &action,
                    &page.url,
                    Kind::InvalidMatrix,
                )
                .await?;
            }
            // A wrong Matrix answer is a 200 with the one-time password page
            // again, not an error page.
            TokyoPage::MethodPicker(_) | TokyoPage::NeedsOtp if matrix_sent => {
                return Err(invalid_matrix())
            }
            TokyoPage::MethodPicker(form) => {
                if method_sent {
                    return Err(Fail::NeedsWeb(
                        Kind::UnknownPage,
                        "the portal asked for the verification method again",
                    ));
                }
                let action = getaccess_action(&form)?;
                method_sent = true;
                log::info!(target: "app_core::school_session", "step: choose Matrix authentication");
                page =
                    tokyo_step(walk, &form, &action, &page.url, Kind::InvalidCredentials).await?;
            }
            TokyoPage::PasswordChange => {
                return Err(Fail::NeedsWeb(
                    Kind::PasswordChange,
                    "the portal asks for a new password",
                ))
            }
            TokyoPage::NeedsOtp => {
                return Err(Fail::NeedsWeb(
                    Kind::UnknownPage,
                    "the portal asks for a one-time password",
                ))
            }
            TokyoPage::Other => break,
        }
    }
    let entry = walk.get(url(KYOMU_ENTRY)?, None).await?;
    maintenance(&entry)?;
    if entry.url.host_str() != Some(Host::Kyomu.name()) {
        return Err(Fail::NeedsWeb(
            Kind::UnknownPage,
            "教務Web sent the sign-in back to the portal",
        ));
    }
    let target = adpt_isct::kyomu::page_url("profile")
        .and_then(|url| Url::parse(&url).ok())
        .ok_or(Fail::NeedsWeb(Kind::UnknownPage, "bad 教務Web address"))?;
    let page = walk.get(target, None).await?;
    maintenance(&page)?;
    let has_session = jar
        .lock()
        .map(|jar| jar.has(&page.url, "ASP.NET_SessionId"))
        .unwrap_or(false);
    if page.url.host_str() != Some(Host::Kyomu.name())
        || !has_session
        || page.html.contains("Template=userpass_key")
    {
        return Err(Fail::NeedsWeb(
            Kind::UnknownPage,
            "the sign-in did not end on 教務Web",
        ));
    }
    Ok(())
}

/// A maintenance page of the target site ends the sign-in with its notice,
/// whatever cookies it set: it proves nothing.
fn maintenance(page: &Page) -> Result<(), Fail> {
    match adpt_isct::maintenance_notice(&page.url, &page.html) {
        Some(notice) => Err(Fail::Maintenance(notice)),
        None => Ok(()),
    }
}

fn invalid_otp() -> Fail {
    Fail::Rejected(
        Kind::InvalidOtp,
        "the school did not accept the one-time password",
    )
}

fn invalid_matrix() -> Fail {
    Fail::Rejected(
        Kind::InvalidMatrix,
        "the school did not accept the Matrix answer",
    )
}

/// One GetAccess post: a rejection is a redirect to the portal's own top
/// page or a `Message=fail` answer, which name the step that failed.
async fn tokyo_step(
    walk: &Walk<'_>,
    form: &Form,
    action: &Url,
    referer: &Url,
    refused: Kind,
) -> Result<Page, Fail> {
    match walk.post(action, form.fields(), referer, false).await? {
        Step::Page(page) => Ok(page),
        Step::Redirect(to) if is_rejection(&to) => Err(Fail::Rejected(
            refused,
            "the school turned the sign-in down",
        )),
        Step::Redirect(to) => Ok(walk.get(to, Some(referer.clone())).await?),
    }
}

/// Posts a SAML hand-off, only to the LMS's assertion consumer.
pub(super) async fn saml(walk: &Walk<'_>, form: &Form, page: &Page) -> Result<Page, WalkError> {
    let action = Url::parse(&form.action)
        .ok()
        .filter(is_lms_acs)
        .ok_or(WalkError::Off(
            "a SAML form names an address that is not the LMS",
        ))?;
    log::info!(target: "app_core::school_session", "step: SAML hand-off");
    let step = walk.post(&action, form.fields(), &page.url, false).await?;
    walk.follow(step, &page.url).await
}

/// Opens `target` and carries it through any SAML hand-off form the
/// provider answers with: the return trip is the HTTP-POST binding, so the
/// last hop is a page to submit rather than a redirect.
async fn arrive(walk: &Walk<'_>, target: Url) -> Result<Page, WalkError> {
    let mut page = walk.get(target, None).await?;
    for _ in 0..4 {
        let SciencePage::Saml(form) = login::science(&page.html, page.url.as_str()) else {
            break;
        };
        page = saml(walk, &form, &page).await?;
    }
    Ok(page)
}

/// Extic takes the password and the code at its own `/auth/session` forms
/// only.
fn extic_action(form: &Form) -> Result<Url, Fail> {
    Url::parse(&form.action)
        .ok()
        .filter(|action| {
            Host::of(action) == Some(Host::ScienceAccount)
                && action.path().starts_with("/auth/session")
        })
        .ok_or(Fail::NeedsWeb(
            Kind::UnknownPage,
            "the sign-in form posts somewhere other than Extic",
        ))
}

/// GetAccess takes the password and the Matrix at its own hosts only.
fn getaccess_action(form: &Form) -> Result<Url, Fail> {
    Url::parse(&form.action)
        .ok()
        .filter(|action| {
            matches!(
                Host::of(action),
                Some(Host::GetAccess | Host::GetAccessRouter)
            )
        })
        .ok_or(Fail::NeedsWeb(
            Kind::UnknownPage,
            "the sign-in form posts somewhere other than GetAccess",
        ))
}

fn url(raw: &str) -> Result<Url, WalkError> {
    Url::parse(raw).map_err(|_| WalkError::Off("a fixed school address is invalid"))
}

fn second_factor(page: &Url) -> Url {
    let mut url = page.clone();
    url.set_path(EXTIC_SECOND_FACTOR);
    url.set_query(None);
    url.set_fragment(None);
    url
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// The code to send now: never one of a window with less than `OTP_MARGIN`
/// left (Extic's clock runs a little ahead); waits for the next window then.
pub(crate) async fn fresh_otp(secret: &str) -> Result<(OtpWindow, u64), SchoolAuthError> {
    let windows = upcoming_otps(secret)?;
    let period = match windows.as_slice() {
        [first, second, ..] => second.from.saturating_sub(first.from),
        _ => 30,
    };
    let now = unix_now();
    let window = windows.into_iter()
        .find(|window| window.from + period >= now + OTP_MARGIN.as_secs())
        .ok_or_else(|| SchoolAuthError::Storage("No usable one-time password window.".into()))?;
    if window.from > now {
        let wait = window.from - now + 1;
        log::info!(target: "app_core::school_session", "waiting {wait}s for a fresh one-time password window");
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
    }
    Ok((window, period))
}

#[cfg(test)]
#[path = "../tests/school_session__login.rs"]
mod tests;
