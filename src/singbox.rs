use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail, ensure};
use semver::Version;

pub trait SingBox {
    fn version(&self) -> Result<Version>;
    fn generate_uuid(&self) -> Result<String>;
    fn generate_random_base64(&self, bytes: usize) -> Result<String>;
    fn check_file(&self, path: &Path) -> Result<()>;
    fn check_directory(&self, path: &Path) -> Result<()>;
}

pub struct Executable {
    path: PathBuf,
}

impl Executable {
    pub fn resolve(explicit: Option<&Path>) -> Self {
        Self {
            path: explicit
                .map(Path::to_path_buf)
                .or_else(|| std::env::var_os("SING_BOX").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("sing-box")),
        }
    }

    fn run(&self, args: &[&OsStr], show_failure_output: bool) -> Result<String> {
        let output = Command::new(&self.path)
            .args(args)
            .output()
            .with_context(|| format!("executing {}", self.path.display()))?;
        if !output.status.success() {
            if show_failure_output {
                bail!(
                    "{} failed ({}):\n{}{}",
                    self.path.display(),
                    output.status,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            // Generator output can contain private key material or passwords.
            bail!(
                "{} generator failed ({})",
                self.path.display(),
                output.status
            );
        }
        String::from_utf8(output.stdout).context("sing-box stdout is not UTF-8")
    }

    fn generate(&self, args: &[&str]) -> Result<String> {
        let args: Vec<_> = args.iter().map(OsStr::new).collect();
        let value = self.run(&args, false)?.trim().to_owned();
        ensure!(
            !value.is_empty() && !value.chars().any(char::is_whitespace),
            "sing-box generator returned an empty or multi-token value"
        );
        Ok(value)
    }
}

impl SingBox for Executable {
    fn version(&self) -> Result<Version> {
        let output = self.run(&[OsStr::new("version")], true)?;
        parse_version(&output)
    }

    fn generate_uuid(&self) -> Result<String> {
        let value = self.generate(&["generate", "uuid"])?;
        ensure!(
            uuid::Uuid::parse_str(&value).is_ok(),
            "sing-box returned an invalid UUID"
        );
        Ok(value)
    }

    fn generate_random_base64(&self, bytes: usize) -> Result<String> {
        self.generate(&["generate", "rand", &bytes.to_string(), "--base64"])
    }

    fn check_file(&self, path: &Path) -> Result<()> {
        self.run(
            &[OsStr::new("check"), OsStr::new("-c"), path.as_os_str()],
            true,
        )
        .with_context(|| format!("validating config {}", path.display()))?;
        Ok(())
    }

    fn check_directory(&self, path: &Path) -> Result<()> {
        self.run(
            &[OsStr::new("check"), OsStr::new("-C"), path.as_os_str()],
            true,
        )
        .with_context(|| format!("validating server config directory {}", path.display()))?;
        Ok(())
    }
}

pub fn require_supported(singbox: &impl SingBox) -> Result<Version> {
    let version = singbox.version()?;
    ensure!(
        version >= Version::new(1, 14, 0),
        "sing-box {version} is unsupported; version >= 1.14.0 is required"
    );
    Ok(version)
}

fn parse_version(output: &str) -> Result<Version> {
    let version = output
        .lines()
        .find_map(|line| line.strip_prefix("sing-box version "))
        .and_then(|line| line.split_whitespace().next())
        .context("could not find 'sing-box version <semver>' in version output")?;
    Version::parse(version).context("invalid sing-box semantic version")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions_and_respects_release_boundary() {
        assert_eq!(
            parse_version("sing-box version 1.14.0\nEnvironment: test").unwrap(),
            Version::new(1, 14, 0)
        );
        assert!(parse_version("sing-box version 1.14.0-beta.9").unwrap() < Version::new(1, 14, 0));
        assert!(parse_version("sing-box version 1.15.0-alpha.1").unwrap() > Version::new(1, 14, 0));
        assert!(parse_version("1.14.0").is_err());
        assert!(parse_version("sing-box version not-a-version").is_err());
    }
}
