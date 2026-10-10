#!/usr/bin/env node

import { access, readdir, readFile } from "node:fs/promises";
import { constants } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import os from "node:os";
import { spawnSync } from "node:child_process";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = new Set(process.argv.slice(2));
const checkArtifacts = args.has("--artifacts");
const strict = args.has("--strict");
const report = [];

function add(scope, state, detail) {
  report.push({ scope, state, detail });
}

function run(command, commandArgs = [], options = {}) {
  const result = spawnSync(command, commandArgs, {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    ...options,
  });
  return result;
}

function executable(command, commandArgs = ["--version"]) {
  const result = run(command, commandArgs);
  return result.status === 0 ? (result.stdout + result.stderr).trim().split("\n")[0] : null;
}

async function exists(path) {
  try {
    await access(path, constants.F_OK);
    return true;
  } catch {
    return false;
  }
}

async function walkFiles(root) {
  if (!(await exists(root))) return [];
  const found = [];
  const stack = [root];
  while (stack.length) {
    const current = stack.pop();
    let entries;
    try {
      entries = await readdir(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const full = join(current, entry.name);
      if (entry.isDirectory()) stack.push(full);
      else if (entry.isFile()) found.push(full);
    }
  }
  return found;
}

function check(condition, scope, success, failure) {
  add(scope, condition ? "PASS" : "BLOCKED", condition ? success : failure);
}

function rustTargets() {
  const result = run("rustup", ["target", "list", "--installed"]);
  return result.status === 0 ? new Set(result.stdout.trim().split(/\s+/).filter(Boolean)) : new Set();
}

function installedAndroidPackages(sdk) {
  return {
    api36: exists(join(sdk, "platforms/android-36")),
    api37: exists(join(sdk, "platforms/android-37.0")),
    buildTools36: exists(join(sdk, "build-tools/36.0.0")),
    ndk30: exists(join(sdk, "ndk/30.0.14904198/toolchains/llvm/prebuilt")),
    cmake331: exists(join(sdk, "cmake/3.31.1/bin/cmake")),
  };
}

async function checkNativeBridge() {
  const files = [
    "src-tauri/plugins/source-engine/src/mobile.rs",
    "src-tauri/plugins/source-engine/android/src/main/java/io/legado/sourceengine/tauri/SourceEnginePlugin.java",
    "src-tauri/plugins/source-engine/android/src/main/java/io/legado/sourceengine/tauri/ExecuteArgs.java",
    "src-tauri/plugins/source-engine/ios/Sources/SourceEnginePlugin.swift",
    "src-tauri/plugins/source-engine/ios/Package.swift",
    "kotlin/kmp-engine/src/androidMain/kotlin/io/legado/sourceengine/bridge/AndroidRustSourceEngineHost.kt",
    "kotlin/kmp-engine/src/iosMain/kotlin/io/legado/sourceengine/bridge/IosRustSourceEngineHost.kt",
    "src-tauri/include/source_engine_host.h",
  ];
  const contents = new Map();
  for (const file of files) {
    try {
      contents.set(file, await readFile(join(repoRoot, file), "utf8"));
      add("bridge source", "PASS", `${file} exists`);
    } catch {
      add("bridge source", "BLOCKED", `${file} is missing`);
    }
  }
  const mobile = contents.get(files[0]) ?? "";
  const android = contents.get(files[1]) ?? "";
  const ios = contents.get(files[3]) ?? "";
  const packageSwift = contents.get(files[4]) ?? "";
  const checks = [
    [mobile.includes('"io.legado.sourceengine.tauri", "SourceEnginePlugin"'), "Android Tauri package/class registration matches the Java plugin."],
    [android.includes("AndroidSourceEngineBlocking.executeJson") && android.includes("@Command"), "Android plugin calls the blocking KMP adapter from a Tauri command."],
    [ios.includes("IosSourceEngine.shared.executeJson") && ios.includes("init_plugin_source_engine"), "iOS Swift plugin enters the Kotlin/Native framework through Tauri's registered symbol."],
    [packageSwift.includes('name: "LegadoSourceEngine"') && packageSwift.includes("LegadoSourceEngine.xcframework"), "Swift package consumes the generated KMP XCFramework."],
  ];
  for (const [ok, detail] of checks) check(ok, "bridge contract", detail, `Bridge contract mismatch: ${detail}`);
}

async function checkDesktop() {
  const rustc = executable("rustc");
  const cargo = executable("cargo");
  const node = executable("node");
  check(Boolean(rustc && cargo), "desktop toolchain", "Rust compiler and Cargo are available.", "Rust compiler or Cargo is missing; source /workspace/.setup/activate.sh in this environment.");
  check(Boolean(node), "desktop toolchain", "Node.js is available.", "Node.js is missing.");
  if (os.platform() === "linux") {
    const pkgConfig = executable("pkg-config", ["--version"]);
    const libraries = ["gtk+-3.0", "webkit2gtk-4.1", "libsoup-3.0", "ayatana-appindicator3-0.1"];
    const missing = pkgConfig
      ? libraries.filter((name) => run("pkg-config", ["--exists", name]).status !== 0)
      : libraries;
    check(missing.length === 0, "desktop Linux", "Tauri Linux GTK/WebKitGTK development packages are discoverable.", `Missing pkg-config development packages: ${missing.join(", ") || "pkg-config"}.`);
  }
  if (checkArtifacts) {
    const files = await walkFiles(join(repoRoot, "src-tauri/target/release/bundle"));
    const hostExt = os.platform() === "win32" ? /\.(msi|exe)$/i : os.platform() === "darwin" ? /\.(dmg|pkg)$/i : /\.(deb|rpm|AppImage|tar\.gz)$/i;
    const bundles = files.filter((file) => hostExt.test(file));
    check(bundles.length > 0, "desktop artifact", "At least one installer exists for this host.", `No installer found under ${relative(repoRoot, join(repoRoot, "src-tauri/target/release/bundle"))}; no platform build has produced a desktop artifact.`);
    for (const bundle of bundles) add("desktop artifact", "INFO", relative(repoRoot, bundle));
  }
}

async function checkAndroid() {
  const sdk = process.env.ANDROID_HOME || process.env.ANDROID_SDK_ROOT || "/workspace/.setup/android-sdk";
  const sdkCli = executable(join(sdk, "cmdline-tools/latest/bin/android"), ["--version"]);
  const adb = await exists(join(sdk, "platform-tools", process.platform === "win32" ? "adb.exe" : "adb"));
  check(Boolean(sdkCli), "Android SDK", `Android SDK command line tool found at ${join(sdk, "cmdline-tools/latest/bin/android")}.`, "Android SDK command line tool is missing; set ANDROID_HOME to a configured SDK.");
  check(adb, "Android SDK", `ADB found at ${join(sdk, "platform-tools", process.platform === "win32" ? "adb.exe" : "adb")}.`, "ADB is missing from Android SDK platform-tools.");
  const packages = await installedAndroidPackages(sdk);
  for (const [key, path, ready] of [
    ["Android API 36", "platforms/android-36", await packages.api36],
    ["Android API 37 preview", "platforms/android-37.0", await packages.api37],
    ["Android Build Tools 36.0.0", "build-tools/36.0.0", await packages.buildTools36],
    ["Android NDK 30.0.14904198", "ndk/30.0.14904198/toolchains/llvm/prebuilt", await packages.ndk30],
    ["Android CMake 3.31.1", "cmake/3.31.1/bin/cmake", await packages.cmake331],
  ]) {
    check(ready, "Android SDK package", `${key} is installed.`, `${key} is required by .github/workflows/build.yml (${path}).`);
  }
  const targets = rustTargets();
  const required = ["aarch64-linux-android", "armv7-linux-androideabi", "i686-linux-android", "x86_64-linux-android"];
  for (const target of required) check(targets.has(target), "Android Rust target", `${target} is installed.`, `Install Rust target ${target} with rustup target add ${target}.`);
  check(await exists(join(repoRoot, "src-tauri/gen/android/gradlew")), "Android project", "Generated Tauri Android Gradle wrapper exists.", "Generated Android project is absent; run npm run tauri -- android init --ci after npm ci.");
  let manifest = "";
  let networkConfig = "";
  try {
    manifest = await readFile(join(repoRoot, "src-tauri/gen/android/app/src/main/AndroidManifest.xml"), "utf8");
    networkConfig = await readFile(join(repoRoot, "src-tauri/gen/android/app/src/main/res/xml/network_security_config.xml"), "utf8");
  } catch {
    // The Android project may not have been initialized and configured on this host.
  }
  const loopbackOnly = manifest.includes('android:networkSecurityConfig="@xml/network_security_config"')
    && networkConfig.includes('<base-config cleartextTrafficPermitted="false"')
    && networkConfig.includes('<domain includeSubdomains="false">127.0.0.1</domain>')
    && networkConfig.includes('<domain includeSubdomains="false">localhost</domain>')
    && !networkConfig.includes('<base-config cleartextTrafficPermitted="true"');
  check(loopbackOnly, "Android resource transport", "Generated manifest allows cleartext only for Rust loopback resources; other cleartext traffic stays denied.", "Run scripts/configure-tauri-android.mjs after Android project initialization to install the loopback-only network security config.");
  if (checkArtifacts) {
    const apks = (await walkFiles(join(repoRoot, "src-tauri/gen/android/app/build/outputs/apk"))).filter((file) => file.endsWith(".apk"));
    check(apks.length > 0, "Android artifact", "At least one APK exists.", "No APK found under src-tauri/gen/android/app/build/outputs/apk.");
    const staged = new Set();
    for (const apk of apks) {
      const listing = run("unzip", ["-Z1", apk]);
      const entries = new Set(listing.stdout.split(/\r?\n/));
      for (const abi of ["arm64-v8a", "armeabi-v7a", "x86", "x86_64"]) {
        if (entries.has(`lib/${abi}/liblegado_lib.so`) && entries.has(`lib/${abi}/liblegado_quickjs.so`)) staged.add(abi);
      }
      const integrity = run("unzip", ["-t", apk]);
      check(integrity.status === 0, "Android artifact", `${relative(repoRoot, apk)} is a readable ZIP/APK.`, `${relative(repoRoot, apk)} failed ZIP integrity validation.`);
    }
    for (const abi of ["arm64-v8a", "armeabi-v7a", "x86", "x86_64"]) check(staged.has(abi), "Android artifact ABI", `APK set includes Rust and QuickJS libraries for ${abi}.`, `APK set is missing liblegado_lib.so or liblegado_quickjs.so for ${abi}.`);
  }
}

async function checkIos() {
  const mac = os.platform() === "darwin";
  check(mac, "iOS host", "Running on macOS.", "iOS builds require macOS and Xcode; this host cannot validate them.");
  const xcodebuild = executable("xcodebuild", ["-version"]);
  const swift = executable("swift", ["--version"]);
  check(Boolean(xcodebuild), "iOS toolchain", "Xcode command line tools are available.", "xcodebuild is unavailable; install/select Xcode on a macOS runner.");
  check(Boolean(swift), "iOS toolchain", "Swift is available.", "Swift is unavailable.");
  const targets = rustTargets();
  for (const target of ["aarch64-apple-ios", "aarch64-apple-ios-sim"]) check(targets.has(target), "iOS Rust target", `${target} is installed.`, `Install Rust target ${target} with rustup target add ${target}.`);
  let iosPlist = "";
  try {
    iosPlist = await readFile(join(repoRoot, "src-tauri/Info.ios.plist"), "utf8");
  } catch {
    // Tauri supports this adjacent Info.ios.plist as an additive plist merge.
  }
  const atsLoopbackOnly = iosPlist.includes("NSAppTransportSecurity")
    && iosPlist.includes("NSExceptionDomains")
    && iosPlist.includes("127.0.0.1")
    && iosPlist.includes("localhost")
    && !iosPlist.includes("NSAllowsArbitraryLoads")
    && !iosPlist.includes("NSAllowsArbitraryLoadsInWebContent");
  check(atsLoopbackOnly, "iOS resource transport", "Info.ios.plist permits ATS exceptions only for Rust loopback resource hosts; broad HTTP exceptions are absent.", "Add exact 127.0.0.1 and localhost exceptions in src-tauri/Info.ios.plist; verify the merged plist on macOS.");
  if (checkArtifacts) {
    const xcframework = join(repoRoot, "src-tauri/plugins/source-engine/ios/Frameworks/LegadoSourceEngine.xcframework");
    const ipaFiles = (await walkFiles(join(repoRoot, "src-tauri/gen/apple/build"))).filter((file) => file.endsWith(".ipa"));
    check(await exists(join(xcframework, "Info.plist")), "iOS artifact", "Kotlin/Native XCFramework has Info.plist.", "LegadoSourceEngine.xcframework is missing; build arm64 and simulator frameworks on macOS first.");
    check(ipaFiles.length > 0, "iOS artifact", "At least one unsigned IPA exists.", "No IPA found under src-tauri/gen/apple/build; the iOS app has not been packaged.");
    for (const ipa of ipaFiles) check(run("unzip", ["-t", ipa]).status === 0, "iOS artifact", `${relative(repoRoot, ipa)} is a readable IPA archive.`, `${relative(repoRoot, ipa)} failed ZIP integrity validation.`);
  }
}

await checkNativeBridge();
await checkDesktop();
await checkAndroid();
await checkIos();

const counts = report.reduce((acc, item) => ({ ...acc, [item.state]: (acc[item.state] ?? 0) + 1 }), {});
console.log("Tauri / Rust / Web platform readiness (no build is started by this script)");
for (const { scope, state, detail } of report) console.log(`${state.padEnd(7)} ${scope}: ${detail}`);
console.log(`\n${counts.PASS ?? 0} passed, ${counts.BLOCKED ?? 0} blocked${counts.INFO ? `, ${counts.INFO} info` : ""}.`);
if (strict && (counts.BLOCKED ?? 0) > 0) process.exitCode = 1;
