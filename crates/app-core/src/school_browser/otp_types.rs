use serde::{Deserialize, Serialize};

/// The visible enrollment, without any credential material.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolOtpEnrollment {
    pub id: u64,
    pub stage: SchoolOtpStage,
    pub owner: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolOtpStage {
    SignIn,
    Checking,
    ConfirmReset,
    SettingUp,
    Saving,
    Complete,
    AccountMismatch,
    Incomplete,
    Unknown,
    SaveFailed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolOtpIntent {
    Reset,
    Stop,
    RetrySave,
}
