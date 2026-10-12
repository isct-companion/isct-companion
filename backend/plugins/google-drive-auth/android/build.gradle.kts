plugins { id("com.android.library"); id("org.jetbrains.kotlin.android") }
android {
    namespace = "app.isctcompanion.googleauth"
    compileSdk = 36
    defaultConfig { minSdk = 34 }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_1_8; targetCompatibility = JavaVersion.VERSION_1_8 }
    kotlinOptions { jvmTarget = "1.8" }
}
dependencies {
    implementation(project(":tauri-android"))
    implementation("com.google.android.gms:play-services-auth:22.0.0")
}
