use std::{cell::Cell, fs, path::Path};

use anyhow::{Result, bail};
use sb_rotate::{
    apply, binding,
    cli::{Input, Protocol},
    config::ConfigSet,
    plan::{self, RotationPlan},
    singbox::{RealityKeyPair, SingBox},
};
use semver::Version;
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Default)]
struct Generator {
    calls: Cell<usize>,
    checks: Cell<usize>,
    fail_check: Cell<bool>,
    fail_keys: Cell<bool>,
}
impl Generator {
    fn next(&self) -> usize {
        let n = self.calls.get() + 1;
        self.calls.set(n);
        n
    }
}
impl SingBox for Generator {
    fn version(&self) -> Result<Version> {
        Ok(Version::new(1, 14, 0))
    }
    fn generate_uuid(&self) -> Result<String> {
        Ok(format!("uuid-{}", self.next()))
    }
    fn generate_random_base64(&self, _: usize) -> Result<String> {
        Ok(format!("password-{}", self.next()))
    }
    fn generate_random_hex(&self, _: usize) -> Result<String> {
        Ok(format!("{:016x}", self.next()))
    }
    fn generate_reality_keypair(&self) -> Result<RealityKeyPair> {
        if self.fail_keys.get() {
            bail!("injected key generation failure");
        }
        let n = self.next();
        Ok(RealityKeyPair {
            private_key: format!("private-{n}"),
            public_key: format!("public-{n}"),
        })
    }
    fn check_file(&self, _: &Path) -> Result<()> {
        self.checks.set(self.checks.get() + 1);
        if self.fail_check.get() {
            bail!("injected validation failure");
        }
        Ok(())
    }
    fn check_directory(&self, path: &Path) -> Result<()> {
        self.check_file(path)
    }
    fn merge(&self, _: &Path, _: &[std::path::PathBuf]) -> Result<()> {
        unreachable!("rotation never merges configs")
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
            clients: vec![root.path().join("clients")],
            ..Input::default()
        };
        let fixture = Self { root, input };
        let mut inbounds = Vec::new();
        let mut outbounds = Vec::new();
        for (kind, field) in [("vless", "uuid"), ("hysteria2", "password")] {
            for n in 0..2 {
                let credential = format!("{kind}-{n}");
                let mut inbound = json!({"type": kind, "tag": format!("{kind}-{n}"),
                    "users": [{field: credential}, {field: format!("unmatched-{kind}-{n}")}]});
                let mut outbound = json!({"type": kind, "tag": format!("{kind}-{n}"),
                    field: credential, "server": "unchanged.example", "server_port": 443});
                if kind == "vless" {
                    inbound["tls"] = json!({"enabled": true, "reality": {"enabled": true,
                        "private_key": format!("old-private-{n}"), "short_id": ["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb"]}});
                    outbound["tls"] = json!({"enabled": true, "reality": {"enabled": true,
                        "public_key": format!("old-public-{n}"), "short_id": "aaaaaaaaaaaaaaaa"}});
                } else {
                    inbound["obfs"] =
                        json!({"type": "salamander", "password": format!("old-obfs-{n}")});
                    outbound["obfs"] = inbound["obfs"].clone();
                }
                inbounds.push(inbound);
                outbounds.push(outbound.clone());
                outbound["tag"] = json!(format!("copy-{kind}-{n}"));
                outbounds.push(outbound);
            }
        }
        fixture.put("server.json", json!({"inbounds": inbounds}));
        fixture.put("clients/all.json", json!({"outbounds": outbounds}));
        fixture
    }
    fn put(&self, name: &str, value: Value) {
        fs::write(
            self.root.path().join(name),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }
    fn get(&self, name: &str) -> Value {
        serde_json::from_slice(&fs::read(self.root.path().join(name)).unwrap()).unwrap()
    }
    fn plan(
        &self,
        protocol: Option<Protocol>,
        sb: &Generator,
    ) -> Result<(ConfigSet, RotationPlan)> {
        let configs = ConfigSet::load(&self.input)?;
        let inventory = binding::discover(&configs, &self.input)?;
        let plan = plan::rotation(&configs, &inventory, &self.input, protocol, sb)?;
        Ok((configs, plan))
    }
}

