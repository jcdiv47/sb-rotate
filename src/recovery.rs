//! Durable rollback journals. A pending marker in every locked directory blocks
//! overlapping mutations until recovery has completed. No protocol knowledge is
//! needed, and recovery never requires a working sing-box executable.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tempfile::{Builder, TempPath};

use crate::{
    config::{ConfigSet, json_layout},
    fsutil::{FileState, private_directory, read_snapshot, stage, sync_directory, write_private},
    journal_path::JournalPath,
    locking::DirectoryLocks,
};

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;

const VERSION: u32 = 1;
const PREFIX: &str = ".sb-rotate-transaction-";
const PENDING: &str = ".sb-rotate.pending";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Signature {
    sha256: String,
    state: FileState,
}
impl Signature {
    fn new(bytes: &[u8], state: FileState) -> Self {
        Self {
            sha256: digest(bytes),
            state,
        }
    }
    fn matches(&self, bytes: &[u8], state: &FileState) -> bool {
        self.state == *state && self.sha256 == digest(bytes)
    }
    fn matches_original(&self, bytes: &[u8], state: &FileState) -> bool {
        self.state.same_attributes(state) && self.sha256 == digest(bytes)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    file: JournalPath,
    staged: JournalPath,
    original: Signature,
    installed: Signature,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryView {
    directory: JournalPath,
    files: BTreeMap<JournalPath, JournalPath>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedFile {
    file: JournalPath,
    signature: Signature,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    platform: String,
    directories: BTreeSet<JournalPath>,
    entries: Vec<Entry>,
    sources: BTreeMap<JournalPath, JournalPath>,
    inventories: Vec<InventoryView>,
    unchanged: Vec<ObservedFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    version: u32,
    journal: JournalPath,
}

pub(crate) struct Transaction {
    path: PathBuf,
    manifest: Manifest,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Original,
    Installed,
    Conflict,
}
impl Status {
    fn name(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Installed => "installed replacement",
            Self::Conflict => "CONFLICT (externally changed)",
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let (bytes, _) = read_snapshot(path)?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("reading journal metadata {}", path.display()))
}
fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("examining {}", path.display())),
    }
}

/// Locate the journal reported by a config directory's pending marker.
pub fn pending_journal(directory: &Path) -> Result<Option<PathBuf>> {
    let marker = directory.join(PENDING);
    if !exists(&marker)? {
        return Ok(None);
    }
    let marker: Marker = read_json(&marker)?;
    ensure!(
        marker.version == VERSION && marker.journal.0.is_absolute(),
        "invalid pending transaction marker"
    );
    Ok(Some(marker.journal.0))
}

/// Fail before discovery/generation when a mutation overlaps a pending journal.
pub fn check_pending(configs: &ConfigSet) -> Result<()> {
    ensure_no_pending(&configs.lock_directories()?)
}

pub(crate) fn ensure_no_pending(directories: &BTreeSet<PathBuf>) -> Result<()> {
    for directory in directories {
        if let Some(journal) = pending_journal(directory)? {
            bail!(
                "pending config transaction in {}; journal: {}; run sb-rotate recover --directory <config-directory> --dry-run before making further changes",
                directory.display(),
                journal.display()
            );
        }
    }
    Ok(())
}

