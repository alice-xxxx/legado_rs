pluginManagement {
    val toolchain = java.util.Properties().apply {
        file("../toolchain.properties").inputStream().use { load(it) }
    }
    plugins {
        kotlin("multiplatform") version toolchain.getProperty("kotlin.version")
        kotlin("jvm") version toolchain.getProperty("kotlin.version")
        kotlin("plugin.serialization") version toolchain.getProperty("kotlin.version")
        id("com.google.devtools.ksp") version toolchain.getProperty("ksp.version")
        id("com.android.kotlin.multiplatform.library") version toolchain.getProperty("agp.version")
    }
    resolutionStrategy {
        eachPlugin {
            if (requested.id.id == "org.jetbrains.kotlin.jvm" ||
                requested.id.id == "org.jetbrains.kotlin.multiplatform") {
                useModule("org.jetbrains.kotlin:kotlin-gradle-plugin:${toolchain.getProperty("kotlin.version")}")
            }
        }
    }
    repositories {
        google()
        gradlePluginPortal()
        mavenCentral()
        maven { url = uri("https://jitpack.io") }
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        maven("https://jitpack.io")
    }
}

rootProject.name = "legado-source-engine-kotlin"

// 桌面 Rust/JNI 直接消费 KMP JVM 变体；根工程只保留桌面运行时和 QuickJS 准备任务。
include(":kmp-engine")
include(":quickjs-processor")
