use std::collections::BTreeSet;

use super::*;

const REGISTRATION_LOG: &str = "reminders-registered.json";
#[cfg(feature = "mobile-reminders")]
const REGISTRATION_DAYS: i64 = 7;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingReminder {
    pub key: String,
    /// Legacy registrations may expose only the key.
    pub at: Option<DateTime<Utc>>,
    pub state: PendingReminderState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingReminderState {
    Pending,
    NeedsRefresh,
    /// A confirmed registration is still recorded on the device, but the
    /// OS discarded the request. It no longer uses a slot.
    Missing,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderPortFacts {
    pub permitted: bool,
    /// Free slots, excluding all requests already pending on the device.
    pub available_slots: u32,
    pub pending: Vec<PendingReminder>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderDeliveryPlan {
    pub deliver_now: Vec<NotificationItem>,
    pub register: Vec<NotificationItem>,
    pub cancel: Vec<String>,
    pub permission_needed: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderReceipt {
    pub registered: Vec<String>,
    pub cancelled: Vec<String>,
    /// Confirmed failures only. An interrupted callback can have posted a
    /// notification whose result Core never received.
    pub failed_deliveries: Vec<String>,
}

pub trait ReminderPort: Send + Sync {
    fn facts(
        &self,
    ) -> futures_util::future::BoxFuture<'_, Result<ReminderPortFacts, ReminderError>>;
    fn apply(
        &self,
        plan: ReminderDeliveryPlan,
    ) -> futures_util::future::BoxFuture<'_, Result<ReminderReceipt, ReminderError>>;
}

// An unknown instant is possible only when recovering a legacy pending
// request whose key the current workspace cannot resolve.
type Registrations = BTreeMap<String, Option<DateTime<Utc>>>;

#[cfg(feature = "mobile-reminders")]
impl ReminderService {
    pub(crate) fn mobile_port(&self) -> Option<Arc<dyn ReminderPort>> {
        self.0.mobile_port.lock().ok()?.clone()
    }

    pub(crate) fn attach_mobile(&self, port: Arc<dyn ReminderPort>) -> Result<(), ReminderError> {
        *self.0.mobile_port.lock().map_err(|_| stopped())? = Some(port);
        self.mobile_changed();
        Ok(())
    }

    pub(crate) fn mobile_changed(&self) -> bool {
        if self.mobile_port().is_some() {
            self.plan_next(Utc::now());
            return true;
        }
        false
    }

    pub(crate) async fn reconcile_mobile(
        &self,
        port: Arc<dyn ReminderPort>,
    ) -> Result<(), ReminderError> {
        let service = self.clone();
        self.0
            .tasks
            .run(async move {
                let _permit = service.0.pass.acquire().await.map_err(|_| stopped())?;
                // Replace the expired deadline before awaiting the OS. A workspace
                // change during this pass can bring the next one forward.
                service.plan_next(Utc::now() + Duration::hours(LOOKAHEAD_HOURS));
                let result = service.apply_mobile_pass(port).await;
                if !matches!(result, Ok(false)) {
                    service.bring_next_forward(Utc::now() + Duration::minutes(1));
                }
                result.map(|_| ())
            })
            .await?
    }

    async fn apply_mobile_pass(&self, port: Arc<dyn ReminderPort>) -> Result<bool, ReminderError> {
        let facts = port.facts().await?;
        let service = self.clone();
        let (plan, next) =
            blocking(move || service.reconcile(Utc::now().fixed_offset(), facts)).await?;
        if let Some(at) = next {
            self.bring_next_forward(at);
        }
        let receipt = port.apply(plan.clone()).await?;
        let service = self.clone();
        blocking(move || service.confirm(&plan, receipt, Utc::now())).await
    }

    fn bring_next_forward(&self, at: DateTime<Utc>) {
        if let Ok(mut next) = self.0.next.lock() {
            *next = Some(next.map_or(at, |next| next.min(at)));
        }
    }

    fn registration_path(&self) -> PathBuf {
        self.0.store.paths().local(REGISTRATION_LOG)
    }

    fn reconcile(
        &self,
        now: DateTime<FixedOffset>,
        facts: ReminderPortFacts,
    ) -> Result<(ReminderDeliveryPlan, Option<DateTime<Utc>>), ReminderError> {
        let fields = crate::workspace_changes::loaded_fields(
            super::super::WorkspaceQuery::NotificationPlan {
                now: String::new(),
                horizon: String::new(),
                locale: None,
            }
            .reads(),
        );
        let (workspace, school, _) = self.0.store.view(&fields)?;
        let candidates = notification_plan(
            &workspace,
            &school,
            now - Duration::seconds(REMINDER_GRACE_SECONDS),
            now + Duration::days(REGISTRATION_DAYS),
            None,
        )?;
        plan_delivery(
            &self.registration_path(),
            &self.0.fired_path,
            now,
            facts,
            candidates.items,
        )
    }

    fn confirm(
        &self,
        plan: &ReminderDeliveryPlan,
        receipt: ReminderReceipt,
        now: DateTime<Utc>,
    ) -> Result<bool, ReminderError> {
        confirm_delivery(
            &self.registration_path(),
            &self.0.fired_path,
            now,
            plan,
            receipt,
        )
    }
}

#[cfg(feature = "mobile-reminders")]
fn stopped() -> ReminderError {
    crate::runtime_tasks::RuntimeTaskError::Stopped.into()
}

#[cfg(feature = "mobile-reminders")]
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ReminderError> + Send + 'static,
) -> Result<T, ReminderError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(crate::runtime_tasks::RuntimeTaskError::from)?
}

fn read_registrations(path: &Path) -> Result<Registrations, ReminderError> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| ReminderError::Storage(error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(ReminderError::Storage(error.to_string())),
    }
}

fn due_at(item: &NotificationItem) -> Result<DateTime<Utc>, ReminderError> {
    DateTime::parse_from_rfc3339(&item.at)
        .map(|at| at.with_timezone(&Utc))
        .map_err(|error| ScheduleError::Compatibility(error.to_string()).into())
}

fn plan_delivery(
    registration_path: &Path,
    fired_path: &Path,
    now: DateTime<FixedOffset>,
    facts: ReminderPortFacts,
    candidates: Vec<NotificationItem>,
) -> Result<(ReminderDeliveryPlan, Option<DateTime<Utc>>), ReminderError> {
    let cutoff = now - Duration::seconds(REMINDER_GRACE_SECONDS);
    let mut candidates = candidates
        .into_iter()
        .map(|item| Ok((due_at(&item)?, item)))
        .collect::<Result<Vec<_>, ReminderError>>()?;
    // RFC 3339 strings can have different UTC offsets for the same instant.
    candidates
        .sort_unstable_by(|(a, item_a), (b, item_b)| a.cmp(b).then(item_a.key.cmp(&item_b.key)));
    let mut keys = BTreeSet::new();
    candidates.retain(|(at, item)| *at >= cutoff && keys.insert(item.key.clone()));
    let planned: BTreeMap<_, _> = candidates
        .iter()
        .map(|(at, item)| (item.key.as_str(), *at))
        .collect();
    let pending: BTreeMap<_, _> = facts
        .pending
        .iter()
        .map(|item| (item.key.as_str(), item))
        .collect();
    let mut registrations = read_registrations(registration_path)?;
    // Recover both pre-upgrade requests and registrations made just before
    // the app was stopped, before deciding which items Core may deliver.
    let mut recovered = false;
    for (&key, item) in &pending {
        let previous = registrations.get(key);
        let at = item
            .at
            .or(previous.copied().flatten())
            .or_else(|| planned.get(key).copied());
        if previous != Some(&at) {
            registrations.insert(key.to_owned(), at);
            recovered = true;
        }
    }
    if recovered {
        write_history(registration_path, &registrations)?;
    }

    // Leave due requests with the OS, including Android alarms still waiting
    // to fire after the grace window.
    let elapsed_pending: BTreeSet<_> = pending
        .iter()
        .filter(|(_, request)| request.state != PendingReminderState::Missing)
        .map(|(&key, _)| key)
        .filter(|key| {
            registrations
                .get(*key)
                .copied()
                .flatten()
                .is_some_and(|at| {
                    at <= now
                        && planned
                            .get(key)
                            .map_or(at < cutoff, |current| *current == at)
                })
        })
        .collect();
    let occupied = pending
        .values()
        .filter(|request| request.state != PendingReminderState::Missing)
        .count();
    let budget = (facts.available_slots as usize).saturating_add(occupied - elapsed_pending.len());
    let mut plan = ReminderDeliveryPlan {
        deliver_now: Vec::new(),
        register: Vec::new(),
        cancel: Vec::new(),
        permission_needed: !facts.permitted && !candidates.is_empty(),
    };
    let mut fired = read_fired(fired_path)?;
    let mut selected = BTreeSet::new();
    let mut first_selected = None;
    let mut overflow = None;
    for (at, item) in candidates {
        if at <= now {
            if facts.permitted
                && !registrations.contains_key(&item.key)
                && claim_fired_entry(&mut fired, &item.key, now)
            {
                plan.deliver_now.push(item);
            }
        } else if facts.permitted && selected.len() == budget {
            overflow.get_or_insert(at);
        } else {
            let current = registrations.get(&item.key) == Some(&Some(at));
            let installed = current
                && pending
                    .get(item.key.as_str())
                    .is_some_and(|entry| entry.state == PendingReminderState::Pending);
            if facts.permitted || current {
                selected.insert(item.key.clone());
                first_selected.get_or_insert(at);
            }
            if facts.permitted && !installed {
                plan.register.push(item);
            }
        }
    }
    for (key, at) in &registrations {
        if selected.contains(key) || elapsed_pending.contains(key.as_str()) {
            continue;
        }
        if pending.contains_key(key.as_str()) || at.is_some_and(|at| at > now) {
            plan.cancel.push(key.clone());
        }
    }
    if !plan.deliver_now.is_empty() {
        write_fired(fired_path, &fired)?;
    }
    // When the port is full, try refilling after the first selected request
    // comes due. Elapsed pending alarms never drive another wake on their own.
    let next =
        overflow.map(|at| first_selected.map_or(at, |first| at.min(first + Duration::seconds(1))));
    Ok((plan, next))
}

fn confirm_delivery(
    registration_path: &Path,
    fired_path: &Path,
    now: DateTime<Utc>,
    plan: &ReminderDeliveryPlan,
    receipt: ReminderReceipt,
) -> Result<bool, ReminderError> {
    if !receipt.failed_deliveries.is_empty() {
        let failed: BTreeSet<_> = receipt.failed_deliveries.iter().collect();
        let mut fired = read_fired(fired_path)?;
        for item in &plan.deliver_now {
            if failed.contains(&item.key) {
                fired.remove(&item.key);
            }
        }
        write_fired(fired_path, &fired)?;
    }

    let mut registrations = read_registrations(registration_path)?;
    let mut changed = false;
    let cancelled: BTreeSet<_> = receipt.cancelled.iter().collect();
    for key in plan.cancel.iter().filter(|key| cancelled.contains(key)) {
        // An alarm can fire while the cancellation travels to the OS. Keep
        // ownership when the due instant passed before we got its receipt.
        if registrations
            .get(key)
            .copied()
            .flatten()
            .is_some_and(|at| at > now)
        {
            registrations.remove(key);
            changed = true;
        }
    }
    let registered: BTreeSet<_> = receipt.registered.iter().collect();
    for item in plan
        .register
        .iter()
        .filter(|item| registered.contains(&item.key))
    {
        let at = Some(due_at(item)?);
        changed |= registrations.insert(item.key.clone(), at) != Some(at);
    }
    if changed {
        write_history(registration_path, &registrations)?;
    }
    Ok(!receipt.failed_deliveries.is_empty()
        || registered.len() < plan.register.len()
        || cancelled.len() < plan.cancel.len())
}

#[cfg(test)]
#[path = "../../tests/schedule__mobile_reminders.rs"]
mod tests;
