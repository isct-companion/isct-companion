//! The tests over pages saved from a real account compile only when their
//! directory is given (`capture-gate`); a page missing from it fails its test.
fn main() {
    capture_gate::dir("kyomu_captures", "KYOMU_CAPTURES");
    capture_gate::dir("lms_module_captures", "LMS_MODULE_CAPTURES");
    capture_gate::dir("vpn_captures", "VPN_CAPTURES");
}