#[test]
fn type_rotation_composes_multiple_inbounds_and_shared_outbounds_in_one_apply() {
    for protocol in [Protocol::Vless, Protocol::Hysteria2] {
        let fixture = Fixture::new();
        let sb = Generator::default();
        let before_server = fixture.get("server.json");
        let before_client = fixture.get("clients/all.json");
        let (configs, plan) = fixture.plan(Some(protocol), &sb).unwrap();
        assert_eq!(fixture.get("server.json"), before_server);
        assert_eq!(fixture.get("clients/all.json"), before_client);
        assert_eq!(sb.checks.get(), 0);
        assert_eq!(apply::apply(&configs, &plan, &sb).unwrap(), 2);
        assert_eq!(sb.checks.get(), 2); // combined config set, not one apply per operation
        let server = fixture.get("server.json");
        let client = fixture.get("clients/all.json");
        let start = if protocol == Protocol::Vless { 0 } else { 2 };
        let field = protocol.credential();
        for i in 0..4 {
            if !(start..start + 2).contains(&i) {
                assert_eq!(server["inbounds"][i], before_server["inbounds"][i]);
                assert_eq!(
                    client["outbounds"][i * 2],
                    before_client["outbounds"][i * 2]
                );
                continue;
            }
            let inbound = &server["inbounds"][i];
            let a = &client["outbounds"][i * 2];
            let b = &client["outbounds"][i * 2 + 1];
            assert_eq!(inbound["users"][0][field], a[field]);
            assert_eq!(a[field], b[field]);
            assert_ne!(a[field], before_client["outbounds"][i * 2][field]);
            assert_eq!(
                inbound["users"][1],
                before_server["inbounds"][i]["users"][1]
            );
            assert_eq!(a["server"], "unchanged.example");
            assert_eq!(a["server_port"], 443);
            if protocol == Protocol::Vless {
                assert_eq!(
                    a["tls"]["reality"]["public_key"],
                    b["tls"]["reality"]["public_key"]
                );
                assert_ne!(
                    a["tls"]["reality"]["short_id"],
                    b["tls"]["reality"]["short_id"]
                );
                let ids = inbound["tls"]["reality"]["short_id"].as_array().unwrap();
                assert_eq!(ids.len(), 3);
                assert_eq!(ids[0], "bbbbbbbbbbbbbbbb"); // unattributed ID retained
                assert!(ids.contains(&a["tls"]["reality"]["short_id"]));
                assert!(ids.contains(&b["tls"]["reality"]["short_id"]));
            } else {
                assert_eq!(inbound["obfs"], a["obfs"]);
                assert_eq!(a["obfs"], b["obfs"]);
                assert_ne!(a["obfs"], before_client["outbounds"][i * 2]["obfs"]);
            }
        }
        let a = &server["inbounds"][start];
        let b = &server["inbounds"][start + 1];
        assert_ne!(a["users"][0][field], b["users"][0][field]);
        if protocol == Protocol::Vless {
            assert_ne!(
                a["tls"]["reality"]["private_key"],
                b["tls"]["reality"]["private_key"]
            );
        } else {
            assert_ne!(a["obfs"]["password"], b["obfs"]["password"]);
        }
    }
}

#[test]
fn untyped_rotation_covers_every_supported_type_in_one_plan() {
    let fixture = Fixture::new();
    let (configs, plan) = fixture.plan(None, &Generator::default()).unwrap();
    for suffix in [
        "/uuid",
        "/private_key",
        "/public_key",
        "/short_id",
        "/password",
        "/obfs/password",
    ] {
        assert!(
            plan.edits
                .iter()
                .any(|edit| edit.target.pointer.ends_with(suffix)),
            "missing {suffix}"
        );
    }
    for inbound in 0..4 {
        let prefix = format!("/inbounds/{inbound}/");
        assert!(
            plan.edits
                .iter()
                .any(|edit| edit.target.pointer.starts_with(&prefix))
        );
    }
    plan.materialize(&configs).unwrap();
}

