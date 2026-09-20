//! Explicit protocol planners. All writes/validation remain protocol-independent.
pub mod hysteria2;
pub mod properties;
pub mod vless;

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::{
    binding::{EndpointRef, Inventory, ServiceBinding, client_selected},
    cli::{Input, Protocol},
    config::ConfigSet,
    plan::{OperationKind, RotationPlan},
};

pub(crate) fn one_service<'a>(
    configs: &ConfigSet,
    inventory: &'a Inventory,
    input: &Input,
    protocol: Option<Protocol>,
    client_selectable: bool,
    eligible: impl Fn(&ServiceBinding) -> bool,
) -> Result<&'a ServiceBinding> {
    if !client_selectable {
        ensure!(
            input.client.is_empty() && input.client_tag.is_empty(),
            "service-wide operations reject --client and --client-tag; supply all clients with --clients and select the service with --inbound-tag"
        );
    }
    for candidate in [Protocol::Vless, Protocol::Hysteria2] {
        if protocol.is_none_or(|protocol| protocol == candidate) {
            inventory.ensure_unambiguous(candidate)?;
        }
    }
    let services: Vec<_> = inventory
        .services
        .iter()
        .filter(|service| {
            protocol.is_none_or(|protocol| service.protocol == protocol)
                && eligible(service)
                && service
                    .clients()
                    .any(|client| !client_selectable || client_selected(client, configs, input))
        })
        .collect();
    ensure!(
        !services.is_empty(),
        "no matching service with bound clients; use inspect to review bindings"
    );
    ensure!(
        services.len() == 1,
        "operation matches multiple services; narrow to one with --inbound-tag"
    );
    Ok(services[0])
}

pub(crate) fn service_plan(service: &ServiceBinding, operation: OperationKind) -> RotationPlan {
    let mut plan = RotationPlan::new(operation);
    plan.contexts.push(format!(
        "{} inbound={}, {} bound client occurrence(s)",
        service.protocol.name(),
        service.inbound.label(),
        service.clients().count()
    ));
    plan
}

pub(crate) fn endpoint_value<'a>(
    configs: &'a ConfigSet,
    endpoint: &EndpointRef,
) -> Result<&'a Value> {
    configs.value(&endpoint.config.file, &endpoint.config.pointer)
}

pub(crate) fn string_field<'a>(
    configs: &'a ConfigSet,
    endpoint: &EndpointRef,
    field: &str,
) -> Result<&'a str> {
    let target = endpoint.field(field);
    configs
        .value(&target.file, &target.pointer)?
        .as_str()
        .with_context(|| format!("{}: {field} must be a string", endpoint.label()))
}
