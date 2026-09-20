use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use sb_rotate::{
    apply, binding,
    cli::{IdentityKind, Input, Protocol},
    config::ConfigSet,
    plan,
    singbox::{SingBox, require_supported},
};
use semver::Version;
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Default)]
struct FakeSingBox {
    generated: Cell<usize>,
    checked: RefCell<Vec<(bool, Vec<Value>)>>,
    fail_check: Cell<bool>,
    fail_check_number: Cell<usize>,
    fail_generate: Cell<bool>,
    change_during_check: RefCell<Option<PathBuf>>,
}

impl SingBox for FakeSingBox {
    fn version(&self) -> Result<Version> {
        Ok(Version::new(1, 14, 0))
    }
    fn generate_uuid(&self) -> Result<String> {
        if self.fail_generate.get() {
            bail!("generation failed");
        }
        let count = self.generated.get() + 1;
        self.generated.set(count);
        Ok(format!("aaaaaaaa-aaaa-4aaa-8aaa-{count:012}"))
    }
    fn generate_random_base64(&self, bytes: usize) -> Result<String> {
        assert_eq!(bytes, 32);
        Ok(format!("secret-{}", self.generate_uuid()?))
    }
    fn check_file(&self, path: &Path) -> Result<()> {
        let value = serde_json::from_slice(&fs::read(path)?)?;
        self.checked.borrow_mut().push((false, vec![value]));
        self.after_check()
    }
    fn check_directory(&self, path: &Path) -> Result<()> {
        let mut files: Vec<_> = fs::read_dir(path)?
            .map(|entry| entry.unwrap().path())
            .collect();
        files.sort();
        let values = files
            .into_iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .map(|file| Ok(serde_json::from_slice(&fs::read(file)?)?))
            .collect::<Result<Vec<_>>>()?;
        self.checked.borrow_mut().push((true, values));
        self.after_check()
    }
}

impl FakeSingBox {
    fn after_check(&self) -> Result<()> {
        if let Some(path) = self.change_during_check.borrow_mut().take() {
            fs::write(path, b"{\"external\":true}\n")?;
        }
        if self.fail_check.get() || self.fail_check_number.get() == self.checked.borrow().len() {
            bail!("validation failed");
        }
        Ok(())
    }
}

struct Fixture {
    root: TempDir,
    input: Input,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("clients")).unwrap();
        let input = Input {
            server: root.path().join("server.json"),
            clients: Some(root.path().join("clients")),
            ..Input::default()
        };
        let fixture = Self { root, input };
        fixture.put("server.json", json!({"inbounds": [inbound("vless", "home", &["shared", "other", "unmatched"])], "unrelated": {"enabled": true}}));
        fixture.put("clients/phone.json", json!({"outbounds": [outbound("vless", "home", "shared"), outbound("vless", "backup", "shared")]}));
        fixture.put("clients/laptop.json", json!({"outbounds": [outbound("vless", "home", "shared"), outbound("vless", "other", "other")]}));
        fixture
    }
    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }
    fn put(&self, relative: &str, value: Value) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn load(&self) -> ConfigSet {
        ConfigSet::load(&self.input).unwrap()
    }
    fn read(&self, relative: &str) -> Value {
        serde_json::from_slice(&fs::read(self.path(relative)).unwrap()).unwrap()
    }
}

fn inbound(protocol: &str, tag: &str, credentials: &[&str]) -> Value {
    let field = if protocol == "vless" {
        "uuid"
    } else {
        "password"
    };
    let users: Vec<_> = credentials
        .iter()
        .map(|value| json!({"name": "user", field: value}))
        .collect();
    json!({"type": protocol, "tag": tag, "users": users})
}

fn outbound(protocol: &str, tag: &str, value: &str) -> Value {
    let field = if protocol == "vless" {
        "uuid"
    } else {
        "password"
    };
    json!({"type": protocol, "tag": tag, field: value, "server": "irrelevant.example", "server_port": 443})
}

#[test]
fn discovery_groups_all_outbounds_by_identity_not_address_or_tag() {
    let fixture = Fixture::new();
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    assert_eq!(inventory.services.len(), 1);
    let identities = &inventory.services[0].identities;
    assert_eq!(identities.len(), 3);
    assert_eq!(identities[0].clients.len(), 3);
    assert_eq!(identities[1].clients.len(), 1);
    assert!(identities[2].clients.is_empty());
    assert!(
        inventory
            .render(&configs, &fixture.input, None)
            .contains("unmatched server identity")
    );
}

