import { mkdir, readFile, writeFile } from "node:fs/promises";
import { toolchain, syncGradleVersions } from "../toolchain.mjs";
const androidPackagePath = JSON.parse(await readFile(new URL("../tauri.conf.json", import.meta.url), "utf8")).identifier.replaceAll(".", "/");

// The generated Gradle Kotlin doFirst source patch cannot be configuration-cache serialized
// by Gradle 9. Disable configuration caching ONLY for this generated Android app project;
// the separate Kotlin/KMP engine build retains its own cache and incremental builds.
const gradlePropertiesFile = new URL("../gen/android/gradle.properties", import.meta.url);
let gradleProperties;
try {
  gradleProperties = await readFile(gradlePropertiesFile, "utf8");
} catch (error) {
  if (error?.code !== "ENOENT") throw error;
  gradleProperties = "";
}
const configCacheProperty = /^org\.gradle\.configuration-cache\s*=.*$/m;
if (configCacheProperty.test(gradleProperties)) {
  gradleProperties = gradleProperties.replace(configCacheProperty, "org.gradle.configuration-cache=false");
} else {
  gradleProperties += `${gradleProperties.endsWith("\n") || !gradleProperties ? "" : "\n"}org.gradle.configuration-cache=false\n`;
}
if (!/^org\.gradle\.logging\.stacktrace\s*=/m.test(gradleProperties)) {
  gradleProperties += `${gradleProperties.endsWith("\n") ? "" : "\n"}org.gradle.logging.stacktrace=all\n`;
}
await writeFile(gradlePropertiesFile, gradleProperties, "utf8");