impl Transaction {
    /// The caller must hold every directory lock through preparation and commit.
    pub(crate) fn prepare(
        configs: &ConfigSet,
        staged: &BTreeMap<PathBuf, TempPath>,
        directories: &BTreeSet<PathBuf>,
    ) -> Result<Self> {
        ensure_no_pending(directories)?;
        ensure!(!staged.is_empty(), "cannot journal an empty transaction");
        let parent = directories.first().context("no transaction directories")?;
        let temp = private_directory(parent, PREFIX)?;
        let mut entries = Vec::new();
        for (index, (file, replacement)) in staged.iter().enumerate() {
            let doc = configs
                .documents
                .get(file)
                .context("transaction file not loaded")?;
            let (bytes, state) = read_snapshot(replacement)?;
            write_private(
                &temp.path().join(format!("{index}.original")),
                &doc.original,
            )?;
            entries.push(Entry {
                file: JournalPath(file.clone()),
                staged: JournalPath(replacement.to_path_buf()),
                original: Signature::new(&doc.original, doc.state.clone()),
                installed: Signature::new(&bytes, state),
            });
        }
        let manifest = Manifest {
            version: VERSION,
            platform: std::env::consts::OS.to_owned(),
            directories: directories.iter().cloned().map(JournalPath).collect(),
            entries,
            sources: configs
                .source_aliases()
                .iter()
                .map(|(source, target)| (JournalPath(source.clone()), JournalPath(target.clone())))
                .collect(),
            inventories: configs
                .inventory_layouts()
                .into_iter()
                .map(|(directory, layout)| InventoryView {
                    directory: JournalPath(directory),
                    files: layout
                        .into_iter()
                        .map(|(name, target)| (JournalPath(name.into()), JournalPath(target)))
                        .collect(),
                })
                .collect(),
            unchanged: configs
                .documents
                .iter()
                .filter(|(file, _)| !staged.contains_key(*file))
                .map(|(file, doc)| ObservedFile {
                    file: JournalPath(file.clone()),
                    signature: Signature::new(&doc.original, doc.state.clone()),
                })
                .collect(),
        };
        write_private(
            &temp.path().join("manifest.json"),
            &serde_json::to_vec_pretty(&manifest)?,
        )?;
        sync_directory(temp.path())?;
        sync_directory(parent)?;
        let transaction = Self {
            path: temp.path().to_path_buf(),
            manifest,
        };
        transaction.validate()?;
        let _kept = temp.keep();
        // From here on, failed/partial publication must keep the journal: an
        // already-published marker may refer to it even after an error or crash.
        let publish = || -> Result<()> {
            let marker = serde_json::to_vec(&Marker {
                version: VERSION,
                journal: JournalPath(transaction.path.clone()),
            })?;
            for directory in directories {
                let mut temp = Builder::new()
                    .prefix(".sb-rotate-marker-")
                    .suffix(".tmp")
                    .tempfile_in(directory)?;
                temp.write_all(&marker)?;
                temp.as_file().sync_all()?;
                drop(temp.persist_noclobber(directory.join(PENDING))?);
                sync_directory(directory)?;
            }
            Ok(())
        };
        publish().with_context(|| transaction.hint())?;
        Ok(transaction)
    }

