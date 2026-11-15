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
            let service = app_core::course_materials::CourseMaterials::new(
                i.core.store.clone(),
                i.files.clone(),
                i.core.runtime.downloads.clone(),
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
    /// it with its own session of that site and names it, and the shell moves
    /// it to the person's downloads. School hosts are reached only through a WebView or
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
                .save_school_file(request, directory.into())
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
            );
            let count = service.clean(days).await.map_err(material_error)?;
            Ok(OperationResult::Text {
                value: count.to_string(),
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
    pub fn reload_school(&self, site: SchoolSite) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::Flag {
                value: i.core.school_browser()?.reload(project_input(site)?)?,
            })
        })
    }
    /// The person's "Cancel, I'll sign in myself" while Core signs a school
    /// view in.
    pub fn cancel_school_sign_in(&self, site: SchoolSite) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::Flag {
                value: i
                    .core
                    .school_browser()?
                    .cancel_sign_in(project_input(site)?)?,
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
                    Ok(OperationResult::Recognitions {
                        cells: scan.cells.into_iter().map(Into::into).collect(),
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

/// Where this platform's releases come from, fixed at build time.
#[cfg(target_os = "android")]
async fn fetch_release(
    running: &str,
) -> Result<Option<app_core::app_updates::MobileRelease>, app_core::app_updates::AppUpdateError> {
    app_core::app_updates::fetch_android(running).await
}

#[cfg(target_os = "ios")]
async fn fetch_release(
    running: &str,
) -> Result<Option<app_core::app_updates::MobileRelease>, app_core::app_updates::AppUpdateError> {
    app_core::app_updates::fetch_app_store(running).await
}

/// Host builds (tests, binding generation) have no store to read.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
async fn fetch_release(
    _running: &str,
) -> Result<Option<app_core::app_updates::MobileRelease>, app_core::app_updates::AppUpdateError> {
    Err(app_core::app_updates::AppUpdateError::Network(
        "This build has no update source.".into(),
    ))
}

/// A newer release for this device: its version and where it comes from (the
/// APK on Android, the App Store page on iOS and iPadOS).
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

/// The phones' hand-in: files shared into the app go to the assignment's
/// workspace `submit/` (the desktop's folder layout), and handing in fills the
/// LMS draft area from it. Core never submits; the person saves the page.
#[uniffi::export]
impl Session {
    /// The update check of a phone or tablet: a newer release, when a check
    /// is due after `last_checked` and this device's settings let it run, or
    /// now (`manual`). Core decides whether the release is offered in the
    /// prompt (`prompt`), skipping the version the person skipped. Android
    /// reads GitHub's release with its APK, which the shell hands to the
    /// system installer; iOS and iPadOS read the App Store listing, which the
    /// shell opens.
    pub fn check_app_update(
        &self,
        running: String,
        last_checked: Option<String>,
        manual: bool,
    ) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |inner, _| async move {
            let workspace = inner.core.store.load_fields(&[])?;
            if !app_core::app_updates::check_now(
                &workspace,
                last_checked.as_deref(),
                chrono::Utc::now(),
                manual,
            ) {
                return Ok(OperationResult::AppUpdate {
                    checked: false,
                    release: None,
                    prompt: false,
                });
            }
            let found = fetch_release(&running)
                .await
                .map_err(|error| crate::error::coded(&error))?;
            let prompt = found.as_ref().is_some_and(|release| {
                app_core::app_updates::prompts(&workspace, &release.version, manual)
            });
            Ok(OperationResult::AppUpdate {
                checked: true,
                release: found.map(|release| AppRelease {
                    version: release.version,
                    url: release.url,
                }),
                prompt,
            })
        })
    }
    /// Assignments with files waiting to be handed in, for 今日's rows.
    pub fn waiting_hand_ins(&self) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            let assignments = i
                .core
                .assignment_workspaces
                .waiting()
                .map_err(workspace_error)?
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
            let files = i
                .core
                .assignment_workspaces
                .submit_files(&assignment_id)
                .map_err(workspace_error)?;
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
            let files = files
                .into_iter()
                .map(|file| (std::path::PathBuf::from(file.path), file.name))
                .collect();
            Ok(OperationResult::Files {
                files: project(
                    i.core
                        .assignment_workspaces
                        .add_submit_files(&assignment_id, files)
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
            Ok(OperationResult::Files {
                files: project(
                    i.core
                        .assignment_workspaces
                        .remove_submit_file(&assignment_id, &name)
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
            let workspaces = i.core.assignment_workspaces.at().map_err(workspace_error)?;
            let workspace = i
                .core
                .assignment_workspaces
                .find(&assignment_id)
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

/// The campus map over the file Core downloaded. Each call runs off the
/// caller's thread like every other read: the first one unpacks the file.
/// Until the file is stored `has` answers false, so shells show no map, and
/// `open` and `search` fail as unavailable.
///
/// The page is Core's, one per session: the shell opens it (replacing any
/// other), sends each intent with the page from the scene, draws the scene
/// with the highest `revision` it has seen, and closes the page when it
/// leaves the screen. Intents for a replaced page answer `None`.
#[cfg(feature = "campus-map")]
#[uniffi::export]
impl Session {
    /// Whether the map knows the room string, or with none whether a map is
    /// stored; false means no map button.
    pub fn campus_map_has(&self, room: Option<String>) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::Flag {
                value: i.core.campus_map.has(room.as_deref())?,
            })
        })
    }
    /// Opens a map page on a room string, or on none to browse, replacing
    /// any other; answers its first scene. The page's id is taken now,
    /// before the file loads, so a close or a later open that comes first
    /// supersedes this one.
    pub fn campus_map_open(&self, room: Option<String>) -> Result<Arc<Operation>, SdkError> {
        let page = self.inner.core.campus_map.reserve();
        self.blocking(move |i| {
            Ok(OperationResult::CampusMapScene {
                scene: Some(project(i.core.campus_map.open(page, room.as_deref())?)?),
            })
        })
    }
    /// The places a search box's text names; the answer carries the text,
    /// so a shell drops answers to text since changed.
    pub fn campus_map_search(&self, query: String) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::CampusMapSearch {
                search: project(i.core.campus_map.search(&query)?)?,
            })
        })
    }
    /// One intent on an open page; answers the new scene, or `None` when
    /// nothing drawn changed.
    pub fn campus_map_send(
        &self,
        page: u64,
        intent: CampusMapIntent,
    ) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            let intent: app_core::campus_map::CampusMapIntent = project_input(intent)?;
            Ok(OperationResult::CampusMapScene {
                scene: project(i.core.campus_map.send(page, intent))?,
            })
        })
    }
    /// Forgets the page, and any open still loading; their scenes name
    /// nothing afterwards.
    pub fn campus_map_close(&self) {
        self.inner.core.campus_map.close();
    }
}
