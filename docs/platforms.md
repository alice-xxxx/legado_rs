# Platform delivery and verification

The app shares one Vue interface across desktop, Android, and iOS. Rust owns application work and resource preparation. The mobile source-engine bridge hands the request to the existing Kotlin Multiplatform engine; the web frontend receives processed JSON and resource URLs. Tauri and KMP are built per target. The iOS build must run on macOS with Xcode.

## Cloud-host verification record (2026-10-03)

This record captures the earlier 02:18 browser run. It used the real Rust service and JVM source engine, but does not cover later PDF/CBZ changes in the current working tree or validate the GTK/WebKitGTK Tauri window, Android runtime, or iOS runtime.

| Target | Status | Evidence |
| --- | --- | --- |
| Browser + real Rust/KMP source engine | **PASS** | Fresh no-default-features `browser_harness` build and a full Playwright Chromium flow; 27 Rust IPC calls. |
| Android Rust/KMP AAR | **PASS** | `stageTauriAndroidAar` built all four configured Android ABI outputs. |
| Android APK | **IN PROGRESS** | `gradlew help` passed after preserving Gradle Plugin Portal in the cloud mirror config; debug APK packaging is being attempted separately. |
| Android emulator/device runtime | **NOT RUN** | API 36 AOSP x86_64 image and `LegadoApi36Aosp` AVD are installed, but the app was not installed or launched in this run. |
| Linux native Tauri desktop | **NOT RUN** | The browser harness uses the real service but does not start a Tauri window or WebKitGTK. |
| iOS build/runtime on this host | **BLOCKED** | This host is Linux and has no Xcode or Apple SDK. A separate macOS CI build result is recorded below; no simulator/device runtime was exercised here. |

The browser run started at `2026-10-03T02:18:04Z` and completed at `02:18:15Z` with `PASS`:

```sh
source /workspace/.setup/activate.sh
cargo build --locked --manifest-path src-tauri/Cargo.toml --no-default-features --example browser_harness -j4
BROWSER_HARNESS_BINARY=/workspace/legado_rs/src-tauri/target/debug/examples/browser_harness node scripts/e2e-browser.mjs
```

It searched the local source fixture through the KMP/JNI source engine, added a book, opened two prepared chapter resources, changed reader font, paged through all 36 fixture paragraphs, saved progress, reopened the book, and repaired a deliberately deleted chapter after the exact resource URL returned 404. Turns between cached chapters made no Rust chapter retrieval calls. The display-only font change left the cached chapter HTML hash unchanged. Private source rules were not served as a browser resource.

At 360px, measured layout was: catalog heading 15px, chapter title 14px, row height 52px, detail action height 44px, reader title 19px, reader page status 14px, and reader page/chapter controls 44–46px. The reader header and children stayed inside the viewport. The script closed the reading settings popover before validating visible chapter text and capturing the reader screenshot. Four screenshots, `result.json`, and `run.log` are under `/tmp/legado-browser-e2e-results/2026-10-03T02-18-04-040Z`.

The cloud Gradle init script redirects Maven Central to a mirror that lacks the Kotlin DSL plugin marker (`org.gradle.kotlin.kotlin-dsl:6.6.4`). The workspace Gradle mirror config was adjusted to retain Gradle Plugin Portal for plugin resolution; Android `gradlew help` then passed. This change is cloud environment setup, not a repository Gradle-file change.

## GitHub Actions matrix evidence

