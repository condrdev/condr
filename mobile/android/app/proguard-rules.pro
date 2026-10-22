# uniffi's bindings reach the Rust library through JNA by reflection (ADR 0039).
-keep class com.sun.jna.** { *; }
-keep class uniffi.condr_mobile.** { *; }
-dontwarn java.awt.**
