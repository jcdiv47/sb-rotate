use super::*;
use crate::cli::Input;
use tempfile::TempDir;

struct Fixture {
    root: TempDir,
    configs: ConfigSet,
    staged: BTreeMap<PathBuf, TempPath>,
    locks: Option<DirectoryLocks>,
    transaction: Transaction,
}

fn setup(root: &Path) -> (ConfigSet, BTreeMap<PathBuf, TempPath>) {
    fs::create_dir(root.join("clients")).unwrap();
    fs::write(root.join("a.json"), b"{\"secret\":\"old-a\"}\n").unwrap();
    fs::write(root.join("clients/b.json"), b"{\"secret\":\"old-b\"}\n").unwrap();
    let configs = ConfigSet::load(&Input {
        server: root.join("a.json"),
        clients: Some(root.join("clients")),
        ..Input::default()
    })
    .unwrap();
    let staged = configs
        .documents
        .iter()
        .map(|(file, doc)| {
            let new = String::from_utf8(doc.original.clone())
                .unwrap()
                .replace("old-", "new-");
            (
                file.clone(),
                stage(file, new.as_bytes(), &doc.state).unwrap(),
            )
        })
        .collect();
    (configs, staged)
}

fn install(transaction: &Transaction, staged: &BTreeMap<PathBuf, TempPath>, count: usize) {
    transaction.set_flag("started").unwrap();
    for (file, temp) in staged.iter().take(count) {
        fs::rename(temp, file).unwrap();
        sync_directory(file.parent().unwrap()).unwrap();
    }
}

impl Fixture {
    fn new() -> Self {
        Self::with_staged_count(2)
    }

    fn with_staged_count(count: usize) -> Self {
        let root = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let (configs, mut staged) = setup(root.path());
        while staged.len() > count {
            staged.pop_last();
        }
        let directories = configs.lock_directories().unwrap();
        let locks = Some(DirectoryLocks::acquire(directories.clone()).unwrap());
        let transaction = Transaction::prepare(&configs, &staged, &directories).unwrap();
        Self {
            root,
            configs,
            staged,
            locks,
            transaction,
        }
    }
    fn release(&mut self) {
        self.locks.take();
    }
    fn originals(&self) {
        for (path, doc) in &self.configs.documents {
            assert_eq!(fs::read(path).unwrap(), doc.original);
        }
    }
    fn assert_clean(&self) {
        assert!(!self.transaction.path.exists());
        for directory in self.configs.lock_directories().unwrap() {
            assert!(!directory.join(PENDING).exists());
            assert!(fs::read_dir(directory).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            }));
        }
    }
}

#[test]
fn ordinary_failure_restores_prior_replacements_and_removes_journal() {
    let fixture = Fixture::new();
    let mut calls = 0;
    let error = fixture
        .transaction
        .commit(&fixture.staged, |source, destination| {
            calls += 1;
            if calls == 2 {
                return Err(std::io::Error::other("injected failure"));
            }
            fs::rename(source, destination)
        })
        .unwrap_err();
    assert!(error.to_string().contains("rolled back"));
    fixture.originals();
    fixture.assert_clean();
}

#[test]
fn ordinary_rollback_refuses_newer_external_edits_and_retains_private_backups() {
    let mut fixture = Fixture::new();
    let first = fixture.configs.server.clone();
    let mut calls = 0;
    let error = fixture
        .transaction
        .commit(&fixture.staged, |source, destination| {
            calls += 1;
            if calls == 2 {
                fs::write(&first, b"external edit")?;
                return Err(std::io::Error::other("injected failure"));
            }
            fs::rename(source, destination)
        })
        .unwrap_err();
    assert!(error.to_string().contains("ROLLBACK INCOMPLETE"));
    assert!(error.to_string().contains("external edit"));
    assert_eq!(fs::read(&first).unwrap(), b"external edit");
    assert!(fixture.transaction.path.join("0.original").exists());
    fixture.release();
    assert!(recover(&fixture.transaction.path, false).is_err());
    assert_eq!(fs::read(first).unwrap(), b"external edit");
}

