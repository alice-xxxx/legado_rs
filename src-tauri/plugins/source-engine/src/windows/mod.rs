//! Windows 动态库装载和 HotSpot 路径适配。
pub(crate) use crate::desktop::init;
use std::path::{Path, PathBuf};
pub(crate) const QUICKJS: &str = "legado_quickjs.dll";
pub(crate) fn java_path(path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(path) = value.strip_prefix(r"\\?\") {
        if path
            .get(..4)
            .is_some_and(|v| v.eq_ignore_ascii_case("UNC\\"))
        {
            return PathBuf::from(format!(r"\\{}", &path[4..]));
        }
        if path.as_bytes().get(1) == Some(&b':') {
            return PathBuf::from(path);
        }
    }
    path.to_owned()
}
pub(crate) fn jvm_library(home: &Path) -> PathBuf {
    let mut paths = vec![home.join("bin"), home.join("bin/server")];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    if let Ok(path) = std::env::join_paths(paths) {
        unsafe {
            std::env::set_var("PATH", path);
        }
    }
    home.join("bin/server/jvm.dll")
}
