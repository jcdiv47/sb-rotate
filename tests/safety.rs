use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use sb_rotate::{
    apply, binding,
    cli::{Input, PropertyKind},
    config::ConfigSet,
    plan::RotationPlan,
    protocol::properties,
    singbox::{RealityKeyPair, SingBox},
};
use semver::Version;
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Default)]
struct Validator {
    calls: Cell<usize>,
    hook: RefCell<Option<Box<dyn FnOnce()>>>,
}
impl SingBox for Validator {
    fn version(&self) -> Result<Version> {
        Ok(Version::new(1, 14, 0))
    }
    fn generate_uuid(&self) -> Result<String> {
        bail!("unexpected generator")
    }
    fn generate_random_base64(&self, _: usize) -> Result<String> {
        bail!("unexpected generator")
    }
    fn generate_random_hex(&self, _: usize) -> Result<String> {
        bail!("unexpected generator")
    }
    fn generate_reality_keypair(&self) -> Result<RealityKeyPair> {
        bail!("unexpected generator")
    }
    fn check_file(&self, _: &Path) -> Result<()> {
        self.calls.set(self.calls.get() + 1);
        if let Some(hook) = self.hook.borrow_mut().take() {
            hook();
        }
        Ok(())
    }
    fn check_directory(&self, path: &Path) -> Result<()> {
        self.check_file(path)
    }
}

struct Fixture {
    root: TempDir,
    input: Input,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        fs::create_dir(root.path().join("clients")).unwrap();
        let input = Input {
            server: root.path().join("server.json"),
            clients: Some(root.path().join("clients")),
            ..Input::default()
        };
        let fixture = Self { root, input };
        fixture.write(
            "server.json",
            json!({"inbounds": [{"type": "vless", "tag": "home", "users": [{"uuid": "shared"}]}]}),
        );
        fixture.write("clients/phone.json", json!({"outbounds": [{"type": "vless", "tag": "home", "uuid": "shared", "server": "old.example"}]}));
        fixture
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }
    fn write(&self, name: &str, value: Value) {
        fs::write(self.path(name), serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn plan(&self, value: &str) -> (ConfigSet, RotationPlan) {
        let configs = ConfigSet::load(&self.input).unwrap();
        let inventory = binding::discover(&configs, &self.input).unwrap();
        let plan = properties::set(
            &configs,
            &inventory,
            &self.input,
            PropertyKind::Server,
            value,
        )
        .unwrap();
        (configs, plan)
    }
}
fn originals_unchanged(configs: &ConfigSet) {
    for (path, doc) in &configs.documents {
        assert_eq!(fs::read(path).unwrap(), doc.original);
    }
}

#[test]
fn competing_lock_fails_before_validation_then_releases_partial_lock_set() {
    let fixture = Fixture::new();
    let (configs, plan) = fixture.plan("new.example");
    let occupied = fs::File::create(fixture.path("clients/.sb-rotate.lock")).unwrap();
    occupied.try_lock().unwrap();
    let validator = Validator::default();
    let error = apply::apply(&configs, &plan, &validator).unwrap_err();
    assert!(error.to_string().contains("another sb-rotate writer"));
    assert_eq!(validator.calls.get(), 0);
    originals_unchanged(&configs);
    // The first directory's lock must be released when the second acquisition fails.
    let released = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.path(".sb-rotate.lock"))
        .unwrap();
    released.try_lock().unwrap();
    drop(released);
    drop(occupied);
    assert_eq!(apply::apply(&configs, &plan, &validator).unwrap(), 1);
    // Lock files persist; dropping guards, not unlinking files, releases ownership.
    for name in [".sb-rotate.lock", "clients/.sb-rotate.lock"] {
        assert!(fixture.path(name).is_file());
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(fixture.path(name))
            .unwrap();
        lock.try_lock().unwrap();
        drop(lock); // Windows exclusive locks also exclude reads via other handles.
        assert!(fs::read(fixture.path(name)).unwrap().is_empty());
    }
}

#[cfg(windows)]
#[test]
fn readonly_windows_destinations_fail_before_staging() {
    let fixture = Fixture::new();
    let path = fixture.path("clients/phone.json");
    let original = fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    let (configs, plan) = fixture.plan("new.example");
    let validator = Validator::default();
    let result = apply::apply(&configs, &plan, &validator);
    // Restore the attribute so tempdir cleanup works on Windows too.
    fs::set_permissions(&path, original).unwrap();
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("read-only Windows config")
    );
    assert_eq!(validator.calls.get(), 0);
    originals_unchanged(&configs);
    assert_eq!(fs::read_dir(fixture.path("clients")).unwrap().count(), 2); // config + lock, no temps
}

