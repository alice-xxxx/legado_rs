fn main() {
    println!("cargo:rustc-check-cfg=cfg(mobile)");
    println!("cargo:rerun-if-changed=../kotlin/kmp-engine/src/commonMain/kotlin/io/legado/sourceengine/bridge/SourceEngineHost.kt");
    println!("cargo:rerun-if-changed=../kotlin/kmp-engine/src/commonMain/kotlin/io/legado/sourceengine/bridge/SourceEngineCallCodec.kt");
    println!("cargo:rerun-if-changed=../kotlin/kmp-engine/src/commonMain/kotlin/io/legado/sourceengine/bridge/WebBookSourceRuleExecutor.kt");
    println!("cargo:rerun-if-changed=../kotlin/kmp-engine/src/jvmMain/kotlin/io/legado/sourceengine/bridge/SourceEngineEmbedded.kt");
    println!("cargo:rerun-if-changed=../kotlin/kmp-engine/src/jvmMain/kotlin/io/legado/sourceengine/bridge/http/SourceEngineNativeHost.kt");
    // Packaging/config generation belongs to real app builds. Pure library
    // tests and the iOS source-engine archive omit this build dependency.
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    {
        // Tauri copies the bundled JRE's read-only license files into the
        // profile directory. Make those generated copies writable before an
        // incremental build replaces them; never follow resource symlinks.
        if let Some(profile) = std::env::var_os("OUT_DIR")
            .as_deref()
            .and_then(|out| std::path::Path::new(out).ancestors().nth(3))
        {
            let resources = profile.join("source-engine");
            if std::fs::symlink_metadata(&resources)
                .is_ok_and(|metadata| metadata.file_type().is_dir())
            {
                make_generated_resources_writable(&resources)
                    .expect("Cannot prepare generated source-engine resources");
            }
        }
        tauri_build::build()
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
fn make_generated_resources_writable(directory: &std::path::Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            make_generated_resources_writable(&entry.path())?;
        } else if kind.is_file() {
            let mut permissions = entry.metadata()?.permissions();
            if permissions.readonly() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    permissions.set_mode(permissions.mode() | 0o200);
                }
                #[cfg(not(unix))]
                permissions.set_readonly(false);
                std::fs::set_permissions(entry.path(), permissions)?;
            }
        }
    }
    Ok(())
}
