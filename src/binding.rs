use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::{
    cli::{Input, Protocol},
    config::ConfigSet,
};

#[derive(Clone, Debug)]
pub struct ConfigRef {
    pub file: PathBuf,
    pub pointer: String,
}

#[derive(Clone, Debug)]
pub struct EndpointRef {
    pub config: ConfigRef,
    pub tag: Option<String>,
}

impl EndpointRef {
    pub fn label(&self) -> String {
        format!(
            "{}#{} tag={}",
            self.config.file.display(),
            self.config.pointer,
            self.tag.as_deref().unwrap_or("<untagged>")
        )
    }

    pub fn field(&self, field: &str) -> ConfigRef {
        ConfigRef {
            file: self.config.file.clone(),
            pointer: format!("{}/{field}", self.config.pointer),
        }
    }
}

pub struct IdentityBinding {
    pub value: String,
    pub server_refs: Vec<ConfigRef>,
    pub user_names: Vec<String>,
    pub clients: Vec<EndpointRef>,
}

pub struct ServiceBinding {
    pub protocol: Protocol,
    pub inbound: EndpointRef,
    pub identities: Vec<IdentityBinding>,
}

pub struct UnboundClient {
    pub protocol: Protocol,
    pub outbound: EndpointRef,
    /// Empty for unmatched clients; more than one for ambiguous clients.
    pub candidates: Vec<EndpointRef>,
}

pub struct Inventory {
    pub services: Vec<ServiceBinding>,
    pub unbound: Vec<UnboundClient>,
}

pub fn client_selected(client: &EndpointRef, configs: &ConfigSet, input: &Input) -> bool {
    (configs.selected_clients.is_empty() || configs.selected_clients.contains(&client.config.file))
        && tag_selected(&client.tag, &input.client_tag)
}

fn tag_selected(tag: &Option<String>, selected: &[String]) -> bool {
    selected.is_empty() || tag.as_ref().is_some_and(|tag| selected.contains(tag))
}

fn endpoint(file: &std::path::Path, pointer: String, value: &Value) -> EndpointRef {
    EndpointRef {
        config: ConfigRef {
            file: file.to_owned(),
            pointer,
        },
        tag: value.get("tag").and_then(Value::as_str).map(str::to_owned),
    }
}

fn array<'a>(value: &'a Value, field: &str, location: &str) -> Result<&'a [Value]> {
    match value.get(field) {
        None => Ok(&[]),
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .with_context(|| format!("{location}: {field} must be an array")),
    }
}

fn credential<'a>(value: &'a Value, protocol: Protocol, location: &str) -> Result<&'a str> {
    value
        .get(protocol.credential())
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{location}: missing or invalid {}", protocol.credential()))
}

