import { mkdir, readFile, writeFile } from "node:fs/promises";

const buildFile = new URL("../src-tauri/gen/android/app/build.gradle.kts", import.meta.url);
const manifestFile = new URL("../src-tauri/gen/android/app/src/main/AndroidManifest.xml", import.meta.url);
const networkConfigFile = new URL("../src-tauri/gen/android/app/src/main/res/xml/network_security_config.xml", import.meta.url);
const marker = "// Source-engine dependencies include a JVM-only JAR index.";
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
