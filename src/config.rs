use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::{
    cli::Input,
    fsutil::{FileState, read_snapshot},
};

pub struct Document {
    pub value: Value,
    pub original: Vec<u8>,
    pub permissions: fs::Permissions,
    pub(crate) state: FileState,
}

pub struct ConfigSet {
    pub documents: BTreeMap<PathBuf, Document>,
    pub server: PathBuf,
    pub server_is_directory: bool,
    pub server_files: BTreeSet<PathBuf>,
    pub server_layout: BTreeMap<OsString, PathBuf>,
    pub client_files: BTreeSet<PathBuf>,
    pub selected_clients: BTreeSet<PathBuf>,
    client_layout: Option<(PathBuf, BTreeMap<OsString, PathBuf>)>,
    source_paths: BTreeMap<PathBuf, PathBuf>,
}

impl ConfigSet {
    pub fn load(input: &Input) -> Result<Self> {
        // Keep aliases as well as resolved paths so a later symlink retarget or
        // directory-entry change cannot silently change the supplied config set.
        let mut source_paths = BTreeMap::new();
        let mut resolve = |path: &Path| -> Result<PathBuf> {
            let source = std::path::absolute(path)?;
            let resolved = canonical(&source)?;
            source_paths.insert(source, resolved.clone());
            Ok(resolved)
        };
        let server = resolve(&input.server)?;
        let server_is_directory = server.is_dir();
        let server_layout = if server_is_directory {
            json_layout(&server)?
        } else {
            BTreeMap::from([(
                server
                    .file_name()
                    .context("server has no filename")?
                    .to_owned(),
                server.clone(),
            )])
        };
        let server_files: BTreeSet<_> = server_layout.values().cloned().collect();
        ensure!(
            server_files.len() == server_layout.len(),
            "server directory contains aliases of the same config file"
        );
        ensure!(
            !server_files.is_empty(),
            "server directory contains no JSON files"
        );
        let selected_clients: BTreeSet<_> = input
            .client
            .iter()
            .map(|p| resolve(p))
            .collect::<Result<_>>()?;
        let client_layout = input
            .clients
            .as_ref()
            .map(|path| -> Result<_> {
                let directory = resolve(path)?;
                let layout = json_layout(&directory)?;
                Ok((directory, layout))
            })
            .transpose()?;
        let mut client_files = client_layout
            .as_ref()
            .map(|(_, layout)| layout.values().cloned().collect())
            .unwrap_or_else(BTreeSet::new);
        client_files.extend(selected_clients.iter().cloned());
        ensure!(!client_files.is_empty(), "no client JSON files supplied");
        ensure!(
            server_files.is_disjoint(&client_files),
            "a file cannot be both a server fragment and an independent client config"
        );
        let mut documents = BTreeMap::new();
        for path in server_files.iter().chain(&client_files) {
            ensure!(
                path.is_file(),
                "not a regular config file: {}",
                path.display()
            );
            let (original, state) = read_snapshot(path)?;
            let value: Value = serde_json::from_slice(&original)
                .with_context(|| format!("parsing JSON in {}", path.display()))?;
            ensure!(
                value.is_object(),
                "config must be an object: {}",
                path.display()
            );
            let permissions = state.permissions.clone();
            documents.insert(
                path.clone(),
                Document {
                    value,
                    original,
                    permissions,
                    state,
                },
            );
        }
        Ok(Self {
            documents,
            server,
            server_is_directory,
            server_files,
            server_layout,
            client_files,
            selected_clients,
            client_layout,
            source_paths,
        })
    }

    pub(crate) fn lock_directories(&self) -> Result<BTreeSet<PathBuf>> {
        let mut directories: BTreeSet<_> = self
            .documents
            .keys()
            .filter_map(|path| path.parent().map(Path::to_owned))
            .collect();
        if self.server_is_directory {
            directories.insert(self.server.clone());
        }
        if let Some((directory, _)) = &self.client_layout {
            directories.insert(directory.clone());
        }
        // Also protect explicit file aliases, not just their resolved targets.
        // Directory inputs lock the directory itself, not its (possibly unwritable) parent.
        for (source, resolved) in &self.source_paths {
            if self.documents.contains_key(resolved)
                && let Some(parent) = source.parent()
            {
                directories.insert(canonical(parent)?);
            }
        }
        Ok(directories)
    }

    pub(crate) fn verify_unchanged(&self) -> Result<()> {
        for (source, expected) in &self.source_paths {
            ensure!(
                canonical(source)? == *expected,
                "config source path changed while planning: {}; refusing to commit",
                source.display()
            );
        }
        if self.server_is_directory {
            ensure!(
                json_layout(&self.server)? == self.server_layout,
                "server directory changed while planning; refusing to commit"
            );
        }
        if let Some((directory, layout)) = &self.client_layout {
            ensure!(
                json_layout(directory)? == *layout,
                "client directory changed while planning; refusing to commit"
            );
        }
        for path in self.documents.keys() {
            self.verify_file(path)?;
        }
        Ok(())
    }

    pub(crate) fn verify_file(&self, path: &Path) -> Result<()> {
        let doc = self.documents.get(path).context("config file not loaded")?;
        let (bytes, state) = read_snapshot(path)?;
        ensure!(
            bytes == doc.original && state == doc.state,
            "config changed while planning (contents or metadata): {}; refusing to overwrite",
            path.display()
        );
        Ok(())
    }

    pub fn value(&self, file: &Path, pointer: &str) -> Result<&Value> {
        self.documents
            .get(file)
            .and_then(|doc| doc.value.pointer(pointer))
            .with_context(|| format!("missing JSON location {}#{pointer}", file.display()))
    }
}

pub fn canonical(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).with_context(|| format!("resolving {}", path.display()))
}

pub fn json_layout(directory: &Path) -> Result<BTreeMap<OsString, PathBuf>> {
    let mut paths = BTreeMap::new();
    for entry in
        fs::read_dir(directory).with_context(|| format!("listing {}", directory.display()))?
    {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "json") && path.is_file() {
            // Canonical paths make relative/absolute selectors and symlink aliases agree.
            paths.insert(entry_file_name(&path)?, canonical(&path)?);
        }
    }
    Ok(paths)
}

fn entry_file_name(path: &Path) -> Result<OsString> {
    Ok(path
        .file_name()
        .context("config has no filename")?
        .to_owned())
}
