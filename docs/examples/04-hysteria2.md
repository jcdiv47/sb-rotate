# Example: Hysteria2

Suppose the server has one Hysteria2 inbound:

```text
hy2-home
  alice -> password-a
  bob   -> password-b
  obfs  -> shared-obfs
```

and clients resolve as:

```text
phone  -> password-a
laptop -> password-b
```

## Rotate user authentication

Rotate all matched Hysteria2 identities:

```bash
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind hysteria2-password
```

The tool generates one new password per identity binding.

Logical result:

```text
alice server password -> new-a
phone password        -> new-a

bob server password   -> new-b
laptop password       -> new-b
```

Rotate only the identity used by the phone:

```bash
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind hysteria2-password \
  --client ./clients/phone.json
```

If another supplied client shares the same old password, it rotates with the phone because the password defines one identity binding.

## Rotate shared obfuscation password

```bash
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind hysteria2-obfs-password \
  --inbound-tag hy2-home
```

One new obfs password is generated and written to:

```text
server hy2-home obfs.password
phone obfs.password
laptop obfs.password
```

## Move clients to another host

```bash
sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag hy2-home \
  --kind server \
  --value hy2-new.example.com
```

## Change Hysteria2 port hopping list

```bash
sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag hy2-home \
  --kind server-ports \
  --value 20000:30000,40000
```

This changes client connection settings only. Firewall/NAT/listen configuration is outside `sb-rotate`.
