use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::cli::Input;

pub struct Document {
    pub value: Value,
    pub original: Vec<u8>,
    pub permissions: fs::Permissions,
}

pub struct ConfigSet {
    pub documents: BTreeMap<PathBuf, Document>,
    pub server: PathBuf,
    pub server_is_directory: bool,
    pub server_files: BTreeSet<PathBuf>,
    pub server_layout: BTreeMap<OsString, PathBuf>,
    pub client_files: BTreeSet<PathBuf>,
    pub selected_clients: BTreeSet<PathBuf>,
}

impl ConfigSet {
    pub fn load(input: &Input) -> Result<Self> {
        let server = canonical(&input.server)?;
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
            .map(|p| canonical(p))
            .collect::<Result<_>>()?;
        let mut client_files = match &input.clients {
            Some(directory) => json_layout(directory)?.into_values().collect(),
            None => BTreeSet::new(),
        };
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
            let original = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let value: Value = serde_json::from_slice(&original)
                .with_context(|| format!("parsing JSON in {}", path.display()))?;
            ensure!(
                value.is_object(),
                "config must be an object: {}",
                path.display()
            );
            let permissions = fs::metadata(path)?.permissions();
            documents.insert(
                path.clone(),
                Document {
                    value,
                    original,
                    permissions,
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
        })
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
