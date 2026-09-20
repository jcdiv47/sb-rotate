#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use tempfile::TempDir;

struct Fixture {
    root: TempDir,
}

impl Fixture {
    fn new(version: &str) -> Self {
        let fixture = Self {
            root: tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap(),
        };
        fs::create_dir(fixture.root.path().join("clients")).unwrap();
        fixture.write("server.json", &json!({"inbounds": [{"type": "vless", "tag": "home", "users": [{"uuid": "11111111-1111-4111-8111-111111111111"}]}]}));
        fixture.write("clients/phone.json", &json!({"outbounds": [{"type": "vless", "tag": "home", "uuid": "11111111-1111-4111-8111-111111111111"}]}));
        fixture.binary("sing-box-test", version);
        fixture
    }

    fn write(&self, name: &str, value: &Value) {
        fs::write(
            self.root.path().join(name),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }

    fn binary(&self, name: &str, version: &str) {
        let private = URL_SAFE_NO_PAD.encode([1u8; 32]);
        let public = URL_SAFE_NO_PAD.encode([2u8; 32]);
        let script = format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$TEST_LOG"
case "$1" in
  version) printf 'sing-box version {version}\n' ;;
  generate)
    if [ "$2" = uuid ]; then
      printf 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n'
    elif [ "$2" = reality-keypair ]; then
      printf 'PrivateKey: {private}\nPublicKey: {public}\n'
    elif [ "$4" = --hex ]; then
      printf '0123456789abcdef\n'
    else
      printf 'aW5kZXBlbmRlbnQtZ2VuZXJhdGVkLXNlY3JldA==\n'
    fi ;;
  check)
    if [ "$TEST_FAIL_CHECK" = yes ]; then
      printf 'injected sing-box config validation error\n' >&2
      exit 1
    fi ;;
  *) exit 42 ;;
esac
"#
        );
        let path = self.root.path().join(name);
        fs::write(&path, script).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn command(&self, command: &str) -> Command {
        let mut process = Command::new(env!("CARGO_BIN_EXE_sb-rotate"));
        process
            .current_dir(self.root.path())
            .env("SING_BOX", self.root.path().join("sing-box-test"))
            .env("TEST_LOG", self.root.path().join("invocations"))
            .env_remove("TEST_FAIL_CHECK")
            .args([command, "--server", "server.json", "--clients", "clients"]);
        process
    }

    fn enable_reality(&self) {
        for (name, reality) in [
            (
                "server.json",
                json!({"enabled": true, "private_key": "old-private", "short_id": ["aaaaaaaaaaaaaaaa"]}),
            ),
            (
                "clients/phone.json",
                json!({"enabled": true, "public_key": "old-public", "short_id": "aaaaaaaaaaaaaaaa"}),
            ),
        ] {
            let mut value: Value =
                serde_json::from_slice(&fs::read(self.root.path().join(name)).unwrap()).unwrap();
            let endpoints = if name == "server.json" {
                "inbounds"
            } else {
                "outbounds"
            };
            value[endpoints][0]["tls"] = json!({"enabled": true, "reality": reality});
            self.write(name, &value);
        }
    }

    fn log(&self) -> String {
        fs::read_to_string(self.root.path().join("invocations")).unwrap()
    }
    fn originals(&self) -> Vec<Vec<u8>> {
        ["server.json", "clients/phone.json"]
            .iter()
            .map(|name| fs::read(self.root.path().join(name)).unwrap())
            .collect()
    }
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn plan_runs_version_then_generator_and_never_writes_or_checks() {
    let fixture = Fixture::new("1.14.0");
    let originals = fixture.originals();
    let output = fixture
        .command("plan")
        .args(["--kind", "vless-uuid"])
        .output()
        .unwrap();
    let text = success(output);
    assert!(text.contains("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
    assert!(text.contains("Preview only"));
    assert_eq!(fixture.originals(), originals);
    assert_eq!(fixture.log(), "version\ngenerate uuid\n");
}

#[test]
fn rotate_uses_generated_uuid_and_checks_before_success() {
    let fixture = Fixture::new("1.14.0");
    let text = success(
        fixture
            .command("rotate")
            .args(["--kind", "vless-uuid"])
            .output()
            .unwrap(),
    );
    assert!(text.contains("Updated 2 config file(s)"));
    for bytes in fixture.originals() {
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
        assert!(!text.contains("11111111-1111-4111-8111-111111111111"));
    }
    assert_eq!(
        fixture
            .log()
            .lines()
            .filter(|line| line.starts_with("check -c "))
            .count(),
        2
    );
}

#[test]
fn failing_validator_exits_nonzero_without_replacing_configs() {
    let fixture = Fixture::new("1.14.0");
    let originals = fixture.originals();
    let output = fixture
        .command("rotate")
        .args(["--kind", "vless-uuid"])
        .env("TEST_FAIL_CHECK", "yes")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("validating config"));
    assert!(error.contains("injected sing-box config validation error"));
    assert_eq!(fixture.originals(), originals);
}

#[test]
fn version_gate_rejects_old_prerelease_and_malformed_versions_before_loading() {
    for version in ["1.13.9", "1.14.0-beta.9", "invalid"] {
        let fixture = Fixture::new(version);
        fs::remove_file(fixture.root.path().join("server.json")).unwrap();
        let output = fixture.command("inspect").output().unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("version")
        );
        assert_eq!(fixture.log(), "version\n");
    }
}

