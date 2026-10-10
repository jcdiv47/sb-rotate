//! Explicit protocol planners. All writes/validation remain protocol-independent.
pub mod hysteria2;
pub mod properties;
pub mod vless;

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::{
    binding::{EndpointRef, Inventory, ServiceBinding},
    cli::{Input, Protocol},
    config::ConfigSet,
    plan::{OperationKind, RotationPlan},
};

pub(crate) fn one_service<'a>(
    inventory: &'a Inventory,
    input: &Input,
    protocol: Option<Protocol>,
) -> Result<&'a ServiceBinding> {
    ensure!(
        input.client.is_empty() && input.client_tag.is_empty(),
        "service-wide operations reject --client and --outbound-tag (--client-tag); supply all client configs with --clients and select the inbound with --inbound-tag"
    );
    for candidate in Protocol::ALL {
        if protocol.is_none_or(|protocol| protocol == candidate) {
            inventory.ensure_unambiguous(candidate)?;
        }
    }
    let services: Vec<_> = inventory
        .services
        .iter()
        .filter(|service| {
            protocol.is_none_or(|protocol| service.protocol == protocol)
                && service.clients().next().is_some()
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

/// Service-wide changes also reach server users whose client configs were not supplied.
pub(crate) fn warn_unsupplied_users(
    plan: &mut RotationPlan,
    service: &ServiceBinding,
    change: &str,
) {
    let unsupplied: Vec<_> = service
        .identities
        .iter()
        .filter(|identity| identity.clients.is_empty())
        .collect();
    if unsupplied.is_empty() {
        return;
    }
    let names: Vec<_> = unsupplied
        .iter()
        .flat_map(|identity| &identity.user_names)
        .map(String::as_str)
        .collect();
    plan.contexts.push(format!(
        "  warning: {} server user(s) [{}] have no supplied client config; update their clients' {change} separately",
        unsupplied.len(),
        names.join(", ")
    ));
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
