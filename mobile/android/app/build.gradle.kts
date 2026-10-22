plugins {
  alias(libs.plugins.android.application)
  alias(libs.plugins.kotlin.compose)
  alias(libs.plugins.kotlin.serialization)
  alias(libs.plugins.ktfmt)
}

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
  }

  buildFeatures { compose = true }

  buildTypes {
    release {
      isMinifyEnabled = true
      isShrinkResources = true
      proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
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
  debugImplementation(libs.compose.ui.tooling)
  debugImplementation(libs.compose.ui.test.manifest)
  testImplementation(libs.junit)
  androidTestImplementation(platform(libs.compose.bom))
  androidTestImplementation(libs.compose.ui.test.junit4)
  androidTestImplementation(libs.androidx.test.ext.junit)
  androidTestImplementation(libs.androidx.test.runner)
}
