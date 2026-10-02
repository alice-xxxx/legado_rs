import { readFile, writeFile } from "node:fs/promises";

const buildFile = new URL("../src-tauri/gen/android/app/build.gradle.kts", import.meta.url);
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
