use super::*;
use crate::lpd::tests::{refusing, serve_one_job, student};
use std::net::TcpListener;
use std::thread;

fn options(copies: u32) -> PrintOptions {
    PrintOptions {
        copies,
        settings: pjl::Settings {
            sides: pjl::Sides::LongEdge,
            color: pjl::ColorMode::Mono,
        },
    }
}

// Each copy is its own upload of the document wrapped in the PJL header,
// billed to the student, so the printer never interprets a copy count.
#[test]
fn print_uploads_each_copy_with_the_pjl_header() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap();
    let handle = thread::spawn(move || [serve_one_job(&listener), serve_one_job(&listener)]);
    let options = options(2);
    assert_eq!(
        print_bytes(server, &student(), b"%PDF-1.7 body", "report.pdf", &options).unwrap(),
        2
    );
    let jobs = handle.join().unwrap();

    let mut expected = pjl::header(pjl::Language::Pdf, &options.settings);
    expected.extend_from_slice(b"%PDF-1.7 body");
    expected.extend_from_slice(pjl::JOB_END);
    for (lines, control, data) in &jobs {
        assert_eq!(lines[0], format!("\x02{QUEUE}\n"));
        assert!(control.contains("\nP00B12345\n"), "{control}");
        assert!(control.contains("\nNreport.pdf\n"), "{control}");
        assert_eq!(data, &expected);
    }
    assert_ne!(
        jobs[0].0[1], jobs[1].0[1],
        "each upload has its own job number"
    );
}

// Nothing is sent for a document the printer cannot render or a copy count
// outside the range.
#[test]
fn print_refuses_other_documents_and_copy_counts_before_connecting() {
    let (_held, server) = refusing();
    assert!(matches!(
        print_bytes(server, &student(), b"plain text", "notes.txt", &options(1)),
        Err(PrintError::NotPrintable)
    ));
    for copies in [0, MAX_COPIES + 1] {
        assert!(matches!(
            print_bytes(server, &student(), b"%PDF-1.4", "ok.pdf", &options(copies)),
            Err(PrintError::Copies)
        ));
    }
    assert!(matches!(
        print_bytes(server, &student(), b"%PDF-1.4", "ok.pdf", &options(1)),
        Err(PrintError::Lpd {
            accepted: 0,
            error: lpd::LpdError::Connect(_)
        })
    ));
}

// A copy the printer refuses ends the print, and the error counts the
// copies it had already accepted, so the message says what is queued.
#[test]
fn a_refused_copy_reports_the_copies_already_accepted() {
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
    let result = print_bytes(server, &student(), b"%PDF-1.7 body", "three", &options(3));
    handle.join().unwrap();
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
