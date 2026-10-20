# The Core SDK binds through JNA by name.
-keep class com.sun.jna.** { *; }
-keep class app.isctcompanion.core.** { *; }
-dontwarn java.awt.**
# rustls-platform-verifier's Kotlin half is reached only through JNI.
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }
