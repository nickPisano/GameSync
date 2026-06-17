//! `gamesync` — a thin command-line driver over `gamesync-core`.
//!
//! This exists to exercise and demonstrate the engine end-to-end before the
//! Tauri UI is built. Every command maps directly to an `Engine` method.

use std::path::PathBuf;
use std::process::exit;

use gamesync_core::{
    BackupOptions, ConflictChoice, Engine, RetentionPolicy, SnapshotKind, SyncOutcome,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");

    let result = match cmd {
        "scan" => cmd_scan(),
        "update-list" => cmd_update_list(),
        "list" | "ls" => cmd_list(),
        "add" => cmd_add(&args[1..]),
        "enable" => cmd_set_enabled(&args[1..], true),
        "disable" => cmd_set_enabled(&args[1..], false),
        "backup" => cmd_backup(&args[1..]),
        "versions" => cmd_versions(&args[1..]),
        "diff" => cmd_diff(&args[1..]),
        "restore" => cmd_restore(&args[1..]),
        "restore-latest" => cmd_restore_latest(&args[1..]),
        "prune" => cmd_prune(&args[1..]),
        "gc" => cmd_gc(),
        "remote" => cmd_remote(&args[1..]),
        "sync" => cmd_sync(&args[1..]),
        "resolve" => cmd_resolve(&args[1..]),
        "serve-lan" => cmd_serve_lan(&args[1..]),
        "verify" => cmd_verify(),
        "encrypt-init" => cmd_encrypt_init(),
        "encrypt-status" => cmd_encrypt_status(),
        "where" => cmd_where(),
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        other => Err(format!("unknown command: {other}\n\nrun `gamesync help`")),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        exit(1);
    }
}

fn engine() -> Result<Engine, String> {
    let dir = Engine::default_data_dir();
    if Engine::is_encrypted(&dir) {
        if let Ok(pass) = std::env::var("GAMESYNC_PASSPHRASE") {
            return Engine::unlock(dir, &pass).map_err(|e| e.to_string());
        }
        if let Ok(recovery) = std::env::var("GAMESYNC_RECOVERY") {
            return Engine::unlock_with_recovery(dir, &recovery).map_err(|e| e.to_string());
        }
        return Err(
            "store is encrypted — set GAMESYNC_PASSPHRASE (or GAMESYNC_RECOVERY) to unlock"
                .to_string(),
        );
    }
    Engine::open(dir).map_err(|e| e.to_string())
}

fn cmd_where() -> Result<(), String> {
    println!("{}", Engine::default_data_dir().display());
    Ok(())
}

fn cmd_update_list() -> Result<(), String> {
    let eng = engine()?;
    eprintln!("Downloading the community game list…");
    let n = eng.update_game_list().map_err(|e| e.to_string())?;
    println!("Updated — GameSync now auto-detects {n} games.");
    Ok(())
}

fn cmd_scan() -> Result<(), String> {
    let eng = engine()?;
    let found = eng.scan_all().map_err(|e| e.to_string())?;
    if found.is_empty() {
        println!("No matching Steam games found.");
        println!(
            "(No Steam install detected, or none of the installed games are in the manifest.)"
        );
        println!("Use `gamesync add \"<name>\" <save-folder>` to add one manually.");
        return Ok(());
    }
    println!("Detected {} game(s):", found.len());
    for g in found {
        println!("  {}  {}", g.id, g.name);
        println!("      saves: {}", g.save_root.display());
    }
    Ok(())
}

fn cmd_list() -> Result<(), String> {
    let eng = engine()?;
    let games = eng.list_games().map_err(|e| e.to_string())?;
    if games.is_empty() {
        println!("No games tracked yet. Run `gamesync scan` or `gamesync add`.");
        return Ok(());
    }
    for g in games {
        let versions = eng.versions(&g.id).map_err(|e| e.to_string())?;
        let last = versions
            .first()
            .map(|v| humanize_ago(v.created_ms))
            .unwrap_or_else(|| "never".to_string());
        let sync = if g.sync_enabled { "on " } else { "off" };
        println!(
            "[{sync}] {:<22} {:<28} versions: {:<3} last backup: {}",
            g.id,
            truncate(&g.name, 28),
            versions.len(),
            last
        );
    }
    Ok(())
}

