use std::{env, ffi::OsStr};

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

    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
