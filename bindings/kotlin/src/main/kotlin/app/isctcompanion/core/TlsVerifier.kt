package app.isctcompanion.core

import android.content.Context

/** Core verifies TLS with Android's trust manager (rustls-platform-verifier),
 * which it reaches through JNI. Call [install] once with the application
 * context before opening a `Session`; until then every HTTPS request Core
 * makes fails. Needs the `org.rustls:rustls-platform-verifier` AAR on the
 * app's classpath (apps/android/app/build.gradle.kts). */
object TlsVerifier {
    init {
        System.loadLibrary("isct_ffi")
    }

    @JvmStatic
    external fun install(context: Context)
}
