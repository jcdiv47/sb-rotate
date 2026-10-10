# Example: Multiple VLESS Outbounds

A client file can contain more than one VLESS outbound. Discovery is performed per outbound, not per file.

Suppose the server has:

```text
inbound=vless-home
  alice -> uuid-a

inbound=vless-office
  bob -> uuid-b
```

and `phone.json` contains:

```text
outbound=home   -> uuid-a
outbound=backup -> uuid-a
outbound=office -> uuid-b
```

Inspection:

```bash
sb-rotate inspect --server ./server.json --client ./phone.json
```

Logical result:

```text
vless-home
  identity uuid-a
    phone.json#home
    phone.json#backup

vless-office
  identity uuid-b
    phone.json#office
```

Rotating what `home` needs:

```bash
sb-rotate rotate --server ./server.json --client ./phone.json \
  --type vless --outbound-tag home
```

also updates the UUID in `backup`, because both outbounds share the same identity binding. If `vless-home` uses Reality, only `home` receives a new short ID. The keypair rotates only when every bound outbound of `vless-home` is selected.

```text
before
server alice -> uuid-a
phone home   -> uuid-a
phone backup -> uuid-a

          ↓

after
server alice -> uuid-new
phone home   -> uuid-new
phone backup -> uuid-new
```

The tool does not silently create a second server user just to split one shared UUID.

To target the office service instead:

```bash
sb-rotate rotate --server ./server.json --client ./phone.json \
  --type vless --outbound-tag office
```
