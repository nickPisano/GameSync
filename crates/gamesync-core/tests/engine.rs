//! End-to-end tests for the core engine: the snapshot → modify → restore loop,
//! content-addressed dedup, default excludes, and integrity verification.

use std::fs;
use std::path::{Path, PathBuf};

use gamesync_core::{
    BackupOptions, ConflictChoice, Engine, RestoreOptions, RetentionPolicy, SnapshotKind,
    SyncOutcome,
};

/// Build a throwaway data dir + save dir under a temp root.
fn setup() -> (tempfile::TempDir, Engine, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let save_dir = tmp.path().join("save");
    fs::create_dir_all(&save_dir).unwrap();
    let engine = Engine::open(data_dir).unwrap();
    (tmp, engine, save_dir)
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

/// Count real objects in the CAS (ignoring the .incoming staging dir).
fn cas_object_count(data_dir: &Path) -> usize {
    let store = data_dir.join("store");
    let mut count = 0;
    for entry in walkdir(&store) {
        if entry.is_file() && !entry.to_string_lossy().contains(".incoming") {
            count += 1;
        }
    }
    count
}

fn walkdir(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push(p);
                }
            }
        }
    }
    out
}

#[test]
fn snapshot_restore_roundtrip() {
    let (tmp, engine, save) = setup();
    let data_dir = tmp.path().join("data");

    // Initial save state: two real files + one that should be excluded.
    write(&save.join("a.txt"), "hello");
    write(&save.join("sub/b.bin"), "binary-ish-data");
    write(&save.join("scratch.tmp"), "junk that must be ignored");

    let game = engine.add_manual_game("Test Game", save.clone()).unwrap();

    // v1: excluded .tmp must NOT be captured.
    let v1 = engine
        .backup(
            &game.id,
            BackupOptions {
                kind: SnapshotKind::Manual,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(v1.file_count(), 2, "the .tmp file should be excluded");
    assert!(v1.files.iter().all(|f| !f.rel_path.ends_with(".tmp")));
    assert!(v1.total_size > 0);

    // Mutate: change a, add c, delete b.
    write(&save.join("a.txt"), "hello world");
    write(&save.join("c.txt"), "new file");
    fs::remove_file(save.join("sub/b.bin")).unwrap();

    let v2 = engine
        .backup(
            &game.id,
            BackupOptions {
                kind: SnapshotKind::Manual,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(v2.file_count(), 2);
    assert_eq!(v2.parent.as_deref(), Some(v1.version_id.as_str()));

    // Restore v1 — should bring back the old a.txt, restore b, remove c.
    let restored = engine.restore(&game.id, &v1.version_id, false).unwrap();
    assert_eq!(restored.version_id, v1.version_id);

    assert_eq!(read(&save.join("a.txt")), "hello");
    assert_eq!(read(&save.join("sub/b.bin")), "binary-ish-data");
    assert!(
        !save.join("c.txt").exists(),
        "c.txt should be gone after restoring v1"
    );

    // A pre-restore safety snapshot must have been created automatically.
    let versions = engine.versions(&game.id).unwrap();
    assert_eq!(
        versions.len(),
        3,
        "v1, v2, and the pre-restore safety snapshot"
    );
    assert_eq!(
        versions
            .iter()
            .filter(|v| v.kind == SnapshotKind::PreRestore)
            .count(),
        1
    );

    // Integrity check passes across every stored object.
    let report = engine.verify().unwrap();
    assert!(report.ok(), "integrity problems: {:?}", report.problems);
    assert!(report.objects_checked > 0);

    // Dedup: number of CAS objects equals the number of *distinct* contents.
    // Distinct contents seen across the whole history:
    //   "hello", "binary-ish-data", "hello world", "new file"  => 4
    let mut hashes: std::collections::HashSet<String> = std::collections::HashSet::new();
    for v in &versions {
        for f in &v.files {
            hashes.insert(f.hash.clone());
        }
    }
    assert_eq!(hashes.len(), 4, "expected 4 distinct contents");
    assert_eq!(
        cas_object_count(&data_dir),
        hashes.len(),
        "each distinct content should be stored exactly once"
    );
}

#[test]
fn restore_into_empty_folder() {
    // Simulates restoring onto a fresh machine where the save folder is empty.
    let (_tmp, engine, save) = setup();
    write(&save.join("save01.dat"), "progress");
    let game = engine.add_manual_game("Fresh", save.clone()).unwrap();
    let v1 = engine.backup(&game.id, BackupOptions::default()).unwrap();

    // Wipe the live folder entirely.
    fs::remove_dir_all(&save).unwrap();
    assert!(!save.exists());

    // Restore without a safety snapshot (nothing to save).
    engine
        .restore_with(
            &game.id,
            &v1.version_id,
            RestoreOptions {
                safety_snapshot: false,
            },
        )
        .unwrap();

    assert_eq!(read(&save.join("save01.dat")), "progress");
}

#[test]
fn prune_and_gc_reclaim_unreferenced_objects() {
    let (tmp, engine, save) = setup();
    let data_dir = tmp.path().join("data");
    let file = save.join("save.dat");

    let game = engine.add_manual_game("Pruner", save.clone()).unwrap();

    // Three versions, each with distinct contents => three distinct objects.
    write(&file, "AAAA");
    let _v1 = engine.backup(&game.id, BackupOptions::default()).unwrap();
    write(&file, "BBBB");
    let _v2 = engine.backup(&game.id, BackupOptions::default()).unwrap();
    write(&file, "CCCC");
    let v3 = engine.backup(&game.id, BackupOptions::default()).unwrap();

    assert_eq!(cas_object_count(&data_dir), 3);
    assert_eq!(engine.versions(&game.id).unwrap().len(), 3);

    // Keep only the newest version; disable the time window so age doesn't save them.
    let policy = RetentionPolicy {
        keep_last: 1,
        keep_days: None,
    };
    let (deleted, gc) = engine.prune(&game.id, &policy).unwrap();

    assert_eq!(deleted.len(), 2, "v1 and v2 records removed");
    assert_eq!(gc.objects_deleted, 2, "AAAA and BBBB objects reclaimed");
    assert_eq!(cas_object_count(&data_dir), 1, "only CCCC remains");

    let remaining = engine.versions(&game.id).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].version_id, v3.version_id);

    // The surviving version is still fully intact.
    assert!(engine.verify().unwrap().ok());
}

#[test]
fn diff_between_versions() {
    let (_tmp, engine, save) = setup();
    write(&save.join("keep.dat"), "same");
    write(&save.join("change.dat"), "v1");
    write(&save.join("gone.dat"), "bye");
    let game = engine.add_manual_game("Differ", save.clone()).unwrap();
    let v1 = engine.backup(&game.id, BackupOptions::default()).unwrap();

    write(&save.join("change.dat"), "v2");
    fs::remove_file(save.join("gone.dat")).unwrap();
    write(&save.join("added.dat"), "new");
    let v2 = engine.backup(&game.id, BackupOptions::default()).unwrap();

    let d = engine
        .diff(&game.id, &v1.version_id, &v2.version_id)
        .unwrap();
    assert_eq!(d.added, vec!["added.dat"]);
    assert_eq!(d.removed, vec!["gone.dat"]);
    assert_eq!(d.modified, vec!["change.dat"]);
    assert_eq!(d.unchanged, 1);
}

#[test]
fn encrypted_store_roundtrip_and_at_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let save = tmp.path().join("save");
    fs::create_dir_all(&save).unwrap();

    // Enable encryption on a fresh store, then unlock it.
    let recovery = Engine::init_encryption(&data_dir, "hunter2-correct-horse").unwrap();
    assert!(Engine::is_encrypted(&data_dir));
    // A locked store can't be opened plaintext.
    assert!(Engine::open(data_dir.clone()).is_err());

    let engine = Engine::unlock(data_dir.clone(), "hunter2-correct-horse").unwrap();

    let secret = "PLAYER_GOLD=999999;BOSS_KILLED=true";
    write(&save.join("profile.sav"), secret);
    let game = engine
        .add_manual_game("Encrypted RPG", save.clone())
        .unwrap();
    let v1 = engine.backup(&game.id, BackupOptions::default()).unwrap();
    assert_eq!(v1.file_count(), 1);

    // The plaintext must NOT appear anywhere in the on-disk store.
    let mut found_plaintext = false;
    for obj in walkdir(&data_dir.join("store")) {
        let bytes = fs::read(&obj).unwrap();
        if bytes.windows(secret.len()).any(|w| w == secret.as_bytes()) {
            found_plaintext = true;
        }
        // Encrypted objects carry the GSE1 header.
        if !obj.to_string_lossy().contains(".incoming") {
            assert_eq!(&bytes[..4], b"GSE1", "object should be encrypted");
        }
    }
    assert!(!found_plaintext, "plaintext leaked into the store!");

    // Integrity (decrypt + re-hash) passes.
    assert!(engine.verify().unwrap().ok());

    // Restore decrypts correctly.
    fs::remove_dir_all(&save).unwrap();
    engine
        .restore_with(
            &game.id,
            &v1.version_id,
            RestoreOptions {
                safety_snapshot: false,
            },
        )
        .unwrap();
    assert_eq!(read(&save.join("profile.sav")), secret);

    // The recovery key opens the same store independently.
    drop(engine);
    let via_recovery = Engine::unlock_with_recovery(data_dir.clone(), &recovery.0).unwrap();
    assert!(via_recovery.verify().unwrap().ok());

    // Wrong passphrase is rejected.
    assert!(Engine::unlock(data_dir, "wrong-pass").is_err());
}

#[test]
fn sync_two_devices_push_pull_and_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let remote = tmp.path().join("remote"); // a shared folder (think Dropbox/Drive)
    let a_save = tmp.path().join("A/save");
    let b_save = tmp.path().join("B/save");
    fs::create_dir_all(&a_save).unwrap();
    fs::create_dir_all(&b_save).unwrap();

    let a = Engine::open(tmp.path().join("A/data")).unwrap();
    let b = Engine::open(tmp.path().join("B/data")).unwrap();
    a.set_remote(&remote).unwrap();
    b.set_remote(&remote).unwrap();

    // Device A: create save, back up, push.
    write(&a_save.join("save.dat"), "A-v1");
    let ga = a.add_manual_game("Shared RPG", a_save.clone()).unwrap();
    a.backup(&ga.id, BackupOptions::default()).unwrap();
    assert!(matches!(
        a.sync_game(&ga.id).unwrap(),
        SyncOutcome::Pushed { .. }
    ));

    // Device B: same game (stable id) pulls A's save.
    let gb = b.add_manual_game("Shared RPG", b_save.clone()).unwrap();
    assert_eq!(ga.id, gb.id, "manual ids must be stable across devices");
    assert!(matches!(
        b.sync_game(&gb.id).unwrap(),
        SyncOutcome::Pulled { .. }
    ));
    assert_eq!(read(&b_save.join("save.dat")), "A-v1");

    // B advances and pushes; A fast-forwards.
    write(&b_save.join("save.dat"), "B-v2");
    b.backup(&gb.id, BackupOptions::default()).unwrap();
    assert!(matches!(
        b.sync_game(&gb.id).unwrap(),
        SyncOutcome::Pushed { .. }
    ));
    assert!(matches!(
        a.sync_game(&ga.id).unwrap(),
        SyncOutcome::Pulled { .. }
    ));
    assert_eq!(read(&a_save.join("save.dat")), "B-v2");
    assert_eq!(a.sync_game(&ga.id).unwrap(), SyncOutcome::InSync);

    // ---- conflict: both edit from the same base, offline ----
    write(&a_save.join("save.dat"), "A-offline");
    a.backup(&ga.id, BackupOptions::default()).unwrap();
    write(&b_save.join("save.dat"), "B-offline");
    b.backup(&gb.id, BackupOptions::default()).unwrap();

    // A syncs first (pushes its branch). B then sees a true conflict.
    assert!(matches!(
        a.sync_game(&ga.id).unwrap(),
        SyncOutcome::Pushed { .. }
    ));
    let conflict = b.sync_game(&gb.id).unwrap();
    assert!(matches!(conflict, SyncOutcome::Conflict { .. }));
    assert_eq!(
        read(&b_save.join("save.dat")),
        "B-offline",
        "a conflict must never overwrite the live save"
    );

    // The remote version is pulled into local history on conflict, so the UI's
    // diff preview works offline against the two diverging versions.
    if let SyncOutcome::Conflict { local, remote } = &conflict {
        let d = b.diff(&gb.id, local, remote).unwrap();
        assert!(!d.is_empty(), "conflict diff should show the divergence");
    }

    // B keeps its own side; the resolution supersedes both branches.
    assert!(matches!(
        b.resolve_conflict(&gb.id, ConflictChoice::KeepLocal)
            .unwrap(),
        SyncOutcome::Pushed { .. }
    ));
    assert_eq!(read(&b_save.join("save.dat")), "B-offline");

    // A converges onto the resolved version.
    assert!(matches!(
        a.sync_game(&ga.id).unwrap(),
        SyncOutcome::Pulled { .. }
    ));
    assert_eq!(read(&a_save.join("save.dat")), "B-offline");

    assert_eq!(a.sync_game(&ga.id).unwrap(), SyncOutcome::InSync);
    assert_eq!(b.sync_game(&gb.id).unwrap(), SyncOutcome::InSync);
    assert!(a.verify().unwrap().ok());
    assert!(b.verify().unwrap().ok());
}

