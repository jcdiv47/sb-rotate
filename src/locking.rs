//! Cooperative, fail-fast writer locks. Sidecars must remain in place: unlinking
//! a locked file would let a second writer lock a different inode at the same path.
use std::{
    collections::BTreeSet,
    fs::{self, File, TryLockError},
    path::PathBuf,
};

use anyhow::{Context, Result, bail, ensure};

use crate::fsutil::{FileState, open_regular};

struct DirectoryLock {
    path: PathBuf,
    state: FileState,
    _file: File,
}

pub(crate) struct DirectoryLocks {
    locks: Vec<DirectoryLock>,
}

impl DirectoryLocks {
    pub fn acquire(directories: BTreeSet<PathBuf>) -> Result<Self> {
        let mut locks = Vec::new();
        // Stable ordering and non-blocking acquisition avoid deadlocks across
        // overlapping server/client sets. Dropping any partial set unlocks it.
        for directory in directories {
            let path = directory.join(".sb-rotate.lock");
            let file = open_regular(&path, true)?;
            let state = FileState::from_metadata(&file.metadata()?)?;
            state.ensure_single_link(&path)?;
            match file.try_lock() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => bail!(
                    "another sb-rotate writer is using {}; retry after it finishes (do not delete {})",
                    directory.display(),
                    path.display()
                ),
                Err(TryLockError::Error(error)) => {
                    return Err(error).with_context(|| format!("locking {}", path.display()));
                }
            }
            locks.push(DirectoryLock {
                path,
                state,
                _file: file,
            });
        }
        let result = Self { locks };
        result.verify()?;
        Ok(result)
    }

    pub fn verify(&self) -> Result<()> {
        for lock in &self.locks {
            let current = fs::symlink_metadata(&lock.path)
                .with_context(|| format!("rechecking writer lock {}", lock.path.display()))?;
            ensure!(
                FileState::from_metadata(&current)? == lock.state,
                "writer lock changed while applying edits: {}; refusing to commit",
                lock.path.display()
            );
        }
        Ok(())
    }
}