#[test]
fn outbound_selection_rotates_only_per_client_material() {
    for protocol in [None, Some(Protocol::Vless)] {
        let mut fixture = Fixture::new();
        fixture.input.client_tag = vec!["vless-0".into()];
        let (configs, plan) = fixture.plan(protocol, &Generator::default()).unwrap();
        let count = |suffix: &str| {
            plan.edits
                .iter()
                .filter(|edit| edit.target.pointer.ends_with(suffix))
                .count()
        };
        // The shared user rotates on both outbounds; only the selected outbound
        // gets a new short ID; the keypair is shared with copy-vless-0.
        assert_eq!(count("/uuid"), 3);
        assert_eq!(count("_key"), 0);
        assert_eq!(count("/short_id"), 2);
        assert!(plan.edits.iter().all(|edit| {
            edit.target.pointer.starts_with("/inbounds/0/")
                || edit.target.pointer.starts_with("/outbounds/0/")
                || edit.target.pointer.starts_with("/outbounds/1/")
        }));
        let changed = plan.materialize(&configs).unwrap();
        let client = &changed[&fixture.root.path().join("clients/all.json")];
        assert_eq!(
            client["outbounds"][0]["uuid"],
            client["outbounds"][1]["uuid"]
        );
        assert_eq!(
            client["outbounds"][1]["tls"]["reality"]["short_id"],
            "aaaaaaaaaaaaaaaa"
        );
        assert_eq!(client["outbounds"][2]["uuid"], "vless-1");
    }
    // Selecting both outbounds of the inbound includes its shared keypair.
    let mut fixture = Fixture::new();
    fixture.input.client_tag = vec!["vless-0".into(), "copy-vless-0".into()];
    let (_, plan) = fixture.plan(None, &Generator::default()).unwrap();
    assert_eq!(
        plan.edits
            .iter()
            .filter(|edit| edit.target.pointer.ends_with("_key"))
            .count(),
        3
    );
}

#[test]
fn inbound_selection_limits_all_material_rotation() {
    let mut fixture = Fixture::new();
    fixture.input.inbound_tag = vec!["vless-1".into()];
    let (_, plan) = fixture
        .plan(Some(Protocol::Vless), &Generator::default())
        .unwrap();
    assert!(plan.edits.iter().all(|edit| {
        edit.target.pointer.starts_with("/inbounds/1/")
            || edit.target.pointer.starts_with("/outbounds/2/")
            || edit.target.pointer.starts_with("/outbounds/3/")
    }));
}

#[test]
fn optional_material_is_not_enabled_and_explicit_empty_selection_fails() {
    let fixture = Fixture::new();
    let mut server = fixture.get("server.json");
    let mut client = fixture.get("clients/all.json");
    for inbound in server["inbounds"].as_array_mut().unwrap() {
        inbound.as_object_mut().unwrap().remove("tls");
        inbound.as_object_mut().unwrap().remove("obfs");
    }
    for outbound in client["outbounds"].as_array_mut().unwrap() {
        outbound.as_object_mut().unwrap().remove("tls");
        outbound.as_object_mut().unwrap().remove("obfs");
    }
    fixture.put("server.json", server);
    fixture.put("clients/all.json", client);
    for protocol in [Protocol::Vless, Protocol::Hysteria2] {
        let (_, plan) = fixture.plan(Some(protocol), &Generator::default()).unwrap();
        assert_eq!(plan.edits.len(), 6);
        assert!(
            plan.edits
                .iter()
                .all(|edit| !edit.target.pointer.contains("tls")
                    && !edit.target.pointer.contains("obfs"))
        );
    }
    let mut fixture = fixture;
    fixture.input.client_tag = vec!["no-such-outbound".into()];
    let sb = Generator::default();
    for protocol in [None, Some(Protocol::Vless), Some(Protocol::Hysteria2)] {
        let error = fixture.plan(protocol, &sb).err().unwrap();
        assert!(error.to_string().contains("no configured material"));
    }
    assert_eq!(sb.calls.get(), 0);
}

#[test]
fn combined_generation_or_validation_failure_never_writes_partial_rotation() {
    let fixture = Fixture::new();
    let sb = Generator::default();
    let originals = ["server.json", "clients/all.json"]
        .map(|name| fs::read(fixture.root.path().join(name)).unwrap());
    sb.fail_keys.set(true);
    assert!(fixture.plan(Some(Protocol::Vless), &sb).is_err());
    sb.fail_keys.set(false);
    let (configs, plan) = fixture.plan(Some(Protocol::Vless), &sb).unwrap();
    sb.fail_check.set(true);
    assert!(apply::apply(&configs, &plan, &sb).is_err());
    for (name, original) in ["server.json", "clients/all.json"]
        .into_iter()
        .zip(originals)
    {
        assert_eq!(fs::read(fixture.root.path().join(name)).unwrap(), original);
    }
}

#[test]
fn ambiguous_inbounds_are_not_silently_split_by_batching() {
    let fixture = Fixture::new();
    let mut server = fixture.get("server.json");
    server["inbounds"][1]["users"] = server["inbounds"][0]["users"].clone();
    fixture.put("server.json", server);
    let sb = Generator::default();
    let error = fixture.plan(None, &sb).err().unwrap();
    assert!(error.to_string().contains("ambiguous"));
    assert_eq!(sb.calls.get(), 0);
}
