//! macOS JVM 动态库位置和原生路径。
pub(crate) use crate::desktop::init;
use std::path::{Path, PathBuf};
pub(crate) const QUICKJS: &str = "liblegado_quickjs.dylib";
pub(crate) fn java_path(path: &Path) -> PathBuf {
    path.to_owned()
}
pub(crate) fn jvm_library(home: &Path) -> PathBuf {
    home.join("lib/server/libjvm.dylib")
}
