use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::{
    binding::Inventory,
    cli::{Input, PropertyKind, Protocol},
    config::ConfigSet,
    plan::{OperationKind, RotationPlan},
};

use super::{endpoint_value, one_service, service_plan};

fn port(value: &str) -> Result<u16> {
    ensure!(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
        "port must be an integer between 1 and 65535"
    );
    let port: u16 = value
        .parse()
        .context("port must be an integer between 1 and 65535")?;
    ensure!(port > 0, "port must be between 1 and 65535");
    Ok(port)
}

fn property_value(kind: PropertyKind, value: &str) -> Result<Value> {
    match kind {
        PropertyKind::Server | PropertyKind::TlsServerName => {
            ensure!(
                !value.is_empty() && !value.chars().any(char::is_whitespace),
                "server address / TLS server name must be nonempty and contain no whitespace"
            );
            Ok(value.into())
        }
        PropertyKind::ServerPort => Ok(port(value)?.into()),
        PropertyKind::ServerPorts => {
            let mut ports = Vec::new();
            for entry in value.split(',').map(str::trim) {
                match entry.split_once(':') {
                    Some((start, end)) => {
                        let start = port(start)?;
                        let end = port(end)?;
                        ensure!(start <= end, "port range start must not exceed its end");
                        ports.push(Value::String(format!("{start}:{end}")));
                    }
                    None => {
                        let port = port(entry)?;
                        ports.push(Value::String(format!("{port}:{port}")));
                    }
                }
            }
            Ok(Value::Array(ports))
        }
    }
}

pub fn set(
    configs: &ConfigSet,
    inventory: &Inventory,
    input: &Input,
    kind: PropertyKind,
    value: &str,
) -> Result<RotationPlan> {
    let protocol = (kind == PropertyKind::ServerPorts).then_some(Protocol::Hysteria2);
    let service = one_service(configs, inventory, input, protocol, false, |_| true)?;
    let value = property_value(kind, value)?;
    let mut plan = service_plan(service, OperationKind::Set(kind));
    for client in service.clients() {
        let outbound = endpoint_value(configs, client)?;
        let field = match kind {
            PropertyKind::Server => "server",
            PropertyKind::ServerPort => {
                if service.protocol == Protocol::Hysteria2 {
                    if let Some(ports) = outbound.get("server_ports") {
                        ensure!(
                            ports.as_array().is_some_and(Vec::is_empty),
                            "{}: outbound uses server_ports; use --kind server-ports instead",
                            client.label()
                        );
                    }
                    ensure!(
                        outbound.get("server_port").is_some_and(Value::is_u64),
                        "{}: server-port requires an existing scalar server_port",
                        client.label()
                    );
                }
                "server_port"
            }
            PropertyKind::ServerPorts => {
                // An explicit switch to port hopping must not leave a competing
                // scalar port behind. The server listen_port is never changed.
                plan.edit(configs, client.field("server_port"), None)?;
                "server_ports"
            }
            PropertyKind::TlsServerName => {
                ensure!(
                    outbound.pointer("/tls/enabled").and_then(Value::as_bool) == Some(true),
                    "{}: TLS must already be enabled; refusing to invent a TLS block",
                    client.label()
                );
                "tls/server_name"
            }
        };
        plan.edit(configs, client.field(field), Some(value.clone()))?;
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_port_lists_and_rejects_invalid_values() {
        assert_eq!(
            property_value(PropertyKind::ServerPorts, "20000:30000, 40000").unwrap(),
            json!(["20000:30000", "40000:40000"])
        );
        for value in [
            "", "0", "65536", "2:1", "1:", ":2", "1,", "1:2:3", "-1", "1.5", "+80",
        ] {
            assert!(
                property_value(PropertyKind::ServerPorts, value).is_err(),
                "{value}"
            );
        }
        assert!(property_value(PropertyKind::ServerPort, "443:444").is_err());
    }
}
