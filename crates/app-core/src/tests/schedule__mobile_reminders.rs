use super::*;
use serde_json::json;

const KEY: &str = "assignment:report:60";

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
    ) -> ReminderDeliveryPlan {
        plan_delivery(&self.registered, &self.fired, instant(now), facts, items)
            .unwrap()
            .0
    }

    fn confirm(&self, plan: &ReminderDeliveryPlan, now: &str, receipt: ReminderReceipt) -> bool {
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
        pending: vec![],
    }
}

fn report() -> Vec<NotificationItem> {
    let workspace = json!({
        "settings": {"notifications": {"assignments": true, "assignmentLeadMinutes": [60]}},
        "assignments": [{"id": "report", "title": "Report", "dueAt": "2026-04-13T11:00:00+09:00"}],
    });
    let now = instant("2026-04-13T09:50:00+09:00");
    notification_plan(
        &workspace,
        &SchoolReads::default(),
        now,
        now + Duration::hours(1),
        None,
    )
    .unwrap()
    .items
}

#[test]
fn exact_and_missed_reminders_share_a_durable_five_minute_grace() {
    for (now, expected) in [
        ("2026-04-13T10:00:00+09:00", true),
        ("2026-04-13T10:03:00+09:00", true),
        ("2026-04-13T10:05:00+09:00", true),
        ("2026-04-13T10:06:00+09:00", false),
    ] {
        let history = History::new();
        let plan = history.plan(now, facts(), report());
        assert_eq!(
            plan.deliver_now.first().map(|item| item.key.as_str()),
            expected.then_some(KEY)
        );
        assert!(plan.register.is_empty());
        assert_eq!(
            read_fired(&history.fired).unwrap().contains_key(KEY),
            expected
        );
        assert!(history.plan(now, facts(), report()).deliver_now.is_empty());
    }
}

#[test]
fn only_confirmed_registrations_own_elapsed_reminders() {
    let before = "2026-04-13T09:58:00+09:00";
    let after = "2026-04-13T10:03:00+09:00";
    for confirmed in [true, false] {
        let history = History::new();
        let plan = history.plan(before, facts(), report());
        assert_eq!(plan.register[0].key, KEY);
        assert!(!read_registrations(&history.registered)
            .unwrap()
            .contains_key(KEY));
        let receipt = ReminderReceipt {
            registered: if confirmed { vec![KEY.into()] } else { vec![] },
            ..ReminderReceipt::default()
        };
        assert_eq!(history.confirm(&plan, before, receipt), !confirmed);

        // Once due, a missing pending request gives no information about
        // whether the OS already showed it.
        let plan = history.plan(after, facts(), report());
        assert_eq!(plan.deliver_now.is_empty(), confirmed);
        assert!(plan.cancel.is_empty());
    }
}

#[test]
fn denying_permission_does_not_claim_due_items_or_register_future_items() {
    let history = History::new();
    let mut items = report();
    let mut future = items[0].clone();
    future.key = "future".into();
    future.at = "2026-04-13T10:10:00+09:00".into();
    items.push(future);
    let blocked = ReminderPortFacts {
        permitted: false,
        ..facts()
    };
    let now = "2026-04-13T10:03:00+09:00";
    let plan = history.plan(now, blocked, items.clone());
    assert!(plan.permission_needed);
    assert!(plan.deliver_now.is_empty() && plan.register.is_empty());
    assert!(read_fired(&history.fired).unwrap().is_empty());
    let allowed = history.plan(now, facts(), items);
    assert_eq!(allowed.deliver_now[0].key, KEY);
    assert_eq!(allowed.register[0].key, "future");
}