#[test]
fn added_client_membership_blocks_rollback_until_reconciled() {
    let mut fixture = Fixture::new();
    install(&fixture.transaction, &fixture.staged, 1);
    let extra = fixture.root.path().join("clients/new.json");
    fs::write(&extra, b"{}").unwrap();
    fixture.release();
    assert!(
        recover(&fixture.transaction.path, true)
            .unwrap()
            .contains("directory membership changed")
    );
    assert!(recover(&fixture.transaction.path, false).is_err());
    assert!(
        fs::read_to_string(&fixture.configs.server)
            .unwrap()
            .contains("new-a")
    );
    fs::remove_file(extra).unwrap();
    recover(&fixture.transaction.path, false).unwrap();
    fixture.originals();
}

#[test]
fn edits_to_unchanged_sources_are_detected_before_rollback() {
    let mut fixture = Fixture::with_staged_count(1);
    install(&fixture.transaction, &fixture.staged, 1);
    let client = fixture.root.path().join("clients/b.json");
    fs::write(&client, b"newer external client").unwrap();
    fixture.release();
    assert!(
        recover(&fixture.transaction.path, true)
            .unwrap()
            .contains("previously unchanged config was edited")
    );
    assert!(recover(&fixture.transaction.path, false).is_err());
    assert!(
        fs::read_to_string(&fixture.configs.server)
            .unwrap()
            .contains("new-a")
    );
    fs::write(&client, &fixture.configs.documents[&client].original).unwrap();
    recover(&fixture.transaction.path, false).unwrap();
    fixture.originals();
}

#[test]
fn successful_commit_cleans_metadata_without_retaining_old_secrets() {
    let fixture = Fixture::new();
    fixture
        .transaction
        .commit(&fixture.staged, |source, destination| {
            fs::rename(source, destination)
        })
        .unwrap();
    fixture.assert_clean();
    assert!(
        fs::read_to_string(&fixture.configs.server)
            .unwrap()
            .contains("new-a")
    );
}

#[test]
fn dry_run_is_read_only_and_never_displays_credentials() {
    let mut fixture = Fixture::new();
    install(&fixture.transaction, &fixture.staged, 1);
    let before: Vec<_> = fixture
        .configs
        .documents
        .keys()
        .map(|path| fs::read(path).unwrap())
        .collect();
    let manifest = fs::read(fixture.transaction.path.join("manifest.json")).unwrap();
    fixture.release();
    let report = recover(&fixture.transaction.path, true).unwrap();
    assert!(report.contains("installed replacement"));
    assert!(report.contains("original"));
    assert!(!report.contains("old-a"));
    assert!(!report.contains("new-a"));
    assert_eq!(
        fs::read(fixture.transaction.path.join("manifest.json")).unwrap(),
        manifest
    );
    assert_eq!(
        fixture
            .configs
            .documents
            .keys()
            .map(|path| fs::read(path).unwrap())
            .collect::<Vec<_>>(),
        before
    );
    recover(&fixture.transaction.path, false).unwrap();
    fixture.originals();
    fixture.assert_clean();
}

#[test]
fn every_target_is_preflighted_before_any_rollback_write() {
    let mut fixture = Fixture::new();
    install(&fixture.transaction, &fixture.staged, 2);
    let server_before = fs::read(&fixture.configs.server).unwrap();
    let client = fixture.root.path().join("clients/b.json");
    fs::write(&client, b"newer external configuration").unwrap();
    fixture.release();
    assert!(
        recover(&fixture.transaction.path, true)
            .unwrap()
            .contains("CONFLICT")
    );
    assert!(
        recover(&fixture.transaction.path, false)
            .unwrap_err()
            .to_string()
            .contains("Recovery refused")
    );
    assert_eq!(fs::read(&fixture.configs.server).unwrap(), server_before);
    assert_eq!(fs::read(client).unwrap(), b"newer external configuration");
}

