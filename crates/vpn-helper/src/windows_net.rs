//! The Windows route hook: what `assets/vpnc-script` does on macOS, through
//! IP Helper and the NRPT rather than a shell. openconnect only runs hooks
//! through cscript, so `assets/isct-vpn-script.js` is a shim that starts
//! `isct-vpn-helper.exe hook` with openconnect's variables.
//!
//! Only the campus networks go through the tunnel: 131.112.0.0/16 and NRPT
//! rules sending isct.ac.jp / titech.ac.jp lookups to the campus resolvers.
//! The default route is never touched, so Clash, Tailscale and other local
//! VPNs keep their paths.

use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::Command;
use windows::Win32::Foundation::{ERROR_OBJECT_ALREADY_EXISTS, NO_ERROR, WIN32_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    CreateIpForwardEntry2, CreateUnicastIpAddressEntry, DeleteIpForwardEntry2, FreeMibTable,
    GetBestRoute2, GetIpForwardTable2, GetIpInterfaceEntry, InitializeIpForwardEntry,
    InitializeIpInterfaceEntry, InitializeUnicastIpAddressEntry, SetIpInterfaceEntry,
    MIB_IPFORWARD_ROW2, MIB_IPFORWARD_TABLE2, MIB_IPINTERFACE_ROW, MIB_UNICASTIPADDRESS_ROW,
};
use windows::Win32::Networking::WinSock::{
    IpDadStatePreferred, IpPrefixOriginManual, IpSuffixOriginManual, AF_INET, IN_ADDR, IN_ADDR_0,
    MIB_IPPROTO_NETMGMT, SOCKADDR_IN, SOCKADDR_INET,
};

const CAMPUS: Ipv4Addr = Ipv4Addr::new(131, 112, 0, 0);
const CAMPUS_PREFIX: u8 = 16;
const RESOLVERS: [&str; 2] = ["131.112.125.58", "131.112.181.2"];
const DOMAINS: [&str; 2] = [".isct.ac.jp", ".titech.ac.jp"];
/// One local NRPT rule, under a key of its own so removing it touches
/// nothing else. The DNS client reads this key as rules change.
const NRPT_KEY: &str = r"SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsPolicyConfig\{6c1f3a52-9d4e-4b7a-8e21-15c0a7d9e3b4}";
/// The NRPT's "use these DNS servers" option.
const NRPT_GENERIC_DNS: u32 = 0x8;
/// Host routes the hook pinned to the physical path, one address a line.
const PINNED: &str = "pinned-routes";

/// What openconnect tells the hook about the tunnel it brought up.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Tunnel {
    pub(crate) index: u32,
    pub(crate) address: Ipv4Addr,
    pub(crate) mtu: Option<u32>,
    /// The VPN server, reached outside the tunnel. An IPv6 server needs no
    /// pin: only an IPv4 campus route could capture it.
    pub(crate) gateway: Option<Ipv4Addr>,
}

impl Tunnel {
    pub(crate) fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let index = get("TUNIDX")
            .and_then(|value| value.parse().ok())
            .ok_or("TUNIDX is missing")?;
        let address = get("INTERNAL_IP4_ADDRESS")
            .and_then(|value| value.parse().ok())
            .ok_or("INTERNAL_IP4_ADDRESS is missing")?;
        Ok(Self {
            index,
            address,
            mtu: get("INTERNAL_IP4_MTU").and_then(|value| value.parse().ok()),
            gateway: get("VPNGATEWAY").and_then(|value| value.parse().ok()),
        })
    }
}

/// openconnect's `reason`; exit code 1 is logged by openconnect and the
/// route check keeps the tunnel from counting as up.
pub(crate) fn hook(data: &Path) -> i32 {
    let get = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let reason = get("reason");
    let result = match reason.as_deref() {
        Some("connect") => Tunnel::from_env(get).and_then(|tunnel| connect(&tunnel, data)),
        Some("disconnect") => {
            clean_up(data, get("TUNIDX").and_then(|value| value.parse().ok()));
            Ok(())
        }
        // pre-init, reconnect, attempt-reconnect: nothing to change.
        _ => return 0,
    };
    let cfg = crate::platform::defaults();
    let reason = reason.unwrap_or_default();
    match result {
        Ok(()) => {
            crate::log(&cfg, &format!("route hook: {reason} done"));
            0
        }
        Err(error) => {
            crate::log(&cfg, &format!("route hook: {reason} failed: {error}"));
            1
        }
    }
}

