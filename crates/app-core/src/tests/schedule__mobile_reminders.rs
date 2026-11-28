use super::*;
use serde_json::json;

const KEY: &str = "assignment:report:60";
const PRE_DUE: &str = "2026-04-13T09:58:00+09:00";
const DUE: &str = "2026-04-13T10:00:00+09:00";
const GRACE: &str = "2026-04-13T10:03:00+09:00";
const LATE: &str = "2026-04-13T10:06:00+09:00";

struct History {
    root: PathBuf,
    registered: PathBuf,
    fired: PathBuf,
}

impl History {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("isct-mobile-reminders-{}", uuid::Uuid::new_v4()));
        Self {
            registered: root.join(REGISTRATION_LOG),
            fired: root.join(REMINDER_LOG),
            root,
        }
    }

    fn plan(
        &self,
        now: &str,
        facts: ReminderPortFacts,
        items: Vec<NotificationItem>,
    ) -> Reconciliation {
        let history = DeliveryHistory::read(&self.registered, &self.fired).unwrap();
        plan_delivery(history, instant(now), facts, items, &BTreeSet::new()).unwrap()
    }

    fn reconcile(&self, now: &str, facts: ReminderPortFacts, workspace: &Value) -> Reconciliation {
        let history = DeliveryHistory::read(&self.registered, &self.fired).unwrap();
        reconcile_workspace(
            history,
            instant(now),
            facts,
            workspace,
            &SchoolReads::default(),
        )
        .unwrap()
    }

    fn confirm(&self, plan: &Reconciliation, now: &str, receipt: ReminderReceipt) -> Confirmation {
        confirm_delivery(
            &self.registered,
            &self.fired,
            instant(now).into(),
            plan,
            receipt,
        )
        .unwrap()
    }
}