#[test]
fn previews_and_noop_updates_do_not_create_writer_locks() {
    let fixture = Fixture::new();
    let (configs, plan) = fixture.plan("new.example");
    plan.materialize(&configs).unwrap();
    let (configs, plan) = fixture.plan("old.example");
    assert_eq!(
        apply::apply(&configs, &plan, &Validator::default()).unwrap(),
        0
    );
    assert!(!fixture.path(".sb-rotate.lock").exists());
    assert!(!fixture.path("clients/.sb-rotate.lock").exists());
}

#[test]
fn newly_discovered_client_during_validation_aborts_instead_of_leaving_it_stale() {
    let fixture = Fixture::new();
    let (configs, plan) = fixture.plan("new.example");
    let phone = fixture.path("clients/phone.json");
    let extra = fixture.path("clients/extra.json");
    let validator = Validator::default();
    validator.hook.replace(Some(Box::new(move || {
        fs::copy(phone, extra).unwrap();
    })));
    let error = apply::apply(&configs, &plan, &validator).unwrap_err();
    assert!(error.to_string().contains("client directory changed"));
    originals_unchanged(&configs);
    assert!(fixture.path("clients/extra.json").exists());
}

#[test]
fn removed_client_after_planning_is_not_recreated() {
    let fixture = Fixture::new();
    let (configs, plan) = fixture.plan("new.example");
    fs::remove_file(fixture.path("clients/phone.json")).unwrap();
    let validator = Validator::default();
    assert!(
        apply::apply(&configs, &plan, &validator)
            .unwrap_err()
            .to_string()
            .contains("client directory changed")
    );
    assert_eq!(validator.calls.get(), 0);
    assert!(!fixture.path("clients/phone.json").exists());
}

#[cfg(unix)]
#[test]
fn retargeted_server_symlink_is_detected_even_when_target_bytes_match() {
    use std::os::unix::fs::symlink;
    let mut fixture = Fixture::new();
    fs::copy(fixture.path("server.json"), fixture.path("alternate.json")).unwrap();
    let alias = fixture.path("server-link.json");
    symlink(fixture.path("server.json"), &alias).unwrap();
    fixture.input.server = alias.clone();
    let (configs, plan) = fixture.plan("new.example");
    let alternate = fixture.path("alternate.json");
    let validator = Validator::default();
    validator.hook.replace(Some(Box::new(move || {
        fs::remove_file(&alias).unwrap();
        symlink(alternate, alias).unwrap();
    })));
    assert!(
        apply::apply(&configs, &plan, &validator)
            .unwrap_err()
            .to_string()
            .contains("source path changed")
    );
    originals_unchanged(&configs);
}

