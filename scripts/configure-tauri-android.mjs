import { mkdir, readFile, writeFile } from "node:fs/promises";

// The generated Gradle Kotlin doFirst source patch cannot be configuration-cache serialized
// by Gradle 9. Disable configuration caching ONLY for this generated Android app project;
// the separate Kotlin/KMP engine build retains its own cache and incremental builds.
const gradlePropertiesFile = new URL("../src-tauri/gen/android/gradle.properties", import.meta.url);
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
await writeFile(gradlePropertiesFile, gradleProperties, "utf8");

const buildFile = new URL("../src-tauri/gen/android/app/build.gradle.kts", import.meta.url);
const manifestFile = new URL("../src-tauri/gen/android/app/src/main/AndroidManifest.xml", import.meta.url);
const networkConfigFile = new URL("../src-tauri/gen/android/app/src/main/res/xml/network_security_config.xml", import.meta.url);
const marker = "// Source-engine dependencies include a JVM-only JAR index.";
const stripNativeDebugSymbols = process.env.LEGADO_ANDROID_STRIP_DEBUG_SYMBOLS === "1";
let contents = await readFile(buildFile, "utf8");
let buildFileChanged = false;

// The generated app targets Java 8 by default. JDK 21 warns on source/target 8, so align the
// generated Java and Kotlin bytecode levels at 11 instead of suppressing the compiler warning.
const javaEightOptions = /    compileOptions \{\r?\n        sourceCompatibility = JavaVersion\.VERSION_1_8\r?\n        targetCompatibility = JavaVersion\.VERSION_1_8\r?\n    \}/;
const javaElevenOptions = /    compileOptions \{\r?\n        sourceCompatibility = JavaVersion\.VERSION_11\r?\n        targetCompatibility = JavaVersion\.VERSION_11\r?\n    \}/;
if (javaEightOptions.test(contents)) {
  contents = contents.replace(javaEightOptions, (block) => block.replaceAll("VERSION_1_8", "VERSION_11"));
  buildFileChanged = true;
} else if (!javaElevenOptions.test(contents)) {
  throw new Error(`Cannot find generated Java 8 or Java 11 compile options in ${buildFile.pathname}`);
}

const kotlinEightTarget = /kotlin \{\r?\n    compilerOptions \{\r?\n        jvmTarget = JvmTarget\.JVM_1_8\r?\n    \}/;
const kotlinElevenTarget = /kotlin \{\r?\n    compilerOptions \{\r?\n        jvmTarget = JvmTarget\.JVM_11\r?\n    \}/;
if (kotlinEightTarget.test(contents)) {
  contents = contents.replace(kotlinEightTarget, (block) => block.replace("JVM_1_8", "JVM_11"));
  buildFileChanged = true;
} else if (!kotlinElevenTarget.test(contents)) {
  throw new Error(`Cannot find generated Kotlin JVM 8 or JVM 11 target in ${buildFile.pathname}`);
}

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
val legadoGeneratedKotlinSources = listOf(
    layout.projectDirectory.file("src/main/java/com/alice/legado/generated/RustWebView.kt").asFile,
    layout.projectDirectory.file("src/main/java/com/alice/legado/generated/WryActivity.kt").asFile,
)
tasks.configureEach {
    if (name.startsWith("compile") && name.endsWith("Kotlin")) {
        doFirst {
            for (sourceFile in legadoGeneratedKotlinSources) {
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
  buildFileChanged = true;
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
  buildFileChanged = true;
}
if (buildFileChanged) await writeFile(buildFile, contents, "utf8");

const appConfig = JSON.parse(await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
const mainActivityFile = new URL(
  `../src-tauri/gen/android/app/src/main/java/${appConfig.identifier.replaceAll(".", "/")}/MainActivity.kt`,
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

if (stripNativeDebugSymbols) {
  const debugStart = contents.indexOf('getByName("debug") {');
  const releaseStart = debugStart < 0 ? -1 : contents.indexOf('getByName("release") {', debugStart);
  if (debugStart < 0 || releaseStart < 0) {
    throw new Error(`Cannot find generated debug/release build types in ${buildFile.pathname}`);
  }

  let debugBlock = contents.slice(debugStart, releaseStart);
  const rulePattern = /^[ \t]*jniLibs\.keepDebugSymbols\.add\("([^"]+)"\)[ \t]*\r?$/gm;
  const rules = [...debugBlock.matchAll(rulePattern)].map((match) => match[1]);
  if (rules.length === 0) {
    if (debugBlock.includes("keepDebugSymbols")) {
      throw new Error(`Unexpected keepDebugSymbols syntax in ${buildFile.pathname}`);
    }
    console.log("CI native packaging: no keepDebugSymbols rule present; Android Gradle defaults remain active.");
  } else {
    const expectedRules = [
      "*/arm64-v8a/*.so",
      "*/armeabi-v7a/*.so",
      "*/x86/*.so",
      "*/x86_64/*.so",
    ];
    if (rules.length !== expectedRules.length || expectedRules.some((rule) => !rules.includes(rule))) {
      throw new Error(`Unexpected JNI keepDebugSymbols rules in ${buildFile.pathname}: ${rules.join(", ")}`);
    }

    const packagingBlock = debugBlock.match(/^[ \t]*packaging[ \t]*\{\r?\n((?:[ \t]*jniLibs\.keepDebugSymbols\.add\("[^"]+"\)[ \t]*\r?\n)+)[ \t]*\}\r?\n?/m);
    if (!packagingBlock) {
      throw new Error(`Cannot locate the generated JNI debug-symbol packaging block in ${buildFile.pathname}`);
    }
    const blockRules = [...packagingBlock[1].matchAll(rulePattern)].map((match) => match[1]);
    if (blockRules.length !== expectedRules.length) {
      throw new Error(`JNI debug-symbol rules are not contained in one packaging block in ${buildFile.pathname}`);
    }

    const indent = packagingBlock[0].match(/^[ \t]*/)?.[0] ?? "";
    debugBlock = debugBlock.replace(packagingBlock[0], `${indent}// CI lets Android Gradle strip JNI debug symbols.\n`);
    contents = contents.slice(0, debugStart) + debugBlock + contents.slice(releaseStart);
    await writeFile(buildFile, contents, "utf8");
    console.log(`CI native packaging: removed ${rules.length} keepDebugSymbols rules for the four Android ABIs.`);
  }
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
if (!applicationMatch[1].includes(networkManifestMarker)) {
  if (/android:networkSecurityConfig\s*=/.test(applicationMatch[1])) {
    throw new Error(`Unexpected Android network security config in ${manifestFile.pathname}`);
  }
  manifest = manifest.replace(applicationTag, `<application$1\n        ${networkManifestMarker}>`);
  await writeFile(manifestFile, manifest, "utf8");
}

await mkdir(new URL("../src-tauri/gen/android/app/src/main/res/xml/", import.meta.url), { recursive: true });
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