fn connect(tunnel: &Tunnel, data: &Path) -> Result<(), String> {
    // Pin the server to the path it has now, before the campus route would
    // pull the tunnel's own packets into it.
    if let Some(gateway) = tunnel.gateway {
        pin(gateway, tunnel.index, data)?;
    }
    add_address(tunnel.index, tunnel.address)?;
    if let Some(mtu) = tunnel.mtu {
        set_mtu(tunnel.index, mtu)?;
    }
    add_route(CAMPUS, CAMPUS_PREFIX, tunnel.index, Ipv4Addr::UNSPECIFIED)?;
    set_resolvers()?;
    flush_dns();
    Ok(())
}

/// Removes the resolver rule and the pinned host routes, and the campus
/// route on `tunnel` if given. Run by openconnect's disconnect, and by the
/// helper after every stop and at start, so it must be safe to repeat.
pub(crate) fn clean_up(data: &Path, tunnel: Option<u32>) {
    let _ = windows_registry::LOCAL_MACHINE.remove_tree(NRPT_KEY);
    let pinned = data.join(PINNED);
    let hosts: Vec<Ipv4Addr> = fs::read_to_string(&pinned)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect();
    for row in routes() {
        let Some((destination, length)) = prefix(&row) else {
            continue;
        };
        let pinned_host = length == 32 && hosts.contains(&destination);
        let campus = Some(row.InterfaceIndex) == tunnel
            && (destination, length) == (CAMPUS, CAMPUS_PREFIX);
        if pinned_host || campus {
            let _ = unsafe { DeleteIpForwardEntry2(&row) };
        }
    }
    let _ = fs::remove_file(pinned);
    flush_dns();
}

/// Whether 131.112.0.0/16 goes through interface `index`.
pub(crate) fn campus_route_on(index: u32) -> bool {
    routes().iter().any(|row| {
        row.InterfaceIndex == index && prefix(row) == Some((CAMPUS, CAMPUS_PREFIX))
    })
}

fn pin(host: Ipv4Addr, tunnel: u32, data: &Path) -> Result<(), String> {
    let mut best = MIB_IPFORWARD_ROW2::default();
    let mut source = SOCKADDR_INET::default();
    check(
        "find the server's route",
        unsafe { GetBestRoute2(None, 0, None, &sockaddr(host), 0, &mut best, &mut source) },
    )?;
    if best.InterfaceIndex == tunnel {
        return Ok(());
    }
    let next_hop = ipv4(&best.NextHop).unwrap_or(Ipv4Addr::UNSPECIFIED);
    add_route(host, 32, best.InterfaceIndex, next_hop)?;
    fs::create_dir_all(data).map_err(|error| error.to_string())?;
    fs::write(data.join(PINNED), format!("{host}\n")).map_err(|error| error.to_string())
}

fn add_address(index: u32, address: Ipv4Addr) -> Result<(), String> {
    let mut row = MIB_UNICASTIPADDRESS_ROW::default();
    unsafe { InitializeUnicastIpAddressEntry(&mut row) };
    row.Address = sockaddr(address);
    row.InterfaceIndex = index;
    row.OnLinkPrefixLength = 32;
    row.PrefixOrigin = IpPrefixOriginManual;
    row.SuffixOrigin = IpSuffixOriginManual;
    row.DadState = IpDadStatePreferred;
    check("set the tunnel address", unsafe { CreateUnicastIpAddressEntry(&row) })
}

