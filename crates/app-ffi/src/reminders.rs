use crate::{
    json::{project, project_input},
    types::{ReminderDeliveryPlan, ReminderPortFacts, ReminderReceipt},
    SdkError, Session,
};
use app_core::schedule as core;
use futures_util::future::BoxFuture;
use std::sync::Arc;

#[uniffi::export(rust, foreign)]
#[async_trait::async_trait]
pub trait NativeReminderPort: Send + Sync {
    async fn facts(&self) -> Result<ReminderPortFacts, SdkError>;
    async fn apply(&self, plan: ReminderDeliveryPlan) -> Result<ReminderReceipt, SdkError>;
}

struct Port(Arc<dyn NativeReminderPort>);

fn callback(error: SdkError) -> core::ReminderError {
    core::NotificationSinkError::Delivery(error.to_string()).into()
}

impl core::ReminderPort for Port {
    fn facts(&self) -> BoxFuture<'_, Result<core::ReminderPortFacts, core::ReminderError>> {
        Box::pin(async {
            let facts = self.0.facts().await.map_err(callback)?;
            project_input(facts).map_err(callback)
        })
    }

    fn apply(
        &self,
        plan: core::ReminderDeliveryPlan,
    ) -> BoxFuture<'_, Result<core::ReminderReceipt, core::ReminderError>> {
        Box::pin(async move {
            let receipt = self
                .0
                .apply(project(plan).map_err(callback)?)
                .await
                .map_err(callback)?;
            project_input(receipt).map_err(callback)
        })
    }
}

#[uniffi::export]
impl Session {
    pub fn attach_reminder_port(&self, port: Arc<dyn NativeReminderPort>) -> Result<(), SdkError> {
        let closed = self.inner.gate.lock().map_err(|_| SdkError::Closed)?;
        if *closed {
            return Err(SdkError::Closed);
        }
        self.inner.core.attach_reminder_port(Arc::new(Port(port)))?;
        Ok(())
    }

    pub fn refresh_reminder_port(&self) -> Result<(), SdkError> {
        let closed = self.inner.gate.lock().map_err(|_| SdkError::Closed)?;
        if *closed {
            return Err(SdkError::Closed);
        }
        self.inner.core.refresh_reminder_port();
        Ok(())
    }
}
