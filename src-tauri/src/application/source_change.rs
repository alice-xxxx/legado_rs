//! 书籍和章节书源切换事务入口。

use super::*;

mod book;
mod chapter;
mod identity;
mod transaction;

pub(super) use identity::{
    candidate_identity_match, chapter_id_for_generation, normalize_identity,
    remove_file_if_present, source_definition_fingerprint,
};
pub(super) use transaction::{
    format_catalog_refresh_transaction_error, format_source_change_transaction_error,
};
