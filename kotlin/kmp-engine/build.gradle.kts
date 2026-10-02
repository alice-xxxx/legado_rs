plugins {
    kotlin("multiplatform")
    kotlin("plugin.serialization")
    id("com.google.devtools.ksp")
    id("androidx.room3")
    id("com.android.kotlin.multiplatform.library")
}

val isAppleHost = System.getProperty("os.name").startsWith("Mac", ignoreCase = true)
val nativeInteropPatterns = listOf(
    "io/legado/app/model/script/*.native.kt",
    "io/legado/app/help/crypto/MbedTls*.native.kt",
    "io/legado/app/help/crypto/MbedTlsOps.native.kt",
)
val nativeInteropOutput = layout.buildDirectory.dir("generated/nativeInterop/iosLeaf")
val androidJniLibsOutput = layout.buildDirectory.dir("generated/androidJniLibs")
val stageNativeInteropForIos = tasks.register<Sync>("stageNativeInteropForIos") {
    from("src/nativeMain/kotlin") { include(*nativeInteropPatterns.toTypedArray()) }
    into(nativeInteropOutput)
    doLast {
        val root = nativeInteropOutput.get().asFile
        val aliases = listOf(
            "io/legado/app/napi/quickjs/CNamesAliases.kt" to """
                @file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)

                package io.legado.app.napi.quickjs

                typealias JSContext = cnames.structs.JSContext
                typealias JSRuntime = cnames.structs.JSRuntime
            """.trimIndent(),
            "io/legado/app/nativecrypto/mbedtls/CNamesAliases.kt" to """
                @file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)

                package io.legado.app.nativecrypto.mbedtls

                typealias mbedtls_md_info_t = cnames.structs.mbedtls_md_info_t
            """.trimIndent(),
        )
        aliases.forEach { (relative, text) ->
            val output = root.resolve(relative)
            output.parentFile.mkdirs()
            output.writeText("$text\n", Charsets.UTF_8)
        }
    }
}

val iosRustTargets = mapOf(
    "buildRustIosArm64" to "aarch64-apple-ios",
    "buildRustIosSimulatorArm64" to "aarch64-apple-ios-sim",
)
iosRustTargets.forEach { (taskName, rustTarget) ->
    tasks.register<Exec>(taskName) {
        val rustRoot = projectDir.resolve("../../src-tauri").canonicalFile
        workingDir(rustRoot)
        commandLine("cargo", "build", "--release", "--lib", "--no-default-features", "--target", rustTarget)
        inputs.files(rustRoot.resolve("Cargo.toml"), rustRoot.resolve("Cargo.lock"))
        inputs.dir(rustRoot.resolve("src"))
        outputs.file(rustRoot.resolve("target/$rustTarget/release/liblegado_lib.a"))
    }
}

val buildIosNativeQuickJs = tasks.register<Exec>("buildIosNativeQuickJs") {
    val script = projectDir.resolve("scripts/build-ios-native.sh")
    workingDir(projectDir)
    commandLine("bash", script.absolutePath)
    inputs.dir("../src/native/quickjs-ng")
    inputs.dir("src/nativeInterop/cinterop/mbedtls")
    outputs.dir(layout.buildDirectory.dir("iosNativeLibs"))
}

data class AndroidRustTarget(val abi: String, val triple: String, val clangPrefix: String)

val androidRustTargets = listOf(
    AndroidRustTarget("arm64-v8a", "aarch64-linux-android", "aarch64-linux-android"),
    AndroidRustTarget("armeabi-v7a", "armv7-linux-androideabi", "armv7a-linux-androideabi"),
    AndroidRustTarget("x86", "i686-linux-android", "i686-linux-android"),
    AndroidRustTarget("x86_64", "x86_64-linux-android", "x86_64-linux-android"),
)
val rustRoot = projectDir.resolve("../../src-tauri").canonicalFile
val quickJsAndroidRoot = projectDir.resolve("../src/native/cpp").canonicalFile
val quickJsAndroidSources = projectDir.resolve("../src/native/quickjs-ng").canonicalFile

