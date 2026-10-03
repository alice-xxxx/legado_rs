import java.util.zip.ZipEntry
import java.util.zip.ZipFile
import java.util.zip.ZipOutputStream

plugins {
    kotlin("multiplatform") version "2.3.20" apply false
    kotlin("jvm") version "2.3.20" apply false
    kotlin("plugin.serialization") version "2.3.20" apply false
    id("com.google.devtools.ksp") version "2.3.11" apply false
    id("androidx.room3") version "3.0.1" apply false
    id("com.android.kotlin.multiplatform.library") version "9.4.1" apply false
    id("java-library")
}

// Kotlin/Native 的 JS 绑定由 KSP 在构建期生成；复制处理器源码到本独立工程，
// 避免编译时再读取原平台仓库或依赖原平台 Gradle project。

dependencies {
    // 桌面宿主直接依赖 KMP 的 JVM 变体；Rust 通过 JNI 调用这份 KMP 编译产物。
    implementation(project(":kmp-engine"))
}

val sourceEngineRuntimeClasspath = sourceSets.main.get().runtimeClasspath
val hostOs = when {
    System.getProperty("os.name").lowercase().contains("win") -> "windows"
    System.getProperty("os.name").lowercase().contains("mac") -> "macos"
    else -> "linux"
}
val hostArch = when (System.getProperty("os.arch").lowercase()) {
    "amd64", "x86_64", "x64" -> "x86_64"
    "aarch64", "arm64" -> "aarch64"
    else -> error("Unsupported QuickJS host architecture: ${System.getProperty("os.arch")}")
}
val hostPlatform = "$hostOs-$hostArch"
val quickJsFileName = when (hostOs) {
    "windows" -> "legado_quickjs.dll"
    "macos" -> "liblegado_quickjs.dylib"
    else -> "liblegado_quickjs.so"
}
val preparedQuickJsLibrary = layout.buildDirectory.file("native-jvm/$hostPlatform/$quickJsFileName").get().asFile
val quickJsCppSources = layout.projectDirectory.dir("src/native/cpp").asFile
val quickJsNgSources = layout.projectDirectory.dir("src/native/quickjs-ng").asFile
val java21Launcher = javaToolchains.launcherFor {
    languageVersion.set(JavaLanguageVersion.of(21))
}
val java21Installation = java21Launcher.map { it.metadata.installationPath.asFile }
val java21JlinkName = if (hostOs == "windows") "jlink.exe" else "jlink"

// `jdeps --multi-release base --recursive --class-path 'build/source-engine/runtime/lib/*'
// --print-module-deps build/source-engine/runtime/lib/*.jar` on the staged engine classpath
// requires these modules. The additional service modules preserve behavior that static analysis
// cannot see: extended source charsets, JCA EC providers, full locale data and JNDI DNS lookup.
val desktopJvmRuntimeModules = listOf(
    "java.base",
    "java.compiler",
    "java.desktop",
    "java.instrument",
    "java.logging",
    "java.management",
    "java.naming",
    "java.sql",
    "jdk.charsets",
    "jdk.crypto.ec",
    "jdk.localedata",
    "jdk.naming.dns",
    "jdk.unsupported",
)
/** Gradle 9 移除了 Project.exec；用参数列表启动工具，避免路径含空格时被 shell 拆分。 */
fun runNativeBuildTool(arguments: List<String>, environment: Map<String, String> = emptyMap()) {
    val processBuilder = ProcessBuilder(arguments)
    processBuilder.environment().putAll(environment)
    processBuilder.inheritIO()
    val exitCode = processBuilder.start().waitFor()
    check(exitCode == 0) {
        "Native build command failed with exit code $exitCode: ${arguments.joinToString(" ")}"
    }
}

/**
 * Build QuickJS JNI from the native sources shipped with this module.
 * CI and local builds therefore use the same source inputs instead of a machine-specific DLL.
 */
