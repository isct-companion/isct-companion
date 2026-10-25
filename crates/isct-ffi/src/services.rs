//! Service adapters share the session's Core, operation scope and platform ports.
#[cfg(not(feature = "ocr"))]
use crate::session::unavailable;
use crate::{
    json::{project, project_input},
    types::*,
    JsonValue, Operation, OperationResult, SdkError, Session,
};
use std::sync::Arc;
fn note_error(e: isct_core::notes::NoteError) -> SdkError {
    match e {
        isct_core::notes::NoteError::Input(detail) => SdkError::InvalidInput { detail },
        other => SdkError::Storage {
            detail: other.to_string(),
        },
    }
}
fn material_error(e: isct_core::course_materials::MaterialError) -> SdkError {
    match e {
        isct_core::course_materials::MaterialError::Input(detail) => {
            SdkError::InvalidInput { detail }
        }
        isct_core::course_materials::MaterialError::Storage(detail)
        | isct_core::course_materials::MaterialError::Denied(detail) => {
            SdkError::Storage { detail }
        }
        other => SdkError::Unavailable {
            detail: other.to_string(),
        },
    }
}
#[uniffi::export]
impl Session {
    pub fn save_note(
        &self,
        context: CourseFolder,
        id: String,
        text: String,
    ) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            let notes = i.core.runtime.notes.at(i.files.clone());
            let device = i.backup.device_id().map_err(|e| SdkError::Storage {
                detail: e.to_string(),
            })?;
            Ok(OperationResult::File {
                file: project(
                    notes
                        .save(project_input(context)?, id, text, &device)
                        .map_err(note_error)?,
                )?,
            })
        })
    }
    pub fn read_note(&self, context: CourseFolder, id: String) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::File {
                file: project(
                    i.core
                        .runtime
                        .notes
                        .at(i.files.clone())
                        .read(project_input(context)?, id)
                        .map_err(note_error)?,
                )?,
            })
        })
    }
    pub fn list_notes(
        &self,
        context: CourseFolder,
        sections: Vec<String>,
    ) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::Files {
                files: project(
                    i.core
                        .runtime
                        .notes
                        .at(i.files.clone())
                        .list(project_input(context)?, sections)
                        .map_err(note_error)?,
                )?,
            })
        })
    }
    pub fn open_material(
        &self,
        context: CourseKey,
        source: String,
        download: bool,
        read_text: bool,
        limit: Option<u64>,
    ) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            let session = i.download_session.clone().map(|port| {
                Arc::new(crate::ports::NativeCookies(port))
                    as Arc<dyn isct_core::net::SessionCookies>
            });
            let service = isct_core::course_materials::CourseMaterials::new(
                i.core.store.clone(),
                i.files.clone(),
                i.core.runtime.downloads.clone(),
                session,
            );
            Ok(OperationResult::Material {
                material: project(
                    service
                        .open(project_input(context)?, source, download, read_text, limit)
                        .await
                        .map_err(material_error)?,
                )?,
            })
        })
    }
    pub fn clean_materials(&self, days: u64) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            let service = isct_core::course_materials::CourseMaterials::new(
                i.core.store.clone(),
                i.files.clone(),
                i.core.runtime.downloads.clone(),
                None,
            );
            let count = service.clean(days).await.map_err(material_error)?;
            Ok(OperationResult::Text {
                value: count.to_string(),
            })
        })
    }
    pub fn school_lifecycle(
        &self,
        event: SchoolSessionLifecycle,
    ) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            i.core
                .school_browser()?
                .lifecycle(project_input(event)?)
                .await?;
            Ok(OperationResult::Done)
        })
    }
    pub fn ensure_school_session(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            Ok(OperationResult::Flag {
                value: i
                    .core
                    .school_browser()?
                    .ensure_session(project_input(profile)?)
                    .await?,
            })
        })
    }
    pub fn school_browsing(&self, site: Option<SchoolSite>) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            i.core
                .school_browser()?
                .user_browsing(project_input(site)?)?;
            Ok(OperationResult::Done)
        })
    }
    pub fn navigate_school(
        &self,
        site: SchoolSite,
        url: String,
    ) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::Flag {
                value: i
                    .core
                    .school_browser()?
                    .navigate(project_input(site)?, &url)?,
            })
        })
    }
    pub fn reload_school(&self, site: SchoolSite) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::Flag {
                value: i.core.school_browser()?.reload(project_input(site)?)?,
            })
        })
    }
    pub fn back_school(&self, site: SchoolSite) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::Flag {
                value: i.core.school_browser()?.back(project_input(site)?)?,
            })
        })
    }
    pub fn forward_school(&self, site: SchoolSite) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::Flag {
                value: i.core.school_browser()?.forward(project_input(site)?)?,
            })
        })
    }
    pub fn set_cloud_automatic(&self, enabled: bool) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            let (sync, services) = i.cloud()?;
            Ok(OperationResult::Cloud {
                state: project(sync.set_automatic(services.clone(), enabled).await?)?,
            })
        })
    }
    pub fn restore_cloud(&self, id: String) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            let (sync, services) = i.cloud()?;
            Ok(OperationResult::Cloud {
                state: project(sync.restore(services.clone(), id).await?)?,
            })
        })
    }
    /// Parser access also allows SDK consumers to validate public fixtures without network.
    pub fn parse_lms_resource(
        &self,
        html: String,
        response_url: String,
        module: String,
    ) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |_| {
            let url = crate::ports::parse_url(&response_url)?;
            let value = isct_core::school_parser::lms::parse_resource_page(&html, &url, &module);
            Ok(OperationResult::Document {
                value: JsonValue(serde_json::to_value(value).map_err(|e| SdkError::Internal {
                    detail: e.to_string(),
                })?),
            })
        })
    }
    pub fn prepare_scanner(&self) -> Result<Arc<Operation>, SdkError> {
        #[cfg(feature = "ocr")]
        {
            self.blocking(|i| {
                i.matrix.warm_up().map_err(crate::matrix::scan_error)?;
                Ok(OperationResult::Done)
            })
        }
        #[cfg(not(feature = "ocr"))]
        {
            self.blocking(|_| Err(unavailable("OCR backend")))
        }
    }
    pub fn scan_matrix(&self, image: Vec<u8>) -> Result<Arc<Operation>, SdkError> {
        #[cfg(feature = "ocr")]
        {
            self.start_identified(true, move |inner, operation_id, cancel| async move {
                if cancel.is_cancelled() {
                    return Err(SdkError::Cancelled);
                }
                tokio::task::spawn_blocking(move || {
                    // The same driver as a camera frame: one detector, cut and model.
                    let scan = inner
                        .matrix
                        .scan_photo(&image, |completed, total| {
                            let _ = isct_core::ports::EventSink::emit(
                                &*inner.events,
                                isct_core::events::CoreEvent::OcrProgress {
                                    operation_id,
                                    completed: completed as u64,
                                    total: total as u64,
                                },
                            );
                            !cancel.is_cancelled()
                        })
                        .map_err(crate::matrix::scan_error)?;
                    let cells: Vec<_> = scan
                        .cells
                        .into_iter()
                        .map(|vote| isct_ocr::Recognition {
                            text: vote.text,
                            confidence: vote.confidence,
                        })
                        .collect();
                    Ok(OperationResult::Recognitions {
                        cells: project(cells)?,
                    })
                })
                .await
                .map_err(isct_core::runtime_tasks::RuntimeTaskError::from)?
            })
        }
        #[cfg(not(feature = "ocr"))]
        {
            let _ = image;
            self.blocking(|_| Err(unavailable("OCR backend")))
        }
    }
}

