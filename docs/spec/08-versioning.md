# Specification: Versioning and sing-box Integration

## Supported version

The minimum supported sing-box version is:

```text
1.14.0
```

At startup, commands that inspect or mutate configs run:

```bash
sing-box version
```

and parse the reported semantic version.

Versions below 1.14.0 are rejected.

Pre-release versions are not required to satisfy the minimum unless their semantic version compares greater than or equal to the release boundary.

## Why sing-box is authoritative

sing-box changes frequently. `sb-rotate` therefore keeps only a small set of stable field mappings and delegates these concerns to the installed binary:

- config validation;
- UUID generation;
- random secret generation;
- Reality keypair generation.

The installed binary may also generate a matching JSON Schema in sing-box 1.14+:

```bash
sing-box schema -o schema.json
```

`sb-rotate` does not need to consume that schema in v1. It is useful as a debugging/version-compatibility aid.

## Required subprocess interface

```rust
trait SingBox {
    fn version(&self) -> Result<Version>;
    fn generate_uuid(&self) -> Result<String>;
    fn generate_random_base64(&self, bytes: usize) -> Result<String>;
    fn generate_random_hex(&self, bytes: usize) -> Result<String>;
    fn generate_reality_keypair(&self) -> Result<RealityKeyPair>;
    fn check_file(&self, path: &Path) -> Result<()>;
    fn check_directory(&self, path: &Path) -> Result<()>;
}
```

Expected commands:

```text
sing-box version
sing-box generate uuid
sing-box generate rand 32 --base64
sing-box generate rand 8 --hex
sing-box generate reality-keypair
sing-box check ...
```

## Binary resolution

Resolve in this order:

```text
--sing-box PATH
SING_BOX environment variable
PATH lookup for sing-box
```

The resolved executable is used for the entire command. Do not mix generators/validators from different sing-box binaries.

## Protocol mapping stability

Protocol field paths belong in the adapter that owns the protocol.

Suggested module structure:

```text
src/
  main.rs
  cli.rs
  singbox.rs

  config/
    load.rs
    edit.rs
    validate.rs

  binding/
    model.rs
    discover.rs

  protocol/
    mod.rs
    vless.rs
    hysteria2.rs

  plan/
    mod.rs
    apply.rs
```

Do not create a central declarative table that tries to describe every relationship type.

## Adding a protocol

A new protocol adapter should implement:

1. discovery of identity/service relationships;
2. supported operation kinds;
3. protocol-specific plan generation.

It returns the same generic `RotationPlan` as existing adapters.

No changes should be needed in the atomic write or sing-box validation layers.

## References

- Configuration/check: https://sing-box.sagernet.org/configuration/
- JSON Schema generation: https://sing-box.sagernet.org/configuration/schema/
- Current generator implementation: https://github.com/SagerNet/sing-box/tree/testing/cmd/sing-box
