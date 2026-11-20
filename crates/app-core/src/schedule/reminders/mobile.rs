use super::*;

const REGISTRATION_LOG: &str = "reminders-registered.json";
const REGISTRATION_DAYS: i64 = 7;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingReminder {
    pub key: String,
    pub at: DateTime<Utc>,
    pub title: String,
    pub body: String,
    pub deep_link: String,
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
    /// Whether content can change while keeping the original OS trigger.
    pub can_update_elapsed_content: bool,
    pub pending: Vec<PendingReminder>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderDeliveryPlan {
    pub deliver_now: Vec<NotificationItem>,
    pub register: Vec<NotificationItem>,
    /// A past-due request still held by the OS can need fresher content.
    pub update_pending: Vec<NotificationItem>,
    pub cancel: Vec<String>,
    pub permission_needed: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderReceipt {
    /// A returned receipt accounts for every registration attempt. When no
    /// receipt arrives, any attempted registration can have reached the OS.
    pub registered: Vec<String>,
    pub updated_pending: Vec<String>,
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

type Registrations = BTreeMap<String, DateTime<Utc>>;

struct DeliveryHistory<'a> {
    registration_path: &'a Path,
    fired_path: &'a Path,
    registrations: Registrations,
    fired: BTreeMap<String, i64>,
}

impl<'a> DeliveryHistory<'a> {
    fn read(registration_path: &'a Path, fired_path: &'a Path) -> Result<Self, ReminderError> {
        Ok(Self {
            registration_path,
            fired_path,
            registrations: read_registrations(registration_path)?,
            fired: read_fired(fired_path)?,
        })
    }
}

struct Reconciliation {
    delivery: ReminderDeliveryPlan,
    refill_at: Option<DateTime<Utc>>,
    replan_after_cancel: BTreeSet<String>,
    prior_registrations: BTreeMap<String, Option<DateTime<Utc>>>,
}

struct Confirmation {
    retry: bool,
    replan: bool,
}

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
        let (confirmation, refill_at) = self.apply_mobile_round(&port).await?;
        let (confirmation, refill_at) = if confirmation.replan {
            // The old future request has been cancelled. Read again at the
            // actual time of the receipt, since grace can expire meanwhile.
            self.apply_mobile_round(&port).await?
        } else {
            (confirmation, refill_at)
        };
        if let Some(at) = refill_at {
            self.bring_next_forward(at);
        }
        Ok(confirmation.retry || confirmation.replan)
    }

    async fn apply_mobile_round(
        &self,
        port: &Arc<dyn ReminderPort>,
    ) -> Result<(Confirmation, Option<DateTime<Utc>>), ReminderError> {
        let facts = port.facts().await?;
        let service = self.clone();
        let plan = blocking(move || service.reconcile(Utc::now().fixed_offset(), facts)).await?;
        let receipt = port.apply(plan.delivery.clone()).await?;
        let receipt_at = Utc::now();
        let refill_at = plan.refill_at;
        let service = self.clone();
        let confirmation = blocking(move || service.confirm(&plan, receipt, receipt_at)).await?;
        Ok((confirmation, refill_at))
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
    ) -> Result<Reconciliation, ReminderError> {
        let fields =
            crate::workspace_changes::loaded_fields(super::super::REMINDER_FIELDS.iter().copied());
        let (workspace, school, _) = self.0.store.view(&fields)?;
        let registration_path = self.registration_path();
        let history = DeliveryHistory::read(&registration_path, &self.0.fired_path)?;
        reconcile_workspace(history, now, facts, &workspace, &school)
    }

    fn confirm(
        &self,
        plan: &Reconciliation,
        receipt: ReminderReceipt,
        now: DateTime<Utc>,
    ) -> Result<Confirmation, ReminderError> {
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

fn reconcile_workspace(
    history: DeliveryHistory<'_>,
    now: DateTime<FixedOffset>,
    facts: ReminderPortFacts,
    workspace: &Value,
    school: &SchoolReads,
) -> Result<Reconciliation, ReminderError> {
    let tracked = history
        .registrations
        .keys()
        .chain(history.fired.keys())
        .map(String::as_str)
        .chain(facts.pending.iter().map(|item| item.key.as_str()))
        .collect();
    let candidates = notification_plan_with_tracked(
        workspace,
        school,
        now - Duration::seconds(REMINDER_GRACE_SECONDS),
        now + Duration::days(REGISTRATION_DAYS),
        None,
        &tracked,
    )?;
    let open_assignments = array(workspace, "assignments")
        .iter()
        .filter(|row| {
            open_assignment(row)
                && matches!(
                    super::super::assignment_state(row, now),
                    super::super::AssignmentState::Open | super::super::AssignmentState::Overdue
                )
        })
        .filter_map(|row| text(row, "id"))
        .collect();
    plan_delivery(history, now, facts, candidates.items, &open_assignments)
}

fn plan_delivery(
    history: DeliveryHistory<'_>,
    now: DateTime<FixedOffset>,
    facts: ReminderPortFacts,
    candidates: Vec<NotificationItem>,
    open_assignments: &BTreeSet<&str>,
) -> Result<Reconciliation, ReminderError> {
    let now = now.to_utc();
    let cutoff = now - Duration::seconds(REMINDER_GRACE_SECONDS);
    let horizon = now + Duration::days(REGISTRATION_DAYS);
    let oldest = now - Duration::seconds(FIRED_TTL_SECONDS);
    let mut candidates = candidates
        .into_iter()
        .map(|item| Ok((due_at(&item)?, item)))
        .collect::<Result<Vec<_>, ReminderError>>()?;
    // RFC 3339 strings can have different UTC offsets for the same instant.
    candidates
        .sort_unstable_by(|(a, item_a), (b, item_b)| a.cmp(b).then(item_a.key.cmp(&item_b.key)));
    let mut keys = BTreeSet::new();
    candidates.retain(|(_, item)| keys.insert(item.key.clone()));
    let planned: BTreeMap<_, _> = candidates
        .iter()
        .map(|(at, item)| (item.key.as_str(), *at))
        .collect();
    let pending: BTreeMap<_, _> = facts
        .pending
        .iter()
        .map(|item| (item.key.as_str(), item))
        .collect();
    let holds_request = |request: &&PendingReminder| request.state != PendingReminderState::Missing;
    let in_flight = |key: &str| pending.get(key).is_some_and(holds_request);
    let still_relevant = |key: &str| {
        planned.get(key).is_some_and(|at| *at >= cutoff)
            || assignment_id(key).is_some_and(|id| open_assignments.contains(id))
    };
    let DeliveryHistory {
        registration_path,
        fired_path,
        registrations,
        fired,
    } = history;
    let mut updated_registrations = registrations.clone();
    let mut updated_fired = fired.clone();
    // Recover registrations that the platform accepted before Core received
    // the receipt. A new business time says nothing about an old request.
    for (&key, item) in &pending {
        updated_registrations.insert(key.to_owned(), item.at);
    }
    updated_fired.retain(|key, fired_at| {
        *fired_at >= oldest.timestamp_millis() || still_relevant(key) || in_flight(key)
    });

    // Valid elapsed requests still belong to the OS, even past grace.
    let elapsed_pending: BTreeSet<_> = pending
        .iter()
        .filter(|(_, request)| holds_request(request))
        .map(|(&key, _)| key)
        .filter(|key| {
            !updated_fired.contains_key(*key)
                && updated_registrations.get(*key).is_some_and(|at| {
                    *at <= now
                        && planned.get(key) == Some(at)
                        && (*at >= cutoff
                            || assignment_id(key).is_none_or(|id| open_assignments.contains(id)))
                })
        })
        .collect();
    let occupied = pending
        .values()
        .filter(|request| holds_request(request))
        .count();
    let budget = (facts.available_slots as usize).saturating_add(occupied - elapsed_pending.len());
    let mut plan = ReminderDeliveryPlan {
        deliver_now: Vec::new(),
        register: Vec::new(),
        update_pending: Vec::new(),
        cancel: Vec::new(),
        permission_needed: false,
    };
    let mut selected = BTreeSet::new();
    let mut prior_registrations = BTreeMap::new();
    let mut replan_after_cancel = BTreeSet::new();
    let mut first_selected = None;
    let mut overflow = None;
    for (at, item) in &candidates {
        if elapsed_pending.contains(item.key.as_str()) {
            if facts.can_update_elapsed_content && !same_content(pending[item.key.as_str()], item) {
                plan.update_pending.push(item.clone());
            }
            continue;
        }
        if !(cutoff..=horizon).contains(at) || updated_fired.contains_key(&item.key) {
            continue;
        }
        let previous = updated_registrations.get(&item.key).copied();
        if previous.is_some_and(|at| at <= now) {
            continue;
        }
        plan.permission_needed |= !facts.permitted;
        if *at <= now {
            if facts.permitted {
                if previous.is_some() {
                    replan_after_cancel.insert(item.key.clone());
                } else {
                    updated_fired.insert(item.key.clone(), now.timestamp_millis());
                    plan.deliver_now.push(item.clone());
                }
            }
        } else if facts.permitted && selected.len() == budget {
            overflow.get_or_insert(*at);
        } else {
            let current = previous == Some(*at);
            if facts.permitted || current {
                selected.insert(item.key.clone());
                first_selected.get_or_insert(*at);
            }
            let installed = current
                && pending.get(item.key.as_str()).is_some_and(|entry| {
                    entry.state == PendingReminderState::Pending && same_content(entry, item)
                });
            if facts.permitted && !installed {
                prior_registrations.insert(item.key.clone(), previous);
                // The process can exit after the OS accepts the request and
                // before we record its receipt. Until then either time can be
                // the one the OS delivered at.
                let earliest = previous.map_or(*at, |previous| previous.min(*at));
                updated_registrations.insert(item.key.clone(), earliest);
                plan.register.push(item.clone());
            }
        }
    }
    let registered_keys: BTreeSet<_> = registrations
        .keys()
        .map(String::as_str)
        .chain(pending.keys().copied())
        .collect();
    for key in registered_keys {
        if selected.contains(key) || elapsed_pending.contains(key) {
            continue;
        }
        if pending.contains_key(key) || updated_registrations.get(key).is_some_and(|at| *at > now) {
            plan.cancel.push(key.to_owned());
        }
    }
    updated_registrations.retain(|key, at| *at >= oldest || still_relevant(key) || in_flight(key));
    if updated_fired != fired {
        write_fired(fired_path, &updated_fired)?;
    }
    if updated_registrations != registrations {
        write_history(registration_path, &updated_registrations)?;
    }
    // Try refilling after the first selected request comes due. Elapsed
    // pending alarms never drive another wake on their own.
    let refill_at =
        overflow.map(|at| first_selected.map_or(at, |first| at.min(first + Duration::seconds(1))));
    Ok(Reconciliation {
        delivery: plan,
        refill_at,
        replan_after_cancel,
        prior_registrations,
    })
}

fn assignment_id(key: &str) -> Option<&str> {
    key.strip_prefix("assignment:")?
        .rsplit_once(':')
        .map(|(id, _)| id)
}

fn same_content(request: &PendingReminder, item: &NotificationItem) -> bool {
    request.title == item.title && request.body == item.body && request.deep_link == item.deep_link
}

fn confirm_delivery(
    registration_path: &Path,
    fired_path: &Path,
    now: DateTime<Utc>,
    reconciliation: &Reconciliation,
    receipt: ReminderReceipt,
) -> Result<Confirmation, ReminderError> {
    let plan = &reconciliation.delivery;
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
    let mut replan = false;
    for key in plan.cancel.iter().filter(|key| cancelled.contains(key)) {
        // An alarm can fire while the cancellation travels to the OS. Keep
        // ownership when the due instant passed before we got its receipt.
        if registrations.get(key).is_some_and(|at| *at > now) {
            registrations.remove(key);
            changed = true;
            replan |= reconciliation.replan_after_cancel.contains(key);
        }
    }
    let registered: BTreeSet<_> = receipt.registered.iter().collect();
    for item in &plan.register {
        let at = if registered.contains(&item.key) {
            Some(due_at(item)?)
        } else {
            reconciliation.prior_registrations[&item.key]
        };
        if let Some(at) = at {
            changed |= registrations.insert(item.key.clone(), at) != Some(at);
        } else {
            changed |= registrations.remove(&item.key).is_some();
        }
    }
    if changed {
        write_history(registration_path, &registrations)?;
    }
    let updated: BTreeSet<_> = receipt.updated_pending.iter().collect();
    Ok(Confirmation {
        retry: !receipt.failed_deliveries.is_empty()
            || plan
                .register
                .iter()
                .any(|item| !registered.contains(&item.key))
            || plan
                .update_pending
                .iter()
                .any(|item| !updated.contains(&item.key))
            || plan.cancel.iter().any(|key| !cancelled.contains(key)),
        replan,
    })
}

#[cfg(test)]
#[path = "../../tests/schedule__mobile_reminders.rs"]
mod tests;