#[cfg(unix)]
#[test]
fn symlinked_client_directory_and_file_aliases_use_canonical_writer_locks() {
    use std::os::unix::fs::symlink;
    let mut fixture = Fixture::new();
    let actual = fixture.path("clients");
    let alias = fixture.path("client-alias");
    symlink(&actual, &alias).unwrap();
    fixture.input.clients = Some(alias);
    let (configs, plan) = fixture.plan("new.example");
    let held = fs::File::create(actual.join(".sb-rotate.lock")).unwrap();
    held.try_lock().unwrap();
    assert!(
        apply::apply(&configs, &plan, &Validator::default())
            .unwrap_err()
            .to_string()
            .contains("another sb-rotate writer")
    );
    drop(held);
    // A server-file alias with a non-normalized parent must not double-lock a directory.
    symlink(
        fixture.path("server.json"),
        fixture.path("server-link.json"),
    )
    .unwrap();
    fixture.input.server = fixture.path("clients/../server-link.json");
    let (configs, plan) = fixture.plan("new.example");
    assert_eq!(
        apply::apply(&configs, &plan, &Validator::default()).unwrap(),
        1
    );
    assert!(
        fs::symlink_metadata(fixture.path("server-link.json"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_at_the_lock_path_is_never_followed_or_truncated() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let (configs, plan) = fixture.plan("new.example");
    symlink(fixture.path("server.json"), fixture.path(".sb-rotate.lock")).unwrap();
    assert!(apply::apply(&configs, &plan, &Validator::default()).is_err());
    originals_unchanged(&configs);
}

#[cfg(unix)]
#[test]
fn hard_linked_configs_are_readable_but_never_silently_split_by_rotation() {
    let fixture = Fixture::new();
    fs::hard_link(
        fixture.path("clients/phone.json"),
        fixture.path("other-copy.json"),
    )
    .unwrap();
    let (configs, plan) = fixture.plan("new.example");
    let validator = Validator::default();
    assert!(
        apply::apply(&configs, &plan, &validator)
            .unwrap_err()
            .to_string()
            .contains("hard-linked")
    );
    assert_eq!(validator.calls.get(), 0);
    originals_unchanged(&configs);
    fs::remove_file(fixture.path("other-copy.json")).unwrap();
    let (configs, plan) = fixture.plan("new.example");
    assert_eq!(apply::apply(&configs, &plan, &validator).unwrap(), 1);
}

#[cfg(unix)]
#[test]
fn permission_changes_before_or_during_validation_are_not_overwritten() {
    use std::os::unix::fs::PermissionsExt;
    for during_validation in [false, true] {
        let fixture = Fixture::new();
        let phone = fixture.path("clients/phone.json");
        fs::set_permissions(&phone, fs::Permissions::from_mode(0o644)).unwrap();
        let (configs, plan) = fixture.plan("new.example");
        let validator = Validator::default();
        if during_validation {
            let path = phone.clone();
            validator.hook.replace(Some(Box::new(move || {
                fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap()
            })));
        } else {
            fs::set_permissions(&phone, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(
            apply::apply(&configs, &plan, &validator)
                .unwrap_err()
                .to_string()
                .contains("metadata")
        );
        originals_unchanged(&configs);
        assert_eq!(
            fs::metadata(phone).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
#[test]
fn replacing_a_config_with_identical_bytes_is_still_detected_by_inode() {
    use std::io::Write;
    let fixture = Fixture::new();
    let (configs, plan) = fixture.plan("new.example");
    let path = fixture.path("clients/phone.json");
    let metadata = fs::metadata(&path).unwrap();
    let mut replacement = tempfile::NamedTempFile::new_in(path.parent().unwrap()).unwrap();
    replacement.write_all(&fs::read(&path).unwrap()).unwrap();
    replacement
        .as_file()
        .set_permissions(metadata.permissions())
        .unwrap();
    replacement
        .as_file()
        .set_times(fs::FileTimes::new().set_modified(metadata.modified().unwrap()))
        .unwrap();
    fs::rename(replacement.path(), &path).unwrap();
    let validator = Validator::default();
    assert!(
        apply::apply(&configs, &plan, &validator)
            .unwrap_err()
            .to_string()
            .contains("metadata")
    );
    assert_eq!(validator.calls.get(), 0);
    originals_unchanged(&configs);
}

#[cfg(unix)]
#[test]
fn lock_file_replacement_during_validation_aborts_the_commit() {
    let fixture = Fixture::new();
    let (configs, plan) = fixture.plan("new.example");
    let lock = fixture.path(".sb-rotate.lock");
    let renamed = fixture.path(".old-lock");
    let validator = Validator::default();
    validator.hook.replace(Some(Box::new(move || {
        fs::rename(&lock, renamed).unwrap();
        fs::write(lock, b"").unwrap();
    })));
    assert!(
        apply::apply(&configs, &plan, &validator)
            .unwrap_err()
            .to_string()
            .contains("writer lock changed")
    );
    originals_unchanged(&configs);
}

#[cfg(unix)]
#[test]
fn successful_replacements_preserve_unix_owner_group_and_permissions() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let fixture = Fixture::new();
    let path = fixture.path("clients/phone.json");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let before = fs::metadata(&path).unwrap();
    let (configs, plan) = fixture.plan("new.example");
    apply::apply(&configs, &plan, &Validator::default()).unwrap();
    let after = fs::metadata(&path).unwrap();
    assert_eq!(
        (before.uid(), before.gid(), before.mode()),
        (after.uid(), after.gid(), after.mode())
    );
    for name in [".sb-rotate.lock", "clients/.sb-rotate.lock"] {
        assert_eq!(
            fs::metadata(fixture.path(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}
