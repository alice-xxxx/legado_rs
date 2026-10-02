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
    outputs.file(preparedQuickJsLibrary)

    doLast {
        preparedQuickJsLibrary.parentFile.mkdirs()
        val javaHome = java21Launcher.get().metadata.installationPath.asFile
        val cmakeBuildDir = layout.buildDirectory.dir("cmake-quickjs/$hostPlatform").get().asFile
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

val desktopJvmClasspathFile = layout.buildDirectory.file("source-engine/classpath.txt")

/**
 * Emit the exact JVM classpath consumed by Rust. Maven dependencies may live in Gradle's cache,
 * while no runtime entry may point back into the original platform checkout outside this extraction.
 */
tasks.register("prepareDesktopJvmRuntime") {
    group = "application"
    description = "Prepare the embedded desktop JVM runtime for the Rust/JNI source-engine host"
    dependsOn(tasks.named("classes"), tasks.named("prepareQuickJsNative"), ":kmp-engine:jvmJar")
    inputs.files(sourceEngineRuntimeClasspath)
    outputs.file(desktopJvmClasspathFile)

    doLast {
        val extractionRoot = projectDir.parentFile.toPath().toAbsolutePath().normalize()
        val originalProjectRoot = extractionRoot.parent
        val runtimeEntries = sourceEngineRuntimeClasspath.files
        val originalProjectEntries = runtimeEntries.filter { entry ->
            val path = entry.toPath().toAbsolutePath().normalize()
            path.startsWith(originalProjectRoot) && !path.startsWith(extractionRoot)
        }
        check(originalProjectEntries.isEmpty()) {
            "Kotlin runtime references files from the original platform checkout: " +
                originalProjectEntries.joinToString { it.absolutePath }
        }

        val output = desktopJvmClasspathFile.get().asFile
        output.parentFile.mkdirs()
        output.writeText(sourceEngineRuntimeClasspath.asPath, Charsets.UTF_8)
    }
}
