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