fun locateAndroidNdk(): File {
    val sdkCandidates = listOfNotNull(
        providers.gradleProperty("legado.androidNdkDir").orNull,
        System.getenv("ANDROID_NDK_HOME"),
        System.getenv("ANDROID_NDK_ROOT"),
        System.getenv("NDK_HOME"),
        System.getenv("ANDROID_SDK_ROOT")?.let { "$it/ndk" },
        System.getenv("ANDROID_HOME")?.let { "$it/ndk" },
    ).map(::File)
    return sdkCandidates.firstOrNull { candidate ->
        candidate.isDirectory && candidate.resolve("toolchains/llvm/prebuilt").isDirectory
    } ?: sdkCandidates.firstNotNullOfOrNull { candidate ->
        candidate.takeIf { it.resolve("toolchains/llvm/prebuilt").isDirectory }
            ?: candidate.listFiles()?.filter { it.isDirectory }
                ?.maxByOrNull { it.name }?.takeIf { it.resolve("toolchains/llvm/prebuilt").isDirectory }
    } ?: error("Set legado.androidNdkDir, ANDROID_NDK_HOME, or ANDROID_NDK_ROOT to an installed NDK")
}

fun androidNdkHostTag(ndkRoot: File): String {
    val hostOs = when {
        System.getProperty("os.name").startsWith("Windows", true) -> "windows"
        System.getProperty("os.name").startsWith("Mac", true) -> "darwin"
        else -> "linux"
    }
    val hostArch = if (System.getProperty("os.arch").lowercase() in setOf("aarch64", "arm64")) "arm64" else "x86_64"
    val hostTag = if (hostOs == "windows") "windows-x86_64" else "$hostOs-$hostArch"
    check(ndkRoot.resolve("toolchains/llvm/prebuilt/$hostTag").isDirectory) {
        "NDK toolchain is missing for host $hostTag: $ndkRoot"
    }
    return hostTag
}

fun findNinjaExecutable(ndkRoot: File): File {
    val sdkRoots = listOfNotNull(
        System.getenv("ANDROID_SDK_ROOT"),
        System.getenv("ANDROID_HOME"),
        ndkRoot.parentFile?.parentFile?.absolutePath,
    ).map(::File)
    val candidates = buildList {
        sdkRoots.forEach { sdk ->
            sdk.resolve("cmake").listFiles()?.filter { it.isDirectory }?.forEach { cmake ->
                add(cmake.resolve("bin/ninja.exe"))
                add(cmake.resolve("bin/ninja"))
            }
        }
        val pathNinja = if (System.getProperty("os.name").startsWith("Windows", true)) "ninja.exe" else "ninja"
        System.getenv("PATH").orEmpty().split(File.pathSeparator).forEach { add(File(it, pathNinja)) }
    }
    return candidates.firstOrNull { it.isFile }
        ?: error("Ninja is required for Android QuickJS builds; install Android SDK CMake or add ninja to PATH")
}

val androidRustBuildTasks = androidRustTargets.map { target ->
    val taskName = "buildRustAndroid${target.abi.split('-').joinToString("") { it.replaceFirstChar(Char::uppercase) }}"
    tasks.register<Exec>(taskName) {
        group = "build"
        description = "Build and stage the Rust source host for Android ABI ${target.abi}"
        workingDir(rustRoot)
        commandLine("cargo", "build", "--release", "--lib", "--no-default-features", "--target", target.triple)
        inputs.files(rustRoot.resolve("Cargo.toml"), rustRoot.resolve("Cargo.lock"))
        inputs.dir(rustRoot.resolve("src"))
        outputs.file(androidJniLibsOutput.map { it.file("${target.abi}/liblegado_lib.so") })
        doFirst {
            val ndkRoot = locateAndroidNdk()
            val hostTag = androidNdkHostTag(ndkRoot)
            val toolchainBin = ndkRoot.resolve("toolchains/llvm/prebuilt/$hostTag/bin")
            val windowsSuffix = if (System.getProperty("os.name").startsWith("Windows", true)) ".cmd" else ""
            val clang = toolchainBin.resolve("${target.clangPrefix}24-clang$windowsSuffix")
            val archiver = toolchainBin.resolve(
                if (System.getProperty("os.name").startsWith("Windows", true)) "llvm-ar.exe" else "llvm-ar",
            )
            check(clang.isFile && archiver.isFile) { "NDK compiler or llvm-ar not found for ${target.triple}" }
            val targetEnv = target.triple.replace('-', '_').uppercase()
            environment("CC_${target.triple.replace('-', '_')}", clang.absolutePath)
            environment("AR_${target.triple.replace('-', '_')}", archiver.absolutePath)
            environment("CARGO_TARGET_${targetEnv}_LINKER", clang.absolutePath)
            // 与 QuickJS .so 一致按 16 KiB 页面对齐，覆盖 Android 15+ 的大页设备。
            environment("RUSTFLAGS", "-C link-arg=-Wl,-z,max-page-size=16384")
        }
        doLast {
            val cargoArtifact = rustRoot.resolve("target/${target.triple}/release/liblegado_lib.so")
            check(cargoArtifact.isFile) { "Cargo did not create Android Rust library: $cargoArtifact" }
            val staged = androidJniLibsOutput.get().file("${target.abi}/liblegado_lib.so").asFile
            staged.parentFile.mkdirs()
            cargoArtifact.copyTo(staged, overwrite = true)
        }
    }
}