    fn load(path: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(path).context("opening recovery journal directory")?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "journal must be a real directory, not a symlink"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                metadata.uid() == rustix::process::geteuid().as_raw()
                    && metadata.mode() & 0o077 == 0,
                "journal must be private (0700) and owned by the current user"
            );
        }
        let path = fs::canonicalize(path)?;
        let manifest = read_json(&path.join("manifest.json"))?;
        let transaction = Self { path, manifest };
        transaction.validate()?;
        Ok(transaction)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.manifest.version == VERSION,
            "unsupported recovery journal version"
        );
        ensure!(
            self.manifest.platform == std::env::consts::OS,
            "journal belongs to another platform"
        );
        ensure!(
            self.path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(PREFIX)),
            "not an sb-rotate transaction directory"
        );
        ensure!(!self.manifest.entries.is_empty(), "empty recovery journal");
        let directories = self.directories();
        ensure!(
            self.path
                .parent()
                .is_some_and(|parent| directories.contains(parent)),
            "journal parent is outside its lock set"
        );
        for directory in &directories {
            ensure!(
                directory.is_absolute()
                    && directory.is_dir()
                    && fs::canonicalize(directory)? == *directory,
                "journal directory path changed: {}",
                directory.display()
            );
        }
        let mut seen = BTreeSet::new();
        for entry in &self.manifest.entries {
            let file = &entry.file.0;
            let temp = &entry.staged.0;
            ensure!(
                file.is_absolute()
                    && file
                        .parent()
                        .is_some_and(|parent| directories.contains(parent))
                    && seen.insert(file),
                "invalid or duplicate journal target"
            );
            ensure!(
                !file
                    .file_name()
                    .context("target has no filename")?
                    .to_string_lossy()
                    .starts_with(".sb-rotate"),
                "reserved transaction target name"
            );
            ensure!(
                temp.parent() == file.parent()
                    && temp.file_name().is_some_and(|name| {
                        let name = name.to_string_lossy();
                        name.starts_with(".sb-rotate-") && name.ends_with(".tmp")
                    }),
                "invalid staged path in journal"
            );
            ensure!(
                entry.original.state.same_attributes(&entry.installed.state),
                "journal changes file attributes"
            );
            entry.original.state.ensure_replaceable(file)?;
            entry.installed.state.ensure_replaceable(file)?;
            for signature in [&entry.original, &entry.installed] {
                ensure!(
                    signature.sha256.len() == 64
                        && signature.sha256.bytes().all(|c| c.is_ascii_hexdigit()),
                    "invalid journal checksum"
                );
            }
        }
        let in_scope = |path: &Path| {
            path.is_absolute()
                && (directories.contains(path)
                    || path
                        .parent()
                        .is_some_and(|parent| directories.contains(parent)))
        };
        for (source, expected) in &self.manifest.sources {
            ensure!(
                source.0.is_absolute() && in_scope(&expected.0),
                "invalid source alias in journal"
            );
        }
        for inventory in &self.manifest.inventories {
            ensure!(
                directories.contains(&inventory.directory.0),
                "inventory is outside journal lock set"
            );
            for (name, target) in &inventory.files {
                ensure!(
                    name.0
                        .file_name()
                        .is_some_and(|file| file == name.0.as_os_str())
                        && in_scope(&target.0),
                    "invalid directory inventory entry"
                );
            }
        }
        for observed in &self.manifest.unchanged {
            ensure!(
                in_scope(&observed.file.0) && seen.insert(&observed.file.0),
                "invalid unchanged-file snapshot"
            );
        }
        Ok(())
    }

    fn directories(&self) -> BTreeSet<PathBuf> {
        self.manifest
            .directories
            .iter()
            .map(|path| path.0.clone())
            .collect()
    }
    fn hint(&self) -> String {
        format!(
            "recovery journal retained at {}; use sb-rotate recover --journal <path> --dry-run",
            self.path.display()
        )
    }
    fn flag(&self, name: &str) -> Result<bool> {
        let path = self.path.join(name);
        if !exists(&path)? {
            return Ok(false);
        }
        ensure!(
            read_snapshot(&path)?.0 == b"1\n",
            "invalid transaction completion flag"
        );
        Ok(true)
    }
    fn set_flag(&self, name: &str) -> Result<()> {
        write_private(&self.path.join(name), b"1\n")?;
        sync_directory(&self.path)
    }
    fn finished(&self) -> Result<Option<&'static str>> {
        let committed = self.flag("committed")?;
        let rolled_back = self.flag("rolled-back")?;
        ensure!(
            !(committed && rolled_back),
            "conflicting transaction completion flags"
        );
        ensure!(
            !(committed || rolled_back) || self.flag("started")?,
            "completed transaction has no start decision"
        );
        Ok(if committed {
            Some("committed")
        } else if rolled_back {
            Some("rolled back")
        } else {
            None
        })
    }
    fn markers(&self, require_all: bool) -> Result<()> {
        for directory in self.directories() {
            match pending_journal(&directory)? {
                Some(journal) => ensure!(
                    journal == self.path,
                    "directory belongs to a different pending transaction: {}",
                    directory.display()
                ),
                None => ensure!(
                    !require_all,
                    "pending transaction marker is missing in {}; refusing automatic rollback",
                    directory.display()
                ),
            }
        }
        Ok(())
    }
    fn check_sources(&self) -> Result<()> {
        for (source, expected) in &self.manifest.sources {
            ensure!(
                fs::canonicalize(&source.0)? == expected.0,
                "input source path changed: {}",
                source.0.display()
            );
        }
        for inventory in &self.manifest.inventories {
            let current: BTreeMap<_, _> = json_layout(&inventory.directory.0)?
                .into_iter()
                .map(|(name, target)| (JournalPath(name.into()), JournalPath(target)))
                .collect();
            ensure!(
                current == inventory.files,
                "config directory membership changed: {}",
                inventory.directory.0.display()
            );
        }
        for observed in &self.manifest.unchanged {
            let (bytes, state) = read_snapshot(&observed.file.0)?;
            ensure!(
                observed.signature.matches_original(&bytes, &state),
                "previously unchanged config was edited: {}",
                observed.file.0.display()
            );
        }
        Ok(())
    }

    fn original(&self, index: usize, entry: &Entry) -> Result<Vec<u8>> {
        let (bytes, _) = read_snapshot(&self.path.join(format!("{index}.original")))?;
        ensure!(
            digest(&bytes) == entry.original.sha256,
            "original backup checksum mismatch for {}",
            entry.file.0.display()
        );
        Ok(bytes)
    }
    fn classify(entry: &Entry, bytes: &[u8], state: &FileState) -> Status {
        if entry.original.matches_original(bytes, state) {
            Status::Original
        } else if entry.installed.matches(bytes, state) {
            Status::Installed
        } else {
            Status::Conflict
        }
    }

    /// Mark intent durably before the first replacement. Ordinary failures use
    /// the same conservative rollback path as an interrupted invocation.
    pub(crate) fn commit(
        &self,
        staged: &BTreeMap<PathBuf, TempPath>,
        mut replace: impl FnMut(&Path, &Path) -> std::io::Result<()>,
    ) -> Result<()> {
        self.set_flag("started").with_context(|| self.hint())?;
        for (path, temp) in staged {
            let result = replace(temp, path)
                .map_err(anyhow::Error::from)
                .and_then(|()| sync_directory(path.parent().context("target has no parent")?));
            if let Err(error) = result {
                match self.rollback() {
                    Ok(()) => bail!(
                        "replacing {} failed: {error}; prior replacements rolled back",
                        path.display()
                    ),
                    Err(rollback) => bail!(
                        "replacing {} failed: {error}; ROLLBACK INCOMPLETE: {rollback:#}; {}",
                        path.display(),
                        self.hint()
                    ),
                }
            }
        }
        // Once this decision is durable, recovery only cleans metadata: it must
        // never undo an already-successful commit, even after subsequent edits.
        self.set_flag("committed").with_context(|| self.hint())?;
        self.cleanup().with_context(|| self.hint())
    }

    pub(crate) fn cancel_unstarted(&self) -> Result<()> {
        ensure!(
            !self.flag("started")?,
            "cannot cancel a started transaction"
        );
        self.cleanup().with_context(|| self.hint())
    }

    fn rollback(&self) -> Result<()> {
        self.markers(true)?;
        self.check_sources()?;
        let mut restore = BTreeMap::new();
        // Check every target and backup before changing even one file.
        for (index, entry) in self.manifest.entries.iter().enumerate() {
            let original = self.original(index, entry)?;
            let current = read_snapshot(&entry.file.0)
                .with_context(|| format!("recovery conflict at {}", entry.file.0.display()))?;
            current.1.ensure_replaceable(&entry.file.0)?;
            match Self::classify(entry, &current.0, &current.1) {
                Status::Original => {}
                Status::Installed => {
                    restore.insert(index, (original, current));
                }
                Status::Conflict => bail!(
                    "recovery conflict at {}: refusing to overwrite an external edit",
                    entry.file.0.display()
                ),
            }
        }
        let mut prepared = BTreeMap::new();
        for (index, (original, _)) in &restore {
            let entry = &self.manifest.entries[*index];
            prepared.insert(
                *index,
                stage(&entry.file.0, original, &entry.original.state)?,
            );
        }
        for (index, temp) in prepared {
            let entry = &self.manifest.entries[index];
            ensure!(
                read_snapshot(&entry.file.0)? == restore[&index].1,
                "recovery conflict at {}: file changed during rollback",
                entry.file.0.display()
            );
            fs::rename(&temp, &entry.file.0)?;
            sync_directory(entry.file.0.parent().context("target has no parent")?)?;
        }
        // A repeat recovery recognizes restored originals by bytes + attributes,
        // not inode/mtime. Thus interruption during rollback is safe to resume.
        for entry in &self.manifest.entries {
            let (bytes, state) = read_snapshot(&entry.file.0)?;
            ensure!(
                Self::classify(entry, &bytes, &state) == Status::Original,
                "recovery conflict at {} after rollback",
                entry.file.0.display()
            );
        }
        self.check_sources()?;
        self.set_flag("rolled-back")?;
        self.cleanup()
    }

    fn cleanup(&self) -> Result<()> {
        self.markers(false)?;
        for entry in &self.manifest.entries {
            if exists(&entry.staged.0)? {
                let (bytes, state) = read_snapshot(&entry.staged.0)?;
                ensure!(
                    entry.installed.matches(&bytes, &state),
                    "staged recovery file was modified; refusing to delete {}",
                    entry.staged.0.display()
                );
                fs::remove_file(&entry.staged.0)?;
                sync_directory(
                    entry
                        .staged
                        .0
                        .parent()
                        .context("staged path has no parent")?,
                )?;
            }
        }
        // Remove all public markers before deleting the only recovery copies.
        // Finished decisions make interruption midway through cleanup harmless.
        for directory in self.directories() {
            let marker = directory.join(PENDING);
            if exists(&marker)? {
                fs::remove_file(marker)?;
                sync_directory(&directory)?;
            }
        }
        fs::remove_dir_all(&self.path)?;
        sync_directory(self.path.parent().context("journal has no parent")?)
    }
}