#[test]
fn identity_selection_expands_to_all_shared_clients_and_duplicate_server_users() {
    let mut fixture = Fixture::new();
    fixture.put(
        "server.json",
        json!({"inbounds": [inbound("vless", "home", &["shared", "shared", "other"])]}),
    );
    fixture.input.client = vec![fixture.path("clients/phone.json")];
    fixture.input.client_tag = vec!["backup".to_owned()];
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let sb = FakeSingBox::default();
    let plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &sb,
    )
    .unwrap();
    assert_eq!(sb.generated.get(), 1);
    assert_eq!(plan.edits.len(), 5); // two server users + three client occurrences
    assert!(plan.edits.iter().all(|edit| edit.new == plan.edits[0].new));
    let changed = plan.materialize(&configs).unwrap();
    assert_eq!(
        changed[&fixture.path("clients/laptop.json")]["outbounds"][1]["uuid"],
        "other"
    );
    for (path, doc) in &configs.documents {
        assert_eq!(fs::read(path).unwrap(), doc.original);
    }
}

#[test]
fn selectors_or_within_categories_and_across_categories() {
    let mut fixture = Fixture::new();
    fixture.input.client = vec![
        fixture.path("clients/phone.json"),
        fixture.path("clients/laptop.json"),
    ];
    fixture.input.client_tag = vec!["other".to_owned(), "nonexistent".to_owned()];
    fixture.input.inbound_tag = vec!["home".to_owned(), "nonexistent".to_owned()];
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &FakeSingBox::default(),
    )
    .unwrap();
    assert_eq!(plan.edits.len(), 2);
    assert!(plan.edits.iter().all(|edit| edit.old == "other"));
}

#[test]
fn duplicate_identity_across_inbounds_is_ambiguous_until_server_selected() {
    let mut fixture = Fixture::new();
    fixture.put("server.json", json!({"inbounds": [inbound("vless", "home", &["shared"]), inbound("vless", "office", &["shared"])]}));
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    assert_eq!(
        inventory
            .unbound
            .iter()
            .filter(|client| client.candidates.len() == 2)
            .count(),
        3
    );
    assert!(
        inventory
            .render(&configs, &fixture.input, None)
            .contains("ambiguous")
    );
    let sb = FakeSingBox::default();
    assert!(
        plan::identities(
            &configs,
            &inventory,
            &fixture.input,
            IdentityKind::VlessUuid,
            &sb
        )
        .is_err()
    );
    assert_eq!(sb.generated.get(), 0);
    fixture.input.inbound_tag = vec!["home".to_owned()];
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &sb,
    )
    .unwrap();
    assert_eq!(plan.edits.len(), 4);
    assert!(
        plan.edits
            .iter()
            .all(|edit| !edit.target.pointer.starts_with("/inbounds/1"))
    );
}

#[test]
fn unmatched_clients_are_reported_and_never_rotated() {
    let fixture = Fixture::new();
    fixture.put(
        "clients/unknown.json",
        json!({"outbounds": [outbound("vless", "home", "unknown")]}),
    );
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    assert_eq!(inventory.unbound.len(), 1);
    assert!(
        inventory
            .render(&configs, &fixture.input, None)
            .contains("unmatched vless")
    );
    let plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &FakeSingBox::default(),
    )
    .unwrap();
    assert_eq!(plan.edits.len(), 6);
    assert!(
        plan.edits
            .iter()
            .all(|edit| edit.target.file != fixture.path("clients/unknown.json"))
    );
}

#[test]
fn hysteria_passwords_are_grouped_rotated_and_masked() {
    let mut fixture = Fixture::new();
    fixture.put(
        "server.json",
        json!({"inbounds": [inbound("hysteria2", "hy2", &["very-secret", "very-secret"])]}),
    );
    fixture.put("clients/phone.json", json!({"outbounds": [outbound("hysteria2", "home", "very-secret"), outbound("hysteria2", "backup", "very-secret")]}));
    fixture.input.client = vec![fixture.path("clients/phone.json")];
    fixture.input.client_tag = vec!["home".to_owned()];
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let rendered = inventory.render(&configs, &fixture.input, Some(Protocol::Hysteria2));
    assert!(!rendered.contains("very-secret"));
    assert!(rendered.contains("********"));
    let plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::Hysteria2Password,
        &FakeSingBox::default(),
    )
    .unwrap();
    assert_eq!(plan.edits.len(), 4);
    assert!(!plan.render().contains("very-secret"));
    assert!(!plan.render().contains("secret-"));
    assert_eq!(
        apply::apply(&configs, &plan, &FakeSingBox::default()).unwrap(),
        2
    );
    let server = fixture.read("server.json");
    let phone = fixture.read("clients/phone.json");
    assert_eq!(
        server["inbounds"][0]["users"][0]["password"],
        phone["outbounds"][1]["password"]
    );
}

