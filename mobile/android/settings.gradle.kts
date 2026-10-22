// The Companion's Android app (ADR 0039, ADR 0048): `app` is the app, `condr` the Rust
// library and its Kotlin bindings that script/build-mobile.sh fills before every build.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode = RepositoriesMode.FAIL_ON_PROJECT_REPOS
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "condr-companion"

include(":app", ":condr")
