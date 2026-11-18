# UniFFI's generated Kotlin calls the Rust library through JNA, which finds classes, fields and
# callback methods by reflection. Keep them whole.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class uniffi.** { *; }
-dontwarn java.awt.**
-dontwarn com.sun.jna.**
