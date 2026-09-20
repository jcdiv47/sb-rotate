#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Output},
};

use serde_json::{Value, json};
use tempfile::TempDir;

struct Fixture {
    root: TempDir,
}

impl Fixture {
    fn new(version: &str) -> Self {
        let fixture = Self {
            root: tempfile::tempdir().unwrap(),
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
        let script = format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$TEST_LOG"
case "$1" in
  version) printf 'sing-box version {version}\n' ;;
  generate)
    if [ "$2" = uuid ]; then
      printf 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n'
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
