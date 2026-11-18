import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// Release signing comes from keystore.properties next to settings.gradle.kts (git-ignored):
// storeFile, storePassword, keyAlias, keyPassword. Without it, from the environment variables
// DREAME_KEYSTORE_FILE, DREAME_KEYSTORE_PASSWORD, DREAME_KEY_ALIAS and DREAME_KEY_PASSWORD
// (the release workflow sets them from secrets). Without either, the release build is signed
// with the debug key, so it still installs for testing.
val keystore = rootProject.file("keystore.properties").takeIf { it.isFile }?.let { f ->
    Properties().apply { f.inputStream().use { load(it) } }
} ?: System.getenv("DREAME_KEYSTORE_FILE")?.takeIf { it.isNotBlank() }?.let { file ->
    Properties().apply {
        setProperty("storeFile", file)
        setProperty("storePassword", System.getenv("DREAME_KEYSTORE_PASSWORD") ?: "")
        setProperty("keyAlias", System.getenv("DREAME_KEY_ALIAS") ?: "")
        // A PKCS12 key usually shares the store's password; a blank key password uses it.
        val keyPassword = System.getenv("DREAME_KEY_PASSWORD")?.takeIf { it.isNotBlank() }
        setProperty("keyPassword", keyPassword ?: System.getenv("DREAME_KEYSTORE_PASSWORD") ?: "")
    }
}

android {
    namespace = "dev.dreame.control"
    compileSdk = 36

    defaultConfig {
        applicationId = "dev.dreame.control"
        minSdk = 26
        targetSdk = 36
        // The release workflow sets both from the version tag; a local build is 1.4.0.
        versionCode = System.getenv("DREAME_VERSION_CODE")?.toIntOrNull() ?: 1
        versionName = System.getenv("DREAME_VERSION_NAME")?.takeIf { it.isNotBlank() } ?: "1.4.0"
    }

    signingConfigs {
        if (keystore != null) {
            create("release") {
                storeFile = rootProject.file(keystore.getProperty("storeFile"))
                storePassword = keystore.getProperty("storePassword")
                keyAlias = keystore.getProperty("keyAlias")
                keyPassword = keystore.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    packaging {
        // The Rust library is already stripped and optimised by cargo.
        jniLibs.useLegacyPackaging = false
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    // UniFFI's generated Kotlin calls the Rust library through JNA and exposes async functions
    // as suspend functions.
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")

    implementation("androidx.core:core-ktx:1.17.0")
    implementation("androidx.activity:activity-compose:1.12.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.4")
    implementation("androidx.compose.ui:ui:1.9.5")
    implementation("androidx.compose.foundation:foundation:1.9.5")
    implementation("androidx.compose.material3:material3:1.4.0")
    implementation("androidx.compose.material:material-icons-extended:1.7.8")
}
