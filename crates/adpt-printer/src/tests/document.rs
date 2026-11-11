use super::*;
use crate::lpd::tests::{refusing, serve_one_job, student};
use std::net::TcpListener;
use std::thread;

fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("adpt-printer-{}-{name}", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    path
}

// Each copy is its own upload of the file wrapped in the PJL header, billed
// to the student, so the printer never interprets a copy count.
#[test]
fn print_uploads_each_copy_with_the_pjl_header() {
    let path = temp_file("report.pdf", b"%PDF-1.7 body");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap();
    let removed = path.clone();
    let handle = thread::spawn(move || {
        let first = serve_one_job(&listener);
        // The file goes away after the first copy: the second is still the
        // same document, read once before anything was sent.
        std::fs::remove_file(&removed).unwrap();
        [first, serve_one_job(&listener)]
    });
    let options = PrintOptions {
        copies: 2,
        settings: pjl::Settings {
            duplex: Some(pjl::Duplex::LongEdge),
            color: Some(pjl::ColorMode::Mono),
        },
    };
    assert_eq!(print(server, &student(), &path, &options).unwrap(), 2);
    let jobs = handle.join().unwrap();

    let mut expected = pjl::header(pjl::Language::Pdf, &options.settings);
    expected.extend_from_slice(b"%PDF-1.7 body");
    expected.extend_from_slice(pjl::JOB_END);
    for (lines, control, data) in &jobs {
        assert_eq!(lines[0], format!("\x02{QUEUE}\n"));
        assert!(control.contains("\nP00B12345\n"), "{control}");
        assert!(control.contains("\nNadpt-printer-"), "{control}");
        assert_eq!(data, &expected);
    }
    assert_ne!(
        jobs[0].0[1], jobs[1].0[1],
        "each upload has its own job number"
    );
}

// Nothing is sent for a file the printer cannot render or a copy count
// outside the window's range.
#[test]
fn print_refuses_other_files_and_copy_counts_before_connecting() {
    let (_held, server) = refusing();
    let text = temp_file("notes.txt", b"plain text");
    let pdf = temp_file("ok.pdf", b"%PDF-1.4");
    let one = PrintOptions {
        copies: 1,
        settings: pjl::Settings::default(),
    };
    assert!(matches!(
        print(server, &student(), &text, &one),
        Err(PrintError::NotPrintable)
    ));
    for copies in [0, MAX_COPIES + 1] {
        assert!(matches!(
            print(server, &student(), &pdf, &PrintOptions { copies, ..one }),
            Err(PrintError::Copies)
        ));
    }
    assert!(matches!(
        print(server, &student(), &pdf, &one),
        Err(PrintError::Lpd {
            accepted: 0,
            error: lpd::LpdError::Connect(_)
        })
    ));
    let missing = std::env::temp_dir().join("adpt-printer-missing.pdf");
    assert!(matches!(
        print(server, &student(), &missing, &one),
        Err(PrintError::File(_))
    ));
    std::fs::remove_file(&text).unwrap();
    std::fs::remove_file(&pdf).unwrap();
}

// The print window sends camelCase options with kebab-case values.
#[test]
fn options_read_from_the_print_window() {
    let options: PrintOptions =
        serde_json::from_str(r#"{"copies":3,"duplex":"short-edge","color":"mono"}"#).unwrap();
    assert_eq!(
        options,
        PrintOptions {
            copies: 3,
            settings: pjl::Settings {
                duplex: Some(pjl::Duplex::ShortEdge),
                color: Some(pjl::ColorMode::Mono),
            },
        }
    );
    let defaults: PrintOptions =
        serde_json::from_str(r#"{"copies":1,"duplex":null,"color":null}"#).unwrap();
    assert_eq!(defaults.settings, pjl::Settings::default());
}

// A copy the printer refuses ends the print, and the error counts the
// copies it had already accepted, so a retry knows what is on paper.
#[test]
fn a_refused_copy_reports_the_copies_already_accepted() {
    let path = temp_file("three.pdf", b"%PDF-1.7 body");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        serve_one_job(&listener);
        let (mut conn, _) = listener.accept().unwrap();
        let mut line = Vec::new();
        std::io::BufRead::read_until(
            &mut std::io::BufReader::new(conn.try_clone().unwrap()),
            b'\n',
            &mut line,
        )
        .unwrap();
        std::io::Write::write_all(&mut conn, &[1]).unwrap();
    });
    let options = PrintOptions {
        copies: 3,
        settings: pjl::Settings::default(),
    };
    let result = print(server, &student(), &path, &options);
    handle.join().unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(
        matches!(
            result,
            Err(PrintError::Lpd {
                accepted: 1,
                error: lpd::LpdError::Rejected { .. }
            })
        ),
        "{result:?}"
    );
}

// A durable progress owner can stop after an acknowledged copy without
// uploading the remaining copies; otherwise a retry may duplicate them.
#[test]
fn progress_callbacks_stop_before_the_next_upload() {
    use std::cell::RefCell;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap();
    let handle = thread::spawn(move || serve_one_job(&listener));
    let events = RefCell::new(Vec::new());
    let result = print_bytes_progress(
        server,
        &student(),
        b"%PDF-1.7 body",
        "report",
        &PrintOptions {
            copies: 3,
            settings: pjl::Settings::default(),
        },
        |accepted| {
            events.borrow_mut().push(("before", accepted));
            if accepted == 1 {
                Err(PrintError::Interrupted)
            } else {
                Ok(())
            }
        },
        |accepted| {
            events.borrow_mut().push(("after", accepted));
            Ok(())
        },
    );
    handle.join().unwrap();
    assert!(matches!(result, Err(PrintError::Interrupted)));
    assert_eq!(
        *events.borrow(),
        [("before", 0), ("after", 1), ("before", 1)]
    );
}