#[test]
fn explicit_executable_overrides_environment_and_missing_binary_is_contextual() {
    let fixture = Fixture::new("1.13.0");
    fixture.binary("supported", "1.14.0");
    success(
        fixture
            .command("inspect")
            .args(["--sing-box", "./supported"])
            .output()
            .unwrap(),
    );
    let output = fixture
        .command("inspect")
        .args(["--sing-box", "./does-not-exist"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("executing ./does-not-exist")
    );
}

#[test]
fn check_uses_directory_flag_for_server_and_file_flag_for_clients() {
    let fixture = Fixture::new("1.14.0");
    fs::create_dir(fixture.root.path().join("server.d")).unwrap();
    fs::rename(
        fixture.root.path().join("server.json"),
        fixture.root.path().join("server.d/inbounds.json"),
    )
    .unwrap();
    let mut process = Command::new(env!("CARGO_BIN_EXE_sb-rotate"));
    let text = success(
        process
            .current_dir(fixture.root.path())
            .env("TEST_LOG", fixture.root.path().join("invocations"))
            .env_remove("TEST_FAIL_CHECK")
            .args([
                "check",
                "--server",
                "server.d",
                "--client",
                "clients/phone.json",
                "--sing-box",
                "./sing-box-test",
            ])
            .output()
            .unwrap(),
    );
    assert!(text.contains("Validated server config set and 1 client config(s)"));
    let log = fixture.log();
    let lines: Vec<_> = log.lines().collect();
    assert!(lines[1].starts_with("check -C "));
    assert!(lines[2].starts_with("check -c "));
}

#[test]
fn help_and_required_client_sources_are_enforced() {
    let text = success(
        Command::new(env!("CARGO_BIN_EXE_sb-rotate"))
            .arg("--help")
            .output()
            .unwrap(),
    );
    assert!(text.contains("inspect"));
    assert!(text.contains("rotate"));
    let output = Command::new(env!("CARGO_BIN_EXE_sb-rotate"))
        .args(["inspect", "--server", "server.json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("--clients")
    );
}

#[test]
fn passwords_are_masked_in_cli_and_use_singbox_base64_generator() {
    let fixture = Fixture::new("1.14.0");
    fixture.write(
        "server.json",
        &json!({"inbounds": [{"type": "hysteria2", "users": [{"password": "old-password"}]}]}),
    );
    fixture.write(
        "clients/phone.json",
        &json!({"outbounds": [{"type": "hysteria2", "password": "old-password"}]}),
    );
    let text = success(
        fixture
            .command("plan")
            .args(["--kind", "hysteria2-password"])
            .output()
            .unwrap(),
    );
    assert!(text.contains("********"));
    assert!(!text.contains("old-password"));
    assert!(!text.contains("aW5kZXBlbmRlbnQtZ2VuZXJhdGVkLXNlY3JldA=="));
    assert!(fixture.log().contains("generate rand 32 --base64"));
}

#[test]
fn reality_keypair_and_hex_generators_are_wired_to_cli_with_safe_output() {
    for (kind, command) in [
        ("vless-reality-keypair", "generate reality-keypair"),
        ("vless-reality-short-id", "generate rand 8 --hex"),
    ] {
        let fixture = Fixture::new("1.14.0");
        fixture.enable_reality();
        let originals = fixture.originals();
        let output = success(
            fixture
                .command("plan")
                .args(["--kind", kind])
                .output()
                .unwrap(),
        );
        assert_eq!(fixture.originals(), originals);
        assert!(!output.contains("old-private"));
        assert!(!output.contains(&URL_SAFE_NO_PAD.encode([1u8; 32])));
        assert!(!output.contains("0123456789abcdef"));
        assert!(fixture.log().contains(command));
        success(
            fixture
                .command("rotate")
                .args(["--kind", kind])
                .output()
                .unwrap(),
        );
        assert_ne!(fixture.originals(), originals);
    }
}

#[test]
fn set_dry_run_and_commit_never_generate_or_edit_the_server() {
    let fixture = Fixture::new("1.14.0");
    let originals = fixture.originals();
    success(
        fixture
            .command("set")
            .args(["--kind", "server", "--value", "new.example", "--dry-run"])
            .output()
            .unwrap(),
    );
    assert_eq!(fixture.originals(), originals);
    assert_eq!(fixture.log(), "version\n");
    success(
        fixture
            .command("set")
            .args(["--kind", "server", "--value", "new.example"])
            .output()
            .unwrap(),
    );
    assert_eq!(fixture.originals()[0], originals[0]);
    let client: Value = serde_json::from_slice(&fixture.originals()[1]).unwrap();
    assert_eq!(client["outbounds"][0]["server"], "new.example");
    assert!(!fixture.log().contains("generate"));
}

#[test]
fn cli_service_selectors_are_rejected_before_generating_keys() {
    let fixture = Fixture::new("1.14.0");
    fixture.enable_reality();
    let output = fixture
        .command("rotate")
        .args(["--kind", "vless-reality-keypair", "--client-tag", "home"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("service-wide")
    );
    assert_eq!(fixture.log(), "version\n");
}

fn wait_for_child(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "child process timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn competing_cli_writers_fail_fast_and_can_retry_after_the_first_finishes() {
    struct Running {
        child: Option<Child>,
        release: std::path::PathBuf,
    }
    impl Drop for Running {
        fn drop(&mut self) {
            // Release the validator even if an assertion fails, then reap the CLI.
            let _ = fs::write(&self.release, b"release");
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    let fixture = Fixture::new("1.14.0");
    let script = fixture.root.path().join("blocking-sing-box");
    fs::write(
        &script,
        r#"#!/bin/sh
case "$1" in
  version) printf 'sing-box version 1.14.0\n' ;;
  check)
    : > "$TEST_READY"
    count=0
    while [ ! -e "$TEST_RELEASE" ]; do
      count=$((count + 1))
      [ "$count" -lt 600 ] || exit 1
      sleep 0.05
    done ;;
  *) exit 42 ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let ready = fixture.root.path().join(".validator-ready");
    let release = fixture.root.path().join(".validator-release");
    let originals = fixture.originals();
    let spawn = |value: &str| {
        fixture
            .command("set")
            .args(["--kind", "server", "--value", value, "--sing-box"])
            .arg(&script)
            .env("TEST_READY", &ready)
            .env("TEST_RELEASE", &release)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let mut first = Running {
        child: Some(spawn("first.example")),
        release: release.clone(),
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ready.exists() {
        assert!(
            Instant::now() < deadline,
            "first writer never reached validation"
        );
        assert!(
            first.child.as_mut().unwrap().try_wait().unwrap().is_none(),
            "first writer exited early"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let second = wait_for_child(spawn("second.example"));
    assert!(!second.status.success());
    assert!(
        String::from_utf8(second.stderr)
            .unwrap()
            .contains("another sb-rotate writer")
    );
    assert_eq!(fixture.originals(), originals);
    fs::write(&release, b"release").unwrap();
    success(wait_for_child(first.child.take().unwrap()));
    let current: Value = serde_json::from_slice(&fixture.originals()[1]).unwrap();
    assert_eq!(current["outbounds"][0]["server"], "first.example");
    success(wait_for_child(spawn("second.example")));
    let current: Value = serde_json::from_slice(&fixture.originals()[1]).unwrap();
    assert_eq!(current["outbounds"][0]["server"], "second.example");
}

#[test]
fn recover_cli_restores_an_interrupted_journal_without_singbox() {
    use base64::engine::general_purpose::STANDARD;
    use sha2::{Digest, Sha256};
    use std::{
        os::unix::{ffi::OsStrExt, fs::MetadataExt},
        path::Path,
    };
    let fixture = Fixture::new("1.14.0");
    let originals = fixture.originals();
    let root = fixture.root.path();
    let journal = root.join(".sb-rotate-transaction-cli-test");
    fs::create_dir(&journal).unwrap();
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o700)).unwrap();
    let encode = |path: &Path| STANDARD.encode(path.as_os_str().as_bytes());
    // This fixture deliberately specifies the versioned on-disk format, testing
    // the public recovery CLI rather than calling internal transaction helpers.
    let signature = |path: &Path| {
        let metadata = fs::metadata(path).unwrap();
        json!({"sha256": format!("{:x}", Sha256::digest(fs::read(path).unwrap())), "state": {
            "length": metadata.len(), "modified": metadata.modified().unwrap(), "mode": metadata.mode(),
            "device": metadata.dev(), "inode": metadata.ino(), "links": metadata.nlink(), "uid": metadata.uid(), "gid": metadata.gid()
        }})
    };
    let mut entries = Vec::new();
    let mut staged = Vec::new();
    for (index, name) in ["server.json", "clients/phone.json"].iter().enumerate() {
        let file = root.join(name);
        let temp = file
            .parent()
            .unwrap()
            .join(format!(".sb-rotate-test-{index}.tmp"));
        let before = fs::read(&file).unwrap();
        let after = String::from_utf8(before.clone()).unwrap().replace(
            "11111111-1111-4111-8111-111111111111",
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        );
        fs::write(journal.join(format!("{index}.original")), before).unwrap();
        fs::write(&temp, after).unwrap();
        fs::set_permissions(&temp, fs::metadata(&file).unwrap().permissions()).unwrap();
        entries.push(json!({"file": encode(&file), "staged": encode(&temp), "original": signature(&file), "installed": signature(&temp)}));
        staged.push((file, temp));
    }
    let clients = root.join("clients");
    let manifest = json!({"version": 1, "platform": std::env::consts::OS,
        "directories": [encode(root), encode(&clients)], "entries": entries,
        "sources": {encode(&root.join("server.json")): encode(&root.join("server.json")), encode(&clients): encode(&clients)},
        "inventories": [{"directory": encode(&clients), "files": {encode(Path::new("phone.json")): encode(&clients.join("phone.json"))}}],
        "unchanged": []});
    fs::write(
        journal.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    for directory in [root, clients.as_path()] {
        fs::write(
            directory.join(".sb-rotate.pending"),
            serde_json::to_vec(&json!({"version": 1, "journal": encode(&journal)})).unwrap(),
        )
        .unwrap();
    }
    fs::write(journal.join("started"), b"1\n").unwrap();
    fs::rename(&staged[0].1, &staged[0].0).unwrap();
    let interrupted = fixture.originals();
    let blocked = fixture
        .command("rotate")
        .args(["--kind", "vless-uuid"])
        .output()
        .unwrap();
    assert!(!blocked.status.success());
    assert!(
        String::from_utf8(blocked.stderr)
            .unwrap()
            .contains("pending config transaction")
    );
    assert_eq!(fixture.log(), "version\n");
    let run = |args: &[&std::ffi::OsStr]| {
        Command::new(env!("CARGO_BIN_EXE_sb-rotate"))
            .arg("recover")
            .args(args)
            .env("SING_BOX", root.join("nonexistent-sing-box"))
            .output()
            .unwrap()
    };
    let preview = success(run(&[
        "--journal".as_ref(),
        journal.as_os_str(),
        "--dry-run".as_ref(),
    ]));
    assert!(preview.contains("installed replacement"));
    assert_eq!(fixture.originals(), interrupted);
    let completed = success(run(&["--directory".as_ref(), clients.as_os_str()]));
    assert!(completed.contains("Recovery complete"));
    assert_eq!(fixture.originals(), originals);
    assert!(!journal.exists());
    assert!(!root.join(".sb-rotate.pending").exists());
    assert!(!clients.join(".sb-rotate.pending").exists());
    assert_eq!(fixture.log(), "version\n"); // recovery never called sing-box
}

#[test]
fn recover_cli_requires_one_locator_and_reports_missing_transactions() {
    let fixture = Fixture::new("1.14.0");
    let output = Command::new(env!("CARGO_BIN_EXE_sb-rotate"))
        .args(["recover"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let output = Command::new(env!("CARGO_BIN_EXE_sb-rotate"))
        .args(["recover", "--journal", "/unused", "--directory", "/unused"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let output = Command::new(env!("CARGO_BIN_EXE_sb-rotate"))
        .args(["recover", "--directory"])
        .arg(fixture.root.path())
        .env("SING_BOX", "does-not-exist")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("no pending transaction")
    );
}

#[test]
fn path_lookup_is_the_final_executable_fallback() {
    let fixture = Fixture::new("1.14.0");
    fixture.binary("sing-box", "1.14.0");
    let output = fixture
        .command("inspect")
        .env_remove("SING_BOX")
        .env("PATH", fixture.root.path())
        .output()
        .unwrap();
    assert!(success(output).contains("vless inbound="));
    assert_eq!(fixture.log(), "version\n");
}