fn cmd_add(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("usage: gamesync add \"<name>\" <save-folder>".to_string());
    }
    let name = &args[0];
    let path = PathBuf::from(&args[1]);
    let eng = engine()?;
    let game = eng.add_manual_game(name, path).map_err(|e| e.to_string())?;
    println!("Added {} ({})", game.name, game.id);
    Ok(())
}

fn cmd_set_enabled(args: &[String], enabled: bool) -> Result<(), String> {
    let id = args
        .first()
        .ok_or("usage: gamesync enable|disable <game_id>")?;
    let eng = engine()?;
    eng.set_sync_enabled(id, enabled)
        .map_err(|e| e.to_string())?;
    println!(
        "sync {} for {id}",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

fn cmd_backup(args: &[String]) -> Result<(), String> {
    let id = args
        .first()
        .ok_or("usage: gamesync backup <game_id> [--force] [--wait] [--label \"...\"]")?;
    let force = has_flag(args, "--force");
    let wait = has_flag(args, "--wait");
    let label = flag_value(args, "--label");
    let eng = engine()?;
    let snap = eng
        .backup(
            id,
            BackupOptions {
                force,
                wait_quiescent: wait,
                kind: SnapshotKind::Manual,
                label,
            },
        )
        .map_err(|e| e.to_string())?;
    println!(
        "Backed up {} file(s), {} -> version {}",
        snap.file_count(),
        human_size(snap.total_size),
        &snap.version_id
    );
    Ok(())
}

fn cmd_versions(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("usage: gamesync versions <game_id>")?;
    let eng = engine()?;
    let versions = eng.versions(id).map_err(|e| e.to_string())?;
    if versions.is_empty() {
        println!("No versions for {id}.");
        return Ok(());
    }
    println!("{} version(s) for {id} (newest first):", versions.len());
    for v in versions {
        println!(
            "  {}  {:<11} {:>4} files  {:>9}  {}{}",
            v.version_id,
            v.kind.as_str(),
            v.file_count(),
            human_size(v.total_size),
            humanize_ago(v.created_ms),
            v.label.map(|l| format!("  — {l}")).unwrap_or_default(),
        );
    }
    Ok(())
}

fn cmd_diff(args: &[String]) -> Result<(), String> {
    if args.len() < 3 {
        return Err("usage: gamesync diff <game_id> <from_version> <to_version>".to_string());
    }
    let eng = engine()?;
    let d = eng
        .diff(&args[0], &args[1], &args[2])
        .map_err(|e| e.to_string())?;
    if d.is_empty() {
        println!("No differences ({} unchanged files).", d.unchanged);
        return Ok(());
    }
    for p in &d.added {
        println!("  + {p}");
    }
    for p in &d.modified {
        println!("  ~ {p}");
    }
    for p in &d.removed {
        println!("  - {p}");
    }
    println!(
        "{} changed ({} added, {} modified, {} removed), {} unchanged.",
        d.changed_count(),
        d.added.len(),
        d.modified.len(),
        d.removed.len(),
        d.unchanged
    );
    Ok(())
}

fn cmd_prune(args: &[String]) -> Result<(), String> {
    let id = args
        .first()
        .ok_or("usage: gamesync prune <game_id> [--keep N] [--days D]")?;
    let keep_last = flag_value(args, "--keep")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let keep_days = flag_value(args, "--days").and_then(|v| v.parse().ok());
    let eng = engine()?;
    let policy = RetentionPolicy {
        keep_last,
        keep_days,
    };
    let (deleted, gc) = eng.prune(id, &policy).map_err(|e| e.to_string())?;
    println!(
        "Pruned {} version(s); reclaimed {} object(s), {}.",
        deleted.len(),
        gc.objects_deleted,
        human_size(gc.bytes_freed)
    );
    Ok(())
}

fn cmd_gc() -> Result<(), String> {
    let eng = engine()?;
    let report = eng.gc().map_err(|e| e.to_string())?;
    println!(
        "Garbage collection freed {} object(s), {}.",
        report.objects_deleted,
        human_size(report.bytes_freed)
    );
    Ok(())
}

fn cmd_encrypt_status() -> Result<(), String> {
    let dir = Engine::default_data_dir();
    if Engine::is_encrypted(&dir) {
        println!("Encryption: ENABLED (unlock with GAMESYNC_PASSPHRASE or GAMESYNC_RECOVERY).");
    } else {
        println!("Encryption: disabled. Enable on a fresh store with `gamesync encrypt-init`.");
    }
    Ok(())
}

fn cmd_encrypt_init() -> Result<(), String> {
    let passphrase = std::env::var("GAMESYNC_PASSPHRASE")
        .map_err(|_| "set GAMESYNC_PASSPHRASE to the passphrase to protect the store")?;
    if passphrase.len() < 8 {
        return Err("passphrase must be at least 8 characters".to_string());
    }
    let dir = Engine::default_data_dir();
    let recovery = Engine::init_encryption(&dir, &passphrase).map_err(|e| e.to_string())?;
    println!("Encryption enabled.\n");
    println!("RECOVERY KEY (write this down — it is shown only once and can");
    println!("unlock your saves if you forget the passphrase):\n");
    println!("    {}\n", recovery.grouped());
    println!("Without the passphrase AND the recovery key, encrypted saves cannot be recovered.");
    Ok(())
}

fn cmd_restore(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("usage: gamesync restore <game_id> <version_id> [--force]".to_string());
    }
    let force = has_flag(args, "--force");
    let eng = engine()?;
    let snap = eng
        .restore(&args[0], &args[1], force)
        .map_err(|e| e.to_string())?;
    println!(
        "Restored version {} ({} files). A pre-restore safety backup was taken first.",
        snap.version_id,
        snap.file_count()
    );
    Ok(())
}

fn cmd_restore_latest(args: &[String]) -> Result<(), String> {
    let id = args
        .first()
        .ok_or("usage: gamesync restore-latest <game_id> [--force]")?;
    let force = has_flag(args, "--force");
    let eng = engine()?;
    let snap = eng.restore_latest(id, force).map_err(|e| e.to_string())?;
    println!(
        "Restored latest version {} ({} files).",
        snap.version_id,
        snap.file_count()
    );
    Ok(())
}

fn cmd_remote(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("set") => {
            let path = args.get(1).ok_or("usage: gamesync remote set <folder>")?;
            engine()?
                .set_remote(std::path::Path::new(path))
                .map_err(|e| e.to_string())?;
            println!("Remote set to {path}");
            Ok(())
        }
        Some("status") | None => {
            match engine()?.remote_path().map_err(|e| e.to_string())? {
                Some(p) => println!("Remote: {}", p.display()),
                None => {
                    println!("Remote: not configured. Set one with `gamesync remote set <folder>`.")
                }
            }
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown remote subcommand '{other}' (use set|status)"
        )),
    }
}

