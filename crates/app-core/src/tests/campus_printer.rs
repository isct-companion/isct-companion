use super::*;

// The status is one connect to the LPD port: a listener answers, a port
// nobody listens on does not, within the probe's bound.
#[tokio::test]
async fn the_probe_is_one_connect_to_the_lpd_port() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let open = listener.local_addr().unwrap();
    assert_eq!(probe_at(open, PROBE_TIMEOUT).await, PrinterReach::Reachable);

    // Bound, so no parallel test takes the port, and never listening.
    let held = tokio::net::TcpSocket::new_v4().unwrap();
    held.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let closed = held.local_addr().unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        probe_at(closed, Duration::from_millis(500)).await,
        PrinterReach::Unreachable
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "bounded by its timeout"
    );
}

// Only a verified account name in the student-ID shape bills jobs; every
// other end says why there is no ID, and the setting reads it in this shape.
#[test]
fn the_student_id_comes_only_from_a_verified_account_name() {
    assert_eq!(
        PrinterAccount::of(Ok("24m12345".into())),
        PrinterAccount::Ready {
            student_id: "24M12345".into()
        }
    );
    assert_eq!(
        PrinterAccount::of(Ok(" 23D00001 ".into())).student(),
        StudentId::parse("23D00001")
    );
    assert_eq!(
        PrinterAccount::of(Ok("staff.member".into())),
        PrinterAccount::NotStudentId
    );
    assert_eq!(
        PrinterAccount::of(Err(WebStop::NotConfigured)),
        PrinterAccount::NotConnected
    );
    for stop in [WebStop::Rejected, WebStop::NeedsWeb] {
        assert_eq!(
            PrinterAccount::of(Err(stop)),
            PrinterAccount::NeedsAttention
        );
    }
    for stop in [
        WebStop::Unreachable,
        WebStop::Storage,
        WebStop::Stale,
        WebStop::Unsupported,
    ] {
        let account = PrinterAccount::of(Err(stop));
        assert_eq!(account, PrinterAccount::Unverified);
        assert_eq!(account.student(), None);
    }
    assert_eq!(
        serde_json::to_value(CampusPrinterStatus {
            account: PrinterAccount::of(Ok("24B12345".into())),
            reach: PrinterReach::Unreachable,
        })
        .unwrap(),
        serde_json::json!({
            "account": { "kind": "ready", "studentId": "24B12345" },
            "reach": "unreachable"
        })
    );
}

// Every shell words a print's end from this one mapping: each key must have
// copy in every catalog, and each value it passes must have a placeholder
// there, or the person reads a raw key or `{{printed}}`. A count of zero
// accepted copies has its own wording.
#[test]
#[cfg(any(windows, target_os = "android"))]
fn every_outcome_message_has_copy_with_its_placeholders() {
    let catalogs = [
        include_str!("../../../../resources/locales/en.json"),
        include_str!("../../../../resources/locales/ja.json"),
        include_str!("../../../../resources/locales/zh.json"),
    ]
    .map(|raw| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(raw).unwrap());
    let accounts = [
        PrinterAccount::NotConnected,
        PrinterAccount::NotStudentId,
        PrinterAccount::NeedsAttention,
        PrinterAccount::Unverified,
    ];
    let mut outcomes: Vec<_> = accounts
        .into_iter()
        .map(|account| CampusPrintOutcome::Account { account })
        .collect();
    for printed in [0, 2] {
        outcomes.extend([
            CampusPrintOutcome::Unreachable { printed },
            CampusPrintOutcome::NotPrinted { printed },
            CampusPrintOutcome::Uncertain { printed },
        ]);
    }
    outcomes.extend([
        CampusPrintOutcome::Printed { copies: 2 },
        CampusPrintOutcome::NotPrintable,
        CampusPrintOutcome::InvalidCopies,
        CampusPrintOutcome::TooLarge,
        CampusPrintOutcome::Unreadable,
    ]);
    for outcome in &outcomes {
        let message = outcome.message();
        for catalog in &catalogs {
            let copy = catalog[message.key].as_str().unwrap();
            for name in message.args.keys() {
                assert!(copy.contains(&format!("{{{{{name}}}}}")), "{copy}");
            }
        }
    }
    let none = CampusPrintOutcome::Uncertain { printed: 0 }.message();
    let some = CampusPrintOutcome::Uncertain { printed: 2 }.message();
    assert_ne!(none.key, some.key);
    assert_eq!(some.args.get("printed"), Some(&2));
}

// Sends go one at a time: a send waits while another holds the turn, and
// then runs (here to a port that refuses, so it ends unreachable).
#[tokio::test]
#[cfg(any(windows, target_os = "android"))]
async fn a_send_waits_for_the_one_before_it() {
    let held = tokio::net::TcpSocket::new_v4().unwrap();
    held.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let refusing = held.local_addr().unwrap();
    let turn = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    let earlier = turn.clone().lock_owned().await;
    let send = tokio::spawn(send_in_turn(
        turn,
        refusing,
        StudentId::parse("24B12345").unwrap(),
        bytes::Bytes::from_static(b"%PDF-1.7 body"),
        "report".into(),
        PrintOptions {
            copies: 1,
            settings: CAPABILITIES.defaults,
        },
    ));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!send.is_finished(), "sent while another send held the turn");
    drop(earlier);
    assert_eq!(
        send.await.unwrap(),
        CampusPrintOutcome::Unreachable { printed: 0 }
    );
}