#[test]
fn validation_failure_leaves_every_original_byte_untouched() {
    let fixture = Fixture::new();
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let sb = FakeSingBox::default();
    let plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &sb,
    )
    .unwrap();
    sb.fail_check.set(true);
    assert!(apply::apply(&configs, &plan, &sb).is_err());
    for (path, doc) in &configs.documents {
        assert_eq!(fs::read(path).unwrap(), doc.original);
    }
    assert_eq!(fs::read_dir(fixture.path("clients")).unwrap().count(), 2);
    assert_eq!(fs::read_dir(fixture.root.path()).unwrap().count(), 2);
}

#[test]
fn successful_rotation_validates_and_preserves_unrelated_fields() {
    let fixture = Fixture::new();
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let sb = FakeSingBox::default();
    let plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &sb,
    )
    .unwrap();
    assert_eq!(apply::apply(&configs, &plan, &sb).unwrap(), 3);
    assert_eq!(sb.checked.borrow().len(), 3);
    let server = fixture.read("server.json");
    assert_eq!(server["unrelated"], json!({"enabled": true}));
    assert_eq!(server["inbounds"][0]["users"][2]["uuid"], "unmatched");
    let uuid = &server["inbounds"][0]["users"][0]["uuid"];
    assert_eq!(
        &fixture.read("clients/phone.json")["outbounds"][0]["uuid"],
        uuid
    );
    assert_eq!(
        &fixture.read("clients/laptop.json")["outbounds"][0]["uuid"],
        uuid
    );
}

#[test]
fn directory_server_is_validated_as_a_complete_set() {
    let mut fixture = Fixture::new();
    let server = fixture.read("server.json");
    fixture.put("server.d/10-inbounds.json", server);
    fixture.put("server.d/00-log.json", json!({"log": {"level": "warn"}}));
    fixture.input.server = fixture.path("server.d");
    let unchanged = fs::read(fixture.path("server.d/00-log.json")).unwrap();
    let configs = fixture.load();
    let sb = FakeSingBox::default();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &sb,
    )
    .unwrap();
    apply::apply(&configs, &plan, &sb).unwrap();
    let checks = sb.checked.borrow();
    assert!(checks[0].0);
    assert_eq!(checks[0].1.len(), 2);
    assert_eq!(checks[0].1[0], json!({"log": {"level": "warn"}}));
    assert_eq!(
        fs::read(fixture.path("server.d/00-log.json")).unwrap(),
        unchanged
    );
    assert_eq!(fs::read_dir(fixture.path("server.d")).unwrap().count(), 2);
}

#[test]
fn check_validates_all_clients_independently() {
    let fixture = Fixture::new();
    let configs = fixture.load();
    let sb = FakeSingBox::default();
    require_supported(&sb).unwrap();
    apply::check(&configs, &sb).unwrap();
    assert_eq!(sb.checked.borrow().len(), 3);
    assert!(
        sb.checked
            .borrow()
            .iter()
            .all(|(directory, docs)| !directory && docs.len() == 1)
    );
}

#[test]
fn stale_source_is_not_overwritten_after_validation() {
    let fixture = Fixture::new();
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let sb = FakeSingBox::default();
    let plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &sb,
    )
    .unwrap();
    sb.change_during_check
        .replace(Some(fixture.path("server.json")));
    let error = apply::apply(&configs, &plan, &sb).unwrap_err();
    assert!(error.to_string().contains("config changed"));
    assert_eq!(fixture.read("server.json"), json!({"external": true}));
    for path in &configs.client_files {
        assert_eq!(fs::read(path).unwrap(), configs.documents[path].original);
    }
}

#[test]
fn generator_failure_and_empty_selection_do_not_write() {
    let mut fixture = Fixture::new();
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let sb = FakeSingBox::default();
    sb.fail_generate.set(true);
    assert!(
        plan::identities(
            &configs,
            &inventory,
            &fixture.input,
            IdentityKind::VlessUuid,
            &sb
        )
        .is_err()
    );
    fixture.input.client_tag = vec!["missing".to_owned()];
    let error = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &sb,
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("no matched identities"));
    for (path, doc) in &configs.documents {
        assert_eq!(fs::read(path).unwrap(), doc.original);
    }
}