Run [37095535654](https://github.com/alice-xxxx/legado_rs/actions/runs/37095535654) built commit `2f57b3331c8f00b15742c703f8c3c1416f56786a`:

| Job | Result | Evidence |
| --- | --- | --- |
| Android APK and AAR | **PASS** | CI built and uploaded the configured Android APKs and four-ABI AAR. |
| Linux, macOS, and Windows desktop | **PASS** | Each runner built and uploaded its unsigned desktop installer. This is build evidence, not an interactive runtime test. |
| iOS unsigned IPA | **FAIL** | Device/simulator KMP frameworks built and were packaged; Swift app compilation then failed at `SourceEnginePlugin.swift`: `no such module 'LegadoSourceEngine'`. |

The iOS failure is a SwiftPM target-selection issue in `swift-rs` 1.0.8. The build passed an iOS compiler target with `-Xswiftc -target` and architecture with `--arch`, but SwiftPM still resolved the binary XCFramework using the macOS host triple, so the iOS-only `LegadoSourceEngine` module was unavailable to the Swift target. Upstream [swift-rs PR #84](https://github.com/Brendonovich/swift-rs/pull/84) fixes cross-compilation by passing the platform triple to SwiftPM and selecting its platform-specific output directory. The repository now pins the upstream fix at `IIK3D/swift-rs` revision `a1bd3b5439aeaacc3bd26624aa611a5e5eee831e`. The locked iOS dependency graph resolves to this exact revision with:

```sh
cargo tree --locked --manifest-path src-tauri/Cargo.toml --target aarch64-apple-ios --invert swift-rs
```

That confirms dependency selection only; a fresh project macOS CI run must compile the Swift package before counting the fix as verified. No source-engine implementation or parsing rules are changed by this build-tool patch.

The Linux cloud host still cannot build or run iOS because it has no Xcode or Apple SDK. The CI result above is from a macOS runner. No iOS simulator or device user flow has been exercised.

## Current bridge and build facts

The repository has working source-engine adapter code for each target:

- Desktop registers the Rust command and calls the embedded KMP JVM engine through JNI.
- Android registers `io.legado.sourceengine.tauri.SourceEnginePlugin`; its Java command calls the Android KMP entry point on a worker thread. Rust provides the HTTP and storage callbacks.
- iOS registers the Swift Tauri plugin symbol `init_plugin_source_engine`; Swift calls the Kotlin/Native framework, whose HTTP and storage callbacks cross the Rust C ABI.

These source checks establish that adapter files and symbol names line up. They do **not** establish that an app installs, launches, or completes an in-app workflow. A platform is accepted only after its app artifact is inspected and a representative user flow is run on that platform.

The existing CI workflow in `.github/workflows/build.yml` describes the intended target matrix and build sequence. On 2026-10-03 this Linux cloud machine has Android build-tools 36.0.0, NDK 30.0.14904198 release candidate 1, CMake 3.31.1, API 36 and API 37.0, and all four Rust Android targets installed. The Tauri Android Gradle project has been generated and its resource-packaging rule configured. Its network security config denies cleartext by default and permits it only for `127.0.0.1` and `localhost`, where Rust serves app resources. The SDK lists NDK 30.0.14904198 as `rc.1`, so the local install used the Android CLI's beta channel. Confirm CI installs that NDK version before depending on it. Linux GTK/WebKitGTK development files are staged under `/workspace/.setup/sysroot`; source `/workspace/.setup/native-env.sh` after activating the cloud environment before building the native Linux app. The `--no-default-features` Rust and browser tests do not need those native libraries. The adjacent iOS `Info.ios.plist` adds ATS exceptions for the same two loopback names and does not allow arbitrary HTTP loads; its merged plist and runtime behavior still need verification on macOS. Android APK and native desktop launch status are recorded above.

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

The harness builds/runs the Rust example with `--no-default-features`, so it does not pull Tauri's GTK/WebKitGTK windowing runtime into browser tests. It creates a fresh temporary app-data directory and local source fixture, exercises search, add-to-shelf, chapter resource delivery, page turns, display-only font changes, progress restore, private source isolation, responsive 360px/1440px layouts, and recovery from a deliberately deleted cached chapter. It saves timestamped screenshots and a machine-readable `result.json` under `/tmp/legado-browser-e2e-results/<run-id>`. Chromium must be installed at `/usr/bin/chromium`, or `CHROMIUM_EXECUTABLE_PATH` can point to it. Set `PLAYWRIGHT_CORE_ENTRY` if the isolated npm prefix differs.

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

The current CI also installs `platform-tools` and both compile platforms, which are already present in this machine's environment. Keep Android package setup non-interactive and use the packages declared in CI. Set `ANDROID_USER_HOME=/workspace/.setup/android-user` and unset `ANDROID_SDK_HOME` to avoid Gradle's conflicting home-directory selection. The API 36 AOSP x86_64 system image and `LegadoApi36Aosp` AVD are installed; image availability is not an app runtime test.

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
