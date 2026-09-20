use std::collections::BTreeSet;

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::{
    binding::{EndpointRef, Inventory, client_selected},
    cli::{Input, Protocol, RotationKind},
    config::ConfigSet,
    plan::{OperationKind, RotationPlan},
    singbox::SingBox,
};

use super::{endpoint_value, one_service, service_plan, string_field};

pub(crate) fn reality_enabled(configs: &ConfigSet, endpoint: &EndpointRef) -> bool {
    endpoint_value(configs, endpoint).is_ok_and(|value| {
        value.pointer("/tls/enabled").and_then(Value::as_bool) == Some(true)
            && value
                .pointer("/tls/reality/enabled")
                .and_then(Value::as_bool)
                == Some(true)
    })
}

pub fn keypair(
    configs: &ConfigSet,
    inventory: &Inventory,
    input: &Input,
    singbox: &impl SingBox,
) -> Result<RotationPlan> {
    let service = one_service(
        configs,
        inventory,
        input,
        Some(Protocol::Vless),
        false,
        |service| {
            reality_enabled(configs, &service.inbound)
                && service
                    .clients()
                    .any(|client| reality_enabled(configs, client))
        },
    )?;
    let old_private = string_field(configs, &service.inbound, "tls/reality/private_key")?;
    let clients: Vec<_> = service
        .clients()
        .filter(|client| reality_enabled(configs, client))
        .collect();
    ensure!(!clients.is_empty(), "service has no bound Reality clients");
    // Preflight every participating endpoint before generation.
    for client in &clients {
        string_field(configs, client, "tls/reality/public_key")?;
    }
    let keys = singbox.generate_reality_keypair()?;
    ensure!(
        !keys.private_key.is_empty() && !keys.public_key.is_empty(),
        "generator returned empty Reality keys"
    );
    ensure!(
        keys.private_key != old_private,
        "generated Reality private key is unchanged; retry"
    );
    for client in &clients {
        ensure!(
            string_field(configs, client, "tls/reality/public_key")? != keys.public_key,
            "generated Reality public key is unchanged; retry"
        );
    }
    let mut plan = service_plan(
        service,
        OperationKind::Rotate(RotationKind::VlessRealityKeypair),
    );
    plan.edit(
        configs,
        service.inbound.field("tls/reality/private_key"),
        Some(keys.private_key.into()),
    )?;
    for client in clients {
        plan.edit(
            configs,
            client.field("tls/reality/public_key"),
            Some(keys.public_key.clone().into()),
        )?;
    }
    Ok(plan)
}

/// Reality decodes a hex ID into an eight-byte, zero-padded buffer. Compare the
/// decoded meaning, not spelling: "ab", "AB", and "ab00000000000000" agree.
pub(crate) fn normalized_short_id(value: &str) -> Result<String> {
    ensure!(
        value.len() <= 16
            && value.len().is_multiple_of(2)
            && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Reality short_id must contain an even number of hex digits (0 through 16)"
    );
    Ok(format!("{value:0<16}").to_ascii_lowercase())
}

pub(crate) fn client_short_id<'a>(configs: &'a ConfigSet, client: &EndpointRef) -> Result<&'a str> {
    match endpoint_value(configs, client)?.pointer("/tls/reality/short_id") {
        None => Ok(""), // sing-box's default is the all-zero short ID.
        Some(value) => value
            .as_str()
            .context("client Reality short_id must be a string"),
    }
}

pub fn short_ids(
    configs: &ConfigSet,
    inventory: &Inventory,
    input: &Input,
    singbox: &impl SingBox,
) -> Result<RotationPlan> {
    let service = one_service(
        configs,
        inventory,
        input,
        Some(Protocol::Vless),
        true,
        |service| {
            reality_enabled(configs, &service.inbound)
                && service.clients().any(|client| {
                    reality_enabled(configs, client) && client_selected(client, configs, input)
                })
        },
    )?;
    let target = service.inbound.field("tls/reality/short_id");
    let accepted = configs
        .value(&target.file, &target.pointer)?
        .as_array()
        .context("server Reality short_id must be an array")?;
    let accepted_ids: Vec<_> = accepted
        .iter()
        .map(|value| {
            normalized_short_id(
                value
                    .as_str()
                    .context("server Reality short_id entries must be strings")?,
            )
        })
        .collect::<Result<_>>()?;
    let mut used: BTreeSet<_> = accepted_ids.iter().cloned().collect();
    let mut selected = Vec::new();
    let mut retained = BTreeSet::new();
    let mut retired = BTreeSet::new();
    for client in service
        .clients()
        .filter(|client| reality_enabled(configs, client))
    {
        let id = normalized_short_id(client_short_id(configs, client)?)
            .with_context(|| format!("invalid Reality short_id in {}", client.label()))?;
        ensure!(
            used.contains(&id),
            "{}: Reality short_id is not accepted by the bound server",
            client.label()
        );
        if client_selected(client, configs, input) {
            selected.push(client);
            retired.insert(id);
        } else {
            retained.insert(id);
        }
    }
    ensure!(!selected.is_empty(), "no Reality client outbounds selected");
    let mut plan = service_plan(
        service,
        OperationKind::Rotate(RotationKind::VlessRealityShortId),
    );
    let mut replacements = Vec::new();
    for client in selected {
        let value = singbox.generate_random_hex(8)?;
        ensure!(
            value.len() == 16,
            "generator must return an eight-byte Reality short ID"
        );
        let id = normalized_short_id(&value)?;
        ensure!(
            used.insert(id.clone()),
            "generated Reality short ID collides with an existing or newly generated ID; retry"
        );
        plan.edit(
            configs,
            client.field("tls/reality/short_id"),
            Some(id.clone().into()),
        )?;
        replacements.push(Value::String(id));
    }
    // Remove only IDs attributable to selected clients, and only once no
    // unselected supplied client still needs them. Preserve unrelated IDs/order.
    let mut next: Vec<_> = accepted
        .iter()
        .zip(accepted_ids)
        .filter_map(|(value, id)| {
            (!retired.contains(&id) || retained.contains(&id)).then(|| value.clone())
        })
        .collect();
    next.extend(replacements);
    plan.edit(configs, target, Some(Value::Array(next)))?;
    Ok(plan)
}
