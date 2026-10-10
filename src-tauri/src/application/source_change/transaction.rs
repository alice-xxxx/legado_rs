//! 书源事务错误说明与文件清理辅助函数。

pub(in crate::application) fn format_source_change_transaction_error(
    commit_state: crate::resource_transactions::CommitState,
    recovery_required: bool,
    detail: &str,
) -> String {
    let outcome = match commit_state {
        crate::resource_transactions::CommitState::NotCommitted if recovery_required => {
            "No commit was recorded, but rollback is incomplete and resource files may be mixed. Restart the application so startup recovery can finish before retrying."
        }
        crate::resource_transactions::CommitState::NotCommitted => {
            "Source replacement was not committed; all changes were rolled back or no target was written."
        }
        crate::resource_transactions::CommitState::Committed => {
            "Source replacement was committed and the new source data is active, but cleanup or recovery is pending. Restart the application to finish recovery before retrying."
        }
        crate::resource_transactions::CommitState::Indeterminate => {
            "Source replacement commit status is indeterminate. Do not retry; restart the application to recover the transaction before further writes."
        }
    };
    format!("{outcome} Details: {detail}")
}

pub(in crate::application) fn format_catalog_refresh_transaction_error(
    commit_state: crate::resource_transactions::CommitState,
    recovery_required: bool,
    detail: &str,
) -> String {
    let outcome = match (commit_state, recovery_required) {
        (crate::resource_transactions::CommitState::NotCommitted, false) => {
            "Catalog refresh was not committed; all changes were rolled back or no target was written."
        }
        (crate::resource_transactions::CommitState::NotCommitted, true) => {
            "Catalog refresh was not committed, but rollback is incomplete and resource files may be mixed. Restart the application before retrying."
        }
        (crate::resource_transactions::CommitState::Committed, _) => {
            "Catalog refresh was committed and is active, but cleanup or recovery is pending. Restart the application before further writes."
        }
        (crate::resource_transactions::CommitState::Indeterminate, _) => {
            "Catalog refresh commit status is indeterminate. Do not retry; restart the application to recover the transaction before further writes."
        }
    };
    format!("{outcome} Details: {detail}")
}