#[test]
fn capacity_preserves_the_earliest_instants_and_reuses_cancelled_slots() {
    let history = History::new();
    let now = "2026-04-13T09:58:00+09:00";
    let mut items = report();
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
    let pending = PendingReminder {
        key: "late".into(),
        at: Some(instant("2026-04-13T01:05:00Z").into()),
        state: PendingReminderState::Pending,
    };
    let plan = history.plan(
        now,
        ReminderPortFacts {
            available_slots: 1,
            pending: vec![pending],
            ..facts()
        },
        items,
    );
    assert_eq!(
        plan.register
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>(),
        [KEY, "middle"]
    );
    assert_eq!(plan.cancel, ["late"]);
}

#[test]
fn confirmed_cancellation_of_a_removed_future_reminder_can_release_ownership() {
    let history = History::new();
    let before = "2026-04-13T09:58:00+09:00";
    let installed = history.plan(before, facts(), report());
    history.confirm(
        &installed,
        before,
        ReminderReceipt {
            registered: vec![KEY.into()],
            ..ReminderReceipt::default()
        },
    );
    let removed = history.plan(before, facts(), vec![]);
    assert_eq!(removed.cancel, [KEY]);
    assert!(history.confirm(&removed, before, ReminderReceipt::default()));
    assert!(read_registrations(&history.registered)
        .unwrap()
        .contains_key(KEY));
    assert!(!history.confirm(
        &removed,
        before,
        ReminderReceipt {
            cancelled: vec![KEY.into()],
            ..ReminderReceipt::default()
        }
    ));
    assert!(!read_registrations(&history.registered)
        .unwrap()
        .contains_key(KEY));
}

#[test]
fn a_discarded_alarm_is_rearmed_only_while_future() {
    let before = "2026-04-13T09:58:00+09:00";
    let after = "2026-04-13T10:03:00+09:00";
    let history = History::new();
    let pending = PendingReminder {
        key: KEY.into(),
        at: Some(instant("2026-04-13T10:00:00+09:00").into()),
        state: PendingReminderState::Pending,
    };
    let installed = history.plan(
        before,
        ReminderPortFacts {
            available_slots: 0,
            pending: vec![pending.clone()],
            ..facts()
        },
        report(),
    );
    assert!(installed.register.is_empty() && installed.cancel.is_empty());
    let discarded = ReminderPortFacts {
        available_slots: 1,
        pending: vec![PendingReminder {
            state: PendingReminderState::Missing,
            ..pending
        }],
        ..facts()
    };
    let plan = history.plan(before, discarded.clone(), report());
    assert_eq!(plan.register[0].key, KEY);
    let plan = history.plan(after, discarded, report());
    assert!(plan.deliver_now.is_empty() && plan.register.is_empty());
    assert_eq!(plan.cancel, [KEY]);
}

#[test]
fn legacy_pending_keys_are_adopted_even_without_a_due_instant() {
    let history = History::new();
    let now = "2026-04-13T10:03:00+09:00";
    assert!(!history.registered.exists());
    let plan = history.plan(
        now,
        ReminderPortFacts {
            available_slots: 63,
            pending: vec![PendingReminder {
                key: KEY.into(),
                at: None,
                state: PendingReminderState::Pending,
            }],
            ..facts()
        },
        report(),
    );
    assert!(plan.deliver_now.is_empty() && plan.cancel.is_empty());
    assert_eq!(
        read_registrations(&history.registered).unwrap()[KEY],
        Some(instant("2026-04-13T10:00:00+09:00").into())
    );
    assert!(history.plan(now, facts(), report()).deliver_now.is_empty());
    assert!(read_fired(&history.fired).unwrap().is_empty());
}

#[test]
fn a_reported_post_failure_releases_the_claim_for_retry() {
    let history = History::new();
    let now = "2026-04-13T10:03:00+09:00";
    let plan = history.plan(now, facts(), report());
    assert_eq!(plan.deliver_now[0].key, KEY);
    assert!(history.confirm(
        &plan,
        now,
        ReminderReceipt {
            failed_deliveries: vec![KEY.into()],
            ..ReminderReceipt::default()
        }
    ));
    assert_eq!(history.plan(now, facts(), report()).deliver_now[0].key, KEY);
}
