# Example: Whole-service VLESS Update

Once clients are bound to a VLESS inbound through their UUIDs, service-level values can be updated without trying to match those values against server-side fields.

Suppose every `vless-home` client currently uses:

```text
server: old.example.com
server_port: 8443
tls.server_name: old.example.com
```

## Change hostname

```bash
sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home \
  --kind server \
  --value new.example.com
```

Every client outbound bound to `vless-home` receives:

```json
"server": "new.example.com"
```

The server inbound is not changed because its listen address is a separate deployment concern.

## Change public port

```bash
sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home \
  --kind server-port \
  --value 443
```

This changes the client outbound port only. It does not assume that the server's `listen_port` is also 443.

## Change TLS hostname/SNI

```bash
sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home \
  --kind tls-server-name \
  --value new.example.com
```

This updates client `tls.server_name` across the service.

## Rotate the Reality keypair

```bash
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home \
  --kind vless-reality-keypair
```

The operation runs:

```bash
sing-box generate reality-keypair
```

and maps the result as:

```text
private key -> server vless-home tls.reality.private_key
public key  -> every bound Reality client's tls.reality.public_key
```
