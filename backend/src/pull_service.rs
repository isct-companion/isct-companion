//! Domain adapters share one coordinator across all triggers and platforms.
use crate::{
    lifecycle::SchoolSite,
    pull_coordinator::{PullCoordinator, PullError, PullOptions, PullPriority, PullTarget},
    school_auth::SchoolAuthProfile,
    school_web::{IngestOutcome, SchoolWebController},
    OsSecretVault, SchoolAuthService,
};
use tauri::{Emitter, Manager};

pub(crate) async fn school(
    app: &tauri::AppHandle,
    site: SchoolSite,
    course_id: Option<u64>,
    options: PullOptions,
    login_first: bool,
) -> Result<Vec<IngestOutcome>, String> {
    if !matches!(site, SchoolSite::ScienceLms | SchoolSite::Kyomu) {
        return Err("Only LMS and 教務Web can be pulled.".into());
    }
    if course_id == Some(0) || (course_id.is_some() && site != SchoolSite::ScienceLms) {
        return Err("LMS course ID must be positive and use the LMS site.".into());
    }
    let coordinator = app.state::<PullCoordinator>().inner().clone();
    let app = app.clone();
    let value = coordinator
        .run(
            PullTarget::School { site, course_id },
            options,
            move |context| {
                let app = app.clone();
                async move {
                    if !context.phase("extracting").await {
                        return Err(PullError::Cancelled);
                    }
                    let handle = app.clone();
                    let executor_context = context.clone();
                    let result = tauri::async_runtime::spawn_blocking(move || {
                        let controller = handle.state::<SchoolWebController>().inner().clone();
                        if login_first {
                            let profile = if site == SchoolSite::ScienceLms {
                                SchoolAuthProfile::Science
                            } else {
                                SchoolAuthProfile::TokyoTech
                            };
                            let credentials = handle
                                .state::<SchoolAuthService<OsSecretVault>>()
                                .load(profile)?
                                .ok_or("School credentials are not configured.")?;
                            controller.request_login(&handle, profile, credentials)?;
                        }
                        if executor_context.priority() == PullPriority::Manual {
                            controller.request_pull(&handle, site, course_id, executor_context)
                        } else {
                            controller.request_pull_background(
                                &handle,
                                site,
                                course_id,
                                executor_context,
                            )
                        }
                    })
                    .await
                    .map_err(|_| {
                        PullError::Execution("The school pull stopped unexpectedly.".into())
                    })?;
                    let outcomes = result.map_err(PullError::Execution)?;
                    // Extractor outcomes are returned unchanged, including partial
                    // 教務Web results and reconnect-required. They already committed.
                    let _ = app.emit(
                        "workspace-changed",
                        serde_json::json!({"source":"school-pull"}),
                    );
                    encode_outcomes(&context, outcomes)
                        .await
                        .map_err(PullError::Execution)
                }
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    serde_json::from_value(value).map_err(|_| "School pull returned an invalid result.".into())
}

async fn encode_outcomes(
    context: &crate::pull_coordinator::PullContext,
    outcomes: Vec<IngestOutcome>,
) -> Result<serde_json::Value, String> {
    if let Some(problem) = outcomes.iter().find(|row| row.state != "ok") {
        context.outcome_error(problem.message.clone()).await;
    }
    serde_json::to_value(outcomes).map_err(|error| error.to_string())
}

pub(crate) fn start(app: tauri::AppHandle) {
    let coordinator = app.state::<PullCoordinator>().inner().clone();
    let mut states = coordinator.subscribe();
    tauri::async_runtime::spawn(async move {
        while states.changed().await.is_ok() {
            let snapshot = states.borrow_and_update().clone();
            let _ = app.emit_to("main", "pull-state-changed", snapshot);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pull_coordinator::PullState;
    use serde_json::json;

    #[tokio::test]
    async fn adapter_preserves_success_and_partial_wire_results() {
        for states in [
            vec!["ok", "ok"],
            vec!["ok", "reconnect-required"],
            vec!["ok", "error"],
        ] {
            let coordinator = PullCoordinator::default();
            let original = serde_json::Value::Array(states.iter().enumerate().map(|(index,state)| {
                json!({"site":"kyomu", "kind":format!("page-{index}"), "state":state,
                    "changed":if *state=="ok" {3} else {0}, "message":if *state=="ok" {"Updated"} else {"Sign in"}})
            }).collect());
            let expected = original.clone();
            let result = coordinator
                .run(
                    PullTarget::School {
                        site: SchoolSite::Kyomu,
                        course_id: None,
                    },
                    PullOptions::manual(),
                    move |context| {
                        let rows: Vec<IngestOutcome> =
                            serde_json::from_value(original.clone()).unwrap();
                        async move {
                            encode_outcomes(&context, rows)
                                .await
                                .map_err(PullError::Execution)
                        }
                    },
                )
                .await
                .unwrap();
            assert_eq!(result, expected);
            assert_eq!(
                coordinator.snapshots().await[0].state,
                if states.iter().all(|state| *state == "ok") {
                    PullState::Succeeded
                } else {
                    PullState::Failed
                }
            );
        }
    }
}
