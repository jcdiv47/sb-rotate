//! Assemble one standalone client config per target from shared and
//! per-device fragments, as listed in a build manifest.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::Value;
use tempfile::Builder;

use crate::{config::canonical, recovery::ensure_no_pending, singbox::SingBox};

/// Relative paths resolve from the manifest's directory.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default = "default_output_dir")]
    output_dir: PathBuf,
    publish_dir: Option<PathBuf>,
    targets: BTreeMap<String, Target>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    /// Merge order: arrays (rules) append in this order; a scalar may be set once.
    fragments: Vec<PathBuf>,
    /// Published file name; random names keep subscription URLs unguessable.
    publish_as: Option<String>,
}

fn default_output_dir() -> PathBuf {
    PathBuf::from("out")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Publish {
    /// Replace published files atomically where this user may write them.
    Direct,
    /// Leave every published file to [`sudo_install`] (root:root 0644).
    Sudo,
}

#[derive(Debug)]
pub struct Report {
    pub lines: Vec<String>,
    /// (output, published path) pairs still to install with root privileges.
    pub install: Vec<(PathBuf, PathBuf)>,
}

const SUDO_INSTALL: [&str; 8] = ["sudo", "install", "-o", "root", "-g", "root", "-m", "644"];

pub fn sudo_install_command(output: &Path, target: &Path) -> String {
    format!(
        "{} {} {}",
        SUDO_INSTALL.join(" "),
        output.display(),
        target.display()
    )
}

/// Install outputs as root-owned subscription files; sudo may prompt.
pub fn sudo_install(files: &[(PathBuf, PathBuf)]) -> Result<()> {
    for (output, target) in files {
        let status = Command::new(SUDO_INSTALL[0])
            .args(&SUDO_INSTALL[1..])
            .arg(output)
            .arg(target)
            .status()
            .context("executing sudo")?;
        ensure!(
            status.success(),
            "{} failed ({status})",
            sudo_install_command(output, target)
        );
    }
    Ok(())
}

pub fn build(
    manifest: &Path,
    selected: &[String],
    publish: Option<Publish>,
    singbox: &impl SingBox,
) -> Result<Report> {
    let manifest = canonical(manifest)?;
    let root = manifest.parent().context("manifest has no parent")?;
    let parsed: Manifest = serde_json::from_slice(
        &fs::read(&manifest).with_context(|| format!("reading {}", manifest.display()))?,
    )
    .with_context(|| format!("parsing build manifest {}", manifest.display()))?;
    for name in parsed.targets.keys() {
        ensure!(file_name_ok(name), "invalid target name: {name:?}");
    }
    for name in selected {
        ensure!(
            parsed.targets.contains_key(name),
            "unknown target {name:?}; known targets: {}",
            parsed
                .targets
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let targets: Vec<_> = parsed
        .targets
        .iter()
        .filter(|(name, _)| selected.is_empty() || selected.contains(name))
        .collect();
    ensure!(!targets.is_empty(), "build manifest lists no targets");
    let publish_dir = if publish.is_some() {
        let directory = parsed
            .publish_dir
            .as_ref()
            .context("--publish requires publish_dir in the build manifest")?;
        for (name, target) in &targets {
            let published = target
                .publish_as
                .as_deref()
                .with_context(|| format!("target {name:?} has no publish_as name"))?;
            ensure!(
                file_name_ok(published),
                "invalid publish_as name for {name:?}: {published:?}"
            );
        }
        let directory = root.join(directory);
        ensure!(
            directory.is_dir(),
            "publish_dir is not a directory: {}",
            directory.display()
        );
        Some(directory)
    } else {
        None
    };

    let mut fragments = BTreeMap::new();
    for (name, target) in &targets {
        ensure!(
            !target.fragments.is_empty(),
            "target {name:?} has no fragments"
        );
        let paths = target
            .fragments
            .iter()
            .map(|fragment| canonical(&root.join(fragment)))
            .collect::<Result<Vec<_>>>()?;
        fragments.insert(name.as_str(), paths);
    }
    // Never publish a half-applied rotation.
    let directories: BTreeSet<_> = fragments
        .values()
        .flatten()
        .filter_map(|path| path.parent().map(Path::to_owned))
        .collect();
    ensure_no_pending(&directories)?;

    // Merge and validate every target before writing any output.
    let mut merged = Vec::new();
    for (name, paths) in &fragments {
        check_conflicts(paths)?;
        let staging = private_directory()?;
        // sing-box orders merge inputs by path, so index prefixes fix the order.
        let mut inputs = Vec::new();
        for (index, path) in paths.iter().enumerate() {
            let file_name = path.file_name().context("fragment has no file name")?;
            let input = staging
                .path()
                .join(format!("{index:02}-{}", file_name.to_string_lossy()));
            fs::copy(path, &input).with_context(|| format!("staging {}", path.display()))?;
            inputs.push(input);
        }
        let output = staging.path().join(format!("{name}.json"));
        singbox.merge(&output, &inputs)?;
        singbox
            .check_file(&output)
            .with_context(|| format!("validating merged target {name:?}"))?;
        merged.push((*name, fs::read(&output)?));
    }

    let output_dir = root.join(&parsed.output_dir);
    create_private_dir(&output_dir)?;
    let mut report = Report {
        lines: Vec::new(),
        install: Vec::new(),
    };
    for (name, bytes) in &merged {
        let output = output_dir.join(format!("{name}.json"));
        replace(&output, bytes, false).with_context(|| format!("writing {}", output.display()))?;
        report.lines.push(output.display().to_string());
        if let Some(directory) = &publish_dir {
            let published = parsed.targets[*name]
                .publish_as
                .as_deref()
                .unwrap_or_default();
            let target = directory.join(published);
            if publish == Some(Publish::Direct) && publish_file(&target, bytes)? {
                report.lines.push(format!("  -> {}", target.display()));
            } else {
                report.install.push((output, target));
            }
        }
    }
    Ok(report)
}

fn file_name_ok(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != ".."
}

/// sing-box keeps the first value of a scalar set in several fragments, which
/// silently depends on fragment order; require each scalar to have one owner.
fn check_conflicts(fragments: &[PathBuf]) -> Result<()> {
    fn scalars(value: &Value, pointer: String, out: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    let key = key.replace('~', "~0").replace('/', "~1");
                    scalars(child, format!("{pointer}/{key}"), out);
                }
            }
            Value::Array(_) => {}
            _ => out.push(pointer),
        }
    }
    let mut owners: BTreeMap<String, &Path> = BTreeMap::new();
    for fragment in fragments {
        let value: Value = serde_json::from_slice(&fs::read(fragment)?)
            .with_context(|| format!("parsing JSON in {}", fragment.display()))?;
        ensure!(
            value.is_object(),
            "fragment must be an object: {}",
            fragment.display()
        );
        let mut paths = Vec::new();
        scalars(&value, String::new(), &mut paths);
        for path in paths {
            if let Some(owner) = owners.insert(path.clone(), fragment) {
                bail!(
                    "{path} is set in both {} and {}",
                    owner.display(),
                    fragment.display()
                );
            }
        }
    }
    Ok(())
}

/// Atomically replace `path`; outputs are private, published copies world-readable.
fn replace(path: &Path, bytes: &[u8], shared: bool) -> io::Result<()> {
    let directory = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let mut temp = Builder::new()
        .prefix(".sb-rotate-")
        .suffix(".tmp")
        .tempfile_in(directory)?;
    temp.write_all(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if shared { 0o644 } else { 0o600 };
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = shared;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Atomic replacement so subscribers never fetch a partial file. Returns false
/// when this user may not write the publish directory. Files in an unwritable
/// directory are never overwritten in place: their ownership may be deliberate.
fn publish_file(target: &Path, bytes: &[u8]) -> Result<bool> {
    match replace(target, bytes, true) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Ok(false),
        Err(error) => Err(error).with_context(|| format!("publishing {}", target.display())),
    }
}

fn create_private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
    };
    #[cfg(not(unix))]
    let builder = fs::DirBuilder::new();
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(error) => Err(error).with_context(|| format!("creating {}", path.display())),
    }
}

fn private_directory() -> Result<tempfile::TempDir> {
    let mut builder = Builder::new();
    builder.prefix("sb-rotate-build-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    Ok(builder.tempdir()?)
}
