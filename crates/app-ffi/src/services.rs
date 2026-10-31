//! Service adapters share the session's Core, operation scope and platform ports.
#[cfg(not(feature = "ocr"))]
use crate::session::unavailable;
use crate::{
    json::{project, project_input},
    types::*,
    JsonValue, Operation, OperationResult, SdkError, Session,
};
use std::sync::Arc;
fn note_error(e: app_core::notes::NoteError) -> SdkError {
    crate::error::coded(&e)
}
fn material_error(e: app_core::course_materials::MaterialError) -> SdkError {
    crate::error::coded(&e)
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
            Ok(OperationResult::File {
                file: project(
                    notes
                        .save(project_input(context)?, id, text)
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
                    as Arc<dyn app_core::net::SessionCookies>
            });
            let service = app_core::course_materials::CourseMaterials::new(
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
    /// Saves a file a school page handed over (the WebView's download
    /// callback) into `directory`, the shell's staging folder: Core fetches
    /// it with the school session and names it, and the shell moves it to the
    /// person's downloads. School hosts are reached only through a WebView or
    /// Core (`constraint/school-network-two-stacks`), never a system
    /// download service.
    pub fn save_school_file(
        &self,
        url: String,
        referer: Option<String>,
        content_disposition: Option<String>,
        mime_type: Option<String>,
        directory: String,
    ) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            let session: Arc<dyn app_core::net::SessionCookies> = match i.download_session.clone() {
                Some(port) => Arc::new(crate::ports::NativeCookies(port)),
                None => return Err(crate::session::unavailable("School downloads")),
            };
            let request = app_core::school_files::SchoolFileRequest {
                url,
                referer,
                content_disposition,
                mime_type,
            };
            let file = i
                .core
                .runtime
                .downloads
                .save_school_file(session, request, directory.into())
                .await
                .map_err(|error| crate::error::coded(&error))?;
            Ok(OperationResult::SchoolFile {
                file: match file {
                    app_core::school_files::SchoolFile::Saved {
                        path,
                        name,
                        mime_type,
                    } => SchoolFile::Saved {
                        path: path.to_string_lossy().into_owned(),
                        name,
                        mime_type,
                    },
                    app_core::school_files::SchoolFile::External { url } => {
                        SchoolFile::External { url }
                    }
                },
            })
        })
    }
    pub fn clean_materials(&self, days: u64) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            let service = app_core::course_materials::CourseMaterials::new(
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
    pub fn school_browsing(&self, site: Option<SchoolSite>) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            i.core
                .school_browser()?
                .user_browsing(project_input(site)?)?;
            Ok(OperationResult::Done)
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
            let value = adpt_isct::lms::parse_resource_page(&html, &url, &module);
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
                            let _ = app_core::ports::EventSink::emit(
                                &*inner.events,
                                app_core::events::CoreEvent::OcrProgress {
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
                        .map(|vote| app_ocr::Recognition {
                            text: vote.text,
                            confidence: vote.confidence,
                        })
                        .collect();
                    Ok(OperationResult::Recognitions {
                        cells: project(cells)?,
                    })
                })
                .await
                .map_err(app_core::runtime_tasks::RuntimeTaskError::from)?
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

/// What became of a file a school page handed over (`save_school_file`).
#[derive(Clone, Debug, uniffi::Enum)]
pub enum SchoolFile {
    /// Staged at `path` under `name`; the shell moves it to Downloads.
    Saved {
        path: String,
        name: String,
        mime_type: String,
    },
    /// Not on a school host: Core fetches only school files, so the shell
    /// opens it in the system browser.
    External { url: String },
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

fn workspace_error(e: app_core::workspaces::WorkspaceError) -> SdkError {
    crate::error::coded(&e)
}

fn submission_error(e: app_core::school_browser::SubmissionError) -> SdkError {
    crate::error::coded(&e)
}

/// Core's reading of one assignment, which names its course folder and page.
fn assignment_detail(
    inner: &crate::session::Inner,
    id: &str,
) -> Result<app_core::schedule::AssignmentDetail, SdkError> {
    let snapshot =
        inner
            .core
            .schedule_query(vec![app_core::schedule::WorkspaceQuery::AssignmentDetail {
                assignment_id: id.into(),
            }])?;
    match snapshot.values.into_iter().next() {
        Some(app_core::schedule::WorkspaceQueryResult::AssignmentDetail(Some(detail))) => {
            Ok(detail)
        }
        _ => Err(SdkError::InvalidInput {
            detail: "The assignment was not found.".into(),
        }),
    }
}

/// The assignment's workspace target, once Core knows its course folder.
fn hand_in_target(
    detail: &app_core::schedule::AssignmentDetail,
) -> Result<app_core::workspaces::HandInTarget, SdkError> {
    app_core::workspaces::HandInTarget::from_detail(detail).ok_or_else(|| SdkError::Unavailable {
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
            let found = app_core::app_updates::check_android(
                &running,
                last_checked.as_deref(),
                chrono::Utc::now(),
                manual,
            )
            .await
            .map_err(|error| crate::error::coded(&error))?;
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
            let workspace = i.core.store.load_fields(&["assignments"])?;
            let assignments = app_core::workspaces::Workspaces::new(i.files.clone())
                .waiting(&workspace)
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
                Some(course) => app_core::workspaces::Workspaces::new(i.files.clone())
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
                    app_core::workspaces::Workspaces::new(i.files.clone())
                        .add_submit_files(&target, files)
                        .map_err(|e| crate::error::coded(&e))?,
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
                    app_core::workspaces::Workspaces::new(i.files.clone())
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
            let workspaces = app_core::workspaces::Workspaces::new(i.files.clone());
            let workspace = workspaces
                .find(&target.course, &assignment_id)
                .map_err(workspace_error)?
                .ok_or_else(|| SdkError::InvalidInput {
                    detail: "There are no files to hand in.".into(),
                })?;
            let prepared = workspaces
                .prepare_upload(&workspace.path, &assignment_id)
                .map_err(|e| crate::error::coded(&e))?;
            let url = i
                .core
                .school_browser()?
                .prepare_submission(
                    &page,
                    prepared.files,
                    app_core::school_browser::SubmissionMessages {
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
