use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use sb_rotate::{
    build::{self, Publish},
    singbox::{RealityKeyPair, SingBox},
};
use semver::Version;
use serde_json::{Value, json};
use tempfile::TempDir;

/// Merges like sing-box for disjoint fragments: objects recurse, arrays append.
#[derive(Default)]
struct Merger {
    merges: Cell<usize>,
    inputs: RefCell<Vec<Vec<String>>>,
}
fn merge_into(target: &mut Value, source: Value) {
    match (target, source) {
        (Value::Object(target), Value::Object(source)) => {
            for (key, value) in source {
                match target.get_mut(&key) {
                    Some(existing) => merge_into(existing, value),
                    None => {
                        target.insert(key, value);
                    }
                }
            }
        }
        (Value::Array(target), Value::Array(source)) => target.extend(source),
        _ => {}
    }
}
impl SingBox for Merger {
    fn version(&self) -> Result<Version> {
        Ok(Version::new(1, 14, 0))
    }
    fn generate_uuid(&self) -> Result<String> {
        unreachable!()
    }
    fn generate_random_base64(&self, _: usize) -> Result<String> {
        unreachable!()
    }
    fn generate_random_hex(&self, _: usize) -> Result<String> {
        unreachable!()
    }
    fn generate_reality_keypair(&self) -> Result<RealityKeyPair> {
        unreachable!()
    }
    fn check_file(&self, path: &Path) -> Result<()> {
        if fs::read_to_string(path)?.contains("invalid") {
            bail!("injected validation failure");
        }
        Ok(())
    }
    fn check_directory(&self, _: &Path) -> Result<()> {
        unreachable!()
    }
    fn merge(&self, output: &Path, inputs: &[PathBuf]) -> Result<()> {
        self.merges.set(self.merges.get() + 1);
        let mut sorted = inputs.to_vec();
        sorted.sort(); // sing-box orders inputs by path
        self.inputs.borrow_mut().push(
            sorted
                .iter()
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect(),
        );
        let mut merged = json!({});
        for input in sorted {
            merge_into(&mut merged, serde_json::from_slice(&fs::read(input)?)?);
        }
        fs::write(output, serde_json::to_vec(&merged)?)?;
        Ok(())
    }
}

