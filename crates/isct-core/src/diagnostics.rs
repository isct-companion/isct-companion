//! Failures of background work that has no caller to return them to.
//!
//! Uses the project's stderr logging (as the native shells do), so automatic
//! work never fails silently. Callers with a result to return must return it.

pub(crate) fn report(scope: &str, error: &dyn std::fmt::Display) {
    eprintln!("[{scope}] {error}");
}
