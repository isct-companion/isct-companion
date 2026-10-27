//! Scripts Core injects into Institute of Science Tokyo pages. Core fills in
//! their configuration placeholders; shells only run what Core hands them.

pub const SCHOOL_LOGIN: &str = include_str!("../assets/school-login.js");
/// Runs in a view whose attempt Core abandoned, before the view confirms the stop.
pub const SCHOOL_STOP: &str = include_str!("../assets/school-stop.js");
pub const SCHOOL_LINKS: &str = include_str!("../assets/school-links.js");
pub const KYOMU_POPUP: &str = include_str!("../assets/kyomu-popup.js");
/// Android injects this support shim, while the shared bytes remain owned by Core.
pub const ANDROID_SUPPORT: &str = include_str!("../assets/android-school-support.js");
pub const OTP_ENROLLMENT: &str = include_str!("../assets/otp-enrollment.js");
pub const ASSIGNMENT_UPLOAD: &str = include_str!("../assets/assignment-upload.js");
