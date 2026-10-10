plugins {
    id("com.android.library")
}

// Tauri includes this module as a subproject, so its settings.gradle is not evaluated. Gradle
// resolves this module's transitive dependencies from the consuming app's repositories.
rootProject.allprojects {
    repositories {
        maven("https://jitpack.io")
    }
}

android {
    namespace = "io.legado.sourceengine.tauri"
    compileSdk = 37

    defaultConfig {
        minSdk = 24

        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
}

dependencies {
    // Invoke.parseArgs exposes Jackson's TypeReference overload in its JVM signature.
    implementation("com.fasterxml.jackson.core:jackson-databind:2.15.3")
    // The KMP AAR is staged by the Android workflow; its non-transitive Maven dependencies are
    // repeated here because a raw local AAR does not carry its Gradle dependency graph.
    implementation(files("libs/kmp-engine.aar"))
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.11.0")
    implementation("org.jetbrains.kotlinx:atomicfu:0.33.0")
    implementation("com.squareup.okio:okio:3.18.1")
    implementation("androidx.room3:room3-common:3.0.1")
    implementation("androidx.room3:room3-runtime:3.0.1")
    implementation("com.fleeksoft.ksoup:ksoup:0.2.6")
    implementation("com.squareup.okhttp3:okhttp:5.4.0")
    implementation("com.github.liuyueyi.quick-chinese-transfer:quick-transfer-core:0.2.16")
    implementation("cn.hutool:hutool-crypto:5.8.22")
    implementation("io.coil-kt.coil3:coil-network-okhttp:3.4.0")
    implementation("org.nanohttpd:nanohttpd:2.3.1")
    implementation("org.nanohttpd:nanohttpd-websocket:2.3.1")
    implementation("androidx.documentfile:documentfile:1.1.0")
    implementation("androidx.core:core-ktx:1.18.0")
    implementation("androidx.annotation:annotation:1.9.1")
    implementation("androidx.collection:collection:1.6.0")
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("com.caverock:androidsvg-aar:1.4")
    implementation(project(":tauri-android"))
}