// Apply the shared SDK and bytecode target after each library's own DSL.
const rootBuildFile = new URL("../gen/android/build.gradle.kts", import.meta.url);
const librarySdkMarker = "// Foliver Android library compile SDK compatibility.";
let rootBuild = await readFile(rootBuildFile, "utf8");
const originalRootBuild = rootBuild;
rootBuild = rootBuild.replace(/com\.android\.tools\.build:gradle:[^"]+/g, "com.android.tools.build:gradle:" + toolchain["agp.version"])
  .replace(/org\.jetbrains\.kotlin:kotlin-gradle-plugin:[^"]+/g, "org.jetbrains.kotlin:kotlin-gradle-plugin:" + toolchain["kotlin.version"]);
const librarySdkConfig = `${librarySdkMarker}
subprojects {
    pluginManager.withPlugin("com.android.library") {
        extensions.getByType<com.android.build.api.variant.LibraryAndroidComponentsExtension>()
            .finalizeDsl { library ->
                library.compileSdk = ${toolchain["android.compileSdk"]}
                library.buildToolsVersion = "${toolchain["android.buildTools"]}"
                library.ndkVersion = "${toolchain["android.ndk"]}"
                library.defaultConfig.minSdk = maxOf(library.defaultConfig.minSdk ?: 0, ${toolchain["android.minSdk"]})
                library.compileOptions {
                    sourceCompatibility = JavaVersion.toVersion("${toolchain["android.jvmTarget"]}")
                    targetCompatibility = JavaVersion.toVersion("${toolchain["android.jvmTarget"]}")
                }
                tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinJvmCompile>().configureEach {
                    compilerOptions.jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.fromTarget("${toolchain["android.jvmTarget"]}"))
                }
            }
    }
}
`;
const librarySdkBlock = /^\/\/ Foliver Android library compile SDK compatibility\.\r?\nsubprojects \{[\s\S]*?^\}\r?\n?/m;
rootBuild = librarySdkBlock.test(rootBuild) ? rootBuild.replace(librarySdkBlock, librarySdkConfig)
  : rootBuild + "\n" + librarySdkConfig;
// reqwest 0.13 uses Android's trust manager through this small Kotlin component.
// Its AAR version must match the Rust shim in Cargo.lock (not the verifier crate version).
const rustlsRepositoryMarker = "// Foliver Android system certificate verifier.";
if (!rootBuild.includes(rustlsRepositoryMarker)) {
  rootBuild += `
${rustlsRepositoryMarker}
allprojects {
    repositories {
        maven {
            url = uri("https://raw.githubusercontent.com/rustls/rustls-platform-verifier/maven-archive/android-release-support/maven/")
            content { includeGroup("org.rustls") }
        }
    }
}
`;
}
if (rootBuild !== originalRootBuild) await writeFile(rootBuildFile, rootBuild, "utf8");

const buildFile = new URL("../gen/android/app/build.gradle.kts", import.meta.url);
const manifestFile = new URL("../gen/android/app/src/main/AndroidManifest.xml", import.meta.url);
const networkConfigFile = new URL("../gen/android/app/src/main/res/xml/network_security_config.xml", import.meta.url);
const marker = "// Source-engine dependencies include a JVM-only JAR index.";
let contents = await readFile(buildFile, "utf8");
const originalContents = contents;
for (const field of ["compileSdk", "targetSdk", "minSdk"]) {
  contents = contents.replace(new RegExp(field + "\\s*=\\s*\\d+"), field + " = " + toolchain["android." + field]);
}
contents = contents.replace(/^\s*(?:ndkVersion|buildToolsVersion)\s*=.*\r?\n/gm, "")
  .replace(/(?:android|extensions\.configure<com\.android\.build\.api\.dsl\.ApplicationExtension>)\s*\{/, block => `${block}\n    ndkVersion = "${toolchain["android.ndk"]}"\n    buildToolsVersion = "${toolchain["android.buildTools"]}"`);
await syncGradleVersions(new URL("../gen/android/", import.meta.url));

const cargoLock = await readFile(new URL("../Cargo.lock", import.meta.url), "utf8");
const rustlsAndroidVersions = [...cargoLock.matchAll(/\[\[package\]\]\r?\nname = "rustls-platform-verifier-android"\r?\nversion = "([^"]+)"/g)];
if (rustlsAndroidVersions.length !== 1) throw new Error("Expected one rustls-platform-verifier-android version in Cargo.lock");
const rustlsAndroidDependency = `implementation("org.rustls:rustls-platform-verifier:${rustlsAndroidVersions[0][1]}")`;
const existingRustlsDependency = /implementation\("org\.rustls:rustls-platform-verifier:[^"]+"\)/;
if (existingRustlsDependency.test(contents)) {
  contents = contents.replace(existingRustlsDependency, rustlsAndroidDependency);
} else {
  contents = contents.replace("dependencies {", `dependencies {\n    ${rustlsAndroidDependency}`);
}
const arm64Marker = "// Foliver supports Android arm64 only.";
if (!contents.includes(arm64Marker)) {
  contents += `
android {
    ${arm64Marker}
    defaultConfig {
        ndk { abiFilters.clear(); abiFilters.add("arm64-v8a") }
    }
}
`;
}
// Keep generated app bytecode aligned with the engine's Android target.
contents = contents.replace(/sourceCompatibility\s*=[^\r\n]+/g, 'sourceCompatibility = JavaVersion.toVersion("' + toolchain["android.jvmTarget"] + '")')
  .replace(/targetCompatibility\s*=[^\r\n]+/g, 'targetCompatibility = JavaVersion.toVersion("' + toolchain["android.jvmTarget"] + '")')
  .replace(/jvmTarget\s*=[^\r\n]+/, 'jvmTarget = JvmTarget.fromTarget("' + toolchain["android.jvmTarget"] + '")');

// Tauri creates these Kotlin sources during `tauri android build`, after `android init`.
// 在 Kotlin 编译任务开始时再修正模板调用，避免 init 阶段读取尚未生成的文件。
const generatedKotlinPatchMarker = "// 修正 Tauri Android 模板中的过时 Kotlin API 调用。";
if (!contents.includes(generatedKotlinPatchMarker)) {
  const rustBlock = /\r?\nrust \{\r?\n/;
  if (!rustBlock.test(contents)) {
    throw new Error(`Cannot find the Rust Gradle block in ${buildFile.pathname}`);
  }

  const generatedKotlinPatch = String.raw`
// 修正 Tauri Android 模板中的过时 Kotlin API 调用。
// Resolve the file references at Gradle configuration time. Task.project is not
// accessible from doFirst with Gradle 9's configuration cache enabled.
val foliverGeneratedKotlinSources = listOf(
    layout.projectDirectory.file("src/main/java/${androidPackagePath}/generated/RustWebView.kt").asFile,
    layout.projectDirectory.file("src/main/java/${androidPackagePath}/generated/WryActivity.kt").asFile,
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
`;
  contents = contents.replace(rustBlock, `${generatedKotlinPatch}\nrust {\n`);
}

if (!contents.includes(marker)) {
  const androidBlock = /android \{\r?\n/;
  if (!androidBlock.test(contents)) {
    throw new Error(`Cannot find the Android DSL block in ${buildFile.pathname}`);
  }

  // The JAR index is unused on Android and duplicated by Hutool's crypto and core JARs.
  contents = contents.replace(
    androidBlock,
    (match) => `${match}    ${marker}\n    packaging {\n        resources {\n            excludes += "META-INF/INDEX.LIST"\n        }\n    }\n\n`,
  );
}
// Release builds use the same private signing key locally and in Actions.
const releaseMarker = "// Foliver release packaging and signing.";
if (!contents.includes(releaseMarker)) {
  const releaseConfig = String.raw`
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
`;
  contents += "\nandroid {\n" + releaseConfig + "}\n";
}
// Use AGP's public DSL types instead of its deprecated generated android() accessors.
contents = contents.replace(/^android \{/gm,
  "extensions.configure<com.android.build.api.dsl.ApplicationExtension> {");
if (contents !== originalContents) await writeFile(buildFile, contents, "utf8");

const appConfig = JSON.parse(await readFile(new URL("../tauri.conf.json", import.meta.url), "utf8"));
const mainActivityFile = new URL(
  `../gen/android/app/src/main/java/${appConfig.identifier.replaceAll(".", "/")}/MainActivity.kt`,
  import.meta.url,
);
let mainActivity = await readFile(mainActivityFile, "utf8");
const activityZoomMarker = "// Disable native WebView zoom gestures.";
if (!mainActivity.includes(activityZoomMarker)) {
  const newline = mainActivity.includes("\r\n") ? "\r\n" : "\n";
  if (!mainActivity.includes("import android.webkit.WebView")) {
    const packageLine = mainActivity.match(/^(package [^\r\n]+)\r?\n/m);
    if (!packageLine) throw new Error(`Cannot find the package declaration in ${mainActivityFile.pathname}`);
    mainActivity = mainActivity.replace(
      packageLine[0],
      `${packageLine[1]}${newline}${newline}import android.webkit.WebView${newline}`,
    );
  }

  const activityDeclaration = /^[ \t]*class MainActivity\s*:\s*TauriActivity\(\)/m;
  const activityMatch = mainActivity.match(activityDeclaration);
  if (!activityMatch) {
    throw new Error(`Cannot find the generated MainActivity class in ${mainActivityFile.pathname}`);
  }

  const zoomHook = [
    `    ${activityZoomMarker}`,
    "    override fun onWebViewCreate(webView: WebView) {",
    "        super.onWebViewCreate(webView)",
    "        webView.settings.setSupportZoom(false)",
    "        webView.settings.builtInZoomControls = false",
    "        webView.settings.displayZoomControls = false",
    "    }",
  ].join(newline);

  const signatureEnd = activityMatch.index + activityMatch[0].length;
  const openingBrace = mainActivity.slice(signatureEnd).match(/^[ \t]*(?:\r?\n[ \t]*)?\{/);
  if (openingBrace) {
    const insertAt = signatureEnd + openingBrace[0].length;
    mainActivity = `${mainActivity.slice(0, insertAt)}${newline}${zoomHook}${newline}${mainActivity.slice(insertAt)}`;
  } else {
    const activityWithZoomDisabled = [
      `${activityMatch[0]} {`,
      zoomHook,
      "}",
    ].join(newline);
    mainActivity = mainActivity.replace(activityDeclaration, activityWithZoomDisabled);
  }
  await writeFile(mainActivityFile, mainActivity, "utf8");
}

const networkConfig = `<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
    <base-config cleartextTrafficPermitted="false" />
    <domain-config cleartextTrafficPermitted="true">
        <domain includeSubdomains="false">127.0.0.1</domain>
        <domain includeSubdomains="false">localhost</domain>
    </domain-config>
</network-security-config>
`;
const networkManifestMarker = 'android:networkSecurityConfig="@xml/network_security_config"';
let manifest = await readFile(manifestFile, "utf8");
const applicationTag = /<application\b([^>]*)>/;
const applicationMatch = manifest.match(applicationTag);
if (!applicationMatch) throw new Error(`Cannot find the application tag in ${manifestFile.pathname}`);
// Android owns edge recognition and predictive-back feedback; Tauri delivers the committed Back event.
const nativeBackAttribute = /android:enableOnBackInvokedCallback\s*=\s*"[^"]*"/;
if (nativeBackAttribute.test(applicationMatch[1])) {
  manifest = manifest.replace(nativeBackAttribute, 'android:enableOnBackInvokedCallback="true"');
} else {
  manifest = manifest.replace(applicationTag, '<application$1\n        android:enableOnBackInvokedCallback="true">');
}
await writeFile(manifestFile, manifest, "utf8");
if (!applicationMatch[1].includes(networkManifestMarker)) {
  if (/android:networkSecurityConfig\s*=/.test(applicationMatch[1])) {
    throw new Error(`Unexpected Android network security config in ${manifestFile.pathname}`);
  }
  manifest = manifest.replace(applicationTag, `<application$1\n        ${networkManifestMarker}>`);
  await writeFile(manifestFile, manifest, "utf8");
}

await mkdir(new URL("../gen/android/app/src/main/res/xml/", import.meta.url), { recursive: true });
let previousNetworkConfig;
try {
  previousNetworkConfig = await readFile(networkConfigFile, "utf8");
} catch (error) {
  if (error?.code !== "ENOENT") throw error;
}
if (previousNetworkConfig !== undefined && previousNetworkConfig !== networkConfig) {
  throw new Error(`Unexpected Android network security policy in ${networkConfigFile.pathname}`);
}
if (previousNetworkConfig === undefined) await writeFile(networkConfigFile, networkConfig, "utf8");
