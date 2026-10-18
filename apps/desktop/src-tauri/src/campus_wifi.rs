//! ScienceTokyo campus Wi-Fi sign-in (macOS only; the module is compiled out
//! elsewhere).
//!
//! `ScienceTokyo` is WPA2-PSK with a passphrase the NOC publishes, followed by
//! a Cisco Meraki sign-on splash that lasts 12 hours. Science and engineering
//! members sign in with their 理工学系 Portal ID and password, the app's
//! `TokyoTech` profile. Sign-in runs only when the user asks; a failure is
//! reported and not retried.
//!
//! Until the splash is passed, macOS keeps Wi-Fi out of the default route and
//! system DNS, so requests are bound to the Wi-Fi interface and names are
//! resolved by hickory against the lease's DNS server, from the Wi-Fi address.

use hickory_resolver::{
    config::{NameServerConfig, ResolverConfig},
    name_server::TokioConnectionProvider,
    proto::xfer::Protocol,
    TokioResolver,
};
use isct_core::school_auth::{SchoolAuthProfile, SchoolAuthService, SchoolCredentials};
use objc2_core_wlan::CWWiFiClient;
use reqwest::Client;
use scraper::{Html, Selector};
use serde::Serialize;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use url::Url;

/// Published on https://www.noc.cii.isct.ac.jp/wireless/st-guide/, as is the
/// client range below.
const SSID: &str = "ScienceTokyo";
const PASSPHRASE: &str = "STWireless";
const CLIENT_PREFIX: [u8; 2] = [10, 2];
/// ScienceTokyoX (802.1X, NOC ID) client ranges, Ookayama/Tamachi then
/// Yokohama, published on
/// https://www.noc.cii.isct.ac.jp/service/nocid/sciencetokyox/. IPv4 is often
/// not leased at all (IPv6 only, macOS adds a 192.0.0.2 CLAT address), so the
/// IPv6 prefixes matter as much.
const X_CLIENT_NETS: [&str; 4] = [
    "10.3.0.0/17",
    "10.3.128.0/20",
    "2403:4440:40:61::/64",
    "2403:4440:50:63::/64",
];
const PROBE_URL: &str = "http://captive.apple.com/hotspot-detect.html";

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CampusWifiOutcome {
    SignedIn,
    /// The network was already open; nothing was submitted.
    AlreadyOnline,
    NeedsAccount,
    Rejected,
    /// On ScienceTokyoX and leaving it was not confirmed; nothing changed.
    ScienceTokyoX,
}

/// What the Wi-Fi interface is on right now, checked when the page opens.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CampusWifiStatus {
    /// ScienceTokyo with the splash already passed.
    SignedIn,
    /// ScienceTokyo, still behind the splash.
    SignInRequired,
    ScienceTokyoX,
    Other,
}

/// Sign-in stages, in order, reported as they start. A stage that is not
/// needed (already on ScienceTokyo) is skipped.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CampusWifiStep {
    Joining,
    OpeningPortal,
    SigningIn,
    Verifying,
}

/// Serialises clicks: one sign-in at a time.
#[derive(Default)]
pub struct CampusWifi(tokio::sync::Mutex<()>);

#[tauri::command]
pub async fn campus_wifi_status(webview: tauri::Webview) -> Result<CampusWifiStatus, String> {
    crate::require_local_main(&webview)?;
    let result = status().await;
    log::info!("[campus-wifi] status {result:?}");
    result
}

async fn status() -> Result<CampusWifiStatus, String> {
    let interface = wifi_interface().ok_or("No Wi-Fi interface found.")?;
    let Some(address) = campus_address(&interface) else {
        return Ok(if on_science_tokyo_x(&interface) {
            CampusWifiStatus::ScienceTokyoX
        } else {
            CampusWifiStatus::Other
        });
    };
    Ok(match probe(&client(&interface, address)?).await? {
        None => CampusWifiStatus::SignedIn,
        Some(_) => CampusWifiStatus::SignInRequired,
    })
}

/// Joins ScienceTokyo if needed, then signs in once. Leaving ScienceTokyoX
/// needs `leave_science_tokyo_x`, so the user can confirm it first.
#[tauri::command]
pub async fn campus_wifi_connect(
    webview: tauri::Webview,
    app: AppHandle,
    state: tauri::State<'_, CampusWifi>,
    leave_science_tokyo_x: bool,
    on_step: tauri::ipc::Channel<CampusWifiStep>,
) -> Result<CampusWifiOutcome, String> {
    crate::require_local_main(&webview)?;
    let _busy = state
        .0
        .try_lock()
        .map_err(|_| "A Wi-Fi sign-in is already running.".to_string())?;
    let step = |step| {
        let _ = on_step.send(step);
    };
    let result = connect(&app, leave_science_tokyo_x, step).await;
    log::info!("[campus-wifi] {result:?}");
    result
}

