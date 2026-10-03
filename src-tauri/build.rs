fn main() {
    println!("cargo:rustc-check-cfg=cfg(mobile)");
    // Packaging/config generation belongs to real app builds. Pure library
    // tests and the iOS source-engine archive omit this build dependency.
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    {
        tauri_build::build()
    }
}