// QuickJS 是 Android 规则执行本身的一部分；Rust JNI 不会替代 JS 引擎，所以与 Rust .so 一起进入 AAR。
val androidQuickJsBuildTasks = androidRustTargets.map { target ->
    val taskName = "buildQuickJsAndroid${target.abi.split('-').joinToString("") { it.replaceFirstChar(Char::uppercase) }}"
    val stagedLibrary = androidJniLibsOutput.map { it.file("${target.abi}/liblegado_quickjs.so") }
    tasks.register(taskName) {
        group = "build"
        description = "Build and stage the source-rule QuickJS engine for Android ABI ${target.abi}"
        inputs.dir(quickJsAndroidRoot)
        inputs.dir(quickJsAndroidSources)
        outputs.file(stagedLibrary)
        doLast {
            val ndkRoot = locateAndroidNdk()
            val hostTag = androidNdkHostTag(ndkRoot)
            val toolchain = ndkRoot.resolve("build/cmake/android.toolchain.cmake")
            check(toolchain.isFile) { "Android CMake toolchain is missing: $toolchain" }
            val ninja = findNinjaExecutable(ndkRoot)
            val buildDir = layout.buildDirectory.dir("cmake-quickjs-android/${target.abi}").get().asFile
            val outputDir = stagedLibrary.get().asFile.parentFile
            outputDir.mkdirs()
            fun run(arguments: List<String>) {
                val process = ProcessBuilder(arguments).directory(projectDir).inheritIO().start()
                val code = process.waitFor()
                check(code == 0) { "Android QuickJS build failed ($code): ${arguments.joinToString(" ")}" }
            }
            run(listOf(
                "cmake", "-S", quickJsAndroidRoot.absolutePath,
                "-B", buildDir.absolutePath,
                "-G", "Ninja",
                "-DCMAKE_MAKE_PROGRAM=${ninja.absolutePath}",
                "-DCMAKE_TOOLCHAIN_FILE=${toolchain.absolutePath}",
                "-DANDROID_ABI=${target.abi}",
                "-DANDROID_PLATFORM=android-24",
                "-DCMAKE_BUILD_TYPE=Release",
                "-DCMAKE_LIBRARY_OUTPUT_DIRECTORY=${outputDir.absolutePath}",
                "-DCMAKE_LIBRARY_OUTPUT_DIRECTORY_RELEASE=${outputDir.absolutePath}",
                "-DCMAKE_RUNTIME_OUTPUT_DIRECTORY=${outputDir.absolutePath}",
                "-DCMAKE_RUNTIME_OUTPUT_DIRECTORY_RELEASE=${outputDir.absolutePath}",
            ))
            run(listOf(
                "cmake", "--build", buildDir.absolutePath,
                "--config", "Release", "--target", "legado_quickjs", "--parallel",
            ))
            check(stagedLibrary.get().asFile.isFile) {
                "CMake completed without producing ${stagedLibrary.get().asFile.absolutePath}"
            }
        }
    }
}

