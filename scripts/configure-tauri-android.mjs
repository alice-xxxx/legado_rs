import { mkdir, readFile, writeFile } from "node:fs/promises";

const buildFile = new URL("../src-tauri/gen/android/app/build.gradle.kts", import.meta.url);
const manifestFile = new URL("../src-tauri/gen/android/app/src/main/AndroidManifest.xml", import.meta.url);
const networkConfigFile = new URL("../src-tauri/gen/android/app/src/main/res/xml/network_security_config.xml", import.meta.url);
const marker = "// Source-engine dependencies include a JVM-only JAR index.";
const stripNativeDebugSymbols = process.env.LEGADO_ANDROID_STRIP_DEBUG_SYMBOLS === "1";
let contents = await readFile(buildFile, "utf8");

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
  await writeFile(buildFile, contents, "utf8");
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
