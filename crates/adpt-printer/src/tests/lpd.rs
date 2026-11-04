use super::*;
use std::net::TcpListener;
use std::thread;

/// An address that refuses connections: bound, so no parallel test takes
/// the port, and never listening. (Linux refuses at once; macOS takes about
/// eight seconds to give up on it.)
pub(crate) fn refusing() -> (Socket, SocketAddr) {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    socket
        .bind(&SocketAddr::from(([127, 0, 0, 1], 0)).into())
        .unwrap();
    let address = socket.local_addr().unwrap().as_socket().unwrap();
    (socket, address)
}

pub(crate) fn student() -> StudentId {
    StudentId::parse("00B12345").unwrap()
}

#[test]
fn control_file_sets_user_and_prints_data_file_as_is() {
    let user = student();
    let job = Job {
        number: 42,
        host: "myhost",
        user: &user,
        title: "report.pdf",
    };
    assert_eq!(
        job.control_file(),
        "Hmyhost\nP00B12345\nJreport.pdf\nldfA042myhost\nUdfA042myhost\nNreport.pdf\n"
    );
}

#[test]
fn control_file_truncates_long_fields() {
    let long_host = "h".repeat(40);
    let long_title = "t".repeat(200);
    let user = student();
    let job = Job {
        number: 7,
        host: &long_host,
        user: &user,
        title: &long_title,
    };
    let control = job.control_file();
    let lines: Vec<&str> = control.lines().collect();
    assert_eq!(lines[0].len(), 1 + 31);
    assert_eq!(lines[2].len(), 1 + 99);
    assert_eq!(lines[3], format!("ldfA007{}", "h".repeat(15)));
    assert_eq!(lines[5].len(), 1 + 131);
}

#[test]
fn sanitize_replaces_non_ascii_and_control_characters() {
    assert_eq!(sanitize("課題 3.pdf"), "__ 3.pdf");
    assert_eq!(sanitize("a\nb\tc"), "a_b_c");
}

#[test]
fn local_hostname_is_never_empty_and_job_numbers_have_three_digits() {
    let name = local_hostname();
    assert!(!name.is_empty());
    assert!(name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'));
    assert!(first_job_number() < 1000);
}

fn read_line(reader: &mut BufReader<TcpStream>) -> String {
    let mut line = Vec::new();
    reader.read_until(b'\n', &mut line).unwrap();
    String::from_utf8(line).unwrap()
}

/// Reads the file announced by a `\x02<len> cf...` or `\x03<len> df...`
/// subcommand, including its trailing NUL.
fn read_file(reader: &mut BufReader<TcpStream>, header: &str) -> Vec<u8> {
    let len: usize = header[1..].split(' ').next().unwrap().parse().unwrap();
    let mut bytes = vec![0; len + 1];
    reader.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes.pop(), Some(0));
    bytes
}

/// Plays the LPD server for one job and returns the subcommand lines, the
/// control file and the data file it received.
pub(crate) fn serve_one_job(listener: &TcpListener) -> (Vec<String>, String, Vec<u8>) {
    let (mut conn, _) = listener.accept().unwrap();
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    let mut lines = Vec::new();

    lines.push(read_line(&mut reader));
    conn.write_all(&[0]).unwrap();
    lines.push(read_line(&mut reader));
    conn.write_all(&[0]).unwrap();
    let control = String::from_utf8(read_file(&mut reader, &lines[1])).unwrap();
    conn.write_all(&[0]).unwrap();
    lines.push(read_line(&mut reader));
    conn.write_all(&[0]).unwrap();
    let data = read_file(&mut reader, &lines[2]);
    conn.write_all(&[0]).unwrap();
    (lines, control, data)
}

#[test]
fn send_transfers_control_file_then_data_file() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap();
    let handle = thread::spawn(move || serve_one_job(&listener));

    let user = student();
    let job = Job {
        number: 42,
        host: "myhost",
        user: &user,
        title: "report.pdf",
    };
    let data = b"%PDF-1.7 test data";
    send(server, "kyocera", &job, &mut &data[..], data.len() as u64).unwrap();

    let (lines, control, received) = handle.join().unwrap();
    assert_eq!(
        lines,
        vec![
            "\x02kyocera\n".to_string(),
            format!("\x02{} cfA042myhost\n", control.len()),
            format!("\x03{} dfA042myhost\n", data.len()),
        ]
    );
    assert_eq!(control, job.control_file());
    assert_eq!(received, data);
}

#[test]
fn send_reports_a_rejected_queue_and_a_closed_port() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut line = Vec::new();
        BufReader::new(conn.try_clone().unwrap())
            .read_until(b'\n', &mut line)
            .unwrap();
        conn.write_all(&[1]).unwrap();
    });
    let user = student();
    let job = Job {
        number: 1,
        host: "h",
        user: &user,
        title: "t",
    };
    let err = send(server, "nosuchqueue", &job, &mut &b"x"[..], 1).unwrap_err();
    handle.join().unwrap();
    assert!(!err.may_have_printed(), "a refused job is not printed");
    assert!(
        matches!(
            err,
            LpdError::Rejected {
                step: "receive job",
                code: 1
            }
        ),
        "{err}"
    );

    // Nobody listens: the printer is off this network.
    let (_held, server) = refusing();
    let err = send(server, "kyocera", &job, &mut &b"x"[..], 1).unwrap_err();
    assert!(matches!(err, LpdError::Connect(_)), "{err}");
}

#[test]
fn send_rejects_data_shorter_than_announced() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(conn.try_clone().unwrap());
        read_line(&mut reader);
        conn.write_all(&[0]).unwrap();
        let header = read_line(&mut reader);
        conn.write_all(&[0]).unwrap();
        read_file(&mut reader, &header);
        conn.write_all(&[0]).unwrap();
        read_line(&mut reader);
        conn.write_all(&[0]).unwrap();
        // The client gives up after the short data and closes.
        let mut rest = Vec::new();
        let _ = reader.read_to_end(&mut rest);
    });
    let user = student();
    let job = Job {
        number: 1,
        host: "h",
        user: &user,
        title: "t",
    };
    let err = send(server, "kyocera", &job, &mut &b"short"[..], 100).unwrap_err();
    handle.join().unwrap();
    assert!(!err.may_have_printed());
    assert!(
        matches!(
            err,
            LpdError::DataChanged {
                sent: 5,
                announced: 100
            }
        ),
        "{err}"
    );
}

// The whole file went out and the printer closed without answering: the
// job may have printed, so the person must check before sending it again.
// Data cut short never ends the job, so that one did not print.
#[test]
fn a_job_without_the_final_answer_may_have_printed() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(conn.try_clone().unwrap());
        read_line(&mut reader);
        conn.write_all(&[0]).unwrap();
        let header = read_line(&mut reader);
        conn.write_all(&[0]).unwrap();
        read_file(&mut reader, &header);
        conn.write_all(&[0]).unwrap();
        let header = read_line(&mut reader);
        conn.write_all(&[0]).unwrap();
        read_file(&mut reader, &header);
        // No answer to the data file: the connection just closes.
    });
    let user = student();
    let job = Job {
        number: 1,
        host: "h",
        user: &user,
        title: "t",
    };
    let err = send(server, "kyocera", &job, &mut &b"data"[..], 4).unwrap_err();
    handle.join().unwrap();
    assert!(err.may_have_printed(), "{err}");
    assert!(!LpdError::DataChanged {
        sent: 1,
        announced: 2
    }
    .may_have_printed());
}