kotlin {
    // 桌面 JNI 适配器只放在 JVM source set；它通过回调把 HTTP 和宿主存储交给 Rust。
    // Android、iOS 不编译这组桌面适配代码，各自通过对应 target 接入 Rust 宿主。
    jvm {
        compilerOptions {
            jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_21)
        }
    }

    android {
        namespace = "io.legado.sourceengine.kmp"
        compileSdk = 37
        minSdk = 24
        compilerOptions {
            jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_21)
        }
    }

    // iOS 应用消费 Kotlin/Native framework，不使用 JVM 运行库 JAR。
    iosArm64 {
        compilations.getByName("main") {
            cinterops {
                create("sourceEngineHost") {
                    defFile(project.file("src/nativeInterop/cinterop/source_engine_host.def"))
                    includeDirs(project.file("../../src-tauri/include"))
                }
                create("quickjs") {
                    defFile(project.file("src/nativeInterop/cinterop/quickjs.def"))
                    // cinterop 与桌面/Android 共用仓库内 QuickJS-NG 源码，不查找原平台目录。
                    includeDirs(quickJsAndroidSources)
                }
                create("mbedtls") {
                    defFile(project.file("src/nativeInterop/cinterop/mbedtls.def"))
                    includeDirs(
                        project.file("src/nativeInterop/cinterop/mbedtls/include"),
                        project.file("src/nativeInterop/cinterop/mbedtls"),
                    )
                }
                create("nskeyvalueobserving") {
                    defFile(project.file("src/nativeInterop/cinterop/nskeyvalueobserving.def"))
                }
            }
        }
        binaries.framework {
            baseName = "LegadoSourceEngine"
            isStatic = true
            val quickJsLibs = layout.buildDirectory.dir("iosNativeLibs/ios_arm64").get().asFile
            val rustLibs = projectDir.resolve("../../src-tauri/target/aarch64-apple-ios/release").canonicalFile
            linkerOpts(
                "-L${quickJsLibs.absolutePath}", "-lquickjs", "-lmbedtls", "-lsqlite3",
                "-L${rustLibs.absolutePath}", "-llegado_lib",
                "-framework", "Security", "-framework", "CoreFoundation",
                "-framework", "SystemConfiguration", "-lz",
            )
        }
    }
    iosSimulatorArm64 {
        compilations.getByName("main") {
            cinterops {
                create("sourceEngineHost") {
                    defFile(project.file("src/nativeInterop/cinterop/source_engine_host.def"))
                    includeDirs(project.file("../../src-tauri/include"))
                }
                create("quickjs") {
                    defFile(project.file("src/nativeInterop/cinterop/quickjs.def"))
                    // cinterop 与桌面/Android 共用仓库内 QuickJS-NG 源码，不查找原平台目录。
                    includeDirs(quickJsAndroidSources)
                }
                create("mbedtls") {
                    defFile(project.file("src/nativeInterop/cinterop/mbedtls.def"))
                    includeDirs(
                        project.file("src/nativeInterop/cinterop/mbedtls/include"),
                        project.file("src/nativeInterop/cinterop/mbedtls"),
                    )
                }
                create("nskeyvalueobserving") {
                    defFile(project.file("src/nativeInterop/cinterop/nskeyvalueobserving.def"))
                }
            }
        }
        binaries.framework {
            baseName = "LegadoSourceEngine"
            isStatic = true
            val quickJsLibs = layout.buildDirectory.dir("iosNativeLibs/ios_simulator_arm64").get().asFile
            val rustLibs = projectDir.resolve("../../src-tauri/target/aarch64-apple-ios-sim/release").canonicalFile
            linkerOpts(
                "-L${quickJsLibs.absolutePath}", "-lquickjs", "-lmbedtls", "-lsqlite3",
                "-L${rustLibs.absolutePath}", "-llegado_lib",
                "-framework", "Security", "-framework", "CoreFoundation",
                "-framework", "SystemConfiguration", "-lz",
            )
        }
    }

    sourceSets {
        androidMain.dependencies {
            // 这组 Android 依赖先对齐原 data/foundation 的平台实现；接通 Rust HTTP 后会移除 OkHttp。
            implementation("com.squareup.okhttp3:okhttp:5.4.0")
            implementation("com.github.liuyueyi.quick-chinese-transfer:quick-transfer-core:0.2.16")
            implementation("cn.hutool:hutool-crypto:5.8.22")
            implementation("io.coil-kt.coil3:coil-network-okhttp:3.4.0")
            implementation("org.nanohttpd:nanohttpd:2.3.1")
            implementation("org.nanohttpd:nanohttpd-websocket:2.3.1")
            implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0")
            implementation("androidx.documentfile:documentfile:1.1.0")
            implementation("androidx.core:core-ktx:1.18.0")
            implementation("androidx.annotation:annotation:1.9.1")
            implementation("androidx.collection:collection:1.6.0")
            implementation("com.caverock:androidsvg-aar:1.4")
        }

        nativeMain {
            kotlin.exclude(*nativeInteropPatterns.toTypedArray())
            dependencies {
                implementation("io.ktor:ktor-server-core:3.5.2")
                implementation("io.ktor:ktor-server-cio:3.5.2")
                implementation("io.ktor:ktor-server-websockets:3.5.2")
            }
        }

        iosMain {
            dependencies {
                implementation("androidx.sqlite:sqlite-framework:2.7.0")
                implementation("io.ktor:ktor-client-core:3.5.2")
                implementation("io.ktor:ktor-client-darwin:3.5.2")
            }
        }

        iosArm64Main {
            kotlin.srcDir(nativeInteropOutput)
        }

        iosSimulatorArm64Main {
            kotlin.srcDir(nativeInteropOutput)
        }

        jvmMain.dependencies {
            // 沿用原公共模块的 JVM actual 实现；这些都是外部库，不是解析器 Kotlin 模块的预编译产物。
            implementation("com.squareup.okhttp3:okhttp:5.4.0")
            implementation("com.github.liuyueyi.quick-chinese-transfer:quick-transfer-core:0.2.16")
            implementation("cn.hutool:hutool-crypto:5.8.22")
            implementation("io.coil-kt.coil3:coil-network-okhttp:3.4.0")
            implementation("org.nanohttpd:nanohttpd:2.3.1")
            implementation("org.nanohttpd:nanohttpd-websocket:2.3.1")
            implementation("net.sf.kxml:kxml2:2.3.0")
            implementation("androidx.sqlite:sqlite-bundled:2.7.0")
            implementation("org.jetbrains.skiko:skiko-awt:0.144.6")
            implementation("org.jetbrains.kotlinx:kotlinx-coroutines-swing:1.11.0")
            implementation("androidx.annotation:annotation:1.9.1")
            implementation("org.json:json:20260814")
        }

        commonMain.dependencies {
            // 这些版本取自原平台 data/foundation 公共源集的版本目录，避免用 JVM JAR 代替 KMP 依赖。
            implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.11.0")
            implementation("org.jetbrains.kotlinx:atomicfu:0.33.0")
            implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.11.0")
            implementation("com.squareup.okio:okio:3.18.1")
            implementation("androidx.room3:room3-common:3.0.1")
            implementation("androidx.room3:room3-runtime:3.0.1")
            implementation("com.fleeksoft.ksoup:ksoup:0.2.6")
        }
    }
}

