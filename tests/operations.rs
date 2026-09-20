use std::{
    cell::{Cell, RefCell},
    fs,
    path::Path,
};

use anyhow::{Result, bail};
use sb_rotate::{
    apply, binding,
    cli::{Input, PropertyKind, RotationKind},
    config::ConfigSet,
    plan::{self, Edit, OperationKind, RotationPlan},
    protocol::properties,
    singbox::{RealityKeyPair, SingBox},
};
use semver::Version;
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Default)]
struct Generator {
    calls: Cell<usize>,
    checks: Cell<usize>,
    hex: RefCell<Option<String>>,
    fail_generation: Cell<bool>,
    fail_validation: Cell<bool>,
}
impl Generator {
    fn next(&self) -> Result<usize> {
        if self.fail_generation.get() {
            bail!("injected generation failure");
        }
        self.calls.set(self.calls.get() + 1);
        Ok(self.calls.get())
    }
}
impl SingBox for Generator {
    fn version(&self) -> Result<Version> {
        Ok(Version::new(1, 14, 0))
    }
    fn generate_uuid(&self) -> Result<String> {
        bail!("unexpected UUID generation")
    }
    fn generate_random_base64(&self, bytes: usize) -> Result<String> {
        assert_eq!(bytes, 32);
        Ok(format!("new-password-{}", self.next()?))
    }
    fn generate_random_hex(&self, bytes: usize) -> Result<String> {
        assert_eq!(bytes, 8);
        let n = self.next()?;
        Ok(self
            .hex
            .borrow()
            .clone()
            .unwrap_or_else(|| format!("{n:016x}")))
    }
    fn generate_reality_keypair(&self) -> Result<RealityKeyPair> {
        self.next()?;
        Ok(RealityKeyPair {
            private_key: "new-private-key".to_owned(),
            public_key: "new-public-key".to_owned(),
        })
    }
    fn check_file(&self, _path: &Path) -> Result<()> {
        self.checks.set(self.checks.get() + 1);
        if self.fail_validation.get() {
            bail!("injected validation failure");
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
    fn new(hysteria: bool) -> Self {
        let root = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        fs::create_dir(root.path().join("clients")).unwrap();
        let input = Input {
            server: root.path().join("server.json"),
            clients: Some(root.path().join("clients")),
            ..Input::default()
        };
        let fixture = Self { root, input };
        let protocol = if hysteria { "hysteria2" } else { "vless" };
        let credential = if hysteria { "password" } else { "uuid" };
        let tls = if hysteria {
            json!({"enabled": true})
        } else {
            json!({"enabled": true, "reality": {"enabled": true, "private_key": "old-private-key", "short_id": ["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb", "cccccccccccccccc"]}})
        };
        fixture.put("server.json", json!({"inbounds": [{"type": protocol, "tag": "home", "listen_port": 443, "users": [{credential: "alice"}, {credential: "bob"}], "tls": tls, "obfs": {"type": "salamander", "password": "old-obfs"}}]}));
        for (file, values) in [
            (
                "phone",
                vec![
                    ("home", "alice", "aaaaaaaaaaaaaaaa"),
                    ("backup", "alice", "aaaaaaaaaaaaaaaa"),
                ],
            ),
            ("laptop", vec![("home", "bob", "bbbbbbbbbbbbbbbb")]),
        ] {
            let outbounds: Vec<_> = values.into_iter().map(|(tag, identity, id)| {
                let tls = if hysteria { json!({"enabled": true, "server_name": "old.example"}) } else { json!({"enabled": true, "server_name": "old.example", "reality": {"enabled": true, "public_key": "old-public-key", "short_id": id}}) };
                json!({"type": protocol, "tag": tag, credential: identity, "server": "old.example", "server_port": 443, "tls": tls, "obfs": {"type": "salamander", "password": "old-obfs"}, "unrelated": {"keep": true}})
            }).collect();
            fixture.put(
                &format!("clients/{file}.json"),
                json!({"outbounds": outbounds}),
            );
        }
        fixture
    }
    fn put(&self, path: &str, value: Value) {
        fs::write(
            self.root.path().join(path),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }
    fn get(&self, path: &str) -> Value {
        serde_json::from_slice(&fs::read(self.root.path().join(path)).unwrap()).unwrap()
    }
    fn mutate(&self, path: &str, f: impl FnOnce(&mut Value)) {
        let mut value = self.get(path);
        f(&mut value);
        self.put(path, value);
    }
    fn configs(&self) -> ConfigSet {
        ConfigSet::load(&self.input).unwrap()
    }
    fn rotate(&self, kind: RotationKind, generator: &Generator) -> Result<RotationPlan> {
        let configs = self.configs();
        let inventory = binding::discover(&configs, &self.input)?;
        plan::rotation(&configs, &inventory, &self.input, kind, generator)
    }
    fn set(&self, kind: PropertyKind, value: &str) -> Result<RotationPlan> {
        let configs = self.configs();
        let inventory = binding::discover(&configs, &self.input)?;
        properties::set(&configs, &inventory, &self.input, kind, value)
    }
    fn materialized(&self, plan: &RotationPlan, file: &str) -> Value {
        plan.materialize(&self.configs())
            .unwrap()
            .remove(&self.root.path().join(file))
            .unwrap_or_else(|| self.get(file))
    }
}

#[test]
fn short_id_selection_preserves_shared_and_unknown_accepted_ids() {
    let mut fixture = Fixture::new(false);
    fixture.input.client = vec![fixture.root.path().join("clients/phone.json")];
    fixture.input.client_tag = vec!["home".to_owned()];
    let generator = Generator::default();
    let plan = fixture
        .rotate(RotationKind::VlessRealityShortId, &generator)
        .unwrap();
    assert_eq!(generator.calls.get(), 1);
    assert_eq!(plan.edits.len(), 2);
    let server = fixture.materialized(&plan, "server.json");
    assert_eq!(
        server["inbounds"][0]["tls"]["reality"]["short_id"],
        json!([
            "aaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbb",
            "cccccccccccccccc",
            "0000000000000001"
        ])
    );
    let phone = fixture.materialized(&plan, "clients/phone.json");
    assert_eq!(
        phone["outbounds"][0]["tls"]["reality"]["short_id"],
        "0000000000000001"
    );
    assert_eq!(
        phone["outbounds"][1]["tls"]["reality"]["short_id"],
        "aaaaaaaaaaaaaaaa"
    );
    assert_eq!(phone["outbounds"][0]["uuid"], "alice");
    assert!(!plan.render().contains("aaaaaaaaaaaaaaaa"));
    assert!(!plan.render().contains("0000000000000001"));
}

#[test]
fn short_ids_are_independent_per_outbound_and_retire_only_observed_old_ids() {
    let fixture = Fixture::new(false);
    let generator = Generator::default();
    let plan = fixture
        .rotate(RotationKind::VlessRealityShortId, &generator)
        .unwrap();
    assert_eq!(generator.calls.get(), 3);
    let server = fixture.materialized(&plan, "server.json");
    let accepted = server["inbounds"][0]["tls"]["reality"]["short_id"]
        .as_array()
        .unwrap();
    assert_eq!(accepted.len(), 4);
    assert_eq!(accepted[0], "cccccccccccccccc");
    let phone = fixture.materialized(&plan, "clients/phone.json");
    assert_ne!(
        phone["outbounds"][0]["tls"]["reality"]["short_id"],
        phone["outbounds"][1]["tls"]["reality"]["short_id"]
    );
}

#[test]
fn short_ids_compare_decoded_meaning_and_keep_equivalent_unselected_ids() {
    let mut fixture = Fixture::new(false);
    fixture.mutate("server.json", |v| {
        v["inbounds"][0]["tls"]["reality"]["short_id"] = json!(["AB", "bbbbbbbbbbbbbbbb"])
    });
    fixture.mutate("clients/phone.json", |v| {
        v["outbounds"][0]["tls"]["reality"]["short_id"] = json!("ab");
        v["outbounds"][1]["tls"]["reality"]["short_id"] = json!("ab00000000000000");
    });
    fixture.input.client_tag = vec!["home".into()];
    fixture.input.client = vec![fixture.root.path().join("clients/phone.json")];
    let plan = fixture
        .rotate(RotationKind::VlessRealityShortId, &Generator::default())
        .unwrap();
    assert_eq!(
        fixture.materialized(&plan, "server.json")["inbounds"][0]["tls"]["reality"]["short_id"][0],
        "AB"
    );
}

#[test]
fn short_ids_reject_collisions_malformed_generators_and_inconsistent_membership() {
    for id in ["aaaaaaaaaaaaaaaa", "not-hex", "123"] {
        let fixture = Fixture::new(false);
        let generator = Generator::default();
        generator.hex.replace(Some(id.into()));
        assert!(
            fixture
                .rotate(RotationKind::VlessRealityShortId, &generator)
                .is_err()
        );
    }
    let fixture = Fixture::new(false);
    let generator = Generator::default();
    generator.hex.replace(Some("1111111111111111".into()));
    assert!(
        fixture
            .rotate(RotationKind::VlessRealityShortId, &generator)
            .err()
            .unwrap()
            .to_string()
            .contains("collides")
    );
    fixture.mutate("clients/phone.json", |v| {
        v["outbounds"][0]["tls"]["reality"]["short_id"] = json!("ffffffffffffffff")
    });
    let generator = Generator::default();
    assert!(
        fixture
            .rotate(RotationKind::VlessRealityShortId, &generator)
            .is_err()
    );
    assert_eq!(generator.calls.get(), 0);
}

#[test]
fn reality_keypair_updates_all_identities_and_masks_only_private_key() {
    let fixture = Fixture::new(false);
    let generator = Generator::default();
    let plan = fixture
        .rotate(RotationKind::VlessRealityKeypair, &generator)
        .unwrap();
    assert_eq!(generator.calls.get(), 1);
    assert_eq!(plan.edits.len(), 4);
    let text = plan.render();
    assert!(!text.contains("old-private-key"));
    assert!(!text.contains("new-private-key"));
    assert!(text.contains("new-public-key"));
    assert_eq!(
        fixture.materialized(&plan, "server.json")["inbounds"][0]["tls"]["reality"]["private_key"],
        "new-private-key"
    );
    for file in ["clients/phone.json", "clients/laptop.json"] {
        for client in fixture.materialized(&plan, file)["outbounds"]
            .as_array()
            .unwrap()
        {
            assert_eq!(client["tls"]["reality"]["public_key"], "new-public-key");
            assert_eq!(client["unrelated"], json!({"keep": true}));
        }
    }
}

#[test]
fn keypair_skips_clients_without_reality_and_preflights_enabled_clients() {
    let fixture = Fixture::new(false);
    fixture.mutate("clients/laptop.json", |v| {
        v["outbounds"][0]["tls"]
            .as_object_mut()
            .unwrap()
            .remove("reality");
    });
    let plan = fixture
        .rotate(RotationKind::VlessRealityKeypair, &Generator::default())
        .unwrap();
    assert_eq!(plan.edits.len(), 3);
    fixture.mutate("clients/phone.json", |v| {
        v["outbounds"][0]["tls"]["reality"]
            .as_object_mut()
            .unwrap()
            .remove("public_key");
    });
    let generator = Generator::default();
    assert!(
        fixture
            .rotate(RotationKind::VlessRealityKeypair, &generator)
            .is_err()
    );
    assert_eq!(generator.calls.get(), 0);
}

#[test]
fn obfs_rotation_updates_whole_service_without_changing_auth_passwords() {
    let fixture = Fixture::new(true);
    let generator = Generator::default();
    let plan = fixture
        .rotate(RotationKind::Hysteria2ObfsPassword, &generator)
        .unwrap();
    assert_eq!(generator.calls.get(), 1);
    assert_eq!(plan.edits.len(), 4);
    assert!(!plan.render().contains("new-password"));
    assert!(!plan.render().contains("old-obfs"));
    let laptop = fixture.materialized(&plan, "clients/laptop.json");
    assert_eq!(laptop["outbounds"][0]["obfs"]["password"], "new-password-1");
    assert_eq!(laptop["outbounds"][0]["password"], "bob");
}

#[test]
fn obfs_rotation_rejects_missing_or_mismatched_blocks_without_generation() {
    for field in ["type", "password"] {
        let fixture = Fixture::new(true);
        fixture.mutate("clients/laptop.json", |v| {
            v["outbounds"][0]["obfs"]
                .as_object_mut()
                .unwrap()
                .remove(field);
        });
        let generator = Generator::default();
        assert!(
            fixture
                .rotate(RotationKind::Hysteria2ObfsPassword, &generator)
                .is_err()
        );
        assert_eq!(generator.calls.get(), 0);
    }
    let fixture = Fixture::new(true);
    fixture.mutate("clients/laptop.json", |v| {
        v["outbounds"][0]["obfs"]["type"] = json!("different")
    });
    assert!(
        fixture
            .rotate(RotationKind::Hysteria2ObfsPassword, &Generator::default())
            .is_err()
    );
}

#[test]
fn service_wide_operations_reject_client_selectors() {
    for hysteria in [false, true] {
        for path_selector in [false, true] {
            let mut fixture = Fixture::new(hysteria);
            if path_selector {
                fixture.input.client = vec![fixture.root.path().join("clients/phone.json")];
            } else {
                fixture.input.client_tag = vec!["home".into()];
            }
            let kind = if hysteria {
                RotationKind::Hysteria2ObfsPassword
            } else {
                RotationKind::VlessRealityKeypair
            };
            assert!(
                fixture
                    .rotate(kind, &Generator::default())
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("service-wide")
            );
            assert!(fixture.set(PropertyKind::Server, "new.example").is_err());
        }
    }
}

#[test]
fn multiple_services_require_unambiguous_inbound_selector() {
    let mut fixture = Fixture::new(false);
    fixture.mutate("server.json", |v| {
        let mut second = v["inbounds"][0].clone();
        second["tag"] = json!("office");
        second["users"] = json!([{"uuid": "office-user"}]);
        v["inbounds"].as_array_mut().unwrap().push(second);
    });
    fixture.mutate("clients/laptop.json", |v| {
        let mut second = v["outbounds"][0].clone();
        second["uuid"] = json!("office-user");
        second["tag"] = json!("office");
        v["outbounds"].as_array_mut().unwrap().push(second);
    });
    for kind in [
        RotationKind::VlessRealityKeypair,
        RotationKind::VlessRealityShortId,
    ] {
        assert!(
            fixture
                .rotate(kind, &Generator::default())
                .err()
                .unwrap()
                .to_string()
                .contains("multiple services")
        );
    }
    assert!(fixture.set(PropertyKind::Server, "new.example").is_err());
    fixture.input.inbound_tag = vec!["home".into()];
    assert!(
        fixture
            .rotate(RotationKind::VlessRealityKeypair, &Generator::default())
            .is_ok()
    );
    assert!(fixture.set(PropertyKind::Server, "new.example").is_ok());
}

#[test]
fn address_port_and_tls_updates_touch_all_clients_but_not_server() {
    for hysteria in [false, true] {
        let fixture = Fixture::new(hysteria);
        for (kind, value, field, expected) in [
            (
                PropertyKind::Server,
                "new.example",
                "/server",
                json!("new.example"),
            ),
            (
                PropertyKind::ServerPort,
                "8443",
                "/server_port",
                json!(8443),
            ),
            (
                PropertyKind::TlsServerName,
                "new-tls.example",
                "/tls/server_name",
                json!("new-tls.example"),
            ),
        ] {
            let plan = fixture.set(kind, value).unwrap();
            assert_eq!(plan.edits.len(), 3);
            assert!(
                plan.edits
                    .iter()
                    .all(|edit| edit.target.file != fixture.input.server)
            );
            for file in ["clients/phone.json", "clients/laptop.json"] {
                for outbound in fixture.materialized(&plan, file)["outbounds"]
                    .as_array()
                    .unwrap()
                {
                    assert_eq!(outbound.pointer(field).unwrap(), &expected);
                }
            }
        }
    }
}

#[test]
fn tls_name_can_be_added_but_tls_block_is_never_invented() {
    let fixture = Fixture::new(false);
    fixture.mutate("clients/phone.json", |v| {
        v["outbounds"][0]["tls"]
            .as_object_mut()
            .unwrap()
            .remove("server_name");
    });
    let plan = fixture
        .set(PropertyKind::TlsServerName, "new.example")
        .unwrap();
    assert!(plan.edits.iter().any(|edit| edit.old.is_none()));
    assert_eq!(
        fixture.materialized(&plan, "clients/phone.json")["outbounds"][0]["tls"]["server_name"],
        "new.example"
    );
    fixture.mutate("clients/laptop.json", |v| {
        v["outbounds"][0]["tls"]["enabled"] = json!(false)
    });
    assert!(
        fixture
            .set(PropertyKind::TlsServerName, "new.example")
            .is_err()
    );
}

#[test]
fn port_hopping_sets_arrays_removes_scalar_ports_and_is_hysteria_only() {
    let fixture = Fixture::new(true);
    let plan = fixture
        .set(PropertyKind::ServerPorts, "20000:30000,40000")
        .unwrap();
    assert_eq!(plan.edits.len(), 6);
    let generator = Generator::default();
    assert_eq!(
        apply::apply(&fixture.configs(), &plan, &generator).unwrap(),
        2
    );
    for file in ["clients/phone.json", "clients/laptop.json"] {
        for outbound in fixture.get(file)["outbounds"].as_array().unwrap() {
            assert!(outbound.get("server_port").is_none());
            assert_eq!(
                outbound["server_ports"],
                json!(["20000:30000", "40000:40000"])
            );
        }
    }
    assert!(fixture.set(PropertyKind::ServerPort, "443").is_err());
    assert_eq!(
        fixture.get("server.json")["inbounds"][0]["listen_port"],
        443
    );
    assert!(
        Fixture::new(false)
            .set(PropertyKind::ServerPorts, "10000:20000")
            .is_err()
    );
}

#[test]
fn noop_set_does_not_validate_or_write_and_bad_values_fail() {
    let fixture = Fixture::new(false);
    let configs = fixture.configs();
    let plan = fixture.set(PropertyKind::Server, "old.example").unwrap();
    assert!(plan.edits.is_empty());
    let generator = Generator::default();
    assert_eq!(apply::apply(&configs, &plan, &generator).unwrap(), 0);
    assert_eq!(generator.checks.get(), 0);
    for (path, doc) in &configs.documents {
        assert_eq!(fs::read(path).unwrap(), doc.original);
    }
    for value in ["", "bad name", "\nname"] {
        assert!(fixture.set(PropertyKind::Server, value).is_err());
    }
    for value in ["0", "65536", "bad"] {
        assert!(fixture.set(PropertyKind::ServerPort, value).is_err());
    }
}

#[test]
fn new_rotation_validation_failures_never_replace_originals() {
    for kind in [
        RotationKind::VlessRealityShortId,
        RotationKind::VlessRealityKeypair,
        RotationKind::Hysteria2ObfsPassword,
    ] {
        let fixture = Fixture::new(kind == RotationKind::Hysteria2ObfsPassword);
        let configs = fixture.configs();
        let generator = Generator::default();
        let plan = fixture.rotate(kind, &generator).unwrap();
        generator.fail_validation.set(true);
        assert!(apply::apply(&configs, &plan, &generator).is_err());
        for (path, doc) in &configs.documents {
            assert_eq!(fs::read(path).unwrap(), doc.original);
        }
        generator.fail_generation.set(true);
        assert!(fixture.rotate(kind, &generator).is_err());
    }
}

#[test]
fn generic_edits_distinguish_missing_and_null_and_reject_overlap() {
    let fixture = Fixture::new(false);
    fixture.mutate("clients/phone.json", |v| {
        v["outbounds"][0]["optional"] = Value::Null
    });
    let configs = fixture.configs();
    let reference = binding::ConfigRef {
        file: fixture.root.path().join("clients/phone.json"),
        pointer: "/outbounds/0/optional".into(),
    };
    let mut plan = RotationPlan::new(OperationKind::Set(PropertyKind::Server));
    plan.edits.push(Edit {
        target: reference.clone(),
        old: None,
        new: Some(json!("value")),
    });
    assert!(plan.materialize(&configs).is_err());
    plan.edits[0].old = Some(Value::Null);
    assert!(plan.materialize(&configs).is_ok());
    plan.edit(
        &configs,
        binding::ConfigRef {
            file: reference.file,
            pointer: "/outbounds/0".into(),
        },
        Some(json!({})),
    )
    .unwrap();
    assert!(
        plan.materialize(&configs)
            .err()
            .unwrap()
            .to_string()
            .contains("overlapping")
    );
}

#[test]
fn omitted_client_short_id_uses_the_empty_id_without_enabling_new_blocks() {
    let mut fixture = Fixture::new(false);
    fixture.mutate("server.json", |v| {
        v["inbounds"][0]["tls"]["reality"]["short_id"] = json!(["", "bbbbbbbbbbbbbbbb"])
    });
    fixture.mutate("clients/phone.json", |v| {
        v["outbounds"][0]["tls"]["reality"]
            .as_object_mut()
            .unwrap()
            .remove("short_id");
        v["outbounds"][1]["tls"]["reality"]["short_id"] = json!("");
    });
    fixture.input.client = vec![fixture.root.path().join("clients/phone.json")];
    fixture.input.client_tag = vec!["home".into()];
    let plan = fixture
        .rotate(RotationKind::VlessRealityShortId, &Generator::default())
        .unwrap();
    assert!(plan.edits.iter().any(|edit| edit.old.is_none()));
    assert_eq!(
        fixture.materialized(&plan, "server.json")["inbounds"][0]["tls"]["reality"]["short_id"][0],
        ""
    );
    assert_eq!(
        fixture.materialized(&plan, "clients/phone.json")["outbounds"][0]["tls"]["reality"]["short_id"],
        "0000000000000001"
    );
}

#[test]
fn service_operations_never_bind_through_keys_or_addresses_and_reject_ambiguity() {
    let fixture = Fixture::new(false);
    let mut unmatched = fixture.get("clients/laptop.json");
    unmatched["outbounds"][0]["uuid"] = json!("unknown-user");
    fixture.put("clients/unmatched.json", unmatched);
    let plan = fixture
        .rotate(RotationKind::VlessRealityKeypair, &Generator::default())
        .unwrap();
    assert_eq!(plan.edits.len(), 4);
    let plan = fixture.set(PropertyKind::Server, "new.example").unwrap();
    assert_eq!(plan.edits.len(), 3);
    fixture.mutate("server.json", |v| {
        let mut duplicate = v["inbounds"][0].clone();
        duplicate["tag"] = json!("other");
        v["inbounds"].as_array_mut().unwrap().push(duplicate);
    });
    let generator = Generator::default();
    for kind in [
        RotationKind::VlessRealityKeypair,
        RotationKind::VlessRealityShortId,
    ] {
        assert!(
            fixture
                .rotate(kind, &generator)
                .err()
                .unwrap()
                .to_string()
                .contains("ambiguous")
        );
    }
    assert_eq!(generator.calls.get(), 0);
    assert!(fixture.set(PropertyKind::Server, "new.example").is_err());
}

#[test]
fn property_validation_failure_leaves_all_files_unchanged() {
    let fixture = Fixture::new(true);
    let configs = fixture.configs();
    let plan = fixture
        .set(PropertyKind::ServerPorts, "10000:20000")
        .unwrap();
    let generator = Generator::default();
    generator.fail_validation.set(true);
    assert!(apply::apply(&configs, &plan, &generator).is_err());
    for (path, doc) in &configs.documents {
        assert_eq!(fs::read(path).unwrap(), doc.original);
    }
}

#[test]
fn generic_object_member_edits_honor_pointer_escapes() {
    let fixture = Fixture::new(false);
    let configs = fixture.configs();
    let reference = binding::ConfigRef {
        file: fixture.root.path().join("clients/phone.json"),
        pointer: "/outbounds/0/a~1b~0c".into(),
    };
    let mut plan = RotationPlan::new(OperationKind::Set(PropertyKind::Server));
    plan.edit(&configs, reference, Some(json!(42))).unwrap();
    let phone = fixture.materialized(&plan, "clients/phone.json");
    assert_eq!(phone["outbounds"][0]["a/b~c"], 42);
    plan.edits[0].target.pointer = "/outbounds/0/bad~2escape".into();
    assert!(plan.materialize(&configs).is_err());
    plan.edits[0].target.pointer = "/outbounds/0/absent/child".into();
    assert!(plan.materialize(&configs).is_err());
}

#[test]
fn inspect_shows_reality_counts_without_exposing_secrets() {
    let fixture = Fixture::new(false);
    let configs = fixture.configs();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let text = inventory.render(&configs, &fixture.input, None);
    assert!(text.contains("3 accepted / 2 observed; clients: 3"));
    assert!(!text.contains("old-private-key"));
    assert!(!text.contains("aaaaaaaaaaaaaaaa"));
}