/// A file the shell received (shared into the app, dropped on an assignment),
/// by its path and the name it is handed in under.
#[derive(Clone, Debug, uniffi::Record)]
pub struct SharedFile {
    pub path: String,
    pub name: String,
}

/// A newer release for this device: its version and the package to install.
#[derive(Clone, Debug, uniffi::Record)]
pub struct AppRelease {
    pub version: String,
    pub url: String,
}

/// An assignment whose `submit/` holds files not yet handed in.
#[derive(Clone, Debug, uniffi::Record)]
pub struct WaitingHandIn {
    pub assignment_id: String,
    pub files: u32,
}

/// What the LMS page says while Core fills its draft area, in the app's language.
#[derive(Clone, Debug, uniffi::Record)]
pub struct UploadMessages {
    pub waiting: String,
    pub queued: String,
    pub failed: String,
}

fn workspace_error(e: isct_core::workspaces::WorkspaceError) -> SdkError {
    use isct_core::workspaces::WorkspaceError as E;
    match e {
        E::Input(detail) => SdkError::InvalidInput { detail },
        E::Missing => SdkError::InvalidInput {
            detail: e.to_string(),
        },
        E::Storage(detail) | E::Archive(detail) => SdkError::Storage { detail },
    }
}

fn submission_error(e: isct_core::school_browser::SubmissionError) -> SdkError {
    use isct_core::school_browser::SubmissionError as E;
    match e {
        E::InvalidInput(detail) => SdkError::InvalidInput { detail },
        E::Storage(detail) => SdkError::Storage { detail },
        E::Browser(detail) => SdkError::Unavailable { detail },
        E::WorkspaceMissing => SdkError::InvalidInput {
            detail: e.to_string(),
        },
    }
}

/// Core's reading of one assignment, which names its course folder and page.
fn assignment_detail(
    inner: &crate::session::Inner,
    id: &str,
) -> Result<isct_core::schedule::AssignmentDetail, SdkError> {
    let snapshot = inner.core.schedule_query(vec![
        isct_core::schedule::WorkspaceQuery::AssignmentDetail {
            assignment_id: id.into(),
        },
    ])?;
    match snapshot.values.into_iter().next() {
        Some(isct_core::schedule::WorkspaceQueryResult::AssignmentDetail(Some(detail))) => {
            Ok(detail)
        }
        _ => Err(SdkError::InvalidInput {
            detail: "The assignment was not found.".into(),
        }),
    }
}

