pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        // The JVM half of rustls-platform-verifier, which Core's TLS calls
        // through JNI (bindings/kotlin TlsVerifier). Published only here.
        maven {
            url = uri("https://github.com/rustls/rustls-platform-verifier/raw/maven-archive/android-release-support/maven/")
            content { includeGroup("org.rustls") }
        }
    }
}
rootProject.name = "IsctCompanion"
include(":app")
