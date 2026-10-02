use std::{env, ffi::OsStr};

const COMMANDS: &[&str] = &["execute"];

fn main() {
    // Kotlin/Native first needs the Rust archive to build the iOS framework, which creates the
    // XCFramework consumed by this Swift package. Skip package generation for that bootstrap step;
    // the later Tauri iOS build reruns this script without the flag after the XCFramework exists.
    println!("cargo:rerun-if-env-changed=LEGADO_KMP_BOOTSTRAP");
    if env::var_os("LEGADO_KMP_BOOTSTRAP").as_deref() == Some(OsStr::new("1")) {
        return;
    }

    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