androidComponents {
    onVariants { variant ->
        // 将 Cargo 产物目录接入 Android KMP variant 的 jniLibs 源集合，随 AAR 一起打包。
        variant.sources.jniLibs?.addStaticSourceDirectory(androidJniLibsOutput.get().asFile.absolutePath)
    }
}

tasks.matching { it.name == "bundleAndroidMainAar" }.configureEach {
    dependsOn(androidRustBuildTasks + androidQuickJsBuildTasks)
}

tasks.matching { it.name == "mergeAndroidMainJniLibFolders" }.configureEach {
    dependsOn(androidRustBuildTasks + androidQuickJsBuildTasks)
}

room3 {
    schemaDirectory(layout.projectDirectory.dir("schemas").asFile.absolutePath)
}

// AppDatabase 的 expect/actual 由 Room KSP 生成。解析核心会逐步把应用数据库访问
// 替换为 Rust 宿主存储；保留生成器只是当前整批迁入源码的构建闭包，不能视为最终存储边界。
dependencies {
    add("kspJvm", "androidx.room3:room3-compiler:3.0.1")
    add("kspAndroid", "androidx.room3:room3-compiler:3.0.1")
    add("kspIosArm64", "androidx.room3:room3-compiler:3.0.1")
    add("kspIosSimulatorArm64", "androidx.room3:room3-compiler:3.0.1")
    add("kspIosArm64", project(":quickjs-processor"))
    add("kspIosSimulatorArm64", project(":quickjs-processor"))
}

tasks.matching {
    it.name.startsWith("compileKotlinIos") || it.name.startsWith("kspKotlinIos")
}.configureEach {
    // KSP also reads the generated C interop aliases, so it must wait for the staging task explicitly.
    dependsOn(stageNativeInteropForIos)
}

tasks.matching {
    it.name.startsWith("link") && it.name.contains("Framework") && it.name.endsWith("IosArm64")
}.configureEach {
    dependsOn("buildRustIosArm64", buildIosNativeQuickJs)
}

tasks.matching {
    it.name.startsWith("link") && it.name.contains("Framework") && it.name.endsWith("IosSimulatorArm64")
}.configureEach {
    dependsOn("buildRustIosSimulatorArm64", buildIosNativeQuickJs)
}
