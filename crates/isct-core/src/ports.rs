use crate::events::CoreEvent;
use thiserror::Error;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum EventError {
    #[error("{0}")]
    Delivery(String),
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: CoreEvent) -> Result<(), EventError>;
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SecretError {
    #[error("Secret account is not allowed.")]
    InvalidAccount,
    #[error("Secret payload is invalid.")]
    InvalidPayload,
    #[error("{0}")]
    Access(String),
}

pub trait SecretVault: Send + Sync {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError>;
    fn load(&self, account: &str) -> Result<Option<String>, SecretError>;
    fn delete(&self, account: &str) -> Result<(), SecretError>;
}
