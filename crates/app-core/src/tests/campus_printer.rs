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