fn cmd_sync(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("usage: gamesync sync <game_id>")?;
    let eng = engine()?;
    match eng.sync_game(id).map_err(|e| e.to_string())? {
        SyncOutcome::InSync => println!("Already in sync."),
        SyncOutcome::Pushed { version_id } => {
            println!("Pushed local version {version_id} to the remote.")
        }
        SyncOutcome::Pulled { version_id } => {
            println!("Pulled and restored remote version {version_id}.")
        }
        SyncOutcome::Conflict { local, remote } => {
            println!(
                "CONFLICT: your version ({local}) and the remote version ({remote}) diverged."
            );
            println!("Your live save was NOT changed. Resolve with one of:");
            println!("  gamesync resolve {id} --keep local    # keep this device's save");
            println!("  gamesync resolve {id} --keep remote   # take the other device's save");
        }
    }
    Ok(())
}

fn cmd_resolve(args: &[String]) -> Result<(), String> {
    let id = args
        .first()
        .ok_or("usage: gamesync resolve <game_id> --keep <local|remote>")?;
    let keep = flag_value(args, "--keep").ok_or("specify --keep local|remote")?;
    let choice = match keep.as_str() {
        "local" => ConflictChoice::KeepLocal,
        "remote" => ConflictChoice::KeepRemote,
        other => return Err(format!("--keep must be 'local' or 'remote', got '{other}'")),
    };
    engine()?
        .resolve_conflict(id, choice)
        .map_err(|e| e.to_string())?;
    println!("Conflict resolved (kept {keep}); resolution pushed to the remote.");
    Ok(())
}

