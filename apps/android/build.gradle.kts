plugins {
    id("com.android.application") version "9.5.0" apply false
    // AGP compiles Kotlin itself (built-in Kotlin); declaring the plugin here
    // only picks the Kotlin Gradle plugin version, newer than AGP's own.
    id("org.jetbrains.kotlin.android") version "2.4.21" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.21" apply false
}