tasks.register("prepareQuickJsNative") {
    group = "build"
    description = "Prepare the standalone QuickJS JNI library for the current host"
    inputs.dir(quickJsCppSources)
    inputs.dir(quickJsNgSources)
    inputs.property("hostPlatform", hostPlatform)
    inputs.property("javaToolchainHome", java21Installation.map { it.absolutePath })
    inputs.property("javaToolchainLanguageVersion", java21Launcher.map { it.metadata.languageVersion.toString() })
    inputs.property("cmakeGenerator", System.getenv("CMAKE_GENERATOR").orEmpty())
    inputs.property("cCompiler", System.getenv("CC").orEmpty())
    inputs.property("cxxCompiler", System.getenv("CXX").orEmpty())
    inputs.file(java21Installation.map { it.resolve("release") })
    inputs.dir(java21Installation.map { it.resolve("include") })
    outputs.file(preparedQuickJsLibrary)

    doLast {
        preparedQuickJsLibrary.parentFile.mkdirs()
        val javaHome = java21Launcher.get().metadata.installationPath.asFile
        // CMake caches its selected generator in CMakeCache.txt. Keep generator-specific build
        // directories so changing from a local IDE generator to CI's Ninja cannot reuse stale state.
        val generatorSuffix = System.getenv("CMAKE_GENERATOR")
            ?.lowercase()
            ?.replace(Regex("[^a-z0-9]+"), "-")
            ?.trim('-')
            ?.takeIf(String::isNotEmpty)
            ?.let { "-$it" }
            .orEmpty()
        val cmakeBuildDir = layout.buildDirectory
            .dir("cmake-quickjs/$hostPlatform$generatorSuffix")
            .get()
            .asFile
        cmakeBuildDir.parentFile.mkdirs()

        runNativeBuildTool(
            listOf(
                "cmake",
                "-S", quickJsCppSources.absolutePath,
                "-B", cmakeBuildDir.absolutePath,
                "-DJDK_INCLUDE_DIR=${javaHome.resolve("include").absolutePath}",
                "-DCMAKE_BUILD_TYPE=Release",
                "-DCMAKE_LIBRARY_OUTPUT_DIRECTORY=${preparedQuickJsLibrary.parentFile.absolutePath}",
                "-DCMAKE_RUNTIME_OUTPUT_DIRECTORY=${preparedQuickJsLibrary.parentFile.absolutePath}",
                "-DCMAKE_LIBRARY_OUTPUT_DIRECTORY_RELEASE=${preparedQuickJsLibrary.parentFile.absolutePath}",
                "-DCMAKE_RUNTIME_OUTPUT_DIRECTORY_RELEASE=${preparedQuickJsLibrary.parentFile.absolutePath}",
            ),
            environment = mapOf("JAVA_HOME" to javaHome.absolutePath),
        )
        runNativeBuildTool(
            listOf(
                "cmake", "--build", cmakeBuildDir.absolutePath,
                "--config", "Release", "--target", "legado_quickjs", "--parallel",
            ),
        )
        check(preparedQuickJsLibrary.isFile) {
            "CMake completed without producing ${preparedQuickJsLibrary.absolutePath}"
        }
    }
}

val desktopJvmRuntimeDirectory = layout.buildDirectory.dir("source-engine/runtime")

/**
 * Emit the exact JVM classpath consumed by Rust. Maven dependencies may live in Gradle's cache,
 * while no runtime entry may point back into the original platform checkout outside this extraction.
 */