/// The assignment's workspace target, once Core knows its course folder.
fn hand_in_target(
    detail: &isct_core::schedule::AssignmentDetail,
) -> Result<isct_core::workspaces::HandInTarget, SdkError> {
    isct_core::workspaces::HandInTarget::from_detail(detail).ok_or_else(|| SdkError::Unavailable {
        detail: "This assignment's course folder is not known yet.".into(),
    })
}

/// The phones' hand-in: files shared into the app go to the assignment's
/// workspace `submit/` (the desktop's folder layout), and handing in fills the
/// LMS draft area from it. Core never submits; the person saves the page.
#[uniffi::export]
impl Session {
    /// Android's update check: a newer published release with its APK, when a
    /// check is due after `last_checked` (or `manual`). The shell downloads
    /// the APK and hands it to the system installer, where the person confirms.
    pub fn check_android_update(
        &self,
        running: String,
        last_checked: Option<String>,
        manual: bool,
    ) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |_, _| async move {
            let found = isct_core::app_updates::check_android(
                &running,
                last_checked.as_deref(),
                chrono::Utc::now(),
                manual,
            )
            .await
            .map_err(|error| match error {
                isct_core::app_updates::AppUpdateError::Network(detail) => {
                    SdkError::Network { detail }
                }
                other => SdkError::InvalidInput {
                    detail: other.to_string(),
                },
            })?;
            Ok(OperationResult::AppUpdate {
                checked: found.is_some(),
                release: found.flatten().map(|release| AppRelease {
                    version: release.version,
                    url: release.apk_url,
                }),
            })
        })
    }
    /// Assignments with files waiting to be handed in, for 今日's rows.
    pub fn waiting_hand_ins(&self) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            let snapshot = i.core.workspace.snapshot()?;
            let assignments = isct_core::workspaces::Workspaces::new(i.files.clone())
                .waiting(&snapshot.workspace)
                .into_iter()
                .map(|(assignment_id, files)| WaitingHandIn {
                    assignment_id,
                    files: u32::try_from(files).unwrap_or(u32::MAX),
                })
                .collect();
            Ok(OperationResult::Waiting { assignments })
        })
    }
    /// The files waiting in the assignment's `submit/`.
    pub fn assignment_submit_files(
        &self,
        assignment_id: String,
    ) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            let detail = assignment_detail(&i, &assignment_id)?;
            let files = match detail.files {
                Some(course) => isct_core::workspaces::Workspaces::new(i.files.clone())
                    .submit_files(&course, &assignment_id)
                    .map_err(workspace_error)?,
                None => Vec::new(),
            };
            Ok(OperationResult::Files {
                files: project(files)?,
            })
        })
    }
    /// Copies the files into the assignment's `submit/`, creating its
    /// workspace on first use; the sources stay where they are.
    pub fn add_assignment_files(
        &self,
        assignment_id: String,
        files: Vec<SharedFile>,
    ) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            let target = hand_in_target(&assignment_detail(&i, &assignment_id)?)?;
            let files = files
                .into_iter()
                .map(|file| (std::path::PathBuf::from(file.path), file.name))
                .collect();
            Ok(OperationResult::Files {
                files: project(
                    isct_core::workspaces::Workspaces::new(i.files.clone())
                        .add_submit_files(&target, files)
                        .map_err(workspace_error)?,
                )?,
            })
        })
    }
    pub fn remove_assignment_file(
        &self,
        assignment_id: String,
        name: String,
    ) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            let target = hand_in_target(&assignment_detail(&i, &assignment_id)?)?;
            Ok(OperationResult::Files {
                files: project(
                    isct_core::workspaces::Workspaces::new(i.files.clone())
                        .remove_submit_file(&target.course, &assignment_id, &name)
                        .map_err(workspace_error)?,
                )?,
            })
        })
    }
    /// Queues `submit/` for the assignment's LMS draft area and returns the
    /// page to show; Core fills the draft area when that page loads.
    pub fn hand_in_assignment(
        &self,
        assignment_id: String,
        messages: UploadMessages,
    ) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            let detail = assignment_detail(&i, &assignment_id)?;
            let page = detail
                .source_url
                .clone()
                .ok_or_else(|| SdkError::Unavailable {
                    detail: "This assignment has no LMS page.".into(),
                })?;
            let target = hand_in_target(&detail)?;
            let workspaces = isct_core::workspaces::Workspaces::new(i.files.clone());
            let workspace = workspaces
                .find(&target.course, &assignment_id)
                .map_err(workspace_error)?
                .ok_or_else(|| SdkError::InvalidInput {
                    detail: "There are no files to hand in.".into(),
                })?;
            let prepared = workspaces
                .prepare_upload(&workspace.path, &assignment_id)
                .map_err(workspace_error)?;
            let url = i
                .core
                .school_browser()?
                .prepare_submission(
                    &page,
                    prepared.files,
                    isct_core::school_browser::SubmissionMessages {
                        waiting: messages.waiting,
                        queued: messages.queued,
                        failed: messages.failed,
                    },
                )
                .map_err(submission_error)?;
            Ok(OperationResult::Text { value: url })
        })
    }
}
