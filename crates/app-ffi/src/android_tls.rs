//! Android TLS setup. Core's HTTP verifies certificates with Android's trust
//! manager through rustls-platform-verifier, which calls into the JVM and needs
//! the app Context first. The SDK binds through JNA, which passes no JNIEnv, so
//! this is a plain JNI entry point for `app.isctcompanion.core.TlsVerifier`.

use jni::{
    errors::ThrowRuntimeExAndDefault,
    objects::{JClass, JObject},
    EnvUnowned,
};

/// `TlsVerifier.install(context)`. Idempotent; throws a RuntimeException in
/// Kotlin if the JVM handles cannot be stored.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_isctcompanion_core_TlsVerifier_install<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    context: JObject<'caller>,
) {
    env.with_env(|env| rustls_platform_verifier::android::init_with_env(env, context))
        .resolve::<ThrowRuntimeExAndDefault>()
}