fn set_mtu(index: u32, mtu: u32) -> Result<(), String> {
    let mut row = MIB_IPINTERFACE_ROW::default();
    unsafe { InitializeIpInterfaceEntry(&mut row) };
    row.Family = AF_INET;
    row.InterfaceIndex = index;
    check("read the tunnel interface", unsafe { GetIpInterfaceEntry(&mut row) })?;
    row.NlMtu = mtu;
    // SetIpInterfaceEntry refuses an IPv4 row that carries a site prefix.
    row.SitePrefixLength = 0;
    check("set the tunnel MTU", unsafe { SetIpInterfaceEntry(&mut row) })
}

fn add_route(destination: Ipv4Addr, length: u8, index: u32, next_hop: Ipv4Addr) -> Result<(), String> {
    let mut row = MIB_IPFORWARD_ROW2::default();
    unsafe { InitializeIpForwardEntry(&mut row) };
    row.InterfaceIndex = index;
    row.DestinationPrefix.Prefix = sockaddr(destination);
    row.DestinationPrefix.PrefixLength = length;
    row.NextHop = sockaddr(next_hop);
    row.Metric = 0;
    row.Protocol = MIB_IPPROTO_NETMGMT;
    check(
        &format!("add the route to {destination}/{length}"),
        unsafe { CreateIpForwardEntry2(&row) },
    )
}

fn set_resolvers() -> Result<(), String> {
    let text = |error: windows_registry::Result<()>| error.map_err(|error| format!("NRPT rule: {error}"));
    let key = windows_registry::LOCAL_MACHINE
        .create(NRPT_KEY)
        .map_err(|error| format!("NRPT rule: {error}"))?;
    text(key.set_multi_string("Name", &DOMAINS))?;
    text(key.set_string("GenericDNSServers", RESOLVERS.join(";")))?;
    text(key.set_u32("ConfigOptions", NRPT_GENERIC_DNS))?;
    text(key.set_u32("Version", 2))?;
    text(key.set_string("IPSECCARestriction", ""))?;
    text(key.set_string("Comment", "IsctCompanion campus VPN"))?;
    text(key.set_string("DisplayName", "IsctCompanion campus VPN"))
}

/// Names looked up before the rule changed would keep their old answers.
fn flush_dns() {
    use std::os::windows::process::CommandExt;
    let _ = Command::new("ipconfig")
        .arg("/flushdns")
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output();
}

fn routes() -> Vec<MIB_IPFORWARD_ROW2> {
    let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
    if unsafe { GetIpForwardTable2(AF_INET, &mut table) } != NO_ERROR || table.is_null() {
        return Vec::new();
    }
    let rows = unsafe {
        std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize).to_vec()
    };
    unsafe { FreeMibTable(table as *const core::ffi::c_void) };
    rows
}

fn prefix(row: &MIB_IPFORWARD_ROW2) -> Option<(Ipv4Addr, u8)> {
    Some((
        ipv4(&row.DestinationPrefix.Prefix)?,
        row.DestinationPrefix.PrefixLength,
    ))
}

fn sockaddr(address: Ipv4Addr) -> SOCKADDR_INET {
    SOCKADDR_INET {
        Ipv4: SOCKADDR_IN {
            sin_family: AF_INET,
            sin_port: 0,
            sin_addr: IN_ADDR {
                S_un: IN_ADDR_0 {
                    S_addr: u32::from_ne_bytes(address.octets()),
                },
            },
            sin_zero: [0; 8],
        },
    }
}

fn ipv4(address: &SOCKADDR_INET) -> Option<Ipv4Addr> {
    unsafe {
        (address.si_family == AF_INET)
            .then(|| Ipv4Addr::from(address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes()))
    }
}

/// An entry that already exists is what the hook wanted.
fn check(what: &str, result: WIN32_ERROR) -> Result<(), String> {
    match result {
        NO_ERROR | ERROR_OBJECT_ALREADY_EXISTS => Ok(()),
        error => Err(format!("{what}: {}", windows::core::Error::from(error.to_hresult()))),
    }
}

#[cfg(test)]
#[path = "tests/windows_net.rs"]
mod tests;
