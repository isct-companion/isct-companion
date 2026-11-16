//! The feedback form: a Google Form for bug reports and suggestions, opened in
//! the system browser from Settings on every platform.
//!
//! Respondents sign in to Google (the form collects verified email
//! addresses), so every response names its sender and anonymous spam has no
//! way in. The form belongs to a maintainer's Google account. To hand it over,
//! add the next maintainer as an editor and transfer ownership in Forms: the
//! address below stays the same, so released apps keep working. Never delete
//! the form, since every released build opens this address. A replacement
//! form needs its own `entry.` ids here (Forms › Get pre-filled link).

use url::form_urlencoded;

const FORM: &str = "https://docs.google.com/forms/d/e/1FAIpQLSdv-AVezMP1aiFZWsOBHIlZErWGUVLniKB53Lr5V1afq4l5mA/viewform";
/// The "App version" and "Device" questions, filled in for the person.
const VERSION_ENTRY: &str = "entry.1396003434";
const DEVICE_ENTRY: &str = "entry.2128236758";

/// The form's address with this build's version and device filled in.
pub fn form_url(version: &str, device: &str) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("usp", "pp_url")
        .append_pair(VERSION_ENTRY, version)
        .append_pair(DEVICE_ENTRY, device)
        .finish();
    format!("{FORM}?{query}")
}
