// The one Rust library the app links and its uniffi Kotlin bindings (ADR 0039).
// script/build-mobile.sh android fills src/main/jniLibs and src/main/kotlin; neither is
// committed, and this module runs the script before every build so the app never links a
// stale library. Cargo's incremental build makes the unchanged case cheap.
plugins { alias(libs.plugins.android.library) }

android {
    namespace = "uniffi.condr_mobile"
    compileSdk = 37
    compileSdkMinor = 2
    defaultConfig { minSdk = 29 }
    // Gradle must not strip the Rust library's symbols with its own tools: cargo-ndk
    // already produced what release profile asked for.
    packaging { jniLibs.keepDebugSymbols += "**/libcondr_mobile.so" }
}

val buildMobile =
    tasks.register<Exec>("buildMobile") {
        description = "Builds libcondr_mobile.so per ABI and generates the Kotlin bindings"
        workingDir = rootDir.parentFile.parentFile
        commandLine("script/build-mobile.sh", "android")
        if (System.getenv("CONDR_MOBILE_PROFILE") == null) {
            environment("CONDR_MOBILE_PROFILE", if (project.hasProperty("release")) "release" else "debug")
        }
        inputs.dir(workingDir.resolve("crates"))
        inputs.dir(workingDir.resolve("proto"))
        outputs.dir(layout.projectDirectory.dir("src/main/jniLibs"))
        outputs.dir(layout.projectDirectory.dir("src/main/kotlin"))
    }

tasks.named("preBuild") { dependsOn(buildMobile) }

dependencies { api("${libs.jna.get()}@aar") }