/// Restore an unfinished transaction, or clean a finalized/not-started journal.
/// Dry runs hold cooperative locks but never edit configs or transaction metadata.
pub fn recover(path: &Path, dry_run: bool) -> Result<String> {
    let transaction = Transaction::load(path)?;
    let locks = DirectoryLocks::acquire(transaction.directories())?;
    let finished = transaction.finished()?;
    let started = transaction.flag("started")?;
    transaction.markers(started && finished.is_none())?;
    let mut lines = vec![format!("Transaction: {}", transaction.path.display())];
    if let Some(status) = finished {
        lines.push(format!(
            "Already {status}; only recovery metadata will be cleaned."
        ));
    } else if !started {
        lines.push("Commit not started; config files will not be changed.".to_owned());
    } else {
        let source_conflict = transaction.check_sources().err();
        let mut conflicts = source_conflict.is_some();
        if let Some(error) = source_conflict {
            lines.push(format!("  CONFLICT: {error:#}"));
        }
        for (index, entry) in transaction.manifest.entries.iter().enumerate() {
            transaction.original(index, entry)?;
            let status = match read_snapshot(&entry.file.0) {
                Ok((bytes, state)) if state.ensure_replaceable(&entry.file.0).is_ok() => {
                    Transaction::classify(entry, &bytes, &state)
                }
                Ok(_) => Status::Conflict,
                Err(_) => Status::Conflict,
            };
            lines.push(format!("  {}: {}", entry.file.0.display(), status.name()));
            conflicts |= status == Status::Conflict;
        }
        if !dry_run {
            ensure!(
                !conflicts,
                "{}\nRecovery refused: resolve external edits before retrying; journal retained.",
                lines.join("\n")
            );
        } else if conflicts {
            lines.push("Recovery would be refused because of conflicts.".to_owned());
        }
    }
    locks.verify()?;
    if dry_run {
        lines.push("Preview only; no configs or recovery metadata changed.".to_owned());
    } else {
        if finished.is_some() || !started {
            transaction.cleanup()
        } else {
            transaction.rollback()
        }
        .with_context(|| transaction.hint())?;
        lines.push("Recovery complete; pending markers and journal removed.".to_owned());
    }
    Ok(lines.join("\n"))
}
