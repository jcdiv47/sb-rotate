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

## Rotate

Rotate all matched Hysteria2 identities and the shared obfs password:

```bash
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --type hysteria2
```

The tool generates one new password per identity binding.

Logical result:

```text
alice server password -> new-a
phone password        -> new-a

bob server password   -> new-b
laptop password       -> new-b
```

One new obfs password is also generated for `hy2-home` and written to:

```text
server hy2-home obfs.password
phone obfs.password
laptop obfs.password
```

Rotate only the phone's credentials:

```bash
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --type hysteria2 \
  --client ./clients/phone.json
```

This rotates alice's password, plus any other supplied client sharing it, because the password defines one identity binding. The shared obfs password is kept because the laptop is not selected, so the laptop is not touched at all.

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
