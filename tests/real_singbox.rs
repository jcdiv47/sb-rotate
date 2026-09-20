//! Opt-in integration coverage using the real generators and validator.
//! Run: cargo test --test real_singbox -- --ignored
//! Requires sing-box >=1.14.0 (SING_BOX or PATH) and openssl. No listeners are
//! started and no remote connections are made; all configs live in a tempdir.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use anyhow::{Context, Result, ensure};
use sb_rotate::singbox::{Executable, SingBox, require_supported};
use serde_json::{Value, json};

fn write(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn read(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn successful(output: Output) -> Result<()> {
    ensure!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
#[ignore = "requires real sing-box >=1.14.0 and openssl"]
fn real_generators_validation_and_service_operations() -> Result<()> {
    let singbox = Executable::resolve(None);
    require_supported(&singbox)?;
    let root = tempfile::tempdir()?;
    let server_dir = root.path().join("server.d");
    let clients_dir = root.path().join("clients");
    fs::create_dir(&server_dir)?;
    fs::create_dir(&clients_dir)?;
    let server_path = server_dir.join("10-inbounds.json");
    let phone_path = clients_dir.join("phone.json");
    let laptop_path = clients_dir.join("laptop.json");
    let cert = root.path().join("cert.pem");
    let key = root.path().join("key.pem");
    successful(
        Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-keyout",
            ])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .output()
            .context("generating temporary test certificate with openssl")?,
    )?;
    let keys = singbox.generate_reality_keypair()?;
    let uuid = singbox.generate_uuid()?;
    let original_id = "aaaaaaaaaaaaaaaa";
    write(
        &server_dir.join("00-log.json"),
        &json!({"log": {"disabled": true}}),
    )?;
    write(
        &server_path,
        &json!({"inbounds": [
            {"type": "vless", "tag": "vless-home", "listen": "127.0.0.1", "listen_port": 18443,
             "users": [{"name": "shared", "uuid": uuid}],
             "tls": {"enabled": true, "reality": {"enabled": true, "handshake": {"server": "localhost", "server_port": 443}, "private_key": keys.private_key, "short_id": [original_id, "cccccccccccccccc"]}}},
            {"type": "hysteria2", "tag": "hy2-home", "listen": "127.0.0.1", "listen_port": 18444,
             "users": [{"name": "shared", "password": "old-auth"}], "obfs": {"type": "salamander", "password": "old-obfs"},
             "tls": {"enabled": true, "certificate_path": cert, "key_path": key}}
        ]}),
    )?;
    for path in [&phone_path, &laptop_path] {
        write(
            path,
            &json!({"outbounds": [
                {"type": "vless", "tag": "vless-home", "server": "127.0.0.1", "server_port": 18443, "uuid": uuid,
                 "tls": {"enabled": true, "server_name": "localhost", "utls": {"enabled": true, "fingerprint": "chrome"},
                    "reality": {"enabled": true, "public_key": keys.public_key, "short_id": original_id}}},
                {"type": "hysteria2", "tag": "hy2-home", "server": "127.0.0.1", "server_port": 18444, "password": "old-auth",
                 "obfs": {"type": "salamander", "password": "old-obfs"}, "tls": {"enabled": true, "server_name": "localhost", "insecure": true}}
            ]}),
        )?;
    }
    let run = |command: &str, args: &[&str]| -> Result<Output> {
        Ok(Command::new(env!("CARGO_BIN_EXE_sb-rotate"))
            .arg(command)
            .arg("--server")
            .arg(&server_dir)
            .arg("--clients")
            .arg(&clients_dir)
            .args(args)
            .output()?)
    };
    let snapshot = || -> Result<Vec<Vec<u8>>> {
        [&server_path, &phone_path, &laptop_path]
            .iter()
            .map(|path| Ok(fs::read(path)?))
            .collect()
    };
    successful(run("check", &[])?)?;
    for kind in [
        "vless-uuid",
        "hysteria2-password",
        "vless-reality-keypair",
        "hysteria2-obfs-password",
    ] {
        let before = snapshot()?;
        successful(run("plan", &["--kind", kind])?)?;
        assert_eq!(before, snapshot()?, "plan must be read-only");
        successful(run("rotate", &["--kind", kind])?)?;
        successful(run("check", &[])?)?;
    }
    let server = read(&server_path)?;
    assert!(server["inbounds"][0]["tls"]["reality"]["private_key"] != keys.private_key);
    for path in [&phone_path, &laptop_path] {
        let client = read(path)?;
        assert_eq!(
            server["inbounds"][0]["users"][0]["uuid"],
            client["outbounds"][0]["uuid"]
        );
        assert_eq!(
            server["inbounds"][1]["users"][0]["password"],
            client["outbounds"][1]["password"]
        );
        assert_eq!(
            server["inbounds"][1]["obfs"]["password"],
            client["outbounds"][1]["obfs"]["password"]
        );
        assert!(client["outbounds"][0]["tls"]["reality"]["public_key"] != keys.public_key);
    }
    let phone = phone_path
        .to_str()
        .context("temporary path must be UTF-8")?;
    successful(run(
        "rotate",
        &[
            "--kind",
            "vless-reality-short-id",
            "--client",
            phone,
            "--client-tag",
            "vless-home",
        ],
    )?)?;
    let server = read(&server_path)?;
    let ids = server["inbounds"][0]["tls"]["reality"]["short_id"]
        .as_array()
        .unwrap();
    assert!(
        ids.contains(&json!(original_id)),
        "unselected laptop still uses the original ID"
    );
    assert!(ids.contains(&read(&phone_path)?["outbounds"][0]["tls"]["reality"]["short_id"]));
    assert_eq!(
        read(&laptop_path)?["outbounds"][0]["tls"]["reality"]["short_id"],
        original_id
    );
    successful(run("rotate", &["--kind", "vless-reality-short-id"])?)?;
    let server = read(&server_path)?;
    let ids = server["inbounds"][0]["tls"]["reality"]["short_id"]
        .as_array()
        .unwrap();
    assert!(!ids.contains(&json!(original_id)));
    assert!(
        ids.contains(&json!("cccccccccccccccc")),
        "unattributed IDs must be preserved"
    );
    assert_ne!(
        read(&phone_path)?["outbounds"][0]["tls"]["reality"]["short_id"],
        read(&laptop_path)?["outbounds"][0]["tls"]["reality"]["short_id"]
    );
    let server_before_properties = fs::read(&server_path)?;
    for (kind, value, tag) in [
        ("server", "new.example", "vless-home"),
        ("tls-server-name", "new-tls.example", "hy2-home"),
        ("server-port", "19444", "hy2-home"),
        ("server-ports", "20000:30000,40000", "hy2-home"),
    ] {
        let before = snapshot()?;
        successful(run(
            "set",
            &[
                "--kind",
                kind,
                "--value",
                value,
                "--inbound-tag",
                tag,
                "--dry-run",
            ],
        )?)?;
        assert_eq!(before, snapshot()?);
        successful(run(
            "set",
            &["--kind", kind, "--value", value, "--inbound-tag", tag],
        )?)?;
    }
    assert_eq!(server_before_properties, fs::read(&server_path)?);
    for path in [&phone_path, &laptop_path] {
        let client = read(path)?;
        assert_eq!(client["outbounds"][0]["server"], "new.example");
        assert_eq!(
            client["outbounds"][1]["tls"]["server_name"],
            "new-tls.example"
        );
        assert!(client["outbounds"][1].get("server_port").is_none());
        assert_eq!(
            client["outbounds"][1]["server_ports"],
            json!(["20000:30000", "40000:40000"])
        );
    }
    successful(run("check", &[])?)?;
    let mut invalid = read(&server_path)?;
    invalid["inbounds"][0]["listen_port"] = json!("invalid-port");
    write(&server_path, &invalid)?;
    let before = snapshot()?;
    let failed = run("rotate", &["--kind", "vless-reality-keypair"])?;
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("validating"));
    assert_eq!(
        before,
        snapshot()?,
        "failed real validation must never replace originals"
    );
    Ok(())
}
