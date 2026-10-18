plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// The Core SDK built by `yarn sdk:android` (bindings/kotlin/README.md).
val coreAar = rootDir.resolve("../../bindings/kotlin/artifacts/isct-core.aar")

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
        versionCode = 1
        versionName = "1.0.0"
    }

    buildTypes {
        release {
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
    packaging {
        jniLibs.useLegacyPackaging = false
    }
}

dependencies {
    implementation(files(coreAar))
    implementation("net.java.dev.jna:jna:5.17.0@aar")
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
    implementation("androidx.navigation:navigation-compose:2.9.0")
    debugImplementation("androidx.compose.ui:ui-tooling")
}
