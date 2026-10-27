use crate::{JsonValue, SdkError};
use app_core::{
    events::CoreEvent,
    ports::{EventError, EventSink},
};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, uniffi::Record)]
pub struct Event {
    pub sequence: u64,
    pub name: String,
    pub payload: JsonValue,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum StreamUpdate {
    Event { event: Event },
    SnapshotRequired { missed: u64 },
}
pub(crate) struct Events {
    sequence: Mutex<u64>,
    sender: broadcast::Sender<Event>,
    closed: CancellationToken,
}
impl Events {
    pub fn new() -> Arc<Self> {
        let (sender, _) = broadcast::channel(128);
        Arc::new(Self {
            sequence: Mutex::new(0),
            sender,
            closed: CancellationToken::new(),
        })
    }
    pub fn publish(&self, name: &str, value: serde_json::Value) -> Result<(), SdkError> {
        let mut sequence = self.sequence.lock().map_err(|_| SdkError::Closed)?;
        if self.closed.is_cancelled() {
            return Err(SdkError::Closed);
        }
        *sequence = sequence.checked_add(1).ok_or(SdkError::Closed)?;
        let _ = self.sender.send(Event {
            sequence: *sequence,
            name: name.into(),
            payload: JsonValue(value),
        });
        Ok(())
    }
    pub fn subscribe(&self) -> Result<Arc<Subscription>, SdkError> {
        let _sequence = self.sequence.lock().map_err(|_| SdkError::Closed)?;
        if self.closed.is_cancelled() {
            return Err(SdkError::Closed);
        }
        Ok(Arc::new(Subscription {
            receiver: AsyncMutex::new(self.sender.subscribe()),
            closed: self.closed.child_token(),
        }))
    }
    pub fn close(&self) {
        self.closed.cancel()
    }
}
impl EventSink for Events {
    fn emit(&self, event: CoreEvent) -> Result<(), EventError> {
        self.publish(event.name(), event.payload())
            .map_err(|e| EventError::Delivery(e.to_string()))
    }
}
#[derive(uniffi::Object)]
pub struct Subscription {
    receiver: AsyncMutex<broadcast::Receiver<Event>>,
    closed: CancellationToken,
}
#[uniffi::export]
impl Subscription {
    pub fn unsubscribe(&self) -> Result<(), SdkError> {
        self.closed.cancel();
        Ok(())
    }
    pub async fn next(&self) -> Result<StreamUpdate, SdkError> {
        tokio::select! {
            biased;
            _=self.closed.cancelled()=>Err(SdkError::Closed),
            result=async {self.receiver.lock().await.recv().await}=>match result{
                Ok(event)=>Ok(StreamUpdate::Event{event}),
                Err(broadcast::error::RecvError::Lagged(missed))=>Ok(StreamUpdate::SnapshotRequired{missed}),
                Err(broadcast::error::RecvError::Closed)=>Err(SdkError::Closed),
            }
        }
    }
}
