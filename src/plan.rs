use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;

use crate::{
    binding::{ConfigRef, Inventory, client_selected},
    cli::{IdentityKind, Input, PropertyKind, Protocol, RotationKind, RotationMaterial},
    config::ConfigSet,
    protocol,
    singbox::SingBox,
};

pub struct Edit {
    pub target: ConfigRef,
    /// None means the object member does not exist (distinct from JSON null).
    pub old: Option<Value>,
    /// None removes an existing object member.
    pub new: Option<Value>,
}

#[derive(Clone, Copy, Debug)]
pub enum OperationKind {
    Rotate(RotationKind),
    RotateType(Protocol),
    Set(PropertyKind),
}

pub struct RotationPlan {
    pub operation: OperationKind,
    pub edits: Vec<Edit>,
    pub contexts: Vec<String>,
}

pub fn rotation(
    configs: &ConfigSet,
    inventory: &Inventory,
    input: &Input,
    kind: RotationKind,
    singbox: &impl SingBox,
) -> Result<RotationPlan> {
    if let Some(identity) = kind.identity() {
        return identities(configs, inventory, input, identity, singbox);
    }
    match kind {
        RotationKind::VlessRealityShortId => {
            protocol::vless::short_ids(configs, inventory, input, singbox)
        }
        RotationKind::VlessRealityKeypair => {
            protocol::vless::keypair(configs, inventory, input, singbox)
        }
        RotationKind::Hysteria2ObfsPassword => {
            protocol::hysteria2::obfs_password(configs, inventory, input, singbox)
        }
        _ => unreachable!("identity rotations handled above"),
    }
}

/// Compose type-wide rotations against one original inventory. No intermediate
/// config is written or rediscovered; the caller validates/applies the union once.
pub fn rotation_by_type(
    configs: &ConfigSet,
    inventory: &Inventory,
    input: &Input,
    protocol: Protocol,
    only: Option<RotationMaterial>,
    singbox: &impl SingBox,
) -> Result<RotationPlan> {
    let kind = only
        .map(|material| {
            material.kind(protocol).with_context(|| {
                format!(
                    "--only {material:?} is not supported for --type {}",
                    protocol.name()
                )
            })
        })
        .transpose()?;
    let client_selectable = kind
        .is_some_and(|kind| kind.identity().is_some() || kind == RotationKind::VlessRealityShortId);
    ensure!(
        client_selectable || (input.client.is_empty() && input.client_tag.is_empty()),
        "all-material and service-wide rotations reject --client and --outbound-tag; supply all client configs with --clients and narrow with --inbound-tag, or use --only uuid/password/reality-short-id"
    );
    inventory.ensure_unambiguous(protocol)?;
    if let Some(identity) = kind.and_then(RotationKind::identity) {
        return identities(configs, inventory, input, identity, singbox);
    }
    let mut combined = RotationPlan::new(OperationKind::RotateType(protocol));
    if only.is_none() {
        let identity = match protocol {
            Protocol::Vless => IdentityKind::VlessUuid,
            Protocol::Hysteria2 => IdentityKind::Hysteria2Password,
        };
        combined.append(identities(configs, inventory, input, identity, singbox)?);
    }
    let wants = |candidate| kind.is_none_or(|kind| kind == candidate);
    // A shared secret is generated once per inbound, not once per outbound.
    let mut generated_shared = BTreeSet::new();
    for service in &inventory.services {
        if service.protocol != protocol
            || !service
                .clients()
                .any(|client| client_selected(client, configs, input))
        {
            continue;
        }
        let mut parts = Vec::new();
        match protocol {
            Protocol::Vless => {
                if !protocol::vless::reality_enabled(configs, &service.inbound)
                    || !service.clients().any(|client| {
                        protocol::vless::reality_enabled(configs, client)
                            && client_selected(client, configs, input)
                    })
                {
                    continue;
                }
                if wants(RotationKind::VlessRealityKeypair) {
                    parts.push(protocol::vless::keypair_for_service(
                        configs, service, singbox,
                    )?);
                }
                if wants(RotationKind::VlessRealityShortId) {
                    parts.push(protocol::vless::short_ids_for_service(
                        configs, service, input, singbox,
                    )?);
                }
            }
            Protocol::Hysteria2 => {
                let inbound = protocol::endpoint_value(configs, &service.inbound)?;
                if wants(RotationKind::Hysteria2ObfsPassword)
                    && inbound.get("obfs").is_some_and(|value| !value.is_null())
                {
                    parts.push(protocol::hysteria2::obfs_password_for_service(
                        configs, service, singbox,
                    )?);
                }
            }
        }
        for part in parts {
            for edit in &part.edits {
                // Client copies intentionally share the inbound's secret.
                if configs.server_files.contains(&edit.target.file)
                    && (edit.target.pointer.ends_with("/private_key")
                        || edit.target.pointer.ends_with("/obfs/password"))
                {
                    let value = edit
                        .new
                        .as_ref()
                        .and_then(Value::as_str)
                        .context("generated shared secret must be a string")?;
                    ensure!(
                        generated_shared.insert(value.to_owned()),
                        "generated shared secret collides with another inbound's replacement; retry"
                    );
                }
            }
            combined.append(part);
        }
    }
    ensure!(
        !combined.edits.is_empty(),
        "no matching configured material with bound outbounds; use inspect to review bindings"
    );
    Ok(combined)
}

