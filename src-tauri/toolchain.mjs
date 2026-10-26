import { appendFile, readFile, writeFile } from "node:fs/promises";

export const toolchain = Object.fromEntries((await readFile(
  new URL("./plugins/source-engine/toolchain.properties", import.meta.url), "utf8",
)).split(/\r?\n/).filter(line => line.trim() && !line.startsWith("#")).map(line => {
  const separator = line.indexOf("=");
  if (separator < 1) throw new Error(`Invalid toolchain setting: ${line}`);
  return [line.slice(0, separator).trim(), line.slice(separator + 1).trim()];
}));

async function writeChanged(file, contents) {
  const previous = await readFile(file, "utf8").catch(error => {
    if (error.code !== "ENOENT") throw error;
    return "";
  });
  if (previous !== contents) await writeFile(file, contents);
}

export async function syncGradleVersions(directory = new URL("./plugins/source-engine/engine/", import.meta.url)) {
  const wrapper = new URL("gradle/wrapper/gradle-wrapper.properties", directory);
  await writeChanged(wrapper, (await readFile(wrapper, "utf8"))
    .replace(/gradle-[0-9.]+-bin/, `gradle-${toolchain["gradle.version"]}-bin`));
  await writeChanged(new URL("gradle/gradle-daemon-jvm.properties", directory),
    `toolchainVersion=${toolchain["java.version"]}\n`);
}
if (process.argv.includes("--sync") || process.argv.includes("--github-env")) {
  await syncGradleVersions();
  const file = new URL("./tauri.conf.json", import.meta.url);
  const config = JSON.parse(await readFile(file, "utf8"));
  config.bundle.android = { ...config.bundle.android, minSdkVersion: Number(toolchain["android.minSdk"]) };
  config.bundle.iOS.minimumSystemVersion = toolchain["ios.deploymentTarget"];
  await writeChanged(file, JSON.stringify(config, null, 2) + "\n");
}

if (process.argv.includes("--github-env")) {
  if (!process.env.GITHUB_ENV) throw new Error("GITHUB_ENV is unavailable");
  await appendFile(process.env.GITHUB_ENV, Object.entries(toolchain)
    .map(([key, value]) => `${key.replaceAll(".", "_").toUpperCase()}=${value}\n`).join(""));
  // NDK/CMake uses the minimum API level, independently of the installed SDK platform.
  await appendFile(process.env.GITHUB_ENV,
    `ANDROID_PLATFORM=android-${toolchain["android.minSdk"]}\n` +
    `IPHONEOS_DEPLOYMENT_TARGET=${toolchain["ios.deploymentTarget"]}\n`);
}
