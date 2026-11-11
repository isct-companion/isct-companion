use super::*;

// A stale shell must never release a newer lease, and restart must never
// retry an upload whose acknowledgement was not durably recorded.
#[test]
fn reservation_and_recovery_preserve_print_safety() {
    let directory = std::env::temp_dir().join(format!(
        "isct-print-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let service = Service::new(directory.clone());
    let job = Job {
        id: "system-job".into(),
        title: "Test".into(),
        copies: 3,
        sides: Some(1),
        color: Some(false),
    };
    let first = match service.reserve(&job).unwrap() {
        Admission::Ready { lease } => lease,
        other => panic!("unexpected {other:?}"),
    };
    assert!(matches!(service.reserve(&job).unwrap(), Admission::Busy));
    service.release(&job.id, first);
    let second = match service.reserve(&job).unwrap() {
        Admission::Ready { lease } => lease,
        other => panic!("unexpected {other:?}"),
    };
    service.release(&job.id, first);
    assert!(matches!(service.reserve(&job).unwrap(), Admission::Busy));
    service.release(&job.id, second);
    let record = Record {
        job: job.clone(),
        digest: "a".repeat(64),
        student: "24B12345".into(),
        accepted: 1,
        in_flight: true,
    };
    service.save(&record).unwrap();
    drop(service);
    let restarted = Service::new(directory.clone());
    assert!(matches!(
        restarted.reserve(&job).unwrap(),
        Admission::Unknown { printed: 1 }
    ));
    assert!(matches!(
        restarted.forget_completed(&job.id),
        Err(Error::Conflict)
    ));
    for corrupt in [
        Record {
            accepted: 4,
            ..record.clone()
        },
        Record {
            accepted: 3,
            ..record.clone()
        },
        Record {
            digest: "bad".into(),
            ..record.clone()
        },
        Record {
            student: "bad".into(),
            ..record.clone()
        },
        Record {
            job: Job {
                sides: Some(3),
                ..job.clone()
            },
            ..record.clone()
        },
        Record {
            job: Job {
                copies: 0,
                ..job.clone()
            },
            ..record.clone()
        },
        Record {
            job: Job {
                id: "other".into(),
                ..job.clone()
            },
            ..record.clone()
        },
    ] {
        std::fs::write(
            restarted.path(&job.id),
            serde_json::to_vec(&corrupt).unwrap(),
        )
        .unwrap();
        assert!(matches!(restarted.reserve(&job), Err(Error::Storage)));
    }
    let mut completed = record;
    completed.in_flight = false;
    completed.accepted = job.copies;
    restarted.save(&completed).unwrap();
    assert!(matches!(
        restarted.reserve(&Job {
            copies: 2,
            ..job.clone()
        }),
        Err(Error::Conflict)
    ));
    restarted.forget_completed(&job.id).unwrap();
    assert!(restarted.load(&job.id).unwrap().is_none());
    std::fs::remove_dir_all(directory).unwrap();
}
