//! 本地书籍解析共用的稳定短哈希。

use sha2::{Digest, Sha256};
pub(super) fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..10]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