fn cmd_serve_lan(args: &[String]) -> Result<(), String> {
    let eng = engine()?;
    let dir = flag_value(args, "--dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| eng.data_dir.join("lan-share"));
    let port: u16 = flag_value(args, "--port")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let token = flag_value(args, "--token").unwrap_or_else(gamesync_core::util::new_id);

    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Host's own games sync into the shared store via the folder transport.
    eng.set_remote(&dir).map_err(|e| e.to_string())?;
    let handle = Engine::serve_lan(dir.clone(), &token, &format!("0.0.0.0:{port}"))
        .map_err(|e| e.to_string())?;
    let ip = Engine::local_ip().unwrap_or_else(|| "127.0.0.1".to_string());

    println!("Hosting GameSync on this network:");
    println!("  dir:   {}", dir.display());
    println!("  addr:  {ip}:{}", handle.port);
    println!("  token: {token}\n");
    println!("On another device, set the remote to:");
    println!("  lan:{token}@{ip}:{}\n", handle.port);
    println!("Press Ctrl-C to stop hosting.");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

fn cmd_verify() -> Result<(), String> {
    let eng = engine()?;
    let report = eng.verify().map_err(|e| e.to_string())?;
    println!(
        "Checked {} version(s), {} object(s).",
        report.versions_checked, report.objects_checked
    );
    if report.ok() {
        println!("OK — all objects verified.");
    } else {
        println!("FOUND {} problem(s):", report.problems.len());
        for (version, path, reason) in &report.problems {
            println!("  {version}  {path}: {reason}");
        }
        return Err("integrity check failed".to_string());
    }
    Ok(())
}

// ---- tiny arg + formatting helpers --------------------------------------

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn humanize_ago(ms: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let delta = (now - ms).max(0) / 1000; // seconds
    match delta {
        0..=59 => format!("{delta}s ago"),
        60..=3599 => format!("{}m ago", delta / 60),
        3600..=86399 => format!("{}h ago", delta / 3600),
        _ => format!("{}d ago", delta / 86400),
    }
}

fn print_help() {
    println!(
        r#"GameSync — safe, versioned backup & sync for game saves

USAGE:
    gamesync <command> [args]

COMMANDS:
    scan                         Detect installed Steam games with known save paths
    update-list                  Download the community game list (thousands of games)
    list                         List tracked games and their status
    add "<name>" <folder>        Track a game manually by its save folder
    enable <game_id>             Turn on sync for a game
    disable <game_id>            Turn off sync for a game
    backup <game_id> [flags]     Snapshot a game's saves now
        --force                  Snapshot even if the game looks like it's running
        --wait                   Wait for the save folder to go quiet first
        --label "<text>"         Attach a label to the snapshot
    versions <game_id>           List a game's version history (newest first)
    diff <game_id> <v1> <v2>     Show what changed between two versions
    restore <game_id> <ver_id>   Restore a specific version (safety backup taken first)
        --force                  Restore even if the game looks like it's running
    restore-latest <game_id>     Restore the newest version
    prune <game_id> [flags]      Apply retention, then reclaim unused storage
        --keep N                 Keep at least the newest N versions (default 20)
        --days D                 Also keep anything newer than D days
    gc                           Reclaim storage for objects no version references
    remote set <folder>          Set the sync target (e.g. a Dropbox/Drive folder)
    remote status                Show the configured remote
    sync <game_id>               Sync a game with the remote (push/pull/conflict)
    resolve <game_id> --keep <local|remote>
                                 Resolve a sync conflict by choosing a side
    serve-lan [--port N] [--token T] [--dir D]
                                 Host this device's saves over the LAN for peers
    verify                       Re-hash all stored objects and report problems
    encrypt-init                 Enable client-side encryption (needs GAMESYNC_PASSPHRASE)
    encrypt-status               Show whether the store is encrypted
    where                        Print the data directory path
    help                         Show this help

ENVIRONMENT:
    GAMESYNC_DATA         Override the data directory
    GAMESYNC_PASSPHRASE   Passphrase to unlock (or initialize) an encrypted store
    GAMESYNC_RECOVERY     Recovery key to unlock an encrypted store
"#
    );
}
