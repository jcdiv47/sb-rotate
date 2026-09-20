//! Filesystem checks shared by config snapshots and mutation locks.
use std::{
    fs::{self, File, Metadata},
    io::{Read, Write},
    path::Path,
    time::SystemTime,
};

#[cfg(unix)]
use std::fs::Permissions;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use tempfile::{Builder, TempPath};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileState {
    length: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    mode: u32,
    #[cfg(not(unix))]
    readonly: bool,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    links: u64,
    #[cfg(unix)]
    uid: u32,
    #[cfg(unix)]
    gid: u32,
}

impl FileState {
    pub fn from_metadata(metadata: &Metadata) -> Result<Self> {
        ensure!(metadata.is_file(), "not a regular file");
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            mode: metadata.mode(),
            #[cfg(not(unix))]
            readonly: metadata.permissions().readonly(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            links: metadata.nlink(),
            #[cfg(unix)]
            uid: metadata.uid(),
            #[cfg(unix)]
            gid: metadata.gid(),
        })
    }

    pub fn ensure_single_link(&self, path: &Path) -> Result<()> {
        #[cfg(unix)]
        ensure!(
            self.links == 1,
            "refusing to replace or lock a hard-linked file: {}",
            path.display()
        );
        #[cfg(not(unix))]
        let _ = path;
        Ok(())
    }

    pub fn ensure_replaceable(&self, path: &Path) -> Result<()> {
        self.ensure_single_link(path)?;
        #[cfg(windows)]
        ensure!(
            !self.readonly,
            "cannot atomically replace a read-only Windows config: {}",
            path.display()
        );
        Ok(())
    }

    /// A restored original has a new inode/mtime; compare only its attributes.
    pub fn same_attributes(&self, other: &Self) -> bool {
        #[cfg(unix)]
        {
            (self.mode, self.uid, self.gid) == (other.mode, other.uid, other.gid)
        }
        #[cfg(not(unix))]
        {
            self.readonly == other.readonly
        }
    }

    pub fn restore_attributes(&self, file: &File) -> Result<()> {
        #[cfg(unix)]
        {
            use rustix::fs::{Gid, Uid, fchown};
            use std::os::unix::fs::MetadataExt;
            let metadata = file.metadata()?;
            // Avoid unnecessary chown calls, including when running unprivileged.
            if metadata.uid() != self.uid || metadata.gid() != self.gid {
                fchown(
                    file,
                    Some(Uid::from_raw(self.uid)),
                    Some(Gid::from_raw(self.gid)),
                )
                .context("preserving config owner/group")?;
            }
        }
        // chown can clear mode bits, so permissions are restored afterwards.
        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            Permissions::from_mode(self.mode)
        };
        #[cfg(not(unix))]
        let permissions = {
            let mut permissions = file.metadata()?.permissions();
            permissions.set_readonly(self.readonly);
            permissions
        };
        file.set_permissions(permissions)
            .context("preserving config permissions")?;
        Ok(())
    }
}

/// Open without following a substituted final symlink or blocking on a FIFO.
/// Parent directories must be trusted; this is not a sandbox for hostile paths.
pub(crate) fn open_regular(path: &Path, create_lock: bool) -> Result<File> {
    #[cfg(unix)]
    let file: File = {
        use rustix::fs::{Mode, OFlags, open};
        let access = if create_lock {
            OFlags::RDWR | OFlags::CREATE
        } else {
            OFlags::RDONLY
        };
        open(
            path,
            access | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::RUSR | Mode::WUSR,
        )
        .with_context(|| format!("opening {}", path.display()))?
        .into()
    };
    #[cfg(not(unix))]
    let file = {
        let mut options = fs::OpenOptions::new();
        options
            .read(true)
            .write(create_lock)
            .create(create_lock)
            .truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_FLAG_OPEN_REPARSE_POINT: do not follow a final symlink.
            options.custom_flags(0x0020_0000);
        }
        options
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?
    };
    ensure!(
        file.metadata()?.is_file() && !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "not a regular non-symlink file: {}",
        path.display()
    );
    Ok(file)
}

pub(crate) fn read_snapshot(path: &Path) -> Result<(Vec<u8>, FileState)> {
    let mut file = open_regular(path, false)?;
    let before = FileState::from_metadata(&file.metadata()?)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("reading {}", path.display()))?;
    let after = FileState::from_metadata(&file.metadata()?)?;
    let at_path = FileState::from_metadata(&fs::symlink_metadata(path)?)?;
    ensure!(
        before == after && after == at_path && after.length == bytes.len() as u64,
        "file changed while reading: {}",
        path.display()
    );
    Ok((bytes, after))
}

pub(crate) fn stage(path: &Path, bytes: &[u8], state: &FileState) -> Result<TempPath> {
    let parent = path
        .parent()
        .context("config file has no parent directory")?;
    let mut temp = Builder::new()
        .prefix(".sb-rotate-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .with_context(|| format!("staging {}", path.display()))?;
    temp.write_all(bytes)?;
    state.restore_attributes(temp.as_file())?;
    temp.as_file().sync_all()?;
    Ok(temp.into_temp_path())
}

pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?
        .sync_all()
        .with_context(|| format!("syncing directory {}", path.display()))?;
    // Portable directory fsync is not available on Windows. File data is still
    // flushed, but Windows recovery guarantees cover process interruption only.
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(crate) fn private_directory(parent: &Path, prefix: &str) -> Result<tempfile::TempDir> {
    let mut builder = Builder::new();
    builder.prefix(prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(Permissions::from_mode(0o700));
    }
    Ok(builder.tempdir_in(parent)?)
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
