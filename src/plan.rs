use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use anyhow::{Result, ensure};
use serde_json::Value;

use crate::{
    binding::{ConfigRef, Inventory, client_selected},
    cli::{IdentityKind, Input, Protocol},
    config::ConfigSet,
    singbox::SingBox,
};

pub struct Edit {
    pub target: ConfigRef,
    pub old: Value,
    pub new: Value,
}

pub struct RotationPlan {
    pub operation: IdentityKind,
    pub edits: Vec<Edit>,
    pub contexts: Vec<String>,
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
        operation: kind,
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
                    old: configs.value(&target.file, &target.pointer)?.clone(),
                    new: Value::String(replacement.clone()),
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

impl RotationPlan {
    /// Protocol-independent edit application with optimistic old-value checks.
    pub fn materialize(&self, configs: &ConfigSet) -> Result<BTreeMap<PathBuf, Value>> {
        let mut changed = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for edit in &self.edits {
            ensure!(
                seen.insert((&edit.target.file, &edit.target.pointer)),
                "duplicate edit at {}#{}",
                edit.target.file.display(),
                edit.target.pointer
            );
            let source = configs.value(&edit.target.file, &edit.target.pointer)?;
            ensure!(
                source == &edit.old,
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
            let target = doc
                .pointer_mut(&edit.target.pointer)
                .ok_or_else(|| anyhow::anyhow!("invalid edit pointer {}", edit.target.pointer))?;
            *target = edit.new.clone();
        }
        Ok(changed)
    }

    pub fn render(&self) -> String {
        let mut lines = self.contexts.clone();
        for edit in &self.edits {
            let (old, new) = match self.operation.protocol() {
                Protocol::Vless => (edit.old.to_string(), edit.new.to_string()),
                Protocol::Hysteria2 => ("********".to_owned(), "********".to_owned()),
            };
            lines.push(format!(
                "  {}#{}: {old} -> {new}",
                edit.target.file.display(),
                edit.target.pointer
            ));
        }
        lines.join("\n")
    }
}
