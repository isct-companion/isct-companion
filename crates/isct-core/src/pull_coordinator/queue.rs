use super::*;

pub(super) fn promote(job: &mut PullJob, options: &PullOptions) {
    if options.priority == PullPriority::Manual {
        job.priority = PullPriority::Manual;
        job.priority_cell
            .store(PullPriority::Manual.encoded(), Ordering::Release);
    }
    job.retry_limit = job.retry_limit.max(options.retry_limit);
    if options.login_first {
        job.login_first.store(true, Ordering::Release);
    }
}

pub(super) fn covering_job(state: &CoordinatorState, target: &PullTarget) -> Option<String> {
    if let Some(id) = state.keys.get(target) {
        if state.jobs.get(id).is_some_and(PullJob::coalescible) {
            return Some(id.clone());
        }
    }
    None
}

pub(super) fn blocker(job: &PullJob, conditions: PullConditions) -> Option<&'static str> {
    if job.priority == PullPriority::Manual || matches!(job.target, PullTarget::Reference { .. }) {
        return None;
    }
    if conditions.network == PullNetwork::Offline {
        Some("offline")
    } else if conditions.network == PullNetwork::Cellular && !conditions.allow_cellular {
        Some("cellular")
    } else if !conditions.foreground {
        Some("background")
    } else if conditions.school_visible {
        Some("schoolVisible")
    } else {
        None
    }
}

pub(super) fn select_next(state: &mut CoordinatorState) -> Option<String> {
    if state.closed {
        return None;
    }
    let candidates = state.queue.iter().cloned().collect::<Vec<_>>();
    let mut selected: Option<(u8, u64, String)> = None;
    for id in candidates {
        let wait_for = state
            .jobs
            .get(&id)
            .and_then(|job| job.wait_for.as_ref())
            .cloned();
        if wait_for
            .as_ref()
            .and_then(|predecessor| state.jobs.get(predecessor))
            .is_some_and(PullJob::live)
        {
            continue;
        }
        let Some(job) = state.jobs.get_mut(&id) else {
            continue;
        };
        job.wait_for = None;
        if !job.ready || !matches!(job.state, PullState::Queued | PullState::Blocked) {
            continue;
        }
        job.state = reduce(job.state, PullEvent::Check).state;
        if let Some(reason) = if state.suspended {
            Some("suspended")
        } else {
            blocker(job, state.conditions)
        } {
            job.state = reduce(job.state, PullEvent::Block).state;
            job.blocked_by = Some(reason.into());
            continue;
        }
        job.state = reduce(job.state, PullEvent::Ready).state;
        job.blocked_by = None;
        let available = match job.target.lane() {
            PullLane::School => !state.school_running,
            PullLane::Http => state.http_running < 2,
        };
        if !available {
            continue;
        }
        let rank = if job.priority == PullPriority::Manual {
            0
        } else {
            1
        };
        let candidate = (rank, job.sequence, id);
        if selected.as_ref().is_none_or(|current| candidate < *current) {
            selected = Some(candidate);
        }
    }
    selected.map(|(_, _, id)| id)
}

pub(super) fn occupy_lane(state: &mut CoordinatorState, lane: PullLane) {
    match lane {
        PullLane::School => state.school_running = true,
        PullLane::Http => state.http_running += 1,
    }
}

pub(super) fn release_lane(state: &mut CoordinatorState, lane: PullLane) {
    match lane {
        PullLane::School => state.school_running = false,
        PullLane::Http => state.http_running = state.http_running.saturating_sub(1),
    }
}

pub(super) fn remove_key_if_current(state: &mut CoordinatorState, target: &PullTarget, id: &str) {
    if state.keys.get(target).is_some_and(|current| current == id) {
        state.keys.remove(target);
    }
}

pub(super) fn snapshots_locked(state: &CoordinatorState) -> Vec<PullSnapshot> {
    let mut jobs = state.jobs.values().collect::<Vec<_>>();
    jobs.sort_by_key(|job| job.sequence);
    jobs.into_iter().map(PullJob::snapshot).collect()
}

pub(super) const TERMINAL_HISTORY_LIMIT: usize = 128;

pub(super) fn prune_terminal(state: &mut CoordinatorState) {
    let mut terminal = state
        .jobs
        .values()
        .filter(|job| !job.live())
        .map(|job| (job.sequence, job.id.clone()))
        .collect::<Vec<_>>();
    if terminal.len() <= TERMINAL_HISTORY_LIMIT {
        return;
    }
    terminal.sort_by_key(|(sequence, _)| *sequence);
    let remove = terminal.len() - TERMINAL_HISTORY_LIMIT;
    for (_, id) in terminal.into_iter().take(remove) {
        state.jobs.remove(&id);
        state.queue.retain(|queued| queued != &id);
        state.keys.retain(|_, current| current != &id);
    }
}