pub fn identities(
    configs: &ConfigSet,
    inventory: &Inventory,
    input: &Input,
    kind: IdentityKind,
    singbox: &impl SingBox,
) -> Result<RotationPlan> {
    let protocol = kind.protocol();
    inventory.ensure_unambiguous(protocol)?;
    let mut plan = RotationPlan {
        operation: OperationKind::Rotate(kind.into()),
        edits: Vec::new(),
        contexts: Vec::new(),
    };
    // Refuse collisions with any supplied server/client identity, including other inbounds.
    let mut used = BTreeSet::new();
    for (file, doc) in &configs.documents {
        if configs.server_files.contains(file) {
            if let Some(inbounds) = doc.value.get("inbounds").and_then(Value::as_array) {
                for inbound in inbounds {
                    if inbound.get("type").and_then(Value::as_str) != Some(protocol.name()) {
                        continue;
                    }
                    if let Some(users) = inbound.get("users").and_then(Value::as_array) {
                        for user in users {
                            if let Some(value) =
                                user.get(protocol.credential()).and_then(Value::as_str)
                            {
                                used.insert(value.to_owned());
                            }
                        }
                    }
                }
            }
        } else if let Some(outbounds) = doc.value.get("outbounds").and_then(Value::as_array) {
            for outbound in outbounds {
                if outbound.get("type").and_then(Value::as_str) == Some(protocol.name())
                    && let Some(value) = outbound.get(protocol.credential()).and_then(Value::as_str)
                {
                    used.insert(value.to_owned());
                }
            }
        }
    }
    for service in &inventory.services {
        if service.protocol != protocol {
            continue;
        }
        for identity in &service.identities {
            if !identity
                .clients
                .iter()
                .any(|client| client_selected(client, configs, input))
            {
                continue;
            }
            let replacement = match kind {
                IdentityKind::VlessUuid => singbox.generate_uuid()?,
                IdentityKind::Hysteria2Password => singbox.generate_random_base64(32)?,
            };
            ensure!(
                !replacement.is_empty(),
                "generator returned an empty identity"
            );
            ensure!(
                used.insert(replacement.clone()),
                "generated identity collides with an existing or newly generated identity; retry"
            );
            plan.contexts.push(format!(
                "{} inbound={} users=[{}], {} client occurrence(s)",
                protocol.name(),
                service.inbound.label(),
                identity.user_names.join(", "),
                identity.clients.len()
            ));
            for target in identity.server_refs.iter().cloned().chain(
                identity
                    .clients
                    .iter()
                    .map(|client| client.field(protocol.credential())),
            ) {
                plan.edits.push(Edit {
                    old: Some(configs.value(&target.file, &target.pointer)?.clone()),
                    new: Some(Value::String(replacement.clone())),
                    target,
                });
            }
        }
    }
    ensure!(
        !plan.edits.is_empty(),
        "no matched identities selected; use inspect to review bindings"
    );
    Ok(plan)
}

