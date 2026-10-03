# Platform delivery and verification

The app shares one Vue interface across desktop, Android, and iOS. Rust owns application work and resource preparation. The mobile source-engine bridge hands the request to the existing Kotlin Multiplatform engine; the web frontend receives processed JSON and resource URLs. Tauri and KMP are built per target. The iOS build must run on macOS with Xcode.

## Current bridge and build facts

The repository has working source-engine adapter code for each target:

- Desktop registers the Rust command and calls the embedded KMP JVM engine through JNI.
- Android registers `io.legado.sourceengine.tauri.SourceEnginePlugin`; its Java command calls the Android KMP entry point on a worker thread. Rust provides the HTTP and storage callbacks.
- iOS registers the Swift Tauri plugin symbol `init_plugin_source_engine`; Swift calls the Kotlin/Native framework, whose HTTP and storage callbacks cross the Rust C ABI.

These source checks establish that adapter files and symbol names line up. They do **not** establish that an app installs, launches, or completes an in-app workflow. A platform is accepted only after its app artifact is inspected and a representative user flow is run on that platform.

The existing CI workflow in `.github/workflows/build.yml` describes the intended target matrix and build sequence. On 2026-10-03 this Linux cloud machine has Android build-tools 36.0.0, NDK 30.0.14904198 release candidate 1, CMake 3.31.1, API 36 and API 37.0, and all four Rust Android targets installed. The Tauri Android Gradle project has been generated and its resource-packaging rule configured. Its network security config denies cleartext by default and permits it only for `127.0.0.1` and `localhost`, where Rust serves app resources. The SDK lists NDK 30.0.14904198 as `rc.1`, so the local install used the Android CLI's beta channel. Confirm CI installs that NDK version before depending on it. Linux GTK/WebKitGTK development files are staged under `/workspace/.setup/sysroot`; source `/workspace/.setup/native-env.sh` after activating the cloud environment before building the native Linux app. The `--no-default-features` Rust and browser tests do not need those native libraries. The adjacent iOS `Info.ios.plist` adds ATS exceptions for the same two loopback names and does not allow arbitrary HTTP loads; its merged plist and runtime behavior still need verification on macOS. Android APK packaging and a device/emulator workflow, plus the final native desktop build and run, remain unverified.

## Readiness report

Run the read-only repository and environment check after activating this cloud environment:

```sh
cd /workspace/legado_rs
source /workspace/.setup/activate.sh
source /workspace/.setup/native-env.sh
node scripts/verify-platforms.mjs
```

The report checks bridge source files and symbol pairings, desktop prerequisites, Android SDK packages and Rust targets, and whether the host can run Xcode. It starts no builds and does not install packages. Missing prerequisites are reported as `BLOCKED`; this default report exits successfully so partial onboarding can be reviewed. Add `--strict` when a CI step should fail for any missing prerequisite.

After platform artifacts exist, inspect their archives and required native libraries:

```sh
node scripts/verify-platforms.mjs --artifacts --strict
```

This checks installer/APK/IPA presence and integrity; for Android it also checks that Rust and QuickJS native libraries are staged together for each supported ABI. Artifact presence and archive integrity still do not replace a device run.

## Desktop

Use the Node and Rust versions pinned by the environment, Java 21 for the current KMP JVM source engine, and the platform's Tauri build prerequisites. On Linux the current Tauri configuration requires GTK 3, WebKitGTK 4.1, libsoup 3, and Ayatana AppIndicator development files (the CI workflow also installs OpenSSL, librsvg, and X11 development packages). Build and run the actual Tauri app, then exercise search, book details, chapter preparation, opening a prepared chapter resource, and saved reading progress. `node scripts/verify-platforms.mjs --artifacts` checks installer output but cannot exercise application behavior.

The standalone Chromium browser flow is tracked separately in `scripts/e2e-browser.mjs`. It calls the real Rust `ApplicationService` through a test-only HTTP adapter and uses the actual Kotlin/JVM source engine against a local HTTP fixture. The adapter does not mock app commands or resource requests. This is browser UI and Rust core integration evidence; it is not evidence that the Tauri WebView or native desktop package works.

Set up its browser automation dependency outside the app's package graph, then run it from the activated cloud environment:

```sh
npm_config_cache=/workspace/.setup/npm-cache \
  npm install --prefix /workspace/.setup/browser-testing --no-save --no-package-lock playwright-core@1.55.0
cd /workspace/legado_rs
source /workspace/.setup/activate.sh
node scripts/e2e-browser.mjs
```

The harness builds/runs the Rust example with `--no-default-features`, so it does not pull Tauri's GTK/WebKitGTK windowing runtime into browser tests. It creates a fresh temporary app-data directory and local source fixture, exercises search, add-to-shelf, chapter resource delivery, page turns, display-only font changes, progress restore, private source isolation, and recovery from a deliberately deleted cached chapter. It saves 360px and 1440px screenshots under `/tmp/legado-browser-e2e-results`. Chromium must be installed at `/usr/bin/chromium`, or `CHROMIUM_EXECUTABLE_PATH` can point to it. Set `PLAYWRIGHT_CORE_ENTRY` if the isolated npm prefix differs.

## Android

