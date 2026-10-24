plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// The Core SDK built by `yarn sdk:android` (bindings/kotlin/README.md).
val coreAar = rootDir.resolve("../../bindings/kotlin/artifacts/isct-core.aar")

/** The Kotlin half of rustls-platform-verifier must match the Rust crate in
 * the SDK exactly, so its version is read from the workspace Cargo.lock
 * (the crate's documented Gradle setup). */
abstract class RustlsVersion : ValueSource<String, RustlsVersion.Params> {
    interface Params : ValueSourceParameters {
        val lockFile: RegularFileProperty
    }

    override fun obtain(): String {
        val lines = parameters.lockFile.get().asFile.readLines()
        val index = lines.indexOfFirst { it.trim() == "name = \"$CRATE\"" }
        return lines.drop(index + 1)
            .takeIf { index >= 0 }
            ?.firstOrNull { it.trimStart().startsWith("version = ") }
            ?.substringAfter('"')
            ?.substringBefore('"')
            ?.takeIf { it.isNotEmpty() }
            ?: error("$CRATE not found in Cargo.lock")
    }

    companion object {
        const val CRATE = "rustls-platform-verifier-android"
    }
}

val rustlsPlatformVerifierVersion = providers.of(RustlsVersion::class.java) {
    parameters.lockFile.set(rootProject.layout.projectDirectory.file("../../Cargo.lock"))
}

configurations.configureEach {
    resolutionStrategy.eachDependency {
        if (requested.group == "org.rustls" && requested.name == "rustls-platform-verifier") {
            useVersion(rustlsPlatformVerifierVersion.get())
            because("must match ${RustlsVersion.CRATE} in Cargo.lock")
        }
    }
}

android {
    namespace = "app.isctcompanion"
    compileSdk = 35
    // Strips the Core library's debug symbols when packaging.
    ndkVersion = "28.2.13676358"

    defaultConfig {
        applicationId = "app.isctcompanion"
        // The Core SDK's floor (docs/core-ffi.md).
        minSdk = 34
        targetSdk = 35
        // Release builds take both from the tag (.github/workflows/android-release.yml).
        versionCode = providers.gradleProperty("appVersionCode").orNull?.toInt() ?: 1
        versionName = providers.gradleProperty("appVersionName").orNull ?: "1.0.0"
    }

    // The release key lives only in CI secrets; without it, release builds
    // stay unsigned and the workflow ships the debug-signed APK instead.
    val releaseKeystore = providers.environmentVariable("ANDROID_KEYSTORE_FILE").orNull
    signingConfigs {
        if (releaseKeystore != null) {
            create("release") {
                storeFile = file(releaseKeystore)
                storePassword = providers.environmentVariable("ANDROID_KEYSTORE_PASSWORD").get()
                keyAlias = providers.environmentVariable("ANDROID_KEY_ALIAS").get()
                keyPassword = providers.environmentVariable("ANDROID_KEY_PASSWORD").get()
            }
        }
    }

    buildTypes {
        release {
            if (releaseKeystore != null) signingConfig = signingConfigs.getByName("release")
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        compose = true
    }
    // The desktop's UI catalogs (ja.json, zh.json), read by `Localizer`.
    sourceSets {
        getByName("main") { assets.srcDir("../../../resources/locales") }
    }
    packaging {
        jniLibs.useLegacyPackaging = false
    }
}

dependencies {
    implementation(files(coreAar))
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    // Versioned from Cargo.lock above.
    implementation("org.rustls:rustls-platform-verifier")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("androidx.annotation:annotation-jvm:1.9.1")

    implementation(platform("androidx.compose:compose-bom:2025.06.01"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.9.1")
    implementation("androidx.lifecycle:lifecycle-process:2.9.1")
    implementation("androidx.work:work-runtime-ktx:2.10.1")
    implementation("androidx.navigation:navigation-compose:2.9.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.1")
    // Pull to refresh around the school WebView, which Compose's nested
    // scrolling cannot see into.
    implementation("androidx.swiperefreshlayout:swiperefreshlayout:1.1.0")
    // Matrix live scanning: YUV frames for Core's camera session.
    implementation("androidx.camera:camera-camera2:1.4.2")
    implementation("androidx.camera:camera-lifecycle:1.4.2")
    // The live preview and tap-to-focus points, as the system camera shows them.
    implementation("androidx.camera:camera-view:1.4.2")
    // OTP QR codes: Google's scanner UI for the camera, ML Kit for screenshots.
    implementation("com.google.android.gms:play-services-code-scanner:16.1.0")
    implementation("com.google.mlkit:barcode-scanning:17.3.0")
    // Google Drive: the authorization API hands Core drive.appdata tokens.
    implementation("com.google.android.gms:play-services-auth:21.4.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-play-services:1.10.2")
    debugImplementation("androidx.compose.ui:ui-tooling")
}
