plugins { id("com.android.library"); id("org.jetbrains.kotlin.android") }
android {
    namespace = "app.isctcompanion.schoolweb"
    compileSdk = 36
    defaultConfig { minSdk = 34; consumerProguardFiles("consumer-rules.pro") }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_1_8; targetCompatibility = JavaVersion.VERSION_1_8 }
    kotlinOptions { jvmTarget = "1.8" }
}
dependencies {
    testImplementation("junit:junit:4.13.2")
    implementation(project(":tauri-android"))
    implementation("androidx.core:core-ktx:1.9.0")
}
