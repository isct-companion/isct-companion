use super::{
    refusal, snapshots, Limit, LmsIdentity, LmsModule, Purpose, RemoteState, SubmissionAction,
    SubmissionError, SubmissionRules, SubmissionService, Unsupported,
};
use crate::workspaces::{HandInError, SubmitFile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

/// How long a preparation can be executed: time for a client's
/// confirmation, while the LMS state it describes is still recent.
pub(super) const PREPARATION_TTL: i64 = 60 * 60;

/// Hex characters of each sha256 the confirmation shows.
pub(super) const DIGEST_PREFIX: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SnapshotFile {
    pub name: String,
    pub bytes: u64,
    pub(crate) sha256: String,
}

/// What a write changes on the LMS, by file name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "agent", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub(crate) struct Effect {
    /// Not on the LMS yet.
    pub added: Vec<String>,
    /// On the LMS, written again.
    pub replaced: Vec<String>,
    /// On the LMS, deleted.
    pub removed: Vec<String>,
    /// The LMS already holds exactly this, as Core confirmed it; nothing is
    /// sent.
    pub unchanged: bool,
}

/// Whether the person can still change their mind after the write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "agent", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum Reversibility {
    /// Can be changed or removed again until the cut-off, in Unix seconds.
    UntilCutoff { at: i64 },
    /// No cut-off: can be changed or removed again while the assignment
    /// stays open.
    WhileOpen,
    /// Deletes the submitted files, which the LMS cannot restore.
    Irreversible,
}

/// One write, fixed when it was prepared: the snapshot, whose LMS it goes
/// to and what that LMS held.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preparation {
    pub id: String,
    pub assignment_id: String,
    pub(crate) assignment_title: String,
    pub identity: LmsIdentity,
    /// The LMS account's generation when it was read; a sign-out or a new
    /// account since refuses the write.
    pub(crate) account_generation: u64,
    pub module: LmsModule,
    pub(crate) assign_id: u64,
    pub action: SubmissionAction,
    /// By name; empty for a removal.
    pub files: Vec<SnapshotFile>,
    /// What the LMS held, attempt included; `execute` refuses once it moved.
    pub(crate) baseline: RemoteState,
    pub effect: Effect,
    pub(crate) reversibility: Reversibility,
    /// Unix seconds.
    pub expires_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "agent", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummaryFile {
    pub name: String,
    pub bytes: u64,
    pub(crate) sha256_prefix: String,
}

/// What the client's confirmation shows, and what `execute` must receive
/// back unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "agent", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubmissionSummary {
    pub(crate) assignment_title: String,
    pub action: SubmissionAction,
    pub files: Vec<SummaryFile>,
    pub effect: Effect,
    pub(crate) reversibility: Reversibility,
}

impl Preparation {
    pub(crate) fn summary(&self) -> SubmissionSummary {
        SubmissionSummary {
            assignment_title: self.assignment_title.clone(),
            action: self.action,
            files: self
                .files
                .iter()
                .map(|file| SummaryFile {
                    name: file.name.clone(),
                    bytes: file.bytes,
                    sha256_prefix: file.sha256.chars().take(DIGEST_PREFIX).collect(),
                })
                .collect(),
            effect: self.effect.clone(),
            reversibility: self.reversibility,
        }
    }

    /// The write's identity, from what Core recorded and nothing a caller
    /// passed: site, user, assignment, attempt, action and snapshot.
    pub(super) fn key(&self) -> String {
        let mut material = format!(
            "submission-v2\n{:?}\n{}\n{}\n{}\n{}\n",
            self.identity.site,
            self.identity.user_id,
            self.assign_id,
            self.baseline.attempt,
            self.action.name(),
        );
        for file in &self.files {
            material.push_str(&format!(
                "{:?}\t{}\t{}\n",
                file.name, file.bytes, file.sha256
            ));
        }
        hex_digest(material.as_bytes())
    }
}

