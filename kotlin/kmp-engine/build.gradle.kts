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

fun commandOutput(vararg arguments: String): String {
    val process = ProcessBuilder(arguments.toList()).redirectErrorStream(true).start()
    val output = process.inputStream.bufferedReader().use { it.readText() }
    val exitCode = process.waitFor()
    check(exitCode == 0) {
        "${arguments.joinToString(" ")} failed with exit code $exitCode: ${output.trim()}"
    }
    return output.trim()
}

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

val buildIosNativeQuickJs = tasks.register<Exec>("buildIosNativeQuickJs") {
    val script = projectDir.resolve("scripts/build-ios-native.sh")
    workingDir(projectDir)
    commandLine("bash", script.absolutePath)
    inputs.file(script)
    inputs.dir("../src/native/quickjs-ng")
    inputs.dir("src/nativeInterop/cinterop/mbedtls")
    inputs.property("iosNativeTargets", listOf("ios_arm64", "ios_simulator_arm64"))
    inputs.property("iosToolchain", providers.provider {
        val developerDir = System.getenv("DEVELOPER_DIR") ?: commandOutput("xcode-select", "-p")
        val deviceClang = commandOutput("xcrun", "--sdk", "iphoneos", "--find", "clang")
        val simulatorClang = commandOutput("xcrun", "--sdk", "iphonesimulator", "--find", "clang")
        listOf(
            developerDir,
            commandOutput("xcodebuild", "-version"),
            commandOutput("xcrun", "--sdk", "iphoneos", "--show-sdk-version"),
            commandOutput("xcrun", "--sdk", "iphonesimulator", "--show-sdk-version"),
            commandOutput("xcrun", "--sdk", "iphoneos", "--show-sdk-path"),
            commandOutput("xcrun", "--sdk", "iphonesimulator", "--show-sdk-path"),
            deviceClang,
            commandOutput(deviceClang, "--version"),
            simulatorClang,
            commandOutput(simulatorClang, "--version"),
            commandOutput("xcrun", "--sdk", "iphoneos", "--find", "libtool"),
            commandOutput("xcrun", "--sdk", "iphonesimulator", "--find", "libtool"),
        ).joinToString("\n")
    })
    outputs.dir(layout.buildDirectory.dir("iosNativeLibs"))
}

val androidAbis = listOf("arm64-v8a", "armeabi-v7a", "x86", "x86_64")
val quickJsAndroidRoot = projectDir.resolve("../src/native/cpp").canonicalFile
val quickJsAndroidSources = projectDir.resolve("../src/native/quickjs-ng").canonicalFile
val androidNdkRoot = providers.provider { locateAndroidNdk().canonicalFile }
val androidNdkToolchain = androidNdkRoot.map { it.resolve("build/cmake/android.toolchain.cmake") }
val androidNdkClang = providers.provider {
    val ndkRoot = locateAndroidNdk()
    val hostTag = androidNdkHostTag(ndkRoot)
    val suffix = if (System.getProperty("os.name").startsWith("Windows", true)) ".exe" else ""
    ndkRoot.resolve("toolchains/llvm/prebuilt/$hostTag/bin/clang$suffix")
}
val androidNinja = providers.provider { findNinjaExecutable(locateAndroidNdk()).canonicalFile }

// These are stale outputs from the former AAR Rust tasks. Tauri builds its own Rust app library;
// deleting only these four generated files keeps the KMP AAR's JNI merge free of duplicate Rust .so.
val cleanLegacyRustAndroidAarLibraries = tasks.register<Delete>("cleanLegacyRustAndroidAarLibraries") {
    androidAbis.forEach { abi ->
        delete(androidJniLibsOutput.map { it.file("$abi/liblegado_lib.so") })
    }
}

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