fn pointer_key(token: &str) -> Result<String> {
    let mut result = String::new();
    let mut chars = token.chars();
    while let Some(character) = chars.next() {
        result.push(match character {
            '~' => match chars.next() {
                Some('0') => '~',
                Some('1') => '/',
                _ => bail!("invalid JSON pointer escape"),
            },
            character => character,
        });
    }
    Ok(result)
}

impl RotationPlan {
    fn append(&mut self, other: Self) {
        self.contexts.extend(other.contexts);
        self.edits.extend(other.edits);
    }

    pub fn new(operation: OperationKind) -> Self {
        Self {
            operation,
            edits: Vec::new(),
            contexts: Vec::new(),
        }
    }

    pub fn edit(
        &mut self,
        configs: &ConfigSet,
        target: ConfigRef,
        new: Option<Value>,
    ) -> Result<()> {
        let source = configs
            .documents
            .get(&target.file)
            .context("edit file not loaded")?;
        let old = source.value.pointer(&target.pointer).cloned();
        if old != new {
            self.edits.push(Edit { target, old, new });
        }
        Ok(())
    }

    /// Protocol-independent edit application with optimistic old-value checks.
    pub fn materialize(&self, configs: &ConfigSet) -> Result<BTreeMap<PathBuf, Value>> {
        let mut changed = BTreeMap::new();
        let mut seen: BTreeSet<(&PathBuf, &String)> = BTreeSet::new();
        for edit in &self.edits {
            ensure!(
                edit.target.pointer.starts_with('/'),
                "edit must use an absolute JSON pointer"
            );
            ensure!(
                !seen.iter().any(|(file, pointer)| *file == &edit.target.file
                    && (pointer.starts_with(&format!("{}/", edit.target.pointer))
                        || edit.target.pointer.starts_with(&format!("{pointer}/")))),
                "overlapping edits at {}#{}",
                edit.target.file.display(),
                edit.target.pointer
            );
            ensure!(
                seen.insert((&edit.target.file, &edit.target.pointer)),
                "duplicate edit at {}#{}",
                edit.target.file.display(),
                edit.target.pointer
            );
            let source = configs
                .documents
                .get(&edit.target.file)
                .context("edit file not loaded")?
                .value
                .pointer(&edit.target.pointer);
            ensure!(
                source == edit.old.as_ref(),
                "stale edit at {}#{}",
                edit.target.file.display(),
                edit.target.pointer
            );
            if edit.old == edit.new {
                continue;
            }
            let doc = changed
                .entry(edit.target.file.clone())
                .or_insert_with(|| configs.documents[&edit.target.file].value.clone());
            let (parent, key) = edit
                .target
                .pointer
                .rsplit_once('/')
                .context("edit must target an object member")?;
            let key = pointer_key(key)?;
            let object = doc
                .pointer_mut(parent)
                .and_then(Value::as_object_mut)
                .with_context(|| {
                    format!(
                        "edit parent must be an existing object: {}#{parent}",
                        edit.target.file.display()
                    )
                })?;
            match &edit.new {
                Some(value) => {
                    object.insert(key, value.clone());
                }
                None => {
                    object.remove(&key);
                }
            }
        }
        Ok(changed)
    }

    pub fn render(&self) -> String {
        let mut lines = self.contexts.clone();
        for edit in &self.edits {
            let secret = matches!(
                edit.target.pointer.rsplit('/').next(),
                Some("password" | "private_key" | "short_id")
            );
            let display = |value: &Option<Value>| match value {
                None => "<absent>".to_owned(),
                Some(_) if secret => "********".to_owned(),
                Some(value) => value.to_string(),
            };
            let (old, new) = (display(&edit.old), display(&edit.new));
            lines.push(format!(
                "  {}#{}: {old} -> {new}",
                edit.target.file.display(),
                edit.target.pointer
            ));
        }
        lines.join("\n")
    }
}
