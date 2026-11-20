//! Whether the system has a usable network path (Network.framework's path
//! monitor), for telling "offline" from "no route to this host" in `failure`.
//! objc2 has no Network.framework binding, so the four C calls are declared
//! here.

use block2::RcBlock;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

static SATISFIED: AtomicBool = AtomicBool::new(false);

#[link(name = "Network", kind = "framework")]
unsafe extern "C" {
    fn nw_path_monitor_create() -> *mut c_void;
    fn nw_path_monitor_set_queue(monitor: *mut c_void, queue: *mut c_void);
    fn nw_path_monitor_set_update_handler(
        monitor: *mut c_void,
        handler: &block2::DynBlock<dyn Fn(*mut c_void)>,
    );
    fn nw_path_monitor_start(monitor: *mut c_void);
    fn nw_path_get_status(path: *mut c_void) -> i32;
}
unsafe extern "C" {
    fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
}

/// `nw_path_status_satisfied`.
const SATISFIED_STATUS: i32 = 1;
/// `QOS_CLASS_UTILITY`.
const UTILITY: isize = 0x11;

/// Starts the monitor once; it runs for the life of the app. Until its
/// first update the path counts as unsatisfied, which keeps WebKit's own
/// reading.
pub(super) fn watch() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let handler = RcBlock::new(|path: *mut c_void| {
            // SAFETY: Network.framework passes a live `nw_path_t`.
            let status = unsafe { nw_path_get_status(path) };
            SATISFIED.store(status == SATISFIED_STATUS, Ordering::Relaxed);
        });
        // SAFETY: plain Network.framework and libdispatch calls; the
        // monitor copies the handler and is never released.
        unsafe {
            let monitor = nw_path_monitor_create();
            if monitor.is_null() {
                return;
            }
            nw_path_monitor_set_queue(monitor, dispatch_get_global_queue(UTILITY, 0));
            nw_path_monitor_set_update_handler(monitor, &handler);
            nw_path_monitor_start(monitor);
        }
    });
}

pub(super) fn online() -> bool {
    SATISFIED.load(Ordering::Relaxed)
}
