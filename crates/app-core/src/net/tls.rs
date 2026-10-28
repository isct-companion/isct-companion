//! Install the process crypto provider used by Core's platform-verified TLS.
//!
//! Android initializes the platform verifier with the app Context before the
//! first handshake; the FFI crate exports that entry point.

use rustls::crypto::CryptoProvider;

/// Makes ring the process crypto provider unless one is already installed
/// (the desktop updater plugin installs the same one).
pub(super) fn install_provider() {
    if CryptoProvider::get_default().is_none() {
        // A concurrent install of the same provider is the only failure.
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}
