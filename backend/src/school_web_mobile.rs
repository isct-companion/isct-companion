//! Mobile contract for the desktop-only embedded school browser.
//!
//! Tauri's child-webview positioning and visibility APIs are desktop-only.
//! Keeping the command surface available lets the shared frontend query the
//! capability and fail predictably, while mobile pages can use their own
//! navigation implementation later.

use crate::lifecycle::SchoolSite;
use crate::school_auth::{SchoolAuthProfile, SchoolCredentials};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

const UNSUPPORTED: &str = "The embedded school browser is not available on mobile.";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolWebPolicy {
    pub restore_across_restarts: bool,
    pub idle_minutes: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct WebBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolWebResult {
    pub site: SchoolSite,
    pub url: String,
    pub phase: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestOutcome {
    pub site: SchoolSite,
    pub kind: String,
    pub state: &'static str,
    pub changed: usize,
    pub message: String,
}

#[derive(Clone, Default)]
pub struct SchoolWebController;

impl SchoolWebController {
    pub fn new() -> Self {
        Self
    }

    pub fn configure(&self, _app: &AppHandle, policy: SchoolWebPolicy) -> Result<bool, String> {
        let _ = (policy.restore_across_restarts, policy.idle_minutes);
        Ok(false)
    }

    pub fn set_bounds(&self, _app: &AppHandle, bounds: WebBounds) -> Result<bool, String> {
        let _ = (bounds.x, bounds.y, bounds.width, bounds.height);
        Ok(false)
    }

    pub fn hide_all(&self, _app: &AppHandle) -> Result<bool, String> {
        Ok(false)
    }

    pub fn reload(&self, _app: &AppHandle, _site: SchoolSite) -> Result<bool, String> {
        Err(UNSUPPORTED.into())
    }

    pub fn navigate(
        &self,
        _app: &AppHandle,
        _site: SchoolSite,
        _url: &str,
    ) -> Result<bool, String> {
        Err(UNSUPPORTED.into())
    }

    pub fn back(&self, _app: &AppHandle, _site: SchoolSite) -> Result<bool, String> {
        Ok(false)
    }

    pub fn reveal_download(&self, _app: &AppHandle, _path: &str) -> Result<bool, String> {
        Ok(false)
    }

    pub fn set_zoom(&self, _site: SchoolSite, _scale: f64) -> Result<bool, String> {
        Ok(false)
    }

    pub fn open(
        &self,
        _app: &AppHandle,
        _site: SchoolSite,
        _css: String,
        _javascript: String,
        _bounds: Option<WebBounds>,
        _target_url: Option<String>,
    ) -> Result<SchoolWebResult, String> {
        Err(UNSUPPORTED.into())
    }

    pub fn ensure_session(
        &self,
        _app: &AppHandle,
        _profile: SchoolAuthProfile,
    ) -> Result<bool, String> {
        Ok(false)
    }

    pub fn login_in_background(
        &self,
        _app: &AppHandle,
        _profile: SchoolAuthProfile,
        _credentials: SchoolCredentials,
    ) -> Result<SchoolWebResult, String> {
        Err(UNSUPPORTED.into())
    }

    pub fn set_guide(&self, items: Vec<crate::kyomu_guide::GuideItem>) -> Result<bool, String> {
        let _ = items;
        Ok(false)
    }

    pub fn pull(&self, _app: &AppHandle, _site: SchoolSite) -> Result<Vec<IngestOutcome>, String> {
        Err(UNSUPPORTED.into())
    }
}