#[test]
fn loading_deduplicates_explicit_clients_and_ignores_nested_or_non_json_files() {
    let mut fixture = Fixture::new();
    fixture.input.client = vec![
        fixture.path("clients/phone.json"),
        fixture.path("clients/./phone.json"),
    ];
    fixture.put("clients/nested/ignored.json", json!({}));
    fs::write(fixture.path("clients/ignored.txt"), b"not json").unwrap();
    let configs = fixture.load();
    assert_eq!(configs.client_files.len(), 2);
    assert_eq!(configs.selected_clients.len(), 1);
}

#[test]
fn explicit_client_without_directory_is_supported() {
    let mut fixture = Fixture::new();
    fixture.input.clients = None;
    fixture.input.client = vec![fixture.path("clients/phone.json")];
    assert_eq!(fixture.load().client_files.len(), 1);
}

#[test]
fn malformed_json_missing_credentials_and_overlapping_roles_fail() {
    let mut fixture = Fixture::new();
    fixture.put(
        "clients/phone.json",
        json!({"outbounds": [{"type": "vless"}]}),
    );
    let error = binding::discover(&fixture.load(), &fixture.input)
        .err()
        .unwrap();
    assert!(error.to_string().contains("missing or invalid uuid"));
    fs::write(fixture.path("clients/phone.json"), b"{").unwrap();
    assert!(ConfigSet::load(&fixture.input).is_err());
    fixture.input.client = vec![fixture.path("server.json")];
    assert!(
        ConfigSet::load(&fixture.input)
            .err()
            .unwrap()
            .to_string()
            .contains("both a server")
    );
}

#[cfg(unix)]
#[test]
fn replacement_preserves_private_config_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fs::set_permissions(
        fixture.path("server.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let sb = FakeSingBox::default();
    let plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &sb,
    )
    .unwrap();
    apply::apply(&configs, &plan, &sb).unwrap();
    assert_eq!(
        fs::metadata(fixture.path("server.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn last_client_validation_failure_still_preserves_all_originals() {
    let fixture = Fixture::new();
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let sb = FakeSingBox::default();
    let plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &sb,
    )
    .unwrap();
    sb.fail_check_number.set(3);
    assert!(apply::apply(&configs, &plan, &sb).is_err());
    assert_eq!(sb.checked.borrow().len(), 3);
    for (path, doc) in &configs.documents {
        assert_eq!(fs::read(path).unwrap(), doc.original);
    }
}

#[test]
fn generated_collision_with_unselected_server_identity_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.put(
        "server.json",
        json!({"inbounds": [
            inbound("vless", "home", &["shared"]),
            inbound("vless", "office", &["aaaaaaaa-aaaa-4aaa-8aaa-000000000001"])
        ]}),
    );
    fixture.input.inbound_tag = vec!["home".to_owned()];
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let error = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &FakeSingBox::default(),
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("collides"));
}

#[test]
fn unmodified_clients_are_not_rewritten_or_validated_during_rotation() {
    let fixture = Fixture::new();
    fixture.put(
        "clients/unmatched.json",
        json!({"outbounds": [outbound("vless", "unknown", "unknown")]}),
    );
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let sb = FakeSingBox::default();
    let plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &sb,
    )
    .unwrap();
    apply::apply(&configs, &plan, &sb).unwrap();
    assert_eq!(sb.checked.borrow().len(), 3);
    let path = fixture.path("clients/unmatched.json");
    assert_eq!(fs::read(&path).unwrap(), configs.documents[&path].original);
}

#[test]
fn generic_materialization_rejects_stale_or_duplicate_edits() {
    let fixture = Fixture::new();
    let configs = fixture.load();
    let inventory = binding::discover(&configs, &fixture.input).unwrap();
    let sb = FakeSingBox::default();
    let mut plan = plan::identities(
        &configs,
        &inventory,
        &fixture.input,
        IdentityKind::VlessUuid,
        &sb,
    )
    .unwrap();
    let old = plan.edits[0].old.clone();
    plan.edits[0].old = json!("stale");
    assert!(plan.materialize(&configs).is_err());
    plan.edits[0].old = old;
    plan.edits.push(plan::Edit {
        target: plan.edits[0].target.clone(),
        old: plan.edits[0].old.clone(),
        new: plan.edits[0].new.clone(),
    });
    assert!(plan.materialize(&configs).is_err());
}
