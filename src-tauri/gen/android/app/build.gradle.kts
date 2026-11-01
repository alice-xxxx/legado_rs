import java.util.Properties
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

extensions.configure<com.android.build.api.dsl.ApplicationExtension> {
    ndkVersion = "30.0.14904198"
    buildToolsVersion = "36.0.0"
    // Source-engine dependencies include a JVM-only JAR index.
    packaging {
        resources {
            excludes += "META-INF/INDEX.LIST"
        }
    }

    compileSdk = 37
    namespace = "com.xyz.foliver"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.xyz.foliver"
        minSdk = 26
        targetSdk = 37
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {
                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            optimization {
               enable = true
            }
            proguardFiles(
                *fileTree(".") {
                  include("**/*.pro")
                  exclude("build/**")
                }.files.toTypedArray()
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.toVersion("11")
        targetCompatibility = JavaVersion.toVersion("11")
    }
    buildFeatures {
        buildConfig = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.fromTarget("11")
    }
}

// 修正 Tauri Android 模板中的过时 Kotlin API 调用。
// Resolve the file references at Gradle configuration time. Task.project is not
// accessible from doFirst with Gradle 9's configuration cache enabled.
val foliverGeneratedKotlinSources = listOf(
    layout.projectDirectory.file("src/main/java/com/xyz/foliver/generated/RustWebView.kt").asFile,
    layout.projectDirectory.file("src/main/java/com/xyz/foliver/generated/WryActivity.kt").asFile,
)
tasks.configureEach {
    if (name.startsWith("compile") && name.endsWith("Kotlin")) {
        doFirst {
            for (sourceFile in foliverGeneratedKotlinSources) {
                if (!sourceFile.isFile) continue
                val original = sourceFile.readText()
                val patched = original
                    .replace("settings.databaseEnabled = true", "")
                    .replace(
                        "this@WryActivity.onBackPressed()",
                        "this@WryActivity.onBackPressedDispatcher.onBackPressed()",
                    )
                if (patched != original) sourceFile.writeText(patched)
            }
        }
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("org.rustls:rustls-platform-verifier:0.2.0")
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = file("tauri.build.gradle.kts"))

extensions.configure<com.android.build.api.dsl.ApplicationExtension> {
    // Foliver supports Android arm64 only.
    defaultConfig {
        ndk { abiFilters.clear(); abiFilters.add("arm64-v8a") }
    }
}

extensions.configure<com.android.build.api.dsl.ApplicationExtension> {

    // Foliver release packaging and signing.
    val foliverKeystore = System.getenv("FOLIVER_ANDROID_KEYSTORE_PATH")
    if (!foliverKeystore.isNullOrBlank()) {
        signingConfigs.create("foliverRelease") {
            storeFile = file(foliverKeystore)
            storePassword = requireNotNull(System.getenv("FOLIVER_ANDROID_KEYSTORE_PASSWORD"))
            keyAlias = requireNotNull(System.getenv("FOLIVER_ANDROID_KEY_ALIAS"))
            keyPassword = requireNotNull(System.getenv("FOLIVER_ANDROID_KEY_PASSWORD"))
            storeType = "PKCS12"
        }
        buildTypes.getByName("release").signingConfig = signingConfigs.getByName("foliverRelease")
    }
    buildTypes.getByName("release") {
        // Source rules resolve public Java APIs by name. Preserve that contract;
        // native release optimization and symbol stripping do not affect it.
        optimization { enable = false }
        isMinifyEnabled = false
        isShrinkResources = false
    }
}