struct Fixture {
    root: TempDir,
}
impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            root: tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap(),
        };
        for directory in ["shared", "devices", "pub"] {
            fs::create_dir(fixture.path(directory)).unwrap();
        }
        // "z-" sorts after the device file, so only the manifest fixes the order.
        fixture.put(
            "shared/z-base.json",
            json!({"log": {"level": "info"}, "route": {"rules": [{"outbound": "shared"}]}}),
        );
        for device in ["phone", "laptop"] {
            fixture.put(
                &format!("devices/{device}.json"),
                json!({"route": {"rules": [{"outbound": device}], "final": device}}),
            );
        }
        fixture.manifest(json!({
            "publish_dir": "pub",
            "targets": {
                "phone": {"fragments": ["devices/phone.json", "shared/z-base.json"], "publish_as": "phone-token.json"},
                "laptop": {"fragments": ["devices/laptop.json", "shared/z-base.json"], "publish_as": "laptop-token.json"}
            }
        }));
        fixture
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }
    fn put(&self, name: &str, value: Value) {
        fs::write(self.path(name), serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn get(&self, name: &str) -> Value {
        serde_json::from_slice(&fs::read(self.path(name)).unwrap()).unwrap()
    }
    fn manifest(&self, value: Value) {
        self.put("build.json", value);
    }
    fn build(
        &self,
        targets: &[&str],
        publish: Option<Publish>,
        sb: &Merger,
    ) -> Result<build::Report> {
        let targets: Vec<_> = targets.iter().map(|name| name.to_string()).collect();
        build::build(&self.path("build.json"), &targets, publish, sb)
    }
}

#[test]
fn merges_fragments_in_manifest_order_into_private_outputs() {
    let fixture = Fixture::new();
    let sb = Merger::default();
    let report = fixture.build(&[], None, &sb).unwrap();
    assert!(report.install.is_empty());
    assert_eq!(report.lines.len(), 2);
    assert_eq!(
        sb.inputs.borrow()[1],
        ["00-phone.json", "01-z-base.json"],
        "staged names must make manifest order the merge order"
    );
    let phone = fixture.get("out/phone.json");
    assert_eq!(
        phone["route"]["rules"],
        json!([{"outbound": "phone"}, {"outbound": "shared"}])
    );
    assert_eq!(phone["route"]["final"], "phone");
    assert!(!fixture.path("pub/phone-token.json").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |name: &str| {
            fs::metadata(fixture.path(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("out"), 0o700);
        assert_eq!(mode("out/phone.json"), 0o600);
    }
    let report = fixture.build(&["laptop"], None, &sb).unwrap();
    assert_eq!(report.lines.len(), 1);
}

#[test]
fn a_failing_target_writes_no_outputs() {
    let fixture = Fixture::new();
    fixture.put(
        "devices/laptop.json",
        json!({"route": {"final": "invalid"}}),
    );
    let error = fixture.build(&[], None, &Merger::default()).unwrap_err();
    assert!(format!("{error:#}").contains("\"laptop\""));
    assert!(!fixture.path("out").exists());
}

#[test]
fn scalars_set_by_several_fragments_are_rejected_before_merging() {
    let fixture = Fixture::new();
    fixture.put(
        "shared/z-base.json",
        json!({"route": {"final": "shared", "rules": []}}),
    );
    let sb = Merger::default();
    let error = fixture.build(&["phone"], None, &sb).unwrap_err();
    assert!(error.to_string().contains("/route/final is set in both"));
    assert_eq!(sb.merges.get(), 0);
}

#[test]
fn invalid_manifests_and_selections_fail_before_merging() {
    let fixture = Fixture::new();
    let sb = Merger::default();
    assert!(fixture.build(&["tablet"], None, &sb).is_err());
    for manifest in [
        json!({"targets": {}}),
        json!({"targets": {"../x": {"fragments": ["devices/phone.json"]}}}),
        json!({"targets": {"phone": {"fragments": []}}}),
        json!({"targets": {"phone": {"fragments": ["devices/missing.json"]}}}),
        json!({"targets": {"phone": {"fragments": ["devices/phone.json"]}}, "unknown": 1}),
    ] {
        fixture.manifest(manifest.clone());
        assert!(fixture.build(&[], None, &sb).is_err(), "{manifest}");
    }
    // Publishing needs a directory and an explicit (unguessable) name per target.
    for manifest in [
        json!({"targets": {"phone": {"fragments": ["devices/phone.json"], "publish_as": "p.json"}}}),
        json!({"publish_dir": "pub", "targets": {"phone": {"fragments": ["devices/phone.json"]}}}),
        json!({"publish_dir": "pub", "targets": {"phone": {"fragments": ["devices/phone.json"], "publish_as": "../p.json"}}}),
    ] {
        fixture.manifest(manifest.clone());
        assert!(fixture.build(&[], None, &sb).is_ok(), "{manifest}");
        assert!(
            fixture.build(&[], Some(Publish::Direct), &sb).is_err(),
            "{manifest}"
        );
    }
    fixture.manifest(json!({"publish_dir": "missing", "targets": {"phone": {"fragments": ["devices/phone.json"], "publish_as": "p.json"}}}));
    let error = fixture.build(&[], Some(Publish::Direct), &sb).unwrap_err();
    assert!(error.to_string().contains("publish_dir is not a directory"));
    assert_eq!(sb.merges.get(), 3);
}

#[test]
fn publishing_copies_outputs_to_their_published_names() {
    let fixture = Fixture::new();
    let report = fixture
        .build(&[], Some(Publish::Direct), &Merger::default())
        .unwrap();
    assert!(report.install.is_empty());
    assert_eq!(
        fs::read(fixture.path("pub/phone-token.json")).unwrap(),
        fs::read(fixture.path("out/phone.json")).unwrap()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(fixture.path("pub/phone-token.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644);
    }
}

#[cfg(unix)]
#[test]
fn unwritable_publish_directories_are_left_for_privileged_install() {
    use std::os::unix::fs::PermissionsExt;
    if rustix::process::geteuid().is_root() {
        return; // root ignores the permissions under test
    }
    let fixture = Fixture::new();
    // Even a file this user owns is not overwritten in place: ownership may be deliberate.
    fixture.put("pub/phone-token.json", json!({"old": true}));
    fs::set_permissions(fixture.path("pub"), fs::Permissions::from_mode(0o555)).unwrap();
    let report = fixture.build(&[], Some(Publish::Direct), &Merger::default());
    fs::set_permissions(fixture.path("pub"), fs::Permissions::from_mode(0o755)).unwrap();
    let report = report.unwrap();
    assert_eq!(fixture.get("pub/phone-token.json"), json!({"old": true}));
    let targets: Vec<_> = report
        .install
        .iter()
        .map(|(_, target)| target.clone())
        .collect();
    assert_eq!(
        targets,
        [
            fixture.path("pub/laptop-token.json"),
            fixture.path("pub/phone-token.json")
        ]
    );
    assert_eq!(report.install[0].0, fixture.path("out/laptop.json"));
    assert!(
        build::sudo_install_command(&report.install[0].0, &report.install[0].1)
            .starts_with("sudo install -o root -g root -m 644 ")
    );
}

#[test]
fn sudo_publishing_writes_nothing_itself() {
    let fixture = Fixture::new();
    let report = fixture
        .build(&[], Some(Publish::Sudo), &Merger::default())
        .unwrap();
    assert_eq!(report.install.len(), 2);
    assert!(fixture.path("out/phone.json").exists());
    assert!(!fixture.path("pub/phone-token.json").exists());
}

#[cfg(unix)]
#[test]
fn pending_rotation_in_a_fragment_directory_blocks_the_build() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::os::unix::ffi::OsStrExt;
    let fixture = Fixture::new();
    let journal = fixture.path(".sb-rotate-transaction-test");
    fixture.put(
        "shared/.sb-rotate.pending",
        json!({"version": 1, "journal": STANDARD.encode(journal.as_os_str().as_bytes())}),
    );
    let sb = Merger::default();
    let error = fixture.build(&[], None, &sb).unwrap_err();
    assert!(error.to_string().contains("pending config transaction"));
    assert_eq!(sb.merges.get(), 0);
}
