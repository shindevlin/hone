plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "network.hone.app"
    compileSdk = 36

    defaultConfig {
        applicationId = "network.hone.app"   // HONE brand — matches the Rust JNI/uniffi package
        minSdk = 26                            // 8.0+ (foreground services, notification channels)
        targetSdk = 36
        versionCode = 11
        versionName = "1.1.0"
        // Commit shown in the signed device status.
        buildConfigField("String", "GIT_COMMIT", "\"" + (runCatching {
            ProcessBuilder("git", "rev-parse", "--short=8", "HEAD").directory(rootDir).start()
                .inputStream.bufferedReader().readText().trim()
        }.getOrNull()?.ifEmpty { null } ?: "unknown") + "\"")
        // The Rust core is loaded per-ABI as libhone_miner.so (Phase 0b/uniffi).
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
            // Release signing config is injected from the vault at build time —
            // NEVER commit a keystore. See docs/SECURE_CLOUD_BACKUP.md.
        }
        debug {
            applicationIdSuffix = ".debug"
            isDebuggable = true
        }
    }

    buildFeatures { compose = true; buildConfig = true }
    androidResources { noCompress += "tflite" }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
        // litert 2.3.2's Kotlin API jar carries Kotlin 2.4 metadata; we only call its Java
        // Interpreter API, so skip the metadata check rather than bump the project's Kotlin.
        freeCompilerArgs += "-Xskip-metadata-version-check"
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.10.00")
    implementation(composeBom)

    // Compose + Material 3
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended") // Material Symbols (Decision #3)
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.fragment:fragment-ktx:1.8.5") // FragmentActivity for BiometricPrompt
    implementation("androidx.navigation:navigation-compose:2.8.3")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")

    // Foreground service / background node (Phase 1)
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.work:work-runtime-ktx:2.9.1")

    // Biometric wallet unlock (Phase 2)
    implementation("androidx.biometric:biometric:1.1.0")
    // In-app reference calls (xeno-canto), streamed with a small phone cache.
    implementation("androidx.media3:media3-exoplayer:1.5.1")
    implementation("androidx.media3:media3-datasource:1.5.1")
    implementation("androidx.media3:media3-database:1.5.1")

    // QR generation for receive flow (Phase 2) — no copy-paste; show/scan a code.
    implementation("com.google.zxing:core:3.5.3")
    // uniffi runtime dep for the generated bindings (JNA), added in Phase 0b:
    implementation("net.java.dev.jna:jna:5.14.0@aar")
    // Ed25519 verify of HONE BLE announces (lightweight API only, no JCA provider).
    implementation("org.bouncycastle:bcprov-jdk18on:1.78.1")
    // H3 cells for the coverage map (ships an android-arm64 JNI build).
    implementation("com.uber:h3:4.5.1")
    // Play services fused location (high accuracy) when GMS is present; LocationManager otherwise.
    implementation("com.google.android.gms:play-services-location:21.3.0")

    // Perch v2 on-device (LiteRT, the TensorFlow Lite runtime).
    implementation("com.google.ai.edge.litert:litert:2.3.2")
    // In-app bird photos with capture evidence (focus distance, parallax, motion).
    implementation("androidx.camera:camera-camera2:1.3.4")
    implementation("androidx.camera:camera-lifecycle:1.3.4")
    implementation("androidx.camera:camera-view:1.3.4")
    // Bird maps: MapLibre (BSD-2) with OpenFreeMap tiles (no key).
    implementation("org.maplibre.gl:android-sdk:11.13.5")

    debugImplementation("androidx.compose.ui:ui-tooling")
    testImplementation("junit:junit:4.13.2")
}

// h3-java ships its Android JNI builds as jar resources, which AGP does not package as
// native libs. Copy them into a generated jniLibs dir; Cells loads them with
// H3Core.newSystemInstance(). The other platforms' binaries are dropped from the APK.
val h3Jni: Configuration by configurations.creating { isTransitive = false }
dependencies { h3Jni("com.uber:h3:4.5.1") }
val extractH3Jni by tasks.registering(Copy::class) {
    from({ zipTree(h3Jni.singleFile) }) {
        include("android-arm64/libh3-java.so", "android-arm/libh3-java.so")
        eachFile { path = path.replace("android-arm64/", "arm64-v8a/").replace("android-arm/", "armeabi-v7a/") }
        includeEmptyDirs = false
    }
    into(layout.buildDirectory.dir("h3jni"))
}
android {
    sourceSets["main"].jniLibs.srcDir(layout.buildDirectory.dir("h3jni"))
    packaging {
        resources.excludes += listOf(
            "android-arm/**", "android-arm64/**", "darwin-arm64/**", "darwin-x64/**",
            "linux-arm64/**", "linux-armv5/**", "linux-armv7/**", "linux-ppc64le/**",
            "linux-s390x/**", "linux-x64/**", "linux-x86/**", "windows-x64/**", "windows-x86/**",
            "META-INF/versions/9/OSGI-INF/MANIFEST.MF",
        )
    }
}
tasks.named("preBuild") { dependsOn(extractH3Jni) }
