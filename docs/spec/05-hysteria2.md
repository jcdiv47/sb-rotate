# Specification: Hysteria2

## Relevant sing-box fields

### Server inbound

```text
inbounds[type=hysteria2]
  tag
  users[]
    name
    password
  obfs
    type
    password
  tls
  realm
    server_url
    token
    realm_id
```

### Client outbound

```text
outbounds[type=hysteria2]
  tag
  server
  server_port
  server_ports[]
  password
  obfs
    type
    password
  tls
    server_name
  realm
    server_url
    token
    realm_id
```

Hysteria2 gained additional Realm/obfs fields in sing-box 1.14. The initial `sb-rotate` scope does not rotate Realm tokens.

## Binding rule

Primary binding:

```text
client.password == server.users[].password
```

The matched server user defines the identity binding. The containing Hysteria2 inbound defines the service binding.

## `hysteria2-password`

Scope: identity.

Default generator:

```bash
sing-box generate rand 32 --base64
```

For every selected identity binding, replace:

```text
server users[].password
client outbound.password
```

across all occurrences of the shared identity.

As with VLESS UUIDs, selecting one client that shares a password with other clients selects the whole identity binding. Automatic splitting is out of scope.

## `hysteria2-obfs-password`

Scope: service.

Default generator:

```bash
sing-box generate rand 32 --base64
```

Requires Hysteria2 obfs to be configured for the service.

Update:

```text
server inbound.obfs.password
all bound client outbound.obfs.password
```

If a bound client does not have obfs enabled, it is not automatically given an obfs block. The planner reports the inconsistency instead of inventing protocol configuration.

## `server`

Scope: service, client-side.

Update every bound client's:

```text
outbound.server
```

No server-side edit is implied.

## `server-port`

Scope: service, client-side.

Update:

```text
outbound.server_port
```

This operation applies only when the selected outbounds use scalar `server_port`.

If an outbound uses `server_ports`, use `server-ports` instead.

## `server-ports`

Scope: service, client-side.

Sets the Hysteria2 port-hopping list:

```text
outbound.server_ports
```

The CLI accepts a comma-delimited value, for example `20000:30000,40000`. sing-box 1.14 requires range syntax for each JSON array entry, so single ports are normalized to equal-ended ranges:

```json
["20000:30000", "40000:40000"]
```

Setting `server-ports` removes any scalar `server_port` on each bound outbound. Switching a port-hopping outbound back to scalar mode is not supported by `server-port` in v1.

No server-side listen/NAT changes are implied.

## `tls-server-name`

Scope: service, client-side.

Updates:

```text
outbound.tls.server_name
```

for all bound Hysteria2 clients.

## Realm tokens

Hysteria Realm uses bearer tokens in sing-box 1.14+. These are deliberately excluded from the first implementation because Realm introduces a third participant: the rendezvous service.

If added later, Realm token rotation should be implemented as its own operation with explicit Realm configuration input rather than pretending it is a normal server/client credential.

## References

- https://sing-box.sagernet.org/configuration/inbound/hysteria2/
- https://sing-box.sagernet.org/configuration/outbound/hysteria2/
- https://sing-box.sagernet.org/configuration/service/hysteria-realm/
