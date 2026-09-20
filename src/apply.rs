//! Stage and validate the complete change before any replacement. Each replacement
//! is atomic; ordinary commit failures trigger rollback. Cross-file crash atomicity
//! (power loss / process termination) would require a persistent recovery journal.
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use tempfile::{Builder, NamedTempFile};

use crate::{
    config::{ConfigSet, json_layout},
    plan::RotationPlan,
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
    let changed = plan.materialize(configs)?;
    let mut staged = BTreeMap::new();
    for (path, value) in &changed {
        let mut bytes = serde_json::to_vec_pretty(value)?;
        bytes.push(b'\n');
        staged.insert(
            path.clone(),
            stage(path, &bytes, &configs.documents[path].permissions)?,
        );
    }

    // Validate the complete server set, including unchanged fragments. Preserve
    // original filenames rather than treating fragments as standalone configs.
    let server_temp = private_server_directory()?;
    for (name, path) in &configs.server_layout {
        let bytes = match staged.get(path) {
            Some(temp) => fs::read(temp.path())?,
            None => configs.documents[path].original.clone(),
        };
        let target = server_temp.path().join(name);
        fs::write(&target, bytes)?;
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
            singbox.check_file(temp.path())?;
        }
    }

    // Prepare rollback files before committing anything. Temp suffixes are not
    // .json, so a concurrent sing-box -C never loads these as config fragments.
    let mut backups = BTreeMap::new();
    for path in staged.keys() {
        let doc = &configs.documents[path];
        backups.insert(path.clone(), stage(path, &doc.original, &doc.permissions)?);
    }
    verify_unchanged(configs)?;
    commit(staged, backups, |source, destination| {
        fs::rename(source, destination)
    })?;
    Ok(changed.len())
}

fn private_server_directory() -> Result<tempfile::TempDir> {
    let mut builder = Builder::new();
    builder.prefix("sb-rotate-server-");
    // tempfile directories otherwise inherit the process umask (often 0755).
    // The snapshot includes private keys/passwords even in unchanged fragments.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    Ok(builder.tempdir()?)
}

fn stage(path: &Path, bytes: &[u8], permissions: &fs::Permissions) -> Result<NamedTempFile> {
    let parent = path
        .parent()
        .context("config file has no parent directory")?;
    let mut temp = Builder::new()
        .prefix(".sb-rotate-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .with_context(|| format!("staging {}", path.display()))?;
    temp.write_all(bytes)?;
    temp.as_file().set_permissions(permissions.clone())?;
    temp.as_file().sync_all()?;
    Ok(temp)
}

fn verify_unchanged(configs: &ConfigSet) -> Result<()> {
    if configs.server_is_directory {
        ensure!(
            json_layout(&configs.server)? == configs.server_layout,
            "server directory changed while planning; refusing to commit"
        );
    }
    for (path, doc) in &configs.documents {
        ensure!(
            fs::read(path).with_context(|| format!("rechecking {}", path.display()))?
                == doc.original,
            "config changed while planning: {}; refusing to overwrite",
            path.display()
        );
    }
    Ok(())
}

fn commit(
    staged: BTreeMap<PathBuf, NamedTempFile>,
    mut backups: BTreeMap<PathBuf, NamedTempFile>,
    mut replace: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    let mut committed: Vec<PathBuf> = Vec::new();
    for (path, temp) in staged {
        if let Err(error) = replace(temp.path(), &path) {
            let mut rollback_errors = Vec::new();
            for previous in committed.iter().rev() {
                let backup = backups
                    .remove(previous)
                    .expect("backup for every committed file");
                if let Err(rollback) = fs::rename(backup.path(), previous) {
                    // Keep the recovery copy if the filesystem also refuses rollback.
                    let recovery = backup.into_temp_path().keep();
                    rollback_errors.push(format!(
                        "{}: {rollback}; recovery copy: {recovery:?}",
                        previous.display()
                    ));
                }
            }
            if rollback_errors.is_empty() {
                bail!(
                    "replacing {} failed: {error}; prior replacements rolled back",
                    path.display()
                );
            }
            bail!(
                "replacing {} failed: {error}; ROLLBACK INCOMPLETE: {}",
                path.display(),
                rollback_errors.join("; ")
            );
        }
        committed.push(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn server_snapshot_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let directory = private_server_directory().unwrap();
        let mode = fs::metadata(directory.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }

    #[test]
    fn replacement_failure_restores_already_committed_files() {
        let directory = tempfile::tempdir().unwrap();
        let mut staged = BTreeMap::new();
        let mut backups = BTreeMap::new();
        for name in ["a.json", "b.json"] {
            let path = directory.path().join(name);
            fs::write(&path, b"original").unwrap();
            let permissions = fs::metadata(&path).unwrap().permissions();
            staged.insert(
                path.clone(),
                stage(&path, b"changed", &permissions).unwrap(),
            );
            backups.insert(
                path.clone(),
                stage(&path, b"original", &permissions).unwrap(),
            );
        }
        let mut calls = 0;
        let result = commit(staged, backups, |source, destination| {
            calls += 1;
            if calls == 2 {
                return Err(std::io::Error::other("injected replacement failure"));
            }
            fs::rename(source, destination)
        });
        assert!(result.unwrap_err().to_string().contains("rolled back"));
        for name in ["a.json", "b.json"] {
            assert_eq!(fs::read(directory.path().join(name)).unwrap(), b"original");
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }
}
