# GameSync

Safe, versioned backup and **cross-device sync for game saves** — built for
titles that lack reliable Steam Cloud (Dark Souls 2/3, older PC games,
emulators, modded/indie games). Think "Syncthing, but scoped to save folders and
built to never corrupt or lose a save."

GameSync is a **desktop app** (a native window powered by Rust + a web UI). It
runs locally and touches your real save folders and game processes, so it isn't
a website — it has to run on your machine.

---

## 1. Getting it running

### Prerequisites

You build it from source (no installer yet):

- **Rust** (the `cargo` command) — install from <https://rustup.rs> or `brew install rust`.
- **Node.js 18+** — from <https://nodejs.org> or `brew install node`.
- **A C toolchain & system webview:**
  - **macOS:** Xcode Command Line Tools (`xcode-select --install`). WebKit ships with macOS.
  - **Windows:** the *Microsoft Visual C++ Build Tools* and the *WebView2 Runtime* (preinstalled on Windows 11).
  - **Linux:** `webkit2gtk` + `libssl` dev packages (e.g. on Debian/Ubuntu: `libwebkit2gtk-4.1-dev build-essential libssl-dev libayatana-appindicator3-dev librsvg2-dev`).

No database to set up — SQLite is bundled.

### Run it (development)

```sh
npm install            # one time: install frontend deps
npm run tauri dev      # build + launch the app (first build takes a few minutes)
```

The window opens automatically. The first compile is slow (it builds the whole
webview stack); later launches are fast.

### Build a distributable app (optional)

```sh
npm run tauri build    # installer/bundle for your OS in src-tauri/target/release/bundle
```

Icons are already set up (regenerate with `npm run tauri icon src-tauri/icons/icon.png`).
Unsigned builds run locally but warn on other machines — see
[`docs/BUILDING.md`](docs/BUILDING.md) for **code signing & notarization**
(macOS / Windows), which needs your own certificates.

---

## 2. How to use it

### First launch

A short **setup wizard** walks you through it: choose local-only vs. cross-device
sync → (if syncing) pick a shared folder → optionally turn on encryption → scan
and select your games. You can **Skip** it and configure things later.

After setup you land on the **library**. To add more games anytime:

- **Scan** (top bar) — auto-detects installed **Steam**, **GOG (Galaxy)**, and
  **Epic** games with known save paths, plus common **emulators** (Dolphin,
  PCSX2, RPCS3, PPSSPP, DuckStation, RetroArch). GOG/Epic saves are matched into
  the game list by title (their stores carry no Steam appid).
- **Add game** — for anything not detected: give it a name and **Browse…** to its
  save folder (or paste the path).

GameSync ships with a small curated game list. **Settings → Game detection →
Update game list** downloads the community manifest (PCGamingWiki via Ludusavi,
~17,000 games) so Scan recognizes far more titles. (CLI: `gamesync update-list`.)

Each game appears as a card showing its platform, save path, version count, and
last backup time. Each card also has **Rename** and **Remove** (removing deletes
the game's backup history in GameSync but never touches your actual save files).

### Backing up and restoring

- **Back up** on a card takes an immediate snapshot. Snapshots are
  content-addressed and deduplicated, so unchanged files cost nothing.
- **History** opens the version timeline. From there you can:
  - **Restore** any version — GameSync first takes a *safety snapshot* of your
    current save, then atomically swaps the chosen version in. A restore is
    always itself undoable.
  - **Compare** two versions to see exactly which files were added, changed, or
    removed.

GameSync refuses to back up a game it detects as running (to avoid capturing a
half-written save).

### Browsing save files

Click **Files** on a game to see exactly which files are in its save folder
(name, size, last modified). **Open save folder** opens it in your file manager,
and **Reveal** next to any file jumps right to it.

### Syncing across devices

GameSync syncs by replicating to a **shared folder** — point each of your
devices at the *same* folder inside something you already sync (Dropbox, Google
Drive, OneDrive, a network share):

1. In the **Remote** bar, **Browse…** to that folder (or paste the path) and click **Save**.
2. Toggle **Sync** on for each game you want to sync.
3. Click **Sync now** on a card, or **Sync all** in the top bar.

Do the same on your other device (point it at the same folder, add the same
games). Sync figures out the rest:

- If you're ahead, it **pushes**.
- If the other device is ahead, it **pulls** and restores into your save folder.
- If both changed independently, it reports a **conflict** (see below).

> Tip: for an automatically-detected game (Steam/emulator), the two devices
> agree on identity automatically. For a **manually-added** game, give it the
> **same name** on both devices so they match.

**Direct cloud (no shared folder):** if you have [rclone](https://rclone.org)
installed and configured (`rclone config`), set the remote to
`rclone:<remote>:<path>` — e.g. `rclone:gdrive:GameSync` — to sync straight to
Google Drive, S3, Dropbox, B2, OneDrive, and 40+ other backends.

**Redirect a save folder (advanced, live sync):** a game card's **Redirect to
synced folder** moves that game's save folder into a folder you pick (e.g. your
OneDrive/Drive folder) and leaves a **symlink** behind so the game still finds
its saves — the cloud client then syncs them live. GameSync takes a backup
first, keeps your original folder (renamed, not deleted), verifies the link, and
rolls back on failure. On Windows this needs Developer Mode or admin rights to
create the link. This is independent of GameSync's own snapshot sync.

**LAN (peer-to-peer, no cloud):** on one device, open **Settings → LAN sync →
Host on this network**. It shows a connect string like
`lan:<token>@192.168.1.5:51234`. On your other device, paste that into the
**Remote** bar and sync — saves transfer directly between the two machines.
(From the CLI: `gamesync serve-lan` on the host, then
`gamesync remote set lan:<token>@<host>:<port>` on the peer. Auto-discovery of
hosts is a planned follow-up; for now you enter the address shown by the host.)

### Conflicts

If you played the same game on two devices without syncing in between, GameSync
detects it and shows a **conflict banner**. Your live save is **never
overwritten**. Click **Preview changes** to see exactly which files differ
between your save and the other device's before deciding, then choose:

- **Keep mine** — your version wins.
- **Take remote** — the other device's version wins (your current save is still
  backed up first).

