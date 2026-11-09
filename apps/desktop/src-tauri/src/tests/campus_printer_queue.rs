use super::*;

const SECOND: Duration = Duration::from_secs(1);

// The receiver answers every document as completed, so finished jobs only
// leave the Windows list, and nothing on the way to the receiver is touched.
#[test]
fn finished_and_in_flight_jobs_need_no_notification() {
    for finished in [JOB_STATUS_PRINTED, JOB_STATUS_COMPLETE, JOB_STATUS_DELETED] {
        assert_eq!(
            spool_job_action(finished, Duration::ZERO, false),
            SpoolAction::Remove
        );
    }
    for in_flight in [
        0,
        JOB_STATUS_SPOOLING,
        JOB_STATUS_PRINTING,
        JOB_STATUS_RETAINED,
    ] {
        assert_eq!(
            spool_job_action(in_flight, 600 * SECOND, false),
            SpoolAction::Keep
        );
    }
    assert_eq!(
        spool_job_action(JOB_STATUS_DELETING | JOB_STATUS_ERROR, 600 * SECOND, true),
        SpoolAction::Keep
    );
}

// A job printed while the app was closed fails in Windows. Once the
// receiver runs it is sent again before anyone is told; only a job still
// failing after the retry grace becomes a notification and is removed.
#[test]
fn a_failed_job_is_retried_once_before_it_is_reported() {
    let failed = JOB_STATUS_ERROR | JOB_STATUS_OFFLINE;
    assert_eq!(
        spool_job_action(failed, 600 * SECOND, false),
        SpoolAction::Restart
    );
    assert_eq!(
        spool_job_action(failed, 5 * SECOND, true),
        SpoolAction::Keep
    );
    assert_eq!(
        spool_job_action(failed, RETRY_GRACE, true),
        SpoolAction::Fail(failed)
    );
}

// A job the person paused is theirs; a job stuck in a state no rule names
// is reported only after its grace.
#[test]
fn paused_jobs_stay_and_unknown_states_wait_before_failing() {
    assert_eq!(
        spool_job_action(JOB_STATUS_PAUSED, 600 * SECOND, false),
        SpoolAction::Keep
    );
    let odd = JOB_STATUS_RETAINED | 0x0001_0000;
    assert_eq!(spool_job_action(odd, 5 * SECOND, false), SpoolAction::Keep);
    assert_eq!(
        spool_job_action(odd, UNKNOWN_JOB_GRACE, false),
        SpoolAction::Fail(odd)
    );
}

// New endpoints listen outside Windows' dynamic range, where boot-time
// exclusions land; repairs replace legacy tokens and ephemeral ports.
#[test]
fn endpoints_stay_out_of_the_dynamic_port_range() {
    let token = uuid::Uuid::new_v4();
    let ports: Vec<u16> = candidate_ports(token).collect();
    assert_eq!(ports.len(), usize::from(PORT_ATTEMPTS));
    assert!(ports.iter().all(|port| STABLE_PORTS.contains(port)));
    assert!(STABLE_PORTS.end <= 49_152);
    let stable = Endpoint {
        port: ports[0],
        token,
        private_token: true,
    };
    assert!(stable.stable());
    assert!(!Endpoint {
        port: 51_234,
        ..stable.clone()
    }
    .stable());
    assert!(!Endpoint {
        private_token: false,
        ..stable.clone()
    }
    .stable());
    // The queue's marker follows the URL, so a moved endpoint needs a repair.
    assert_ne!(
        stable.marker(),
        Endpoint {
            port: ports[1],
            ..stable.clone()
        }
        .marker()
    );
}