#[test]
fn committed_transaction_cleanup_never_rolls_back_later_edits() {
    let mut fixture = Fixture::new();
    install(&fixture.transaction, &fixture.staged, 2);
    fixture.transaction.set_flag("committed").unwrap();
    // Simulate interruption halfway through cleanup and a later external edit.
    fs::remove_file(fixture.root.path().join("clients").join(PENDING)).unwrap();
    fs::write(&fixture.configs.server, b"later edit").unwrap();
    fixture.release();
    assert!(
        recover(&fixture.transaction.path, false)
            .unwrap()
            .contains("Already committed")
    );
    assert_eq!(fs::read(&fixture.configs.server).unwrap(), b"later edit");
    fixture.assert_clean();
}

#[test]
fn incomplete_publication_is_cleanup_only_and_does_not_touch_external_changes() {
    let mut fixture = Fixture::new();
    fs::remove_file(fixture.root.path().join("clients").join(PENDING)).unwrap();
    fs::write(&fixture.configs.server, b"external before commit").unwrap();
    fixture.release();
    assert!(
        recover(&fixture.transaction.path, false)
            .unwrap()
            .contains("Commit not started")
    );
    assert_eq!(
        fs::read(&fixture.configs.server).unwrap(),
        b"external before commit"
    );
    fixture.assert_clean();
}

#[test]
fn rollback_can_resume_after_restoring_only_one_original() {
    let mut fixture = Fixture::new();
    install(&fixture.transaction, &fixture.staged, 2);
    let entry = &fixture.transaction.manifest.entries[0];
    let original = fixture.transaction.original(0, entry).unwrap();
    let temp = stage(&entry.file.0, &original, &entry.original.state).unwrap();
    fs::rename(temp, &entry.file.0).unwrap();
    let restored_before = read_snapshot(&entry.file.0).unwrap();
    fixture.release();
    recover(&fixture.transaction.path, false).unwrap();
    assert!(read_snapshot(&fixture.configs.server).unwrap() == restored_before);
    fixture.originals();
    fixture.assert_clean();
}