pub fn discover(configs: &ConfigSet, input: &Input) -> Result<Inventory> {
    let mut inventory = Inventory {
        services: Vec::new(),
        unbound: Vec::new(),
    };
    for file in &configs.server_files {
        let doc = &configs.documents[file].value;
        for (index, inbound) in array(doc, "inbounds", &file.display().to_string())?
            .iter()
            .enumerate()
        {
            let Some(protocol) = inbound
                .get("type")
                .and_then(Value::as_str)
                .and_then(Protocol::parse)
            else {
                continue;
            };
            let endpoint = endpoint(file, format!("/inbounds/{index}"), inbound);
            if !tag_selected(&endpoint.tag, &input.inbound_tag) {
                continue;
            }
            let mut service = ServiceBinding {
                protocol,
                inbound: endpoint,
                identities: Vec::new(),
            };
            for (user_index, user) in array(inbound, "users", &service.inbound.label())?
                .iter()
                .enumerate()
            {
                let reference = service
                    .inbound
                    .field(&format!("users/{user_index}/{}", protocol.credential()));
                let location = format!("{}#{}", reference.file.display(), reference.pointer);
                let value = credential(user, protocol, &location)?;
                let id = match service
                    .identities
                    .iter()
                    .position(|identity| identity.value == value)
                {
                    Some(id) => id,
                    None => {
                        service.identities.push(IdentityBinding {
                            value: value.to_owned(),
                            server_refs: Vec::new(),
                            user_names: Vec::new(),
                            clients: Vec::new(),
                        });
                        service.identities.len() - 1
                    }
                };
                let identity = &mut service.identities[id];
                identity.server_refs.push(reference);
                if let Some(name) = user.get("name").and_then(Value::as_str) {
                    identity.user_names.push(name.to_owned());
                }
            }
            inventory.services.push(service);
        }
    }
    for file in &configs.client_files {
        let doc = &configs.documents[file].value;
        for (index, outbound) in array(doc, "outbounds", &file.display().to_string())?
            .iter()
            .enumerate()
        {
            let Some(protocol) = outbound
                .get("type")
                .and_then(Value::as_str)
                .and_then(Protocol::parse)
            else {
                continue;
            };
            let endpoint = endpoint(file, format!("/outbounds/{index}"), outbound);
            let value = credential(outbound, protocol, &endpoint.label())?;
            let mut matches = Vec::new();
            for (service_id, service) in inventory.services.iter().enumerate() {
                if service.protocol != protocol {
                    continue;
                }
                for (identity_id, identity) in service.identities.iter().enumerate() {
                    if identity.value == value {
                        matches.push((service_id, identity_id));
                    }
                }
            }
            if let [(service_id, identity_id)] = matches.as_slice() {
                inventory.services[*service_id].identities[*identity_id]
                    .clients
                    .push(endpoint);
            } else {
                inventory.unbound.push(UnboundClient {
                    protocol,
                    outbound: endpoint,
                    candidates: matches
                        .iter()
                        .map(|(id, _)| inventory.services[*id].inbound.clone())
                        .collect(),
                });
            }
        }
    }
    Ok(inventory)
}

impl Inventory {
    pub fn ensure_unambiguous(&self, protocol: Protocol) -> Result<()> {
        for client in &self.unbound {
            if client.protocol == protocol && !client.candidates.is_empty() {
                let candidates: Vec<_> = client.candidates.iter().map(EndpointRef::label).collect();
                bail!(
                    "ambiguous client {}; matches {}; narrow with --inbound-tag",
                    client.outbound.label(),
                    candidates.join(", ")
                );
            }
        }
        Ok(())
    }

    pub fn render(&self, configs: &ConfigSet, input: &Input, protocol: Option<Protocol>) -> String {
        let mut lines = Vec::new();
        for service in &self.services {
            if protocol.is_some_and(|p| p != service.protocol) {
                continue;
            }
            lines.push(format!(
                "{} inbound={}",
                service.protocol.name(),
                service.inbound.label()
            ));
            for identity in &service.identities {
                let selected: Vec<_> = identity
                    .clients
                    .iter()
                    .filter(|c| client_selected(c, configs, input))
                    .collect();
                if (!input.client.is_empty() || !input.client_tag.is_empty()) && selected.is_empty()
                {
                    continue;
                }
                let value = match service.protocol {
                    Protocol::Vless => &identity.value,
                    Protocol::Hysteria2 => "********",
                };
                lines.push(format!(
                    "  identity {}={} users=[{}]{}",
                    service.protocol.credential(),
                    value,
                    identity.user_names.join(", "),
                    if identity.clients.is_empty() {
                        " (unmatched server identity)"
                    } else {
                        ""
                    }
                ));
                for client in selected {
                    lines.push(format!("    {}", client.label()));
                }
            }
        }
        for client in &self.unbound {
            if protocol.is_some_and(|p| p != client.protocol)
                || !client_selected(&client.outbound, configs, input)
            {
                continue;
            }
            let status = if client.candidates.is_empty() {
                "unmatched"
            } else {
                "ambiguous"
            };
            lines.push(format!(
                "{status} {} client={}",
                client.protocol.name(),
                client.outbound.label()
            ));
            for candidate in &client.candidates {
                lines.push(format!("  candidate {}", candidate.label()));
            }
        }
        if lines.is_empty() {
            lines.push("No supported bindings found.".to_owned());
        }
        lines.join("\n")
    }
}