Either way both versions stay in history, and the resolution supersedes both
branches so the other device converges on the next sync.

### Automatic sync + system tray

Open **Settings** → **Automatic sync**:

- Turn it on and set an interval (minutes).
- GameSync will, in the background, back up any changed saves and sync enabled
  games on that schedule. Running games are skipped; conflicts are reported, not
  auto-resolved.
- **Back up automatically when a game closes** (on by default) — GameSync notices
  when a tracked game exits, waits a moment for the save to flush, then backs it
  up (and syncs it if set up). Exit detection works for games with a known
  install location (e.g. Steam); manual/emulator games use the timer or manual
  backup.

The app lives in the **system tray**:

- **Closing the window hides it to the tray** so background auto-sync keeps
  running.
- The tray menu has **Open GameSync**, **Sync all now**, and **Quit GameSync**.
  Use **Quit** to fully exit.

### Encryption (optional)

To encrypt everything GameSync stores (and uploads) with zero-knowledge
encryption:

1. **Enable encryption** (top bar) on a fresh store. Choose a passphrase.
2. **Save the recovery key it shows you** — it appears only once and can unlock
   your saves if you forget the passphrase. Without the passphrase *and* the
   recovery key, encrypted saves cannot be recovered.

After that, the app asks for your passphrase on launch. For encrypted sync, all
devices must use the **same passphrase/keystore**.

### Integrity check

**Settings → Integrity → Verify all data** re-hashes every stored object
(decrypting first when encrypted) and reports any corruption.

### Storage & retention

**Settings → Storage** shows how much space backups use (total and per game,
deduplicated). Set a retention policy — keep the newest *N* versions, and
optionally anything from the last *D* days — and **Apply & clean up** prunes
older versions across all games and reclaims the space. **Reclaim unused space**
runs garbage collection on its own.

**Compress backups** (also in Storage) stores objects with **LZMA2** (the 7-Zip
compression codec), which shrinks both your local backups and the data uploaded
to a synced folder / sent over LAN. It composes with encryption (compress, then
encrypt) and can be turned on before you take any backups. Note: this compresses
*stored backups* — a **redirected (symlinked) folder** holds the live files the
game reads, so it stays uncompressed by necessity.

### Plugins

Open **Plugins** to extend GameSync with drop-in `.json` files: add games or
emulator save paths to detection, run **hooks** before/after a backup or restore,
or register **file viewers** that open matching saves with an external tool.
Game/emulator definitions are pure data; hooks and viewers run commands, so they
only execute after you enable **"Allow plugins to run commands"** (off by
default). See [`docs/PLUGINS.md`](docs/PLUGINS.md) for the format.

### Where your data lives

Backups and metadata are stored in your OS app-data directory (shown in the
status bar at the bottom of the window):

- macOS: `~/Library/Application Support/dev.GameSync.GameSync/` (or `com.gamesync.desktop`)
- Windows: `%APPDATA%\GameSync\`
- Linux: `~/.local/share/GameSync/`

Your **original save folders are the source of truth** — GameSync only ever
copies *out* of them (for backup) and writes *into* them (on restore, after a
safety snapshot).

---

## 3. Command-line tool (optional)

A headless CLI mirrors the engine — handy for scripting, servers, or quick
checks. Build it with `cargo build -p gamesync-cli` (binary at
`target/debug/gamesync`):

```sh
gamesync scan                          # detect Steam games + emulators
gamesync add "My RPG" /path/to/saves   # track a game manually
gamesync list                          # tracked games + status
gamesync backup <game_id>              # snapshot now
gamesync versions <game_id>            # history
gamesync diff <game_id> <v1> <v2>      # what changed
gamesync restore <game_id> <ver_id>    # restore (safety snapshot taken first)
gamesync prune <game_id> --keep 20 --days 30   # retention + reclaim storage
gamesync remote set /path/to/shared/folder
gamesync sync <game_id>                # push / pull / report conflict
gamesync resolve <game_id> --keep local   # or --keep remote
gamesync verify                        # integrity check
gamesync help
```

Environment: `GAMESYNC_DATA` overrides the data dir; `GAMESYNC_PASSPHRASE` /
`GAMESYNC_RECOVERY` unlock an encrypted store.

---

## 4. Development

```sh
cargo test --workspace      # run the engine test suite
cargo build -p gamesync-cli # build the CLI
npm run build               # type-check + bundle the frontend
```

Layout:

```
crates/gamesync-core/   # the engine (library): detection, snapshots, sync, crypto
crates/gamesync-cli/    # CLI driver
src/                    # React + TypeScript frontend
src-tauri/              # Tauri desktop shell + command layer
docs/ARCHITECTURE.md    # design, safety model, sync protocol, roadmap
```

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for how the sync engine and
safety guarantees work.

## License

MIT