// QuickJS 执行书源规则，必须随 KMP AAR 提供；Rust .so 由 Tauri Android app 构建并打包。
val androidQuickJsBuildTasks = androidAbis.map { abi ->
    val taskName = "buildQuickJsAndroid${abi.split('-').joinToString("") { it.replaceFirstChar(Char::uppercase) }}"
    val stagedLibrary = androidJniLibsOutput.map { it.file("$abi/liblegado_quickjs.so") }
    tasks.register(taskName) {
        group = "build"
        description = "Build and stage the source-rule QuickJS engine for Android ABI $abi"
        inputs.dir(quickJsAndroidRoot)
        inputs.dir(quickJsAndroidSources)
        inputs.property("androidAbi", abi)
        inputs.property("androidPlatform", "android-24")
        inputs.property("cmakeGenerator", "Ninja")
        inputs.property("buildType", "Release")
        inputs.property("androidNdkRoot", androidNdkRoot.map { it.absolutePath })
        inputs.file(androidNdkRoot.map { it.resolve("source.properties") })
        inputs.file(androidNdkToolchain)
        inputs.file(androidNdkClang)
        inputs.property("androidNdkClangVersion", androidNdkClang.map {
            commandOutput(it.absolutePath, "--version")
        })
        inputs.file(androidNinja)
        inputs.property("androidNinjaVersion", androidNinja.map {
            commandOutput(it.absolutePath, "--version")
        })
        inputs.property("cmakeVersion", providers.provider { commandOutput("cmake", "--version") })
        outputs.file(stagedLibrary)
        doLast {
            val ndkRoot = locateAndroidNdk()
            val hostTag = androidNdkHostTag(ndkRoot)
            val toolchain = ndkRoot.resolve("build/cmake/android.toolchain.cmake")
            check(toolchain.isFile) { "Android CMake toolchain is missing: $toolchain" }
            val ninja = findNinjaExecutable(ndkRoot)
            val buildDir = layout.buildDirectory.dir("cmake-quickjs-android/$abi").get().asFile
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
                "-DANDROID_ABI=$abi",
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
    compilerOptions {
        // 项目明确使用 expect/actual 类，启用对应语言能力并移除默认 Beta 提示。
        freeCompilerArgs.add("-Xexpect-actual-classes")
    }

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
            binaryOption("bundleId", "com.alice.legado.sourceengine")
            isStatic = true
            val quickJsLibs = layout.buildDirectory.dir("iosNativeLibs/ios_arm64").get().asFile
            linkerOpts(
                "-L${quickJsLibs.absolutePath}", "-lquickjs", "-lmbedtls", "-lsqlite3",
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
            binaryOption("bundleId", "com.alice.legado.sourceengine")
            isStatic = true
            val quickJsLibs = layout.buildDirectory.dir("iosNativeLibs/ios_simulator_arm64").get().asFile
            linkerOpts(
                "-L${quickJsLibs.absolutePath}", "-lquickjs", "-lmbedtls", "-lsqlite3",
                "-framework", "Security", "-framework", "CoreFoundation",
                "-framework", "SystemConfiguration", "-lz",
            )
        }
    }

    sourceSets {
        androidMain.dependencies {
            // 这组 Android 依赖先对齐原 data/foundation 的平台实现；接通 Rust HTTP 后会移除 OkHttp。
            implementation("androidx.webkit:webkit:1.14.0")
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
        // 将 QuickJS 原生产物目录接入 Android KMP variant 的 jniLibs 源集合，随 AAR 一起打包。
        variant.sources.jniLibs?.addStaticSourceDirectory(androidJniLibsOutput.get().asFile.absolutePath)
    }
}

tasks.matching { it.name == "bundleAndroidMainAar" }.configureEach {
    dependsOn(cleanLegacyRustAndroidAarLibraries, androidQuickJsBuildTasks)
}

tasks.matching { it.name == "mergeAndroidMainJniLibFolders" }.configureEach {
    dependsOn(cleanLegacyRustAndroidAarLibraries, androidQuickJsBuildTasks)
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

// iOS 用手写的 QuickJS native bridge，不具备 JVM 反射分发器运行环境；让 KSP 生成
// NativeGeneratedDispatch 和 JS 方法表，供 NativeJsExtensionsBridge 按 handle 分派。
ksp {
    arg("jsapi.native", "true")
    arg(
        "jsapi.nativeTargets",
        listOf(
            "io.legado.app.data.entities.BaseSource",
            "io.legado.app.help.http.StrResponse",
            "org.jsoup.Connection.Response",
            "org.jsoup.Connection.Base",
            "io.legado.app.model.analyzeRule.QueryTTF",
            "io.legado.app.utils.JsURL",
        ).joinToString(","),
    )
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
    dependsOn(buildIosNativeQuickJs)
}

tasks.matching {
    it.name.startsWith("link") && it.name.contains("Framework") && it.name.endsWith("IosSimulatorArm64")
}.configureEach {
    dependsOn(buildIosNativeQuickJs)
}