#[test]
fn corrupted_backup_is_detected_before_restoring_any_file() {
    let mut fixture = Fixture::new();
    install(&fixture.transaction, &fixture.staged, 2);
    fs::write(fixture.transaction.path.join("1.original"), b"corrupted").unwrap();
    fixture.release();
    assert!(
        recover(&fixture.transaction.path, false)
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
    assert!(
        fs::read_to_string(&fixture.configs.server)
            .unwrap()
            .contains("new-a")
    );
    assert!(pending_journal(fixture.root.path()).unwrap().is_some());
}

#[test]
fn unsupported_manifest_and_missing_started_marker_membership_are_rejected() {
    let mut fixture = Fixture::new();
    install(&fixture.transaction, &fixture.staged, 1);
    fs::remove_file(fixture.root.path().join("clients").join(PENDING)).unwrap();
    fixture.release();
    assert!(
        recover(&fixture.transaction.path, false)
            .unwrap_err()
            .to_string()
            .contains("marker is missing")
    );
    let path = fixture.transaction.path.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["version"] = serde_json::json!(999);
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(
        recover(&fixture.transaction.path, false)
            .unwrap_err()
            .to_string()
            .contains("unsupported")
    );
}

#[test]
fn recovery_requires_all_writer_locks_and_pending_blocks_even_noop_mutations() {
    let mut fixture = Fixture::new();
    assert!(
        recover(&fixture.transaction.path, false)
            .unwrap_err()
            .to_string()
            .contains("another sb-rotate writer")
    );
    fixture.release();
    let plan = crate::plan::RotationPlan::new(crate::plan::OperationKind::Set(
        crate::cli::PropertyKind::Server,
    ));
    // apply fails on the pending marker before it needs the executable.
    let binary = crate::singbox::Executable::resolve(Some(Path::new("does-not-exist")));
    assert!(
        crate::apply::apply(&fixture.configs, &plan, &binary)
            .unwrap_err()
            .to_string()
            .contains("pending config transaction")
    );
    recover(&fixture.transaction.path, false).unwrap();
}

#[cfg(unix)]
#[test]
fn journals_are_private_and_symlink_targets_or_backups_are_never_followed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let mut fixture = Fixture::new();
    assert_eq!(
        fs::metadata(&fixture.transaction.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o077,
        0
    );
    for name in ["manifest.json", "0.original", "1.original"] {
        assert_eq!(
            fs::metadata(fixture.transaction.path.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
    install(&fixture.transaction, &fixture.staged, 1);
    let actual = fixture.root.path().join("external.json");
    fs::write(&actual, b"must not change").unwrap();
    fs::remove_file(&fixture.configs.server).unwrap();
    symlink(&actual, &fixture.configs.server).unwrap();
    fixture.release();
    assert!(recover(&fixture.transaction.path, false).is_err());
    assert_eq!(fs::read(actual).unwrap(), b"must not change");
}

#[test]
fn cleanup_refuses_modified_scratch_files_without_deleting_them() {
    let mut fixture = Fixture::new();
    install(&fixture.transaction, &fixture.staged, 1);
    let remaining = &fixture.transaction.manifest.entries[1].staged.0;
    fs::write(remaining, b"external scratch content").unwrap();
    let remaining = remaining.clone();
    fixture.release();
    assert!(recover(&fixture.transaction.path, false).is_err());
    fixture.originals();
    assert!(fixture.transaction.flag("rolled-back").unwrap());
    assert_eq!(fs::read(&remaining).unwrap(), b"external scratch content");
    // Explicitly removing the conflicting scratch file makes cleanup resumable.
    fs::remove_file(remaining).unwrap();
    recover(&fixture.transaction.path, false).unwrap();
    fixture.assert_clean();
}

// These fault hooks exist only in the test executable, never the production CLI.
#[test]
fn crash_worker() {
    let Some(root) = std::env::var_os("SB_ROTATE_TEST_JOURNAL_ROOT") else {
        return;
    };
    let phase = std::env::var("SB_ROTATE_TEST_JOURNAL_PHASE").unwrap();
    let root = PathBuf::from(root);
    let (configs, staged) = setup(&root);
    let directories = configs.lock_directories().unwrap();
    let _locks = DirectoryLocks::acquire(directories.clone()).unwrap();
    let transaction = Transaction::prepare(&configs, &staged, &directories).unwrap();
    match phase.as_str() {
        "prepared" => {}
        "started" => install(&transaction, &staged, 0),
        "first" => install(&transaction, &staged, 1),
        "all" => install(&transaction, &staged, 2),
        "committed" => {
            install(&transaction, &staged, 2);
            transaction.set_flag("committed").unwrap();
        }
        "rollback" => {
            install(&transaction, &staged, 2);
            let entry = &transaction.manifest.entries[0];
            let temp = stage(
                &entry.file.0,
                &transaction.original(0, entry).unwrap(),
                &entry.original.state,
            )
            .unwrap();
            fs::rename(temp, &entry.file.0).unwrap();
            sync_directory(entry.file.0.parent().unwrap()).unwrap();
        }
        _ => panic!("unknown crash phase"),
    }
    std::process::exit(86); // no Rust destructors / TempPath cleanup
}

#[test]
fn abrupt_process_exit_is_recoverable_at_each_transaction_boundary() {
    for phase in [
        "prepared",
        "started",
        "first",
        "all",
        "committed",
        "rollback",
    ] {
        let root = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "recovery::tests::crash_worker", "--nocapture"])
            .env("SB_ROTATE_TEST_JOURNAL_ROOT", root.path())
            .env("SB_ROTATE_TEST_JOURNAL_PHASE", phase)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(86),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let journal = pending_journal(root.path()).unwrap().unwrap();
        let preview = recover(&journal, true).unwrap();
        assert!(preview.contains("Preview only"));
        recover(&journal, false).unwrap();
        let expected = if phase == "committed" { "new-" } else { "old-" };
        assert!(
            fs::read_to_string(root.path().join("a.json"))
                .unwrap()
                .contains(&format!("{expected}a"))
        );
        assert!(
            fs::read_to_string(root.path().join("clients/b.json"))
                .unwrap()
                .contains(&format!("{expected}b"))
        );
        assert!(!journal.exists());
        for directory in [root.path().to_path_buf(), root.path().join("clients")] {
            assert!(pending_journal(&directory).unwrap().is_none());
            assert!(fs::read_dir(directory).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            }));
        }
    }
}
