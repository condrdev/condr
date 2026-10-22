plugins {
  alias(libs.plugins.android.application)
  alias(libs.plugins.kotlin.compose)
  alias(libs.plugins.kotlin.serialization)
  alias(libs.plugins.ktfmt)
  alias(libs.plugins.google.services) apply false
}

// Firebase's configuration comes from google-services.json, which stays out of the
// repository; a checkout without it still builds, and its app reports pushes unavailable.
if (file("google-services.json").exists())
  apply(plugin = libs.plugins.google.services.get().pluginId)

android {
  namespace = "dev.condr.companion"
  // Compiled against 37.2, which the Compose 1.13 betas need; targets 36, which Google Play
  // requires of new apps since 2026-08-31; the floor stays at 29
  // (ADR 0039, ADR 0048).
  compileSdk = 37
  compileSdkMinor = 2

  defaultConfig {
    applicationId = "dev.condr.companion"
    minSdk = 29
    targetSdk = 36
    // The iOS app's marketing version; CI passes the run number (ADR 0048).
    versionName = "0.1"
    versionCode = (project.findProperty("versionCode") as String?)?.toInt() ?: 1
    testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    // The pairing test against a real Server runs only when an Invite is passed:
    // -Pandroid.testInstrumentationRunnerArguments.invite='tcp://…'
    if (!project.hasProperty("android.testInstrumentationRunnerArguments.invite")) {
      testInstrumentationRunnerArguments["notClass"] = "dev.condr.companion.LivePairingTest"
    }
    // The two ABIs the library is built for (ADR 0039); JNA's other ones stay out.
    ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
  }

  buildFeatures {
    compose = true
    buildConfig = true
  }

  // The upload key (ADR 0048): CI passes the keystore and its passwords through the
  // environment; a build without them is signed with the debug key, as a local one is.
  val keystore = System.getenv("ANDROID_KEYSTORE_FILE")?.let(::file)?.takeIf { it.exists() }
  if (keystore != null) {
    signingConfigs.create("upload") {
      storeFile = keystore
      storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
      keyAlias = "condr-upload"
      keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
    }
  }

  buildTypes {
    release {
      isMinifyEnabled = true
      isShrinkResources = true
      proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
      signingConfig =
        if (keystore != null) signingConfigs.getByName("upload")
        else signingConfigs.getByName("debug")
    }
  }

  compileOptions {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
  }
}

ktfmt { googleStyle() }

dependencies {
  implementation(project(":condr"))
  implementation(platform(libs.compose.bom))
  implementation(libs.compose.ui)
  implementation(libs.compose.ui.graphics)
  implementation(libs.compose.ui.tooling.preview)
  implementation(libs.compose.material3)
  implementation(libs.compose.material.icons.core)
  implementation(libs.activity.compose)
  implementation(libs.navigation3.runtime)
  implementation(libs.navigation3.ui)
  implementation(libs.lifecycle.process)
  implementation(libs.lifecycle.runtime.compose)
  implementation(libs.core.ktx)
  implementation(libs.core.splashscreen)
  implementation(libs.datastore)
  implementation(libs.datastore.preferences)
  implementation(libs.biometric)
  implementation(libs.serialization.json)
  implementation(libs.coroutines.android)
  implementation(libs.play.services.code.scanner)
  implementation(platform(libs.firebase.bom))
  implementation(libs.firebase.messaging)
  debugImplementation(libs.compose.ui.tooling)
  debugImplementation(libs.compose.ui.test.manifest)
  testImplementation(libs.junit)
  testImplementation(libs.coroutines.test)
  androidTestImplementation(platform(libs.compose.bom))
  androidTestImplementation(libs.compose.ui.test.junit4)
  androidTestImplementation(libs.androidx.test.ext.junit)
  androidTestImplementation(libs.androidx.test.runner)
  androidTestImplementation(libs.androidx.test.core)
}
