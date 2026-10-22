//! 数据目录进程锁与备份恢复错误；业务事务和缓存清理由 database 负责。

use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};

/// 锁文件保留在数据目录外，备份恢复替换目录时仍持有同一个进程锁。
pub(crate) struct AppDataProcessLock {
    _file: File,
    canonical_root: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommitState {
    NotCommitted,
    Committed,
    Indeterminate,
}

#[derive(Debug)]
pub(crate) struct TransactionError {
    message: String,
    commit_state: CommitState,
    recovery_required: bool,
}

impl TransactionError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            commit_state: CommitState::NotCommitted,
            recovery_required: false,
        }
    }

    pub(crate) fn recovery_required(message: impl Into<String>, commit_state: CommitState) -> Self {
        Self {
            message: message.into(),
            commit_state,
            recovery_required: true,
        }
    }

    pub(crate) fn commit_state(&self) -> CommitState {
        self.commit_state
    }

    pub(crate) fn recovery_required_flag(&self) -> bool {
        self.recovery_required
    }
}

impl fmt::Display for TransactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TransactionError {}

#[path = "transactions/process_lock.rs"]
mod process_lock;
