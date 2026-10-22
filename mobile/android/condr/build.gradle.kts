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
}

val buildMobile =
  tasks.register<Exec>("buildMobile") {
    description = "Builds libcondr_mobile.so per ABI and generates the Kotlin bindings"
    workingDir = rootDir.parentFile.parentFile
    commandLine("script/build-mobile.sh", "android")
    val profile =
      System.getenv("CONDR_MOBILE_PROFILE") ?: if (project.hasProperty("release")) "release" else "debug"
    environment("CONDR_MOBILE_PROFILE", profile)
    // The profile is an input: a release build after a debug one rebuilds the library.
    inputs.property("profile", profile)
    inputs.dir(workingDir.resolve("crates"))
    inputs.dir(workingDir.resolve("proto"))
    outputs.dir(layout.projectDirectory.dir("src/main/jniLibs"))
    outputs.dir(layout.projectDirectory.dir("src/main/kotlin"))
  }

tasks.named("preBuild") { dependsOn(buildMobile) }

dependencies { api("${libs.jna.get()}@aar") }