#[test]
fn backup_if_changed_skips_identical_state() {
    let (_tmp, engine, save) = setup();
    write(&save.join("s.dat"), "same");
    let game = engine.add_manual_game("Idem", save.clone()).unwrap();

    // First call snapshots; second (no change) is a no-op.
    assert!(engine.backup_if_changed(&game.id).unwrap().is_some());
    assert!(engine.backup_if_changed(&game.id).unwrap().is_none());
    assert_eq!(engine.versions(&game.id).unwrap().len(), 1);

    // After a real change it snapshots again.
    write(&save.join("s.dat"), "different");
    assert!(engine.backup_if_changed(&game.id).unwrap().is_some());
    assert_eq!(engine.versions(&game.id).unwrap().len(), 2);
}

#[test]
fn auto_sync_pass_pushes_then_pulls() {
    let tmp = tempfile::tempdir().unwrap();
    let remote = tmp.path().join("remote");
    let a_save = tmp.path().join("A/save");
    let b_save = tmp.path().join("B/save");
    fs::create_dir_all(&a_save).unwrap();
    fs::create_dir_all(&b_save).unwrap();

    let a = Engine::open(tmp.path().join("A/data")).unwrap();
    let b = Engine::open(tmp.path().join("B/data")).unwrap();
    a.set_remote(&remote).unwrap();
    b.set_remote(&remote).unwrap();

    write(&a_save.join("p.sav"), "v1");
    let ga = a.add_manual_game("Auto", a_save.clone()).unwrap();
    a.set_sync_enabled(&ga.id, true).unwrap();
    let gb = b.add_manual_game("Auto", b_save.clone()).unwrap();
    b.set_sync_enabled(&gb.id, true).unwrap();

    // Device A: auto pass snapshots the new save and pushes it.
    let ra = a.auto_sync_pass().unwrap();
    assert_eq!(ra.backed_up, 1);
    assert_eq!(ra.pushed, 1);
    assert!(ra.conflicts.is_empty() && ra.errors.is_empty());

    // Device B: auto pass pulls A's save into place.
    let rb = b.auto_sync_pass().unwrap();
    assert_eq!(rb.pulled, 1);
    assert_eq!(read(&b_save.join("p.sav")), "v1");
}

