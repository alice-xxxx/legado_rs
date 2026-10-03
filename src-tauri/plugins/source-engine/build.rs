use std::{env, ffi::OsStr, path::PathBuf};

const COMMANDS: &[&str] = &["execute"];

fn main() {
    println!("cargo:rustc-check-cfg=cfg(desktop)");
    println!("cargo:rustc-check-cfg=cfg(mobile)");
    // Kotlin/Native first needs the Rust archive to build the iOS framework, which creates the
    // XCFramework consumed by this Swift package. Skip package generation for that bootstrap step;
    // the later Tauri iOS build reruns this script without the flag after the XCFramework exists.
    println!("cargo:rerun-if-env-changed=LEGADO_KMP_BOOTSTRAP");
    if env::var_os("LEGADO_KMP_BOOTSTRAP").as_deref() == Some(OsStr::new("1")) {
        // The normal builder emits this target cfg as part of Swift package setup; retain it while
        // skipping that setup so the standalone iOS Rust archive still compiles the mobile adapter.
        if matches!(
            env::var("CARGO_CFG_TARGET_OS").as_deref(),
            Ok("ios" | "android")
        ) {
            println!("cargo:rustc-cfg=mobile");
        }
        return;
    }

    link_ios_engine_framework();

    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}

fn link_ios_engine_framework() {
    let Ok(target) = env::var("TARGET") else {
        return;
    };
    let (slice, native_lib_target) = match target.as_str() {
        "aarch64-apple-ios" => ("ios-arm64", "ios_arm64"),
        "aarch64-apple-ios-sim" => ("ios-arm64-simulator", "ios_simulator_arm64"),
        // The CI package currently contains the Apple Silicon simulator slice only.
        // Fail clearly for unsupported simulator architectures instead of silently
        // producing an app with an unresolved Kotlin/Native framework symbol.
        "x86_64-apple-ios" => {
            panic!("The LegadoSourceEngine XCFramework is packaged for aarch64-apple-ios-sim only")
        }
        _ if target.contains("apple-ios") => {
            panic!("Unsupported iOS target for LegadoSourceEngine linking: {target}")
        }
        _ => return,
    };

    println!("cargo:rerun-if-env-changed=TARGET");
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let slice_dir = manifest_dir
        .join("ios/Frameworks/LegadoSourceEngine.xcframework")
        .join(slice);
    let framework_dir = slice_dir.join("LegadoSourceEngine.framework");
    if !framework_dir.join("LegadoSourceEngine").is_file() {
        panic!(
            "Missing LegadoSourceEngine framework slice for {target} at {}; build and package the Kotlin/Native XCFramework before building the iOS Tauri app",
            framework_dir.display()
        );
    }

    println!("cargo:rerun-if-changed={}", framework_dir.display());
    println!("cargo:rustc-link-search=framework={}", slice_dir.display());
    println!("cargo:rustc-link-lib=framework=LegadoSourceEngine");

    // Kotlin/Native packages its static framework separately from its C archives. The
    // framework's Gradle linker options are not forwarded into Tauri's final Rust link, so
    // link the generated mbedTLS archive and Apple's system sqlite3 library at the app link.
    let ios_native_libs = manifest_dir
        .join("../../../kotlin/kmp-engine/build/iosNativeLibs")
        .join(native_lib_target);
    let mbedtls_archive = ios_native_libs.join("libmbedtls.a");
    if !mbedtls_archive.is_file() {
        panic!(
            "Missing Kotlin/Native mbedTLS archive for {target} at {}; build the iOS KMP framework before linking the Tauri app",
            mbedtls_archive.display()
        );
    }

    println!("cargo:rerun-if-changed={}", mbedtls_archive.display());
    println!(
        "cargo:rustc-link-search=native={}",
        ios_native_libs.display()
    );
    println!("cargo:rustc-link-lib=static=mbedtls");
    println!("cargo:rustc-link-lib=sqlite3");
}