impl Drop for History {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn instant(raw: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(raw).unwrap()
}

fn facts() -> ReminderPortFacts {
    ReminderPortFacts {
        permitted: true,
        available_slots: 64,
        can_update_elapsed_content: true,
        pending: vec![],
    }
}

fn pending(item: &NotificationItem) -> PendingReminder {
    PendingReminder {
        key: item.key.clone(),
        at: due_at(item).unwrap(),
        title: item.title.clone(),
        body: item.body.clone(),
        deep_link: item.deep_link.clone(),
        state: PendingReminderState::Pending,
    }
}

fn report(due: &str) -> Value {
    json!({
        "settings": {"notifications": {"assignments": true, "assignmentLeadMinutes": [60]}},
        "assignments": [{"id": "report", "title": "Report", "dueAt": due}],
    })
}

fn workspace() -> Value {
    report("2026-04-13T11:00:00+09:00")
}

#[test]
fn exact_and_missed_reminders_share_a_durable_grace() {
    for (now, expected) in [(DUE, true), (GRACE, true), (LATE, false)] {
        let history = History::new();
        let workspace = workspace();
        let plan = history.reconcile(now, facts(), &workspace);
        assert_eq!(
            plan.delivery
                .deliver_now
                .first()
                .map(|item| item.key.as_str()),
            expected.then_some(KEY)
        );
        assert!(history
            .reconcile(now, facts(), &workspace)
            .delivery
            .deliver_now
            .is_empty());
    }
}

#[test]
fn due_delivery_and_future_registration_share_the_same_business_key() {
    let history = History::new();
    let plan = history.reconcile(GRACE, facts(), &workspace());
    assert_eq!(plan.delivery.deliver_now[0].key, KEY);

    // Preserve the claim across a move beyond this week's registration
    // window, while an independent reminder lead can still be registered.
    let mut moved = report("2026-04-30T11:00:00+09:00");
    assert!(history
        .reconcile("2026-04-18T10:00:00+09:00", facts(), &moved)
        .delivery
        .register
        .is_empty());
    moved["settings"]["notifications"]["assignmentLeadMinutes"] = json!([60, 15]);
    let next = history.reconcile("2026-04-30T09:58:00+09:00", facts(), &moved);
    let keys: Vec<_> = next
        .delivery
        .register
        .iter()
        .map(|item| item.key.as_str())
        .collect();
    assert_eq!(keys, ["assignment:report:15"]);
}

#[test]
fn ownership_survives_an_interrupted_registration_until_the_os_confirms_failure() {
    let workspace = workspace();
    for confirmed_failure in [false, true] {
        let history = History::new();
        let plan = history.reconcile(PRE_DUE, facts(), &workspace);
        assert_eq!(plan.delivery.register[0].key, KEY);
        if confirmed_failure {
            assert!(
                history
                    .confirm(&plan, PRE_DUE, ReminderReceipt::default())
                    .retry
            );
        }
        // The receipt can be lost even when the OS already showed the
        // notification. An empty pending list cannot release ownership.
        let after = history.reconcile(GRACE, facts(), &workspace);
        assert_eq!(after.delivery.deliver_now.is_empty(), !confirmed_failure);
        assert!(after.delivery.cancel.is_empty());
    }

    let now = instant(PRE_DUE);
    let active = notification_plan(
        &workspace,
        &SchoolReads::default(),
        now,
        now + Duration::hours(1),
        None,
    )
    .unwrap();
    let history = History::new();
    let recovered = history.reconcile(
        PRE_DUE,
        ReminderPortFacts {
            pending: vec![pending(&active.items[0])],
            ..facts()
        },
        &workspace,
    );
    assert!(recovered.delivery.register.is_empty() && recovered.delivery.cancel.is_empty());
    let after = history.reconcile(GRACE, facts(), &workspace);
    assert!(after.delivery.deliver_now.is_empty());
}

#[test]
fn denying_permission_does_not_claim_due_items_or_register_future_items() {
    let history = History::new();
    let mut workspace = workspace();
    workspace["settings"]["notifications"]["assignmentLeadMinutes"] = json!([60, 30]);
    let blocked = ReminderPortFacts {
        permitted: false,
        ..facts()
    };
    let plan = history.reconcile(GRACE, blocked, &workspace);
    assert!(plan.delivery.permission_needed);
    assert!(plan.delivery.deliver_now.is_empty() && plan.delivery.register.is_empty());
    let allowed = history.reconcile(GRACE, facts(), &workspace);
    assert_eq!(allowed.delivery.deliver_now[0].key, KEY);
    assert_eq!(allowed.delivery.register[0].key, "assignment:report:30");
}

#[test]
fn full_queue_reuses_cancelled_slots_and_refills_at_its_next_wake() {
    let history = History::new();
    let due = instant(DUE);
    let mut items = notification_plan(&workspace(), &SchoolReads::default(), due, due, None)
        .unwrap()
        .items;
    for (key, at) in [
        ("middle", "2026-04-13T01:02:00Z"),
        ("late", "2026-04-12T21:05:00-04:00"),
    ] {
        let mut item = items[0].clone();
        item.key = key.into();
        item.at = at.into();
        items.push(item);
    }
    items.reverse();
    let plan = history.plan(
        PRE_DUE,
        ReminderPortFacts {
            available_slots: 1,
            pending: vec![pending(&items[0])],
            ..facts()
        },
        items.clone(),
    );
    assert_eq!(
        plan.delivery
            .register
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>(),
        [KEY, "middle"]
    );
    assert_eq!(plan.delivery.cancel, ["late"]);
    history.confirm(
        &plan,
        PRE_DUE,
        ReminderReceipt {
            registered: vec![KEY.into(), "middle".into()],
            cancelled: vec!["late".into()],
            ..ReminderReceipt::default()
        },
    );

    let wake = plan.refill_at.unwrap();
    assert_eq!(wake, instant(DUE).to_utc() + Duration::seconds(1));
    let still_full = history.plan(
        &wake.to_rfc3339(),
        ReminderPortFacts {
            available_slots: 0,
            pending: plan.delivery.register.iter().map(pending).collect(),
            ..facts()
        },
        items.clone(),
    );
    assert!(still_full.delivery.register.is_empty());
    // Android reports a freed slot when the deferred alarm actually leaves
    // the queue, before the fallback wake at the next scheduled request.
    let released = "2026-04-13T10:01:00+09:00";
    assert!(instant(released).to_utc() < still_full.refill_at.unwrap());
    let refill = history.plan(
        released,
        ReminderPortFacts {
            available_slots: 1,
            pending: vec![pending(&plan.delivery.register[1])],
            ..facts()
        },
        items,
    );
    assert!(refill.delivery.deliver_now.is_empty());
    assert_eq!(refill.delivery.register[0].key, "late");
    assert!(refill.refill_at.is_none());
}

#[test]
fn moved_reminder_catches_up_only_after_timely_confirmed_cancellation() {
    for (cancelled, receipt_time, replan, delivered) in [
        (false, GRACE, false, false),
        (true, GRACE, true, true),
        (true, LATE, true, false),
        (true, "2026-04-13T11:00:00+09:00", false, false),
    ] {
        let history = History::new();
        let original = report("2026-04-13T12:00:00+09:00");
        let installed = history.reconcile(PRE_DUE, facts(), &original);
        history.confirm(
            &installed,
            PRE_DUE,
            ReminderReceipt {
                registered: vec![KEY.into()],
                ..ReminderReceipt::default()
            },
        );
        let changed = workspace();
        let cancel = history.reconcile(
            "2026-04-13T10:02:00+09:00",
            ReminderPortFacts {
                pending: vec![pending(&installed.delivery.register[0])],
                ..facts()
            },
            &changed,
        );
        assert_eq!(cancel.delivery.cancel, [KEY]);
        assert!(cancel.delivery.deliver_now.is_empty());
        let confirmation = history.confirm(
            &cancel,
            receipt_time,
            ReminderReceipt {
                cancelled: if cancelled { vec![KEY.into()] } else { vec![] },
                ..ReminderReceipt::default()
            },
        );
        assert_eq!(confirmation.retry, !cancelled);
        assert_eq!(confirmation.replan, replan);
        let reconsidered = history.reconcile(receipt_time, facts(), &changed);
        assert_eq!(
            reconsidered
                .delivery
                .deliver_now
                .first()
                .map(|item| item.key.as_str()),
            delivered.then_some(KEY)
        );
    }
}

#[test]
fn same_instant_with_changed_payload_replaces_the_request() {
    let history = History::new();
    let mut workspace = crate::schedule::tests::workspace();
    let now = "2026-04-13T08:39:00+09:00";
    let original = history.reconcile(now, facts(), &workspace);
    let old = &original.delivery.register[0];
    let held = |request| ReminderPortFacts {
        available_slots: 0,
        pending: vec![request],
        ..facts()
    };
    assert!(history
        .reconcile(now, held(pending(old)), &workspace)
        .delivery
        .register
        .is_empty());

    workspace["courses"][0]["meetings"][0]["room"] = json!("M-202");
    let replacement = history.reconcile(now, held(pending(old)), &workspace);
    let new = &replacement.delivery.register[0];
    assert_eq!(new.key, old.key);
    assert_eq!(new.at, old.at);
    assert!(new.body.ends_with("M-202"));
    assert!(replacement.delivery.cancel.is_empty());

    for stale in [
        PendingReminder {
            title: "Previous title".into(),
            ..pending(new)
        },
        PendingReminder {
            deep_link: "/previous".into(),
            ..pending(new)
        },
    ] {
        assert_eq!(
            history
                .reconcile(now, held(stale), &workspace)
                .delivery
                .register[0]
                .key,
            new.key
        );
    }
    assert!(history
        .reconcile(now, held(pending(new)), &workspace)
        .delivery
        .register
        .is_empty());

    let immutable = history.reconcile(
        "2026-04-13T08:42:00+09:00",
        ReminderPortFacts {
            can_update_elapsed_content: false,
            ..held(pending(old))
        },
        &workspace,
    );
    assert!(immutable.delivery.update_pending.is_empty() && immutable.delivery.cancel.is_empty());

    for elapsed in ["2026-04-13T08:42:00+09:00", "2026-04-13T08:48:00+09:00"] {
        let refresh = history
            .reconcile(elapsed, held(pending(old)), &workspace)
            .delivery;
        assert_eq!(refresh.update_pending[0].key, old.key);
        assert!(refresh.update_pending[0].body.ends_with("M-202"));
        assert!(
            refresh.register.is_empty()
                && refresh.deliver_now.is_empty()
                && refresh.cancel.is_empty()
        );
    }
}

#[test]
fn overdue_pending_alarm_is_kept_only_while_its_original_business_plan_is_valid() {
    let history = History::new();
    let mut workspace = workspace();
    let original = history.reconcile(PRE_DUE, facts(), &workspace);
    let held = ReminderPortFacts {
        available_slots: 0,
        pending: vec![pending(&original.delivery.register[0])],
        ..facts()
    };
    let valid = history.reconcile(LATE, held.clone(), &workspace);
    assert!(valid.delivery.cancel.is_empty() && valid.delivery.deliver_now.is_empty());

    workspace["assignments"][0]["submission"] = json!("submitted");
    let submitted = history.reconcile(LATE, held.clone(), &workspace);
    assert_eq!(submitted.delivery.cancel, [KEY]);

    workspace["assignments"][0]
        .as_object_mut()
        .unwrap()
        .remove("submission");
    workspace["settings"]["notifications"]["assignments"] = json!(false);
    let disabled = history.reconcile(LATE, held.clone(), &workspace);
    assert_eq!(disabled.delivery.cancel, [KEY]);

    // An OS request may still have delivered. A move to the future must
    // cancel the elapsed alarm and preserve the business key's ownership.
    let moved = report("2026-04-13T11:30:00+09:00");
    let changed = history.reconcile(LATE, held, &moved);
    assert_eq!(changed.delivery.cancel, [KEY]);
    assert!(changed.delivery.register.is_empty());
}

#[test]
fn registration_history_is_kept_for_pending_alarms_and_open_work() {
    let key = "assignment:report:10080";
    let mut workspace = report("2026-04-20T10:00:00+09:00");
    workspace["settings"]["notifications"]["assignmentLeadMinutes"] = json!([10080]);
    let history = History::new();
    let original = history.reconcile(PRE_DUE, facts(), &workspace);
    assert_eq!(original.delivery.register[0].key, key);
    history.confirm(
        &original,
        PRE_DUE,
        ReminderReceipt {
            registered: vec![key.into()],
            ..ReminderReceipt::default()
        },
    );

    let old = "2026-04-17T10:00:00+09:00";
    let pending = ReminderPortFacts {
        available_slots: 0,
        pending: vec![pending(&original.delivery.register[0])],
        ..facts()
    };
    let valid = history.reconcile(old, pending.clone(), &workspace);
    assert!(valid.delivery.cancel.is_empty());
    // The request is gone and its trigger is four days old. The assignment
    // is still open when housekeeping runs, before the date changes.
    history.reconcile(old, facts(), &workspace);
    workspace["assignments"][0]["dueAt"] = json!("2026-04-30T10:00:00+09:00");
    let near_moved = history.reconcile("2026-04-23T09:58:00+09:00", facts(), &workspace);
    assert!(near_moved.delivery.register.is_empty());

    let ended = "2026-04-30T10:06:00+09:00";
    let cancel = history.reconcile(ended, pending, &workspace);
    assert_eq!(cancel.delivery.cancel, [key]);
    assert!(read_registrations(&history.registered)
        .unwrap()
        .contains_key(key));
    history.reconcile(ended, facts(), &workspace);
    assert!(read_registrations(&history.registered).unwrap().is_empty());
}

#[test]
fn a_discarded_alarm_is_rearmed_only_while_future() {
    let history = History::new();
    let workspace = workspace();
    let original = history.reconcile(PRE_DUE, facts(), &workspace);
    let discarded = ReminderPortFacts {
        available_slots: 1,
        pending: vec![PendingReminder {
            state: PendingReminderState::Missing,
            ..pending(&original.delivery.register[0])
        }],
        ..facts()
    };
    let plan = history.reconcile(PRE_DUE, discarded.clone(), &workspace);
    assert_eq!(plan.delivery.register[0].key, KEY);
    let plan = history.reconcile(GRACE, discarded, &workspace);
    assert!(plan.delivery.deliver_now.is_empty() && plan.delivery.register.is_empty());
    assert_eq!(plan.delivery.cancel, [KEY]);
}

#[test]
fn a_reported_post_failure_releases_the_claim_for_retry() {
    let history = History::new();
    let workspace = workspace();
    let plan = history.reconcile(GRACE, facts(), &workspace);
    assert_eq!(plan.delivery.deliver_now[0].key, KEY);
    assert!(
        history
            .confirm(
                &plan,
                GRACE,
                ReminderReceipt {
                    failed_deliveries: vec![KEY.into()],
                    ..ReminderReceipt::default()
                }
            )
            .retry
    );
    assert_eq!(
        history
            .reconcile(GRACE, facts(), &workspace)
            .delivery
            .deliver_now[0]
            .key,
        KEY
    );
}
