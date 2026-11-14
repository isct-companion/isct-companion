//! The mobile main app's snapshot storage and system reload port.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::{Operation, OperationResult, SdkError, Session};
use app_core::{
    schedule::widgets::{WidgetDestination, WidgetKind},
    widgets::{WidgetError, WidgetPort, WidgetRoute as CoreRoute},
};
use std::sync::Arc;

#[uniffi::export(rust, foreign)]
pub trait NativeWidgets: Send + Sync {
    fn current(&self) -> Result<Option<String>, SdkError>;
    fn replace(&self, json: String) -> Result<(), SdkError>;
    fn clear(&self) -> Result<(), SdkError>;
    fn reload(&self, courses: bool, assignments: bool) -> Result<(), SdkError>;
}

struct Widgets(Arc<dyn NativeWidgets>);

fn storage(error: SdkError) -> WidgetError {
    WidgetError::Storage(error.to_string())
}

impl WidgetPort for Widgets {
    fn current(&self) -> Result<Option<String>, WidgetError> {
        self.0.current().map_err(storage)
    }

    fn replace(&self, json: &str) -> Result<(), WidgetError> {
        self.0.replace(json.to_owned()).map_err(storage)
    }

    fn clear(&self) -> Result<(), WidgetError> {
        self.0.clear().map_err(storage)
    }

    fn reload(&self, kinds: &[WidgetKind]) -> Result<(), WidgetError> {
        self.0
            .reload(
                kinds.contains(&WidgetKind::Courses),
                kinds.contains(&WidgetKind::Assignments),
            )
            .map_err(storage)
    }
}

#[derive(Clone, Debug, uniffi::Enum)]
pub enum WidgetRoute {
    Course {
        course_id: String,
    },
    Exam {
        key: String,
        term: String,
        date: String,
    },
    Assignment {
        id: String,
        course_id: Option<String>,
    },
    Timetable,
    Assignments,
    Sync,
    UnavailableCourse,
    UnavailableAssignment,
}

impl From<CoreRoute> for WidgetRoute {
    fn from(route: CoreRoute) -> Self {
        match route {
            CoreRoute::Course { course_id } => Self::Course { course_id },
            CoreRoute::Exam { key, term, date } => Self::Exam { key, term, date },
            CoreRoute::Assignment { id, course_id } => Self::Assignment { id, course_id },
            CoreRoute::Timetable => Self::Timetable,
            CoreRoute::Assignments => Self::Assignments,
            CoreRoute::Sync => Self::Sync,
            CoreRoute::UnavailableCourse => Self::UnavailableCourse,
            CoreRoute::UnavailableAssignment => Self::UnavailableAssignment,
        }
    }
}

#[uniffi::export]
impl Session {
    pub fn attach_widgets(&self, port: Arc<dyn NativeWidgets>) -> Result<(), SdkError> {
        let closed = self.inner.gate.lock().map_err(|_| SdkError::Closed)?;
        if *closed {
            return Err(SdkError::Closed);
        }
        self.inner
            .core
            .attach_widgets(Arc::new(Widgets(port)), cfg!(target_os = "android"));
        Ok(())
    }

    pub fn refresh_widgets(&self) -> Result<(), SdkError> {
        let closed = self.inner.gate.lock().map_err(|_| SdkError::Closed)?;
        if *closed {
            return Err(SdkError::Closed);
        }
        self.inner.core.refresh_widgets();
        Ok(())
    }

    pub fn widget_route(&self, url: String) -> Result<Arc<Operation>, SdkError> {
        let destination = url::Url::parse(&url)
            .ok()
            .as_ref()
            .and_then(WidgetDestination::from_url)
            .ok_or_else(|| SdkError::InvalidInput {
                detail: "Invalid widget destination".into(),
            })?;
        self.blocking(move |inner| {
            Ok(OperationResult::WidgetRoute {
                route: inner.core.widget_route(destination)?.into(),
            })
        })
    }
}