fn rclone_available() -> bool {
    std::process::Command::new("rclone")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn rclone_transport_push_pull() {
    if !rclone_available() {
        eprintln!("skipping rclone_transport_push_pull: rclone not installed");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    // A bare local path is rclone's "local" backend — exercises the transport
    // end-to-end without any cloud account.
    let remote_dir = tmp.path().join("cloud");
    fs::create_dir_all(&remote_dir).unwrap();
    let spec = format!("rclone:{}", remote_dir.display());

    let a_save = tmp.path().join("A/save");
    let b_save = tmp.path().join("B/save");
    fs::create_dir_all(&a_save).unwrap();
    fs::create_dir_all(&b_save).unwrap();

    let a = Engine::open(tmp.path().join("A/data")).unwrap();
    let b = Engine::open(tmp.path().join("B/data")).unwrap();
    a.set_remote(Path::new(&spec)).unwrap();
    b.set_remote(Path::new(&spec)).unwrap();

    write(&a_save.join("p.sav"), "rclone-v1");
    let ga = a.add_manual_game("Rcl", a_save.clone()).unwrap();
    a.backup(&ga.id, BackupOptions::default()).unwrap();
    assert!(matches!(
        a.sync_game(&ga.id).unwrap(),
        SyncOutcome::Pushed { .. }
    ));

    let gb = b.add_manual_game("Rcl", b_save.clone()).unwrap();
    assert!(matches!(
        b.sync_game(&gb.id).unwrap(),
        SyncOutcome::Pulled { .. }
    ));
    assert_eq!(read(&b_save.join("p.sav")), "rclone-v1");
    assert!(b.verify().unwrap().ok());
}

#[test]
fn lan_transport_push_pull_over_loopback() {
    let tmp = tempfile::tempdir().unwrap();
    // Host serves this directory; device A syncs into it directly (folder),
    // device B reaches it over TCP via the LAN transport.
    let share = tmp.path().join("share");
    std::fs::create_dir_all(&share).unwrap();
    let server = Engine::serve_lan(share.clone(), "secret-token", "127.0.0.1:0").unwrap();
    let lan_spec = format!("lan:secret-token@127.0.0.1:{}", server.port);

    let a_save = tmp.path().join("A/save");
    let b_save = tmp.path().join("B/save");
    fs::create_dir_all(&a_save).unwrap();
    fs::create_dir_all(&b_save).unwrap();

    let a = Engine::open(tmp.path().join("A/data")).unwrap();
    let b = Engine::open(tmp.path().join("B/data")).unwrap();
    a.set_remote(&share).unwrap(); // host: folder access to the shared store
    b.set_remote(Path::new(&lan_spec)).unwrap(); // peer: over the network

    // A publishes via the folder; B pulls it over LAN.
    write(&a_save.join("p.sav"), "lan-v1");
    let ga = a.add_manual_game("Lan", a_save.clone()).unwrap();
    a.backup(&ga.id, BackupOptions::default()).unwrap();
    assert!(matches!(
        a.sync_game(&ga.id).unwrap(),
        SyncOutcome::Pushed { .. }
    ));

    let gb = b.add_manual_game("Lan", b_save.clone()).unwrap();
    assert!(matches!(
        b.sync_game(&gb.id).unwrap(),
        SyncOutcome::Pulled { .. }
    ));
    assert_eq!(read(&b_save.join("p.sav")), "lan-v1");

    // B pushes a change over LAN (exercises put_object/put_version/set_head/lock);
    // A picks it up via the folder.
    write(&b_save.join("p.sav"), "lan-v2");
    b.backup(&gb.id, BackupOptions::default()).unwrap();
    assert!(matches!(
        b.sync_game(&gb.id).unwrap(),
        SyncOutcome::Pushed { .. }
    ));
    assert!(matches!(
        a.sync_game(&ga.id).unwrap(),
        SyncOutcome::Pulled { .. }
    ));
    assert_eq!(read(&a_save.join("p.sav")), "lan-v2");

    // Wrong token is rejected.
    let bad = Engine::open(tmp.path().join("C/data")).unwrap();
    let bad_spec = format!("lan:wrong@127.0.0.1:{}", server.port);
    bad.set_remote(Path::new(&bad_spec)).unwrap();
    let gc = bad.add_manual_game("Lan", a_save.clone()).unwrap();
    assert!(bad.sync_game(&gc.id).is_err());

    server.stop();
}

#[test]
fn lists_save_files_with_excludes() {
    let (_tmp, engine, save) = setup();
    write(&save.join("a.sav"), "x");
    write(&save.join("sub/b.dat"), "yy");
    write(&save.join("junk.tmp"), "z"); // excluded by default
    let game = engine.add_manual_game("Files", save.clone()).unwrap();

    let files = engine.list_save_files(&game.id).unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();
    assert!(paths.contains(&"a.sav"));
    assert!(paths.contains(&"sub/b.dat"));
    assert!(
        !paths.iter().any(|p| p.ends_with(".tmp")),
        "tmp must be excluded"
    );
    // absolute paths resolve under the save root
    assert!(files.iter().all(|f| f
        .abs_path
        .ends_with(&f.rel_path.replace('/', std::path::MAIN_SEPARATOR_STR))));
}

#[test]
fn process_running_detection() {
    use std::path::PathBuf;
    let cur = std::env::current_exe().unwrap();
    let here = cur.parent().unwrap().to_path_buf();
    let res = gamesync_core::process::running_install_dirs(&[
        here,
        PathBuf::from("/no/such/dir/xyzzy123"),
    ]);
    assert_eq!(res.len(), 2);
    assert!(!res[1], "a bogus dir must not look like a running game");
    assert!(
        res[0],
        "the test binary runs from this dir, so it should be detected"
    );
}

#[test]
fn running_game_ids_detects_known_install_dir() {
    let (tmp, engine, _save) = setup();
    let here = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let game = gamesync_core::Game {
        id: "steam:test".into(),
        name: "Running Game".into(),
        platform: gamesync_core::Platform::Steam,
        save_root: tmp.path().join("rg"),
        install_dir: Some(here),
        includes: vec!["**".into()],
        excludes: vec![],
        sync_enabled: false,
    };
    engine.db.upsert_game(&game).unwrap();
    assert!(engine
        .running_game_ids()
        .unwrap()
        .contains(&"steam:test".to_string()));
}

#[test]
fn set_game_exe_enables_exit_detection() {
    let (_tmp, engine, save) = setup();
    write(&save.join("s.dat"), "x");
    let game = engine
        .add_manual_game("Manual With Exe", save.clone())
        .unwrap();
    // Manual games have no install dir, so they're not in the running set.
    assert!(!engine.running_game_ids().unwrap().contains(&game.id));

    // Point it at where the test binary runs → now detectable as "running".
    let here = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    engine.set_game_exe(&game.id, Some(here.clone())).unwrap();
    assert_eq!(engine.get_game(&game.id).unwrap().install_dir, Some(here));
    assert!(engine.running_game_ids().unwrap().contains(&game.id));

    // Clearing it removes detection again.
    engine.set_game_exe(&game.id, None).unwrap();
    assert!(!engine.running_game_ids().unwrap().contains(&game.id));
}

#[test]
fn compressed_backup_roundtrip() {
    let (_tmp, engine, save) = setup();
    engine.set_compression(true).unwrap();
    assert!(engine.compression_enabled());

    // Highly compressible content so we can prove it shrank on disk.
    let blob = "A".repeat(20_000);
    write(&save.join("big.sav"), &blob);
    let game = engine.add_manual_game("Zip", save.clone()).unwrap();
    let v = engine.backup(&game.id, BackupOptions::default()).unwrap();

    // Object on disk is much smaller than the 20 KB original.
    let stored = engine.cas.object_size(&v.files[0].hash);
    assert!(
        stored < 2_000,
        "expected compression, object was {stored} bytes"
    );
    assert!(engine.verify().unwrap().ok());

    // Restore decompresses back to the exact original.
    fs::remove_dir_all(&save).unwrap();
    engine
        .restore_with(
            &game.id,
            &v.version_id,
            RestoreOptions {
                safety_snapshot: false,
            },
        )
        .unwrap();
    assert_eq!(read(&save.join("big.sav")), blob);
}

#[test]
fn compression_locked_once_data_exists() {
    let (_tmp, engine, save) = setup();
    write(&save.join("s.dat"), "x");
    let game = engine.add_manual_game("Late", save.clone()).unwrap();
    engine.backup(&game.id, BackupOptions::default()).unwrap();
    assert!(
        engine.set_compression(true).is_err(),
        "can't toggle once backups exist"
    );
}

#[test]
fn redirect_save_folder_via_symlink() {
    let (tmp, engine, save) = setup();
    write(&save.join("hero.sav"), "progress");
    write(&save.join("sub/x.dat"), "more");
    let game = engine.add_manual_game("Redir", save.clone()).unwrap();

    let cloud = tmp.path().join("Cloud");
    fs::create_dir_all(&cloud).unwrap();
    let report = engine
        .redirect_save_folder(&game.id, cloud.clone())
        .unwrap();

    // The original path is now a symlink the game still reads through.
    assert!(fs::symlink_metadata(&save)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(read(&save.join("hero.sav")), "progress");

    // The data physically lives in the target, which GameSync now tracks.
    let target = cloud.join("Redir");
    assert_eq!(read(&target.join("hero.sav")), "progress");
    assert_eq!(read(&target.join("sub/x.dat")), "more");
    assert_eq!(engine.get_game(&game.id).unwrap().save_root, target);

    // The original is preserved (never deleted), and a safety backup was taken.
    let orig = std::path::Path::new(&report.original_backup);
    assert!(orig.is_dir());
    assert_eq!(read(&orig.join("hero.sav")), "progress");
    assert!(engine
        .versions(&game.id)
        .unwrap()
        .iter()
        .any(|v| v.label.as_deref() == Some("before redirecting save folder")));
}

#[test]
fn remove_and_rename_game() {
    let (tmp, engine, save) = setup();
    let data_dir = tmp.path().join("data");
    write(&save.join("s.dat"), "x");
    let game = engine.add_manual_game("Old Name", save.clone()).unwrap();
    engine.backup(&game.id, BackupOptions::default()).unwrap();
    assert_eq!(cas_object_count(&data_dir), 1);

    engine.rename_game(&game.id, "New Name").unwrap();
    assert_eq!(engine.get_game(&game.id).unwrap().name, "New Name");

    let gc = engine.remove_game(&game.id).unwrap();
    assert!(engine.get_game(&game.id).is_err(), "game should be gone");
    assert!(engine.list_games().unwrap().is_empty());
    assert_eq!(gc.objects_deleted, 1, "its sole object should be reclaimed");
    assert_eq!(cas_object_count(&data_dir), 0);
}

#[test]
fn plugin_listing_and_toggle() {
    let (tmp, engine, _save) = setup();
    let plugins_dir = tmp.path().join("data/plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    std::fs::write(
        plugins_dir.join("games.json"),
        r#"{ "name": "Extra", "games": { "steam:42": { "name": "Q", "paths": ["{HOME}/q"] } },
            "viewers": [ { "name": "Hex", "match": "*.sav", "command": "x {file}" } ] }"#,
    )
    .unwrap();

    let list = engine.list_plugins().unwrap();
    assert_eq!(list.plugins.len(), 1);
    assert_eq!(list.plugins[0].games, 1);
    assert_eq!(list.plugins[0].viewers, 1);
    assert!(list.plugins[0].enabled);
    assert!(
        !list.commands_allowed,
        "commands are opt-in, off by default"
    );

    engine.set_plugin_enabled("games", false).unwrap();
    assert!(!engine.list_plugins().unwrap().plugins[0].enabled);
}

#[test]
fn plugin_hooks_run_only_when_commands_allowed() {
    let (tmp, engine, save) = setup();
    let plugins_dir = tmp.path().join("data/plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let marker = tmp.path().join("hook-ran.txt");
    std::fs::write(
        plugins_dir.join("hooky.json"),
        format!(
            r#"{{ "name": "Hooky", "hooks": {{ "post_backup": "touch '{}'" }} }}"#,
            marker.display()
        ),
    )
    .unwrap();

    write(&save.join("s.dat"), "x");
    let game = engine.add_manual_game("Hooked", save.clone()).unwrap();

    // Disabled by default → hook must not run.
    engine.backup(&game.id, BackupOptions::default()).unwrap();
    assert!(
        !marker.exists(),
        "hook must not run while commands are disabled"
    );

    // Opt in → hook runs.
    engine.set_commands_allowed(true).unwrap();
    write(&save.join("s.dat"), "y");
    engine.backup(&game.id, BackupOptions::default()).unwrap();
    assert!(marker.exists(), "hook should run once commands are allowed");
}

#[test]
fn sync_toggle_persists_across_rescan() {
    let (_tmp, engine, save) = setup();
    write(&save.join("a.sav"), "x");
    let game = engine.add_manual_game("Toggle", save).unwrap();

    engine.set_sync_enabled(&game.id, true).unwrap();
    let reloaded = engine.get_game(&game.id).unwrap();
    assert!(reloaded.sync_enabled);
}

// ---------------------------------------------------------------------------
// Fault-injection tests for snapshot/restore.
//
// The whole point of the app is to never lose a save and never half-overwrite
// the live one. These tests deliberately damage the content-addressed store
// (corrupt or delete the objects a version depends on) and assert the engine
// *fails closed*: it aborts at the verify gate before touching the live save,
// leaves no half-finished temp dirs behind, keeps the automatic pre-restore
// snapshot as a usable recovery point, and surfaces the damage via `verify`.
// ---------------------------------------------------------------------------

/// Absolute path of a stored CAS object (mirrors `Cas::object_path`).
fn object_file(data_dir: &Path, hash: &str) -> PathBuf {
    data_dir.join("store").join(&hash[..2]).join(hash)
}

/// Restore staging/rollback dirs that must never outlive a restore attempt.
fn leftover_restore_temps(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(".gamesync-staging-") || name.starts_with(".gamesync-old-") {
                out.push(name);
            }
        }
    }
    out
}

#[test]
fn restore_aborts_on_corrupt_object_and_preserves_live_save() {
    let (tmp, engine, save) = setup();
    let data_dir = tmp.path().join("data");
    let hero = save.join("hero.sav");

    write(&hero, "good-v1");
    let game = engine.add_manual_game("Faulty", save.clone()).unwrap();
    let v1 = engine.backup(&game.id, BackupOptions::default()).unwrap();

    // The live save moves on; this is the state a restore must never trash.
    write(&hero, "current-state");

    // Bit-rot: scribble over the v1 object so its bytes no longer match the hash.
    let obj = object_file(&data_dir, &v1.files[0].hash);
    assert!(obj.is_file(), "v1 object should exist before we corrupt it");
    fs::write(&obj, b"this is not the original content").unwrap();

    // Restore must fail closed at the staging/verify gate.
    let err = engine
        .restore(&game.id, &v1.version_id, false)
        .expect_err("restoring a corrupt version must fail");
    assert!(
        err.to_string().contains("checksum"),
        "expected a checksum failure, got: {err}"
    );

    // The live save is exactly as it was — the swap never happened.
    assert_eq!(read(&hero), "current-state");
    // No half-finished staging/old directories leaked next to the save.
    assert!(
        leftover_restore_temps(save.parent().unwrap()).is_empty(),
        "restore must clean up its temp dirs on failure"
    );
}

#[test]
fn restore_aborts_on_missing_object_and_preserves_live_save() {
    let (tmp, engine, save) = setup();
    let data_dir = tmp.path().join("data");
    let hero = save.join("hero.sav");

    write(&hero, "good-v1");
    let game = engine.add_manual_game("Gappy", save.clone()).unwrap();
    let v1 = engine.backup(&game.id, BackupOptions::default()).unwrap();

    write(&hero, "current-state");

    // The object vanishes (partial sync, manual delete, disk loss).
    fs::remove_file(object_file(&data_dir, &v1.files[0].hash)).unwrap();

    let err = engine
        .restore(&game.id, &v1.version_id, false)
        .expect_err("restoring a version with a missing object must fail");
    assert!(
        err.to_string().contains("missing object"),
        "expected a missing-object error, got: {err}"
    );

    assert_eq!(read(&hero), "current-state");
    assert!(leftover_restore_temps(save.parent().unwrap()).is_empty());
}

#[test]
fn failed_restore_remains_recoverable_via_safety_snapshot() {
    // Even when the *target* version is unrecoverable, the pre-restore safety
    // snapshot taken automatically before the attempt is a valid recovery point.
    let (tmp, engine, save) = setup();
    let data_dir = tmp.path().join("data");
    let hero = save.join("hero.sav");

    write(&hero, "good-v1");
    let game = engine.add_manual_game("Recoverable", save.clone()).unwrap();
    let v1 = engine.backup(&game.id, BackupOptions::default()).unwrap();

    // Advance the live save, then corrupt the target so the restore fails
    // *after* it has already captured a safety snapshot of this state.
    write(&hero, "precious-current");
    fs::write(object_file(&data_dir, &v1.files[0].hash), b"corrupted").unwrap();
    assert!(engine.restore(&game.id, &v1.version_id, false).is_err());

    // A pre-restore safety snapshot of "precious-current" must now exist.
    let safety = engine
        .versions(&game.id)
        .unwrap()
        .into_iter()
        .find(|v| v.kind == SnapshotKind::PreRestore)
        .expect("a pre-restore safety snapshot should have been captured");

    // Simulate further loss of the live folder, then recover from the safety
    // snapshot — the "undo a restore" path must still work end to end.
    fs::remove_dir_all(&save).unwrap();
    engine
        .restore_with(
            &game.id,
            &safety.version_id,
            RestoreOptions {
                safety_snapshot: false,
            },
        )
        .unwrap();
    assert_eq!(read(&hero), "precious-current");
}

#[test]
fn verify_detects_corrupt_and_missing_objects() {
    let (tmp, engine, save) = setup();
    let data_dir = tmp.path().join("data");
    write(&save.join("a.sav"), "alpha");
    write(&save.join("b.sav"), "bravo");
    let game = engine.add_manual_game("Scanned", save.clone()).unwrap();
    let v1 = engine.backup(&game.id, BackupOptions::default()).unwrap();

    assert!(engine.verify().unwrap().ok(), "store should start healthy");

    // Corrupt one object, delete another — verify must flag both by path.
    let hash_of = |p: &str| {
        v1.files
            .iter()
            .find(|f| f.rel_path == p)
            .unwrap()
            .hash
            .clone()
    };
    fs::write(object_file(&data_dir, &hash_of("a.sav")), b"rot").unwrap();
    fs::remove_file(object_file(&data_dir, &hash_of("b.sav"))).unwrap();

    let report = engine.verify().unwrap();
    assert!(!report.ok(), "verify must flag the damaged store");
    let damaged: Vec<&str> = report.problems.iter().map(|(_, p, _)| p.as_str()).collect();
    assert!(damaged.contains(&"a.sav"), "corrupt object not reported");
    assert!(damaged.contains(&"b.sav"), "missing object not reported");
}

#[test]
fn tampered_encrypted_object_is_rejected_by_verify_and_restore() {
    // AEAD authentication must catch tampering with an encrypted object: both
    // the integrity scan and a restore have to refuse the poisoned bytes.
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let save = tmp.path().join("save");
    fs::create_dir_all(&save).unwrap();

    Engine::init_encryption(&data_dir, "correct horse battery staple").unwrap();
    let engine = Engine::unlock(data_dir.clone(), "correct horse battery staple").unwrap();

    write(&save.join("profile.sav"), "HP=100;LEVEL=7");
    let game = engine.add_manual_game("Crypted", save.clone()).unwrap();
    let v1 = engine.backup(&game.id, BackupOptions::default()).unwrap();
    assert!(engine.verify().unwrap().ok());

    // Flip a byte inside the encrypted blob (GSE1 || nonce || ciphertext+tag).
    let obj = object_file(&data_dir, &v1.files[0].hash);
    let mut bytes = fs::read(&obj).unwrap();
    *bytes.last_mut().unwrap() ^= 0xff;
    fs::write(&obj, &bytes).unwrap();

    assert!(
        !engine.verify().unwrap().ok(),
        "AEAD must catch the tampered object"
    );

    write(&save.join("profile.sav"), "HP=1;LEVEL=99");
    assert!(
        engine.restore(&game.id, &v1.version_id, false).is_err(),
        "restore must refuse a tampered encrypted object"
    );
    assert_eq!(
        read(&save.join("profile.sav")),
        "HP=1;LEVEL=99",
        "the live save must be untouched after a refused restore"
    );
}
