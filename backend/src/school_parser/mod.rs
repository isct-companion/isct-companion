//! Pure parsing boundary for school-owned pages and extractor payloads.
//!
//! Code in this module must not open network connections, touch Tauri, or
//! write to SQLite. Acquisition, session recovery, downloads, and persistence
//! are deliberately kept in their own services.

pub(crate) mod kyomu;
pub(crate) mod lms;
pub(crate) mod syllabus;
