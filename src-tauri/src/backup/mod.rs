//! 版本化资源快照格式与备份模块入口。
//! 归档包含公开资源及恢复书源所需的私有记录；可重建缓存和日志留在本机。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod create;
mod restore;

pub use create::create_backup;
pub use restore::recover_interrupted_restore;
pub(crate) use restore::restore_backup;

const BACKUP_FORMAT: &str = "legado-rs-resource-backup";
const BACKUP_VERSION: u32 = 1;
const MANIFEST_PATH: &str = "manifest.json";
const MAX_ENTRIES: usize = 100_000;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const BACKED_SOURCE_STORAGE: &[&str] = &["cookies", "book-variables", "explore-kinds"];
const DEVICE_SOURCE_CACHES: &[&str] = &["source-cache", "file-cache", "persistent-file-cache"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    format: String,
    backup_version: u32,
    app_schema_version: u32,
    files: Vec<BackupEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupEntry {
    path: String,
    size: u64,
    sha256: String,
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