impl SubmissionService {
    /// Reads the LMS, snapshots `submit/` for a save, and returns the write
    /// as it would happen. Nothing is sent to the LMS. Refuses while an
    /// earlier write to the assignment is unsettled, and refuses to replace
    /// a submission the LMS already holds: it lists what would be replaced.
    pub(crate) async fn prepare(
        &self,
        assignment: &str,
        action: SubmissionAction,
    ) -> Result<Preparation, SubmissionError> {
        let (detail, _) = self.workspaces.detail(assignment)?;
        let module = self.workspaces.lms_module(assignment)?;
        let permit = Arc::new(self.transport.permit()?);
        let account_generation = permit.generation;
        let remote = self.read(Purpose::Prepare, module, permit, false).await?;
        // An earlier write still decides what the LMS holds; an unknown one
        // is read on its schedule, and soon.
        if let Some(operation) = self
            .ledger
            .unsettled(&remote.identity, remote.assign_id)?
            .into_iter()
            .next()
        {
            self.ensure_checked(&operation)?;
            return Err(refusal(&operation));
        }
        supported(&remote.rules)?;
        let capture = match action {
            SubmissionAction::Save => Some(self.capture(assignment).await?),
            SubmissionAction::Remove => None,
        };
        let files = capture
            .as_ref()
            .map(snapshots::Capture::files)
            .unwrap_or_default();
        if action == SubmissionAction::Save {
            within_limits(&files, &remote.rules)?;
        }
        let reversibility = reversibility(action, &remote.rules);
        let mut preparation = Preparation {
            id: String::new(),
            assignment_id: assignment.into(),
            assignment_title: detail.title,
            identity: remote.identity,
            account_generation,
            module,
            assign_id: remote.assign_id,
            action,
            files,
            baseline: remote.state,
            effect: Effect::default(),
            reversibility,
            expires_at: 0,
        };
        let receipt = self
            .ledger
            .receipt(&preparation.key(), &preparation.baseline)?;
        preparation.effect = effect(&preparation, receipt.is_some())?;
        // Content-addressed: the same snapshot, account, LMS state and
        // effect give the same id.
        preparation.id = hex_digest(&serde_json::to_vec(&preparation)?);
        let now = self.now();
        preparation.expires_at = now + PREPARATION_TTL;
        let snapshots = self.snapshots.clone();
        let ledger = self.ledger.clone();
        tokio::task::spawn_blocking(move || {
            snapshots.publish(capture, || ledger.publish_preparation(&preparation, now))
        })
        .await
        .map_err(|_| SubmissionError::Storage("The snapshot was interrupted".into()))?
    }

    async fn capture(&self, assignment: &str) -> Result<snapshots::Capture, SubmissionError> {
        let workspace = self
            .workspaces
            .find(assignment)?
            .ok_or(HandInError::Empty)?;
        if workspace.zip {
            return Err(Unsupported::Zip.into());
        }
        let submit = PathBuf::from(&workspace.path).join(crate::workspaces::SUBMIT_DIR);
        let snapshots = self.snapshots.clone();
        tokio::task::spawn_blocking(move || snapshots.capture(&submit, &workspace.submit_files))
            .await
            .map_err(|_| SubmissionError::Storage("The snapshot was interrupted".into()))?
    }

    /// The snapshot is what was confirmed, so a damaged one is never sent;
    /// and `submit/` changed since means the person now wants to hand in
    /// something they have not confirmed.
    pub(super) async fn check_files(
        &self,
        preparation: &Preparation,
    ) -> Result<(), SubmissionError> {
        let snapshots = self.snapshots.clone();
        let workspaces = self.workspaces.clone();
        let assignment = preparation.assignment_id.clone();
        let files = preparation.files.clone();
        let (intact, current) = tokio::task::spawn_blocking(move || {
            Ok::<_, SubmissionError>((snapshots.intact(&files), workspaces.status(&assignment)?))
        })
        .await
        .map_err(|_| SubmissionError::Storage("The file check was interrupted".into()))??;
        match current {
            Some((workspace, current))
                if intact && !workspace.zip && same_files(&current, &preparation.files) =>
            {
                Ok(())
            }
            _ => Err(SubmissionError::FilesChanged),
        }
    }
}

/// The individual file assignments the write survey verified, and nothing
/// else (`approach/submission-capabilities`).
pub(super) fn supported(rules: &SubmissionRules) -> Result<(), Unsupported> {
    let reason = if !rules.files_enabled {
        Unsupported::NoFiles
    } else if rules.online_text {
        Unsupported::OnlineText
    } else if rules.drafts {
        Unsupported::Drafts
    } else if rules.statement {
        Unsupported::Statement
    } else if rules.team {
        Unsupported::Team
    } else if rules.time_limit {
        Unsupported::TimeLimit
    } else if rules.locked || !rules.can_edit {
        Unsupported::Closed
    } else {
        return Ok(());
    };
    Err(reason)
}

