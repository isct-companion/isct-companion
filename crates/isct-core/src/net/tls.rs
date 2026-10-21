//! TLS for Core's clients: the platform verifier everywhere, plus the missing
//! intermediate for 教務Web.
//!
//! kyomu0 sends the NII Open Domain CA G8 intermediate although its leaf is
//! signed by G7. Browsers and Apple/Windows verification fetch or cache the
//! right intermediate; Android's trust manager and Linux webpki do not. So the
//! Kyomu client offers the vendored G7 intermediate (expires 2029-05-29) to the
//! platform verifier as one more *untrusted* chain certificate. Chain building
//! on every platform (SecTrust, CertGetCertificateChain, Android's
//! TrustManagerImpl, webpki) ignores the unrelated G8 and still has to reach an
//! OS trust anchor, so trust is not widened to the intermediate. This is used
//! instead of reqwest's `tls_certs_merge`/extra roots, which would make G7 a
//! trust anchor and which rustls-platform-verifier does not support on Android.
//!
//! Android: the platform verifier calls the system trust manager through JNI
//! and must be initialized with the app Context before the first handshake;
//! the FFI crate exports that entry point and the app calls it at startup.

use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{pem::PemObject, CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, SignatureScheme,
};
use std::sync::Arc;

const KYOMU_INTERMEDIATE: &[u8] = include_bytes!("certs/nii-odca4g7rsa.pem");

/// Makes ring the process crypto provider unless one is already installed
/// (the desktop updater plugin installs the same one).
pub(super) fn install_provider() {
    if CryptoProvider::get_default().is_none() {
        // A concurrent install of the same provider is the only failure.
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

fn provider() -> Arc<CryptoProvider> {
    install_provider();
    CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()))
}

/// A client config for kyomu0: the platform verifier with the G7 intermediate.
pub(super) fn kyomu_config() -> Result<rustls::ClientConfig, String> {
    let provider = provider();
    let intermediate = CertificateDer::from_pem_slice(KYOMU_INTERMEDIATE)
        .map_err(|error| format!("vendored intermediate: {error}"))?;
    let platform = rustls_platform_verifier::Verifier::new(provider.clone())
        .map_err(|error| format!("platform verifier: {error}"))?;
    let verifier = WithIntermediate {
        platform,
        intermediate,
    };
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| format!("TLS versions: {error}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth())
}

#[derive(Debug)]
struct WithIntermediate {
    platform: rustls_platform_verifier::Verifier,
    intermediate: CertificateDer<'static>,
}

impl ServerCertVerifier for WithIntermediate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let mut chain = intermediates.to_vec();
        if !chain
            .iter()
            .any(|cert| cert.as_ref() == self.intermediate.as_ref())
        {
            chain.push(self.intermediate.clone());
        }
        self.platform
            .verify_server_cert(end_entity, &chain, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.platform.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.platform.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.platform.supported_verify_schemes()
    }
}