The checked-in CI pipeline currently pins these SDK and Rust target requirements:

| Component | Required value |
| --- | --- |
| Java | 21 |
| Android compile platforms | API 36 and API 37.0 preview |
| Build Tools | 36.0.0 |
| NDK | 30.0.14904198 |
| CMake | 3.31.1 |
| Rust targets | `aarch64-linux-android`, `armv7-linux-androideabi`, `i686-linux-android`, `x86_64-linux-android` |
| Android ABIs | `arm64-v8a`, `armeabi-v7a`, `x86`, `x86_64` |
| QuickJS | CMake build for each ABI, packaged beside `liblegado_lib.so` |

The SDK manager package is a preview NDK in the current Android repository catalogue. Check availability first and include the beta channel when installing it with the new Android CLI:

```sh
export ANDROID_HOME=/workspace/.setup/android-sdk
export ANDROID_USER_HOME=/workspace/.setup/android-user
ANDROID_CLI="$ANDROID_HOME/cmdline-tools/latest/bin/android"
"$ANDROID_CLI" --sdk="$ANDROID_HOME" sdk install 'build-tools/36.0.0'
"$ANDROID_CLI" --sdk="$ANDROID_HOME" sdk install --beta 'ndk/30.0.14904198'
"$ANDROID_CLI" --sdk="$ANDROID_HOME" sdk install 'cmake/3.31.1'
```

The current CI also installs `platform-tools` and both compile platforms, which are already present in this machine's environment. Keep Android package setup non-interactive and use the packages declared in CI. The Android command-line tool may create user state under `ANDROID_USER_HOME`; set that to a writable environment-owned path in this cloud machine. On this image, `adb devices -l` still aborts while trying to create `/home/agent/.android` even when `ANDROID_USER_HOME` and `ANDROID_SDK_HOME` point to `/workspace/.setup/android-user`; no emulator/device runtime can be verified here until adb has a writable key location or a connected test runner.

Build in the order encoded in CI so the app consumes the correct per-ABI engine libraries:

```sh
source /workspace/.setup/activate.sh
export ANDROID_HOME=/workspace/.setup/android-sdk
export ANDROID_USER_HOME=/workspace/.setup/android-user
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
cd /workspace/legado_rs
npm ci
cd kotlin
./gradlew --no-daemon --console=plain stageTauriAndroidAar
cd ..
npm run tauri -- android init --ci
node scripts/configure-tauri-android.mjs
npm run tauri -- android build --debug --apk --ci
node scripts/verify-platforms.mjs --artifacts
```

The checked-in release matrix builds all four ABI variants. Do not treat a successful `cargo check` as a packaged Android app. Inspect the APK set, install it on an emulator/device, launch `com.alice.legado`, and exercise a source search through reading a prepared chapter and reopening the book with restored progress.

For future CI, verify the NDK's preview-channel availability explicitly. The workflow currently names NDK 30.0.14904198 without an explicit preview channel, while this environment's SDK catalogue lists that build as `rc.1` and requires an opt-in channel. If the pinned setup action does not install that version from its default channel, change CI's install step to use the beta channel or pin an available stable NDK and align Rust/CMake settings; do not silently build with whatever NDK happens to be present. The generated project configuration script installs a narrow `network_security_config.xml`: all cleartext is denied except exact loopback hosts, in both debug and release variants. It runs after `tauri android init` so regeneration keeps the policy:

```sh
npm run tauri -- android init --ci
node scripts/configure-tauri-android.mjs
```

## iOS

iOS requires a macOS runner with Xcode command-line tools, Swift, Java 21, Kotlin/Native, the Apple Rust targets, and Apple's SDKs. `src-tauri/Info.ios.plist` permits ATS exceptions only for `127.0.0.1` and `localhost`, matching the Rust resource server; it does not enable arbitrary HTTP loads. The checked-in build links the device and Apple Silicon simulator slices:

- Rust: `aarch64-apple-ios`, `aarch64-apple-ios-sim`.
- KMP Gradle: `:kmp-engine:linkReleaseFrameworkIosArm64` and `:kmp-engine:linkReleaseFrameworkIosSimulatorArm64`.
- Package both `LegadoSourceEngine.framework` outputs with `xcodebuild -create-xcframework` as `src-tauri/plugins/source-engine/ios/Frameworks/LegadoSourceEngine.xcframework`.
- Initialize and build the Tauri iOS app with `npm run tauri -- ios init --ci` and `npm run tauri -- ios build --no-sign --ci`.

Install the unsigned IPA only after re-signing it for a device, or launch the simulator build from the generated Xcode project. On the device or simulator, run the same source search, add-to-shelf, open/prepared chapter, resource display, and progress restore flow. An XCFramework archive, Rust cross-compilation, or unsigned IPA alone is not an iOS runtime validation. This environment is Linux and has no Xcode; iOS remains unverified until the CI macOS job completes and a simulator/device flow is run.

## Platform test record

Record each runtime verification with the commit, OS/device or simulator, artifact path, tested flow, and result. Use `BLOCKED` for unavailable toolchains and `NOT RUN` for work that has not yet been attempted. Keep macOS/iOS limitations explicit instead of inferring success from Android or desktop.