/// With drafts off, a save is final at the cut-off, and a removal deletes
/// what it removes for good.
pub(super) fn reversibility(action: SubmissionAction, rules: &SubmissionRules) -> Reversibility {
    match action {
        SubmissionAction::Save => {
            rules
                .cutoff
                .map_or(Reversibility::WhileOpen, |at| Reversibility::UntilCutoff {
                    at,
                })
        }
        SubmissionAction::Remove => Reversibility::Irreversible,
    }
}

pub(super) fn within_limits(
    files: &[SnapshotFile],
    rules: &SubmissionRules,
) -> Result<(), SubmissionError> {
    if rules.max_files > 0 && files.len() > rules.max_files as usize {
        return Err(SubmissionError::Limit(Limit::Files));
    }
    if rules.max_file_bytes > 0 && files.iter().any(|file| file.bytes > rules.max_file_bytes) {
        return Err(SubmissionError::Limit(Limit::FileSize));
    }
    // Moodle's `PARAM_FILE` drops these characters from an uploaded name.
    let kept = |name: &str| {
        name != "."
            && name != ".."
            && !name
                .chars()
                .any(|character| character.is_control() || "&<>\"`|':\\/".contains(character))
    };
    if !files.iter().all(|file| kept(&file.name)) {
        return Err(SubmissionError::Limit(Limit::FileName));
    }
    let types: Vec<String> = rules
        .file_types
        .iter()
        .flat_map(|entry| entry.split([',', ';', ' ']))
        .map(|entry| entry.trim().to_lowercase())
        .filter(|entry| !entry.is_empty())
        .collect();
    if types.is_empty() || types.iter().any(|entry| entry == "*") {
        return Ok(());
    }
    if types.iter().any(|entry| !entry.starts_with('.')) {
        return Err(Unsupported::FileTypeGroups.into());
    }
    let accepted = |name: &str| {
        let name = name.to_lowercase();
        types
            .iter()
            .any(|extension| name.ends_with(extension.as_str()))
    };
    if files.iter().all(|file| accepted(&file.name)) {
        Ok(())
    } else {
        Err(SubmissionError::Limit(Limit::FileType))
    }
}

/// What the write changes. A save never replaces a submission the LMS
/// already holds unless Core's own receipt shows it is this very content.
fn effect(preparation: &Preparation, receipt: bool) -> Result<Effect, SubmissionError> {
    if receipt {
        return Ok(Effect {
            unchanged: true,
            ..Effect::default()
        });
    }
    let baseline = &preparation.baseline;
    let remote: BTreeSet<&str> = baseline
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect();
    let local: BTreeSet<&str> = preparation
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect();
    let names = |set: BTreeSet<&&str>| {
        set.into_iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    };
    match preparation.action {
        SubmissionAction::Save if baseline.holds_content() => {
            Err(SubmissionError::RemoteNotEmpty {
                added: names(local.difference(&remote).collect()),
                replaced: names(local.intersection(&remote).collect()),
                removed: names(remote.difference(&local).collect()),
            })
        }
        SubmissionAction::Save => Ok(Effect {
            added: names(local.iter().collect()),
            ..Effect::default()
        }),
        SubmissionAction::Remove if baseline.holds_content() => Ok(Effect {
            removed: names(remote.iter().collect()),
            ..Effect::default()
        }),
        SubmissionAction::Remove => Err(SubmissionError::NothingToRemove),
    }
}

/// The echo must be the summary, field by field.
pub(super) fn confirm(
    prepared: &SubmissionSummary,
    echo: &SubmissionSummary,
) -> Result<(), SubmissionError> {
    let field = if prepared.assignment_title != echo.assignment_title {
        "assignmentTitle"
    } else if prepared.action != echo.action {
        "action"
    } else if prepared.files != echo.files {
        "files"
    } else if prepared.effect != echo.effect {
        "effect"
    } else if prepared.reversibility != echo.reversibility {
        "reversibility"
    } else {
        return Ok(());
    };
    Err(SubmissionError::ConfirmationMismatch(field))
}

/// Whether `submit/` still holds exactly the snapshot; both are by name.
fn same_files(current: &[SubmitFile], snapshot: &[SnapshotFile]) -> bool {
    current.len() == snapshot.len()
        && current.iter().zip(snapshot).all(|(current, kept)| {
            current.name == kept.name
                && current.bytes == Some(kept.bytes)
                && current.sha256.as_deref() == Some(kept.sha256.as_str())
        })
}

pub(super) fn hex_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    data_encoding::HEXLOWER.encode(&hasher.finalize())
}