async fn connect(
    app: &AppHandle,
    leave_science_tokyo_x: bool,
    step: impl Fn(CampusWifiStep),
) -> Result<CampusWifiOutcome, String> {
    let interface = wifi_interface().ok_or("No Wi-Fi interface found.")?;
    let address = match campus_address(&interface) {
        Some(address) => address,
        None => {
            if !leave_science_tokyo_x && on_science_tokyo_x(&interface) {
                return Ok(CampusWifiOutcome::ScienceTokyoX);
            }
            step(CampusWifiStep::Joining);
            let name = interface.clone();
            tauri::async_runtime::spawn_blocking(move || join(&name))
                .await
                .map_err(|error| error.to_string())??;
            let started = Instant::now();
            loop {
                if let Some(address) = campus_address(&interface) {
                    break address;
                }
                if started.elapsed() > Duration::from_secs(30) {
                    return Err(format!("{SSID} did not assign an address."));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    };
    let client = client(&interface, address)?;
    step(CampusWifiStep::OpeningPortal);
    let Some((portal, html)) = probe(&client).await? else {
        return Ok(CampusWifiOutcome::AlreadyOnline);
    };
    step(CampusWifiStep::SigningIn);
    let handle = app.clone();
    let credentials = tauri::async_runtime::spawn_blocking(move || {
        handle
            .state::<std::sync::Arc<SchoolAuthService>>()
            .load(SchoolAuthProfile::TokyoTech)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    let Some(credentials) = credentials else {
        return Ok(CampusWifiOutcome::NeedsAccount);
    };
    if submit(&client, &portal, &html, &credentials).await? {
        return Ok(CampusWifiOutcome::Rejected);
    }
    // The access point applies the grant a moment after the portal answers;
    // errors meanwhile are transient.
    step(CampusWifiStep::Verifying);
    for _ in 0..6 {
        if let Ok(None) = probe(&client).await {
            return Ok(CampusWifiOutcome::SignedIn);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err("The portal answered but the network is still closed.".into())
}

/// `None` when online, otherwise the splash page the network redirects to,
/// with its HTML.
async fn probe(client: &Client) -> Result<Option<(Url, String)>, String> {
    let response = client
        .get(PROBE_URL)
        .send()
        .await
        .map_err(|error| format!("Wi-Fi check failed: {error}"))?;
    let url = response.url().clone();
    let body = response.text().await.unwrap_or_default();
    if is_splash(&url) {
        Ok(Some((url, body)))
    } else if body.contains("<TITLE>Success</TITLE>") {
        Ok(None)
    } else {
        Err("Redirected to an unknown sign-in page.".into())
    }
}

/// Meraki's hosted splash pages.
fn is_splash(url: &Url) -> bool {
    url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|host| host.ends_with(".network-auth.com"))
        && url.path().starts_with("/splash/")
}

/// Submits the sign-on form; `true` when the portal refused the account.
async fn submit(
    client: &Client,
    page: &Url,
    html: &str,
    credentials: &SchoolCredentials,
) -> Result<bool, String> {
    let (action, mut fields) = {
        let document = Html::parse_document(html);
        let select = |css: &str| Selector::parse(css).expect("static selector");
        // Meraki serves one template to every network; only the title names
        // ScienceTokyo, and it gates where the portal password may go.
        let title: String = document
            .select(&select("title"))
            .flat_map(|title| title.text())
            .collect();
        let form = document
            .select(&select("form#sign_in_form[method=post]"))
            .next()
            .filter(|form| {
                title.contains(SSID)
                    && form.select(&select("input[name=email]")).next().is_some()
                    && form
                        .select(&select("input[name=password]"))
                        .next()
                        .is_some()
            })
            .ok_or("The ScienceTokyo sign-in page has changed.")?;
        let action = form
            .value()
            .attr("action")
            .and_then(|action| page.join(action).ok())
            .filter(|action| is_splash(action) && action.host_str() == page.host_str())
            .ok_or("The ScienceTokyo sign-in form points elsewhere.")?;
        let hidden: Vec<(String, String)> = form
            .select(&select("input[type=hidden][name]"))
            .map(|input| {
                let value = input.value();
                let field = |name| value.attr(name).unwrap_or_default().to_string();
                (field("name"), field("value"))
            })
            .collect();
        (action, hidden)
    };
    fields.push(("email".into(), credentials.username.clone()));
    fields.push(("password".into(), credentials.password.clone()));
    // The redirect chain ends outside the splash once the grant is applied;
    // a failure there is left to the follow-up probe.
    let Ok(response) = client.post(action).form(&fields).send().await else {
        return Ok(false);
    };
    if !is_splash(response.url()) {
        return Ok(false);
    }
    // A refused login returns to the splash with its error box shown.
    let html = response.text().await.unwrap_or_default();
    let error = Selector::parse(".login-error").expect("static selector");
    Ok(Html::parse_document(&html)
        .select(&error)
        .any(|node| !node.value().classes().any(|class| class == "hidden")))
}

/// BSD name of the Wi-Fi interface, e.g. `en0`.
fn wifi_interface() -> Option<String> {
    // SAFETY: CoreWLAN getters without side effects; reading the interface
    // name needs no location permission (only SSID/BSSID do).
    unsafe {
        CWWiFiClient::sharedWiFiClient()
            .interface()?
            .interfaceName()
            .map(|name| name.to_string())
    }
}

fn addresses(interface: &str) -> Vec<IpAddr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry.name == interface)
        .map(|entry| entry.ip())
        .collect()
}

/// The interface's IPv4 address when it is a ScienceTokyo client address.
fn campus_address(interface: &str) -> Option<Ipv4Addr> {
    addresses(interface).into_iter().find_map(|ip| match ip {
        IpAddr::V4(ip) if ip.octets()[..2] == CLIENT_PREFIX => Some(ip),
        _ => None,
    })
}

/// Told apart by address, like ScienceTokyo: reading the SSID would need
/// location permission.
fn on_science_tokyo_x(interface: &str) -> bool {
    addresses(interface)
        .into_iter()
        .any(is_science_tokyo_x_address)
}

fn is_science_tokyo_x_address(ip: IpAddr) -> bool {
    X_CLIENT_NETS.iter().any(|net| {
        net.parse::<ipnet::IpNet>()
            .expect("static network")
            .contains(&ip)
    })
}

fn join(interface: &str) -> Result<(), String> {
    let output = Command::new("/usr/sbin/networksetup")
        .args(["-setairportnetwork", interface, SSID, PASSPHRASE])
        .output()
        .map_err(|error| format!("Could not run networksetup: {error}"))?;
    // networksetup reports most failures on stdout with exit status 0.
    let message = String::from_utf8_lossy(&output.stdout).trim().to_string();
    match (output.status.success(), message.is_empty()) {
        (true, true) => Ok(()),
        (_, false) => Err(message),
        (false, true) => Err(format!("Could not join {SSID}.")),
    }
}

/// HTTP over the Wi-Fi interface with a cookie store (the splash keeps its
/// session in cookies) and a resolver bound to the Wi-Fi address.
fn client(interface: &str, address: Ipv4Addr) -> Result<Client, String> {
    // The lease's DNS server, then a public one (the campus allows public DNS
    // before sign-in).
    let lease = Command::new("/usr/sbin/ipconfig")
        .args(["getoption", interface, "domain_name_server"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8_lossy(&output.stdout).trim().parse().ok());
    let mut config = ResolverConfig::new();
    for server in lease.into_iter().chain([Ipv4Addr::new(1, 1, 1, 1)]) {
        let mut name_server =
            NameServerConfig::new(SocketAddr::new(server.into(), 53), Protocol::Udp);
        name_server.bind_addr = Some(SocketAddr::new(address.into(), 0));
        config.add_name_server(name_server);
    }
    let resolver =
        TokioResolver::builder_with_config(config, TokioConnectionProvider::default()).build();
    Client::builder()
        .interface(interface)
        .dns_resolver(Arc::new(WifiResolver(resolver)))
        .cookie_store(true)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())
}

struct WifiResolver(TokioResolver);

impl reqwest::dns::Resolve for WifiResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let resolver = self.0.clone();
        Box::pin(async move {
            let found = resolver.ipv4_lookup(name.as_str()).await?;
            let addrs: reqwest::dns::Addrs = Box::new(
                found
                    .into_iter()
                    .map(|record| SocketAddr::new(IpAddr::V4(record.0), 0))
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            Ok(addrs)
        })
    }
}

#[cfg(test)]
#[path = "tests/campus_wifi.rs"]
mod tests;
