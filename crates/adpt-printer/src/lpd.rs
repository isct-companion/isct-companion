//! A minimal RFC 1179 (LPD) client. It sends one job per connection the way
//! the CUPS lpd backend does by default: the control file first, then the
//! data file, printed as-is (`l`), with the LPD user field (`P`) set to the
//! student ID the printer bills.

use crate::StudentId;
#[cfg(windows)]
use socket2::{Domain, Protocol, Socket, Type};
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
#[cfg(windows)]
use std::net::{Ipv4Addr, Ipv6Addr};
use std::net::{SocketAddr, TcpStream};
#[cfg(windows)]
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The source ports RFC 1179 section 3.1 reserves for clients.
#[cfg(windows)]
const RESERVED_TOP: u16 = 1023;
#[cfg(windows)]
const RESERVED_BOTTOM: u16 = 512;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Job<'a> {
    pub number: u16,
    pub host: &'a str,
    pub user: &'a StudentId,
    pub title: &'a str,
}

impl Job<'_> {
    /// The shared tail of the control and data file names, e.g. `A042myhost`.
    fn file_suffix(&self) -> String {
        format!("A{:03}{}", self.number % 1000, truncate(self.host, 15))
    }

    /// Field length limits are the ones in RFC 1179 section 7.2.
    pub fn control_file(&self) -> String {
        let suffix = self.file_suffix();
        format!(
            "H{}\nP{}\nJ{}\nldf{suffix}\nUdf{suffix}\nN{}\n",
            truncate(self.host, 31),
            truncate(self.user.as_str(), 31),
            truncate(self.title, 99),
            truncate(self.title, 131),
        )
    }
}

/// Why a job did not reach the printer.
#[derive(Debug)]
pub enum LpdError {
    /// No connection: the printer is off this network, or down.
    Connect(io::Error),
    /// The printer answered `step` with a non-zero code.
    Rejected { step: &'static str, code: u8 },
    /// The connection failed or closed during `step`.
    Transfer {
        step: &'static str,
        error: Option<io::Error>,
    },
    /// The data source yielded another length than announced.
    DataChanged { sent: u64, announced: u64 },
}

impl fmt::Display for LpdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(error) => write!(f, "cannot connect to the printer: {error}"),
            Self::Rejected { step, code } => {
                write!(f, "printer rejected {step} (reply code {code})")
            }
            Self::Transfer {
                step,
                error: Some(error),
            } => write!(f, "{step} failed: {error}"),
            Self::Transfer { step, error: None } => {
                write!(f, "printer closed the connection after {step}")
            }
            Self::DataChanged { sent, announced } => write!(
                f,
                "print data changed while sending: {sent} bytes instead of {announced}"
            ),
        }
    }
}

impl std::error::Error for LpdError {}

/// The step after the whole data file: its terminating byte and the answer.
const DATA_END: &str = "end of print data";

impl LpdError {
    /// The whole file reached the printer and only its answer is missing,
    /// so the job may have printed. Every other failure is a job the
    /// printer did not take: refused, or cut off before its data was all
    /// sent.
    pub fn may_have_printed(&self) -> bool {
        matches!(self, Self::Transfer { step: DATA_END, .. })
    }
}

/// Sends one job of `data_len` bytes from `data` to `queue` on `server`.
pub fn send(
    server: SocketAddr,
    queue: &str,
    job: &Job,
    data: &mut dyn Read,
    data_len: u64,
) -> Result<(), LpdError> {
    let mut stream = connect(server).map_err(LpdError::Connect)?;
    let mut replies = BufReader::new(stream.try_clone().map_err(LpdError::Connect)?);
    let suffix = job.file_suffix();
    let mut control = job.control_file().into_bytes();
    let control_len = control.len();
    control.push(0);

    let mut step = |bytes: &[u8], name| command(&mut stream, &mut replies, bytes, name);
    step(format!("\x02{queue}\n").as_bytes(), "receive job")?;
    step(
        format!("\x02{control_len} cf{suffix}\n").as_bytes(),
        "control file header",
    )?;
    step(&control, "control file")?;
    step(
        format!("\x03{data_len} df{suffix}\n").as_bytes(),
        "data file header",
    )?;

    let sent = io::copy(data, &mut stream).map_err(|error| LpdError::Transfer {
        step: "print data",
        error: Some(error),
    })?;
    if sent != data_len {
        return Err(LpdError::DataChanged {
            sent,
            announced: data_len,
        });
    }
    command(&mut stream, &mut replies, &[0], DATA_END)
}

