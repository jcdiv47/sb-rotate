# Example: Per-client Reality `short_id`

Reality short IDs are different from VLESS UUIDs because the server accepts a list while each client uses one value.

Initial state:

```text
server vless-home short_id:
  [aaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbb]

phone:
  aaaaaaaaaaaaaaaa

laptop:
  aaaaaaaaaaaaaaaa

tablet:
  bbbbbbbbbbbbbbbb
```

Rotate only the phone:

```bash
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-reality-short-id \
  --client ./clients/phone.json \
  --client-tag home
```

Suppose the new ID is `cccccccccccccccc`.

Result:

```text
server:
  [aaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbb, cccccccccccccccc]

phone:
  cccccccccccccccc

laptop:
  aaaaaaaaaaaaaaaa

tablet:
  bbbbbbbbbbbbbbbb
```

The old `aaaaaaaaaaaaaaaa` remains because the laptop still uses it.

Now rotate the laptop as well:

```bash
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-reality-short-id \
  --client ./clients/laptop.json \
  --client-tag home
```

If the laptop receives `dddddddddddddddd`, the server can remove `aaaaaaaaaaaaaaaa` because none of the supplied bound clients still references it:

```text
server:
  [bbbbbbbbbbbbbbbb, cccccccccccccccc, dddddddddddddddd]
```

Any server short ID that cannot be attributed to the supplied clients is preserved.

Rotate phone and laptop in one command:

```bash
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-reality-short-id \
  --client ./clients/phone.json \
  --client ./clients/laptop.json \
  --client-tag home
```

Each selected outbound receives its own newly generated short ID.
