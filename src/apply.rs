//! Stage and validate every edit before publishing a recoverable transaction.
//! Replacements are atomic per file. Persistent journals permit conservative
//! rollback after interruption; they do not make cross-file reads atomic.
use std::{collections::BTreeMap, fs};

use anyhow::{Context, Result, bail};
use tempfile::Builder;

use crate::{
    config::ConfigSet,
    fsutil::stage,
    locking::DirectoryLocks,
    plan::RotationPlan,
    recovery::{Transaction, ensure_no_pending},
    singbox::SingBox,
};

pub fn check(configs: &ConfigSet, singbox: &impl SingBox) -> Result<()> {
    if configs.server_is_directory {
        singbox.check_directory(&configs.server)?;
    } else {
        singbox.check_file(&configs.server)?;
    }
    for file in &configs.client_files {
        singbox.check_file(file)?;
    }
    Ok(())
}

pub fn apply(configs: &ConfigSet, plan: &RotationPlan, singbox: &impl SingBox) -> Result<usize> {
    let directories = configs.lock_directories()?;
    // Even an otherwise no-op mutation must not report success over a pending
    // transaction. This read-only check creates no locks for ordinary no-ops.
    ensure_no_pending(&directories)?;
    let changed = plan.materialize(configs)?;
    if changed.is_empty() {
        return Ok(0);
    }
    let locks = DirectoryLocks::acquire(directories.clone())?;
    ensure_no_pending(&directories)?;
    configs.verify_unchanged()?;
    for path in changed.keys() {
        configs.documents[path].state.ensure_replaceable(path)?;
    }
    let mut staged = BTreeMap::new();
    for (path, value) in &changed {
        let mut bytes = serde_json::to_vec_pretty(value)?;
        bytes.push(b'\n');
        staged.insert(
            path.clone(),
            stage(path, &bytes, &configs.documents[path].state)?,
        );
    }

    let server_temp = private_server_directory()?;
    for (name, path) in &configs.server_layout {
        let bytes = match staged.get(path) {
            Some(temp) => fs::read(temp)?,
            None => configs.documents[path].original.clone(),
        };
        fs::write(server_temp.path().join(name), bytes)?;
    }
    if configs.server_is_directory {
        singbox.check_directory(server_temp.path())?;
    } else {
        let name = configs
            .server
            .file_name()
            .context("server has no filename")?;
        singbox.check_file(&server_temp.path().join(name))?;
    }
    for path in &configs.client_files {
        if let Some(temp) = staged.get(path) {
            singbox.check_file(temp)?;
        }
    }
    locks.verify()?;
    configs.verify_unchanged()?;
    let transaction = Transaction::prepare(configs, &staged, &directories)?;
    if let Err(error) = locks.verify().and_then(|()| configs.verify_unchanged()) {
        if let Err(cleanup) = transaction.cancel_unstarted() {
            bail!("{error:#}; journal cleanup failed: {cleanup:#}");
        }
        return Err(error);
    }
    transaction.commit(&staged, |source, destination| {
        configs
            .verify_file(destination)
            .map_err(std::io::Error::other)?;
        fs::rename(source, destination)
    })?;
    Ok(changed.len())
}

fn private_server_directory() -> Result<tempfile::TempDir> {
    let mut builder = Builder::new();
    builder.prefix("sb-rotate-server-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    Ok(builder.tempdir()?)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn server_snapshot_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let directory = private_server_directory().unwrap();
        let mode = fs::metadata(directory.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }
}
