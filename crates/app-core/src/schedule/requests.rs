use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct ColorCourse {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub id: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub code: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub color: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub overrides: Option<ColorOverrides>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct ColorOverrides {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub color: Option<String>,
}
