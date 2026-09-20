use anyhow::{Result, ensure};

use crate::{
    binding::Inventory,
    cli::{Input, Protocol, RotationKind},
    config::ConfigSet,
    plan::{OperationKind, RotationPlan},
    singbox::SingBox,
};

use super::{one_service, service_plan, string_field};

pub fn obfs_password(
    configs: &ConfigSet,
    inventory: &Inventory,
    input: &Input,
    singbox: &impl SingBox,
) -> Result<RotationPlan> {
    let service = one_service(
        configs,
        inventory,
        input,
        Some(Protocol::Hysteria2),
        false,
        |_| true,
    )?;
    let obfs_type = string_field(configs, &service.inbound, "obfs/type")?;
    ensure!(
        !obfs_type.is_empty(),
        "Hysteria2 obfs must be configured on the server"
    );
    let mut old = vec![string_field(configs, &service.inbound, "obfs/password")?];
    for client in service.clients() {
        ensure!(
            string_field(configs, client, "obfs/type")? == obfs_type,
            "{}: bound client's obfs type differs from the server",
            client.label()
        );
        old.push(string_field(configs, client, "obfs/password")?);
    }
    let replacement = singbox.generate_random_base64(32)?;
    ensure!(
        !replacement.is_empty(),
        "generator returned an empty obfs password"
    );
    ensure!(
        !old.contains(&replacement.as_str()),
        "generated obfs password is unchanged; retry"
    );
    let mut plan = service_plan(
        service,
        OperationKind::Rotate(RotationKind::Hysteria2ObfsPassword),
    );
    for endpoint in std::iter::once(&service.inbound).chain(service.clients()) {
        plan.edit(
            configs,
            endpoint.field("obfs/password"),
            Some(replacement.clone().into()),
        )?;
    }
    Ok(plan)
}