fn command(
    stream: &mut TcpStream,
    replies: &mut BufReader<TcpStream>,
    bytes: &[u8],
    step: &'static str,
) -> Result<(), LpdError> {
    stream
        .write_all(bytes)
        .map_err(|error| LpdError::Transfer {
            step,
            error: Some(error),
        })?;
    let reply = replies.fill_buf().map_err(|error| LpdError::Transfer {
        step,
        error: Some(error),
    })?;
    match reply.first().copied() {
        Some(0) => {
            replies.consume(1);
            Ok(())
        }
        Some(code) => Err(LpdError::Rejected { step, code }),
        None => Err(LpdError::Transfer { step, error: None }),
    }
}

/// RFC 1179 expects clients to send from a port below 1024, as the CUPS
/// lpd backend does, taking the first free port counting down from 1023.
/// Windows lets any user bind these ports.
///
/// Each connection starts one port below the last one used: the previous
/// upload's port is still in TIME_WAIT for that printer, and reusing it does
/// not always fail at once, so a second copy must never try it first.
#[cfg(windows)]
fn connect(server: SocketAddr) -> io::Result<TcpStream> {
    static NEXT: AtomicU16 = AtomicU16::new(RESERVED_TOP);
    let start = NEXT.load(Ordering::Relaxed);
    let span = RESERVED_TOP - RESERVED_BOTTOM + 1;
    let taken = |e: &io::Error| {
        matches!(
            e.kind(),
            io::ErrorKind::AddrInUse | io::ErrorKind::AddrNotAvailable
        )
    };
    for offset in 0..span {
        let port = RESERVED_BOTTOM + (start - RESERVED_BOTTOM + span - offset) % span;
        let socket = new_socket(server)?;
        match socket.bind(&local_address(server, port).into()) {
            Ok(()) => {}
            Err(e) if taken(&e) => continue,
            Err(e) => return Err(e),
        }
        match socket.connect_timeout(&server.into(), CONNECT_TIMEOUT) {
            Ok(()) => {
                let below = if port == RESERVED_BOTTOM {
                    RESERVED_TOP
                } else {
                    port - 1
                };
                NEXT.store(below, Ordering::Relaxed);
                return into_stream(socket);
            }
            // The port may still be in TIME_WAIT from an earlier run.
            Err(e) if taken(&e) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "every source port below 1024 is in use",
    ))
}

/// Android reserves the ports below 1024 for root, so an app cannot send
/// from one as RFC 1179 asks; the system picks the source port, as it does
/// for the macOS queue's lpd backend when that runs without one reserved.
#[cfg(target_os = "android")]
fn connect(server: SocketAddr) -> io::Result<TcpStream> {
    with_timeouts(TcpStream::connect_timeout(&server, CONNECT_TIMEOUT)?)
}

#[cfg(windows)]
fn new_socket(server: SocketAddr) -> io::Result<Socket> {
    Socket::new(
        Domain::for_address(server),
        Type::STREAM,
        Some(Protocol::TCP),
    )
}

#[cfg(windows)]
fn local_address(server: SocketAddr, port: u16) -> SocketAddr {
    match server {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, port).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, port).into(),
    }
}

#[cfg(windows)]
fn into_stream(socket: Socket) -> io::Result<TcpStream> {
    with_timeouts(TcpStream::from(socket))
}

fn with_timeouts(stream: TcpStream) -> io::Result<TcpStream> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    Ok(stream)
}

/// LPD fields are single lines of ASCII, so anything else becomes `_`.
pub fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c == ' ' || c.is_ascii_graphic() {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn truncate(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// This machine's name for the control file's `H` line, never empty.
#[cfg(windows)]
pub fn local_hostname() -> String {
    let name: String = std::env::var("COMPUTERNAME")
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.')
        .collect();
    if name.is_empty() {
        "localhost".to_string()
    } else {
        name
    }
}

/// The control file's `H` line. An app cannot read the phone's name, and
/// the printer bills by the `P` line alone, so every phone sends this one.
#[cfg(target_os = "android")]
pub fn local_hostname() -> String {
    "android".to_string()
}

/// LPD job numbers have three digits. Start somewhere unlikely to repeat a
/// recent run and count up from there for each upload.
pub fn first_job_number() -> u16 {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_micros())
        .unwrap_or(0);
    (micros.wrapping_add(std::process::id()) % 1000) as u16
}

// The tests bind the reserved ports the Windows client sends from.
#[cfg(all(test, windows))]
#[path = "tests/lpd.rs"]
pub(crate) mod tests;