tasks.register("prepareDesktopJvmRuntime") {
    group = "application"
    description = "Stage the embedded desktop JVM, Kotlin classes, dependencies, and QuickJS library"
    dependsOn(tasks.named("classes"), tasks.named("prepareQuickJsNative"), ":kmp-engine:jvmJar")
    inputs.files(sourceEngineRuntimeClasspath)
    inputs.file(preparedQuickJsLibrary)
    inputs.property("hostPlatform", hostPlatform)
    inputs.property("javaToolchainHome", java21Installation.map { it.absolutePath })
    inputs.property("javaToolchainLanguageVersion", java21Launcher.map { it.metadata.languageVersion.toString() })
    inputs.property("jlinkModules", desktopJvmRuntimeModules.joinToString(","))
    inputs.file(java21Installation.map { it.resolve("release") })
    inputs.file(java21Installation.map { it.resolve("bin/$java21JlinkName") })
    inputs.dir(java21Installation.map { it.resolve("jmods") })
    outputs.dir(desktopJvmRuntimeDirectory)

    doLast {
        val extractionRoot = projectDir.parentFile.toPath().toAbsolutePath().normalize()
        val originalProjectRoot = extractionRoot.parent
        val runtimeEntries = sourceEngineRuntimeClasspath.files.filter(File::exists)
        val originalProjectEntries = runtimeEntries.filter { entry ->
            val path = entry.toPath().toAbsolutePath().normalize()
            path.startsWith(originalProjectRoot) && !path.startsWith(extractionRoot)
        }
        check(originalProjectEntries.isEmpty()) {
            "Kotlin runtime references files from the original platform checkout: " +
                originalProjectEntries.joinToString { it.absolutePath }
        }

        val runtimeRoot = desktopJvmRuntimeDirectory.get().asFile
        runtimeRoot.deleteRecursively()
        val librariesDirectory = runtimeRoot.resolve("lib")
        val nativeDirectory = runtimeRoot.resolve("native")
        librariesDirectory.mkdirs()
        nativeDirectory.mkdirs()

        // Package each Gradle runtime entry beside the app. The Gradle cache and extraction
        // checkout are build-time inputs only; an installed app must not use either path.
        val stagedEntries = runtimeEntries.mapIndexed { index, entry ->
            val stagedName = "%03d-%s".format(index, entry.name)
            val stagedEntry = librariesDirectory.resolve(stagedName)
            if (entry.isDirectory) {
                entry.copyRecursively(stagedEntry, overwrite = true)
                check(stagedEntry.isDirectory) {
                    "Could not stage Kotlin runtime directory: ${entry.absolutePath}"
                }
            } else {
                check(entry.copyTo(stagedEntry, overwrite = true).isFile) {
                    "Could not stage Kotlin runtime artifact: ${entry.absolutePath}"
                }
            }
            "lib/$stagedName"
        }
        runtimeRoot.resolve("classpath.txt").writeText(stagedEntries.joinToString("\n"), Charsets.UTF_8)

        val stagedQuickJs = nativeDirectory.resolve(quickJsFileName)
        check(preparedQuickJsLibrary.copyTo(stagedQuickJs, overwrite = true).isFile) {
            "Could not stage QuickJS JNI library: ${preparedQuickJsLibrary.absolutePath}"
        }

        // The app embeds HotSpot through JNI, so bundle a matching Java runtime instead of
        // requiring a user-installed JDK or relying on machine-specific Gradle toolchains.
        val javaHome = java21Installation.get()
        val jlink = javaHome.resolve("bin/$java21JlinkName")
        check(jlink.isFile) { "Java 21 jlink executable is missing: ${jlink.absolutePath}" }
        runNativeBuildTool(
            listOf(
                jlink.absolutePath,
                "--add-modules", desktopJvmRuntimeModules.joinToString(","),
                "--strip-debug", "--no-man-pages", "--no-header-files", "--compress=2",
                "--output", runtimeRoot.resolve("jre").absolutePath,
            ),
        )
        val javaName = if (hostOs == "windows") "java.exe" else "java"
        check(runtimeRoot.resolve("jre/bin/$javaName").isFile) {
            "jlink completed without creating the bundled Java runtime"
        }
    }
}

val stagedTauriAndroidAar = projectDir.parentFile.resolve(
    "src-tauri/plugins/source-engine/android/libs/kmp-engine.aar",
)
tasks.register("stageTauriAndroidAar") {
    group = "application"
    description = "Stage the KMP Android engine AAR for the Tauri mobile plugin"
    dependsOn(":kmp-engine:bundleAndroidMainAar")
    inputs.file(projectDir.resolve("kmp-engine/build/outputs/aar/kmp-engine.aar"))
    outputs.file(stagedTauriAndroidAar)

    doLast {
        val sourceAar = projectDir.resolve("kmp-engine/build/outputs/aar/kmp-engine.aar")
        check(sourceAar.isFile) { "KMP Android AAR was not produced: ${sourceAar.absolutePath}" }
        stagedTauriAndroidAar.parentFile.mkdirs()

        // The standalone AAR contains the same Rust crate that Tauri already loads as its Android
        // app library. Keep QuickJS JNI in the AAR, but omit the duplicate Rust .so to avoid APK
        // merge collisions and ensure Kotlin callbacks enter the app's single Rust Host instance.
        ZipFile(sourceAar).use { input ->
            ZipOutputStream(stagedTauriAndroidAar.outputStream().buffered()).use { output ->
                input.entries().asSequence()
                    .filterNot { it.name.startsWith("jni/") && it.name.endsWith("/liblegado_lib.so") }
                    .forEach { entry ->
                        output.putNextEntry(ZipEntry(entry.name))
                        input.getInputStream(entry).use { it.copyTo(output) }
                        output.closeEntry()
                    }
            }
        }
        check(stagedTauriAndroidAar.isFile && stagedTauriAndroidAar.length() > 0L) {
            "Could not stage the KMP Android AAR for Tauri"
        }
    }
}
