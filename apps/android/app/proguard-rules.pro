# The Core SDK binds through JNA by name.
-keep class com.sun.jna.** { *; }
-keep class app.isctcompanion.core.** { *; }
-dontwarn java.awt.**
# rustls-platform-verifier's Kotlin half is reached only through JNI.
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }
# LiteRT is used only through its native library (Core opens libLiteRt.so);
# its Java classes refer to the excluded litert-api module and are removed.
-dontwarn org.tensorflow.lite.**
