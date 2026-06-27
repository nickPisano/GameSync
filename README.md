# GameSync

Safe, versioned backup and **cross-device sync for game saves** — built for
titles that lack reliable Steam Cloud (Dark Souls 2/3, older PC games,
emulators, modded/indie games). Think "Syncthing, but scoped to save folders and
built to never corrupt or lose a save."

GameSync is a **desktop app** (a native Rust window — egui, no web stack). It
runs locally and touches your real save folders and game processes, so it isn't
a website — it has to run on your machine.

## Contents

- [Install & run](#install--run) — [download a build](#download-a-prebuilt-build-recommended) · [build from source](#build-from-source)
- [Supported games](docs/GAMES.md) — ~70 built in, ~17,000 via in-app update
- Official add-ons — [themes](https://github.com/nickPisano/GameSync-Themes) · [plugins](https://github.com/nickPisano/GameSync-Plugins)
- **Tutorials**
  - [First-time setup](#tutorial-first-time-setup)
  - [Find & add games](#tutorial-find--add-games) — Scan · Add game · Update game list
  - [Back up & restore](#tutorial-back-up--restore) — Back up · Files · History · Compare · Restore
  - [Manage games](#tutorial-manage-games) — Rename · Remove
  - [Sync across devices](#tutorial-sync-across-devices) — Remote · Sync · rclone · LAN · Redirect · Conflicts
  - [Automate it](#tutorial-automate-it) — Auto-sync · Backup-on-close · Tray
  - [Protect your data](#tutorial-protect-your-data) — Encryption · Integrity · Storage/Retention/Compression
  - [Customize it](#tutorial-customize-it) — Themes · Plugins
- [Command-line tool](#command-line-tool)
- [Where your data lives](#where-your-data-lives)
- [Development](#development)

---

## Install & run

The fastest way is to **download a prebuilt build** for your OS. Prefer to
compile it yourself? See [Build from source](#build-from-source) below.

### Download a prebuilt build (recommended)

**1. Get the file for your OS.** Open the [**Releases**](https://github.com/nickPisano/GameSync/releases)
page, expand the latest version's **Assets**, and download either an
**installer** or a **portable** build (`<ver>` is the version number in the
filename, e.g. `0.3.0`). None of them need a WebView runtime — the app is a
native window:

| Your system | Installer | Portable (no install — just run) |
| --- | --- | --- |
| **macOS** (Intel *or* Apple Silicon) | `GameSync_<ver>_universal.dmg` | the **GameSync.app** inside that `.dmg`, or `GameSync_<ver>_macos-universal` |
| **Windows x64** | `GameSync_<ver>_x64_en-US.msi` *or* `…_x64-setup.exe` | `GameSync_<ver>_windows-x64-portable.exe` |
| **Windows arm64** | `GameSync_<ver>_arm64_en-US.msi` *or* `…_arm64-setup.exe` | `GameSync_<ver>_windows-arm64-portable.exe` |
| **Linux x64** | `GameSync_<ver>_amd64.deb` *or* `…_x86_64.AppImage` | `GameSync_<ver>_linux-x64` |
| **Linux arm64** | `GameSync_<ver>_arm64.deb` *or* `…_aarch64.AppImage` | `GameSync_<ver>_linux-arm64` |

> **Which architecture?** The macOS build is *universal* (runs on both), so just
> take the `.dmg`. On **Windows**: Settings → System → About → *System type*. On
> **Linux**: run `uname -m` (`x86_64` → x64, `aarch64` → arm64). *(Fedora/RHEL:
> use the `.AppImage` — no `.rpm` is published.)*

**2. Install or run it:**

- **macOS** — open the `.dmg`, drag **GameSync** into **Applications**, launch
  it. (Or run the portable binary: `chmod +x GameSync_<ver>_macos-universal && ./GameSync_<ver>_macos-universal`.)
- **Windows** — run the `.msi` or `-setup.exe` to install, **or** just
  double-click the `-portable.exe` to run with no install. No WebView2 Runtime
  needed.
- **Linux** — installer: `sudo apt install ./GameSync_<ver>_amd64.deb`. Portable:
  `chmod +x GameSync_<ver>_x86_64.AppImage && ./GameSync_<ver>_x86_64.AppImage`
  (or the bare `GameSync_<ver>_linux-x64` binary).

> **First run shows an "unidentified developer" warning.** The builds aren't
> code-signed yet, so the OS blocks them by default. It's safe to allow:
> - **macOS:** right-click **GameSync** → **Open** → **Open** (just the first
>   time), or run `xattr -dr com.apple.quarantine /Applications/GameSync.app`.
> - **Windows:** on the SmartScreen dialog, click **More info → Run anyway**.

**3.** The app opens — continue to [First-time setup](#tutorial-first-time-setup).

### Build from source

For development, or a platform/arch without a prebuilt binary.

**1. Install the prerequisites** — **Rust** and your OS's C toolchain. The UI is
a native [egui](https://github.com/emilk/egui) app, so there's **no Node.js, no
WebView, and no system browser** to install:

- **Rust** — from <https://rustup.rs> (or `brew install rust`); check `cargo --version`.
- **C toolchain + native GUI libs:**
  - **macOS:** `xcode-select --install`.
  - **Windows:** *Microsoft Visual C++ Build Tools*.
  - **Linux (Debian/Ubuntu):** `sudo apt install build-essential libssl-dev libgtk-3-dev libxkbcommon-dev libayatana-appindicator3-dev`

  (No database to install — SQLite is bundled.)

**2. Get the code & run it:**

```sh
git clone https://github.com/nickPisano/GameSync.git
cd GameSync
cargo run -p gamesync-gui      # builds + launches the app window
```

The first compile takes a few minutes; later launches are fast.

**3. (Optional) Build your own portable binary:**

```sh
cargo build --release -p gamesync-gui   # binary in target/release/gamesync-gui
```

See [`docs/BUILDING.md`](docs/BUILDING.md) for multi-arch builds, the automated
release workflow, and **code signing** (needs your own certs).

---

## Tutorial: First-time setup

On first launch a **setup wizard** appears:

1. **Choose a mode** — local-only backups, or cross-device sync.
2. **Pick a sync folder** (only if syncing) — a folder inside something you
   already sync (Dropbox/Drive/OneDrive/network share). Use the *same* folder on
   every device.
3. **Scan & select** — GameSync detects installed games; tick the ones to track.

Encryption is optional and set up later from **Settings → Encryption**.

Prefer to dive in? Click **Skip setup** at any step and configure things later
from the toolbar. Either way you land on the **library**, where each game is a
card showing its platform, save path, version count, and last-backup time.

---

## Tutorial: Find & add games

### Auto-detect installed games (Scan)

1. Click **Scan** in the toolbar.
2. GameSync detects installed **Steam**, **GOG (Galaxy)**, and **Epic** games
   with known save paths, plus common **emulators** and select **standalone**
   titles (free/itch/modpacks like *Voices of the Void*). Out of the box it
   recognizes **~70 games, 11 emulators, and more** — see the full built-in list
   in **[docs/GAMES.md](docs/GAMES.md)**.
3. Detected games appear as cards. (GOG/Epic games are matched to save paths by
   title, since those stores carry no Steam app id.)

> That built-in list is just a starter set. You can pull a database of
> **~17,000 games right inside the app** — no reinstall — via **Settings → Game
> detection → Update game list**; see [Recognize more
> games](#recognize-more-games-update-game-list) below.

### Add a game manually

For anything not auto-detected:

1. Click **Add game**.
2. Enter a **Name**.
3. Set the **Save folder** — click **Browse…** to pick it, or paste the path.
4. *(Optional)* Set the **Game folder or app** so GameSync can back up
   automatically when that game closes.
5. Click **Add game**. It appears in your library.

### Recognize more games (Update game list)

GameSync ships with a [curated starter list](docs/GAMES.md) (~70 games + 11
emulators). To recognize thousands more — **right inside the app, no reinstall**:

1. Open **Settings → Game detection**.
2. Click **Update game list** — the app downloads the full community manifest
   (PCGamingWiki via Ludusavi, **~17,000 games**) and stores it locally.
3. The detected-game count updates. Run **Scan** again to pick up newly
   recognized titles. *(CLI: `gamesync update-list`.)*

---

## Tutorial: Back up & restore

### Back up now

1. On a game card, click **Back up**.
2. GameSync snapshots the save folder and shows e.g. *"Backed up 3 file(s)."*
   Snapshots are content-addressed and **deduplicated**, so unchanged files cost
   no extra space, and transient junk (`*.tmp`, `*.bak`, `.DS_Store`, …) is
   excluded automatically.

> GameSync refuses to back up a game it detects as **running**, to avoid
> capturing a half-written save.

### Browse the save files (Files)

1. Click **Files** on a card.
2. See every file in the save folder with its size and last-modified time.
3. Click **Open save folder** to open it in your file manager, or **Reveal**
   next to any file to jump straight to it.

### View history & compare versions

1. Click **History** on a card to open the version timeline.
2. Each version shows a short id, a tag (`manual`, `auto`, or `pre_restore`),
   file count, size, and age.
3. To compare two versions, pick them in the **Compare versions** dropdowns and
   click **Compare** — you'll see exactly which files were **added (+)**,
   **changed (~)**, or **removed (−)**.

### Restore a version

1. Open **History** and click **Restore** on the version you want.
2. GameSync first takes an automatic **safety snapshot** of your *current* save
   (tagged `pre_restore`), then atomically swaps in the chosen version. You'll
   see *"Restored. A safety snapshot of the previous save was taken."*
3. Changed your mind? Restore the `pre_restore` snapshot to undo — every restore
   is itself undoable.

> A restore makes the folder match the snapshot exactly, so files that were
> **excluded** from backups (e.g. `*.tmp`) are not preserved across a restore.

---

## Tutorial: Manage games

### Rename a game

1. Click **Rename** on the card.
2. Type a new name and click **Save**.

### Remove a game

1. Click **Remove** on the card and confirm.
2. This deletes the game's **backup history inside GameSync** and reclaims its
   storage. **Your actual save files on disk are never touched.**

---

## Tutorial: Sync across devices

GameSync syncs by replicating snapshots to a **shared folder** that each device
can reach. (Conflict resolution needs no server — it's peer-to-peer.)

> **Using Google Drive or OneDrive?** See the in-depth, step-by-step
> **[Cloud sync guide](docs/CLOUD-SYNC.md)** — per-provider setup (desktop client
> *and* rclone), the placeholder/offline-files gotcha, encrypted-sync notes, and
> troubleshooting.

### Set up a shared-folder remote

1. In the **Remote** bar, click **Browse…** and pick a folder inside a service
   you already sync (Dropbox/Drive/OneDrive/network share), or paste its path.
2. Click **Save** — the status flips to *configured*.
3. Toggle **Sync** on for each game you want to sync.

### Sync

1. Click **Sync now** on a card, or **Sync all** in the toolbar.
2. GameSync **pushes** if you're ahead, **pulls** (and restores) if the other
   device is ahead, or reports **InSync** if nothing changed.
3. Repeat the setup on your other device: point it at the *same* folder and add
   the same games.

> For auto-detected games the two devices agree on identity automatically. For a
> **manually-added** game, give it the **same name** on both devices so they match.

### Sync straight to a cloud provider (rclone)

If you have [rclone](https://rclone.org) installed and configured (`rclone config`):

1. In the **Remote** bar, enter `rclone:<remote>:<path>` — e.g.
   `rclone:gdrive:GameSync`.
2. Click **Save** and sync as above. Works with Google Drive, S3, Dropbox, B2,
   OneDrive, and 40+ other backends.

### Sync over your local network (LAN, no cloud)

1. On the **host** device, open **Settings → LAN sync → Host on this network**.
   It starts serving and shows a **token**.
2. On the **other** device, click **Find hosts** in the **Remote** bar — the
   host appears by name; click it to fill in its address. (No host found? Make
   sure both are on the same network; on macOS, allow GameSync **Local Network**
   access when first prompted.)
3. Insert the host's **token** right after `lan:` (so the remote reads
   `lan:<token>@<host>:<port>`), click **Save**, and sync — saves transfer
   directly between the two machines. You can also paste the full connect string
   the host shows instead of using Find hosts.

*(CLI: `gamesync serve-lan` on the host, `gamesync discover-lan` to find hosts,
then `gamesync remote set lan:<token>@<host>:<port>` on the peer.)*

### Live-sync via your cloud client (Redirect to synced folder)

This makes the cloud client (not GameSync) sync the live files:

1. On a card, click **Redirect to synced folder** and pick a destination (e.g.
   your OneDrive/Drive folder).
2. GameSync backs the game up first, **moves** the save folder into the
   destination, and leaves a **symlink** behind so the game still finds its
   saves. Your original folder is kept (renamed, never deleted), and the move
   rolls back on failure.

> On Windows this needs Developer Mode or admin rights to create the link. This
> is independent of GameSync's own snapshot sync.

### Resolve a conflict

If you played the same game on two devices without syncing in between, GameSync
shows a **conflict banner** and **never overwrites your live save**:

1. Click **Preview changes** to see which files differ between your save and the
   other device's.
2. Choose **Keep mine** (your version wins), **Take remote** (the other device's
   version wins — your current save is backed up first), or **Keep both** — your
   save stays live and the other device's save is preserved as a **new game**
   (named "… (fork)", in its own folder) you can play independently.
3. Both versions stay in history, and the resolution supersedes both branches so
   the other device converges on its next sync. *(CLI: `gamesync resolve
   <game_id> --keep <local|remote|both>`.)*

---

## Tutorial: Automate it

### Automatic background sync

1. Open **Settings → Automatic sync**.
2. Tick **"Automatically back up & sync enabled games in the background"** and
   set an interval (minutes).
3. GameSync now backs up changed saves and syncs enabled games on that schedule.
   Running games are skipped; conflicts are reported, not auto-resolved.

### Back up automatically when a game closes

- In **Settings → Automatic sync**, **"Back up automatically when a game closes"**
  is **on by default**. GameSync notices when a tracked game exits, waits for the
  save to flush, then backs it up (and syncs it if configured). Exit detection
  needs a known install location (e.g. Steam, or a manual game with its app set);
  manual/emulator games without one rely on the timer or manual backup.

### System tray

- **Closing the window hides GameSync to the tray** so background sync keeps
  running.
- The tray menu has **Open GameSync**, **Sync all now**, and **Quit GameSync**.
  Use **Quit** to fully exit.

---

## Tutorial: Protect your data

### Enable encryption (zero-knowledge)

Encrypts everything GameSync stores and uploads.

1. **Do this before backing up any games** — encryption can only be enabled on a
   store with no backups yet.
2. Click **Enable encryption** in the toolbar.
3. Enter a **passphrase** (min 8 chars) twice and click **Enable**.
4. **Save the recovery key it shows you** — it appears only **once** and can
   unlock your saves if you forget the passphrase. Losing both the passphrase
   *and* the recovery key means the data is unrecoverable (that's the point).
5. From now on the app asks for your passphrase on launch (**Unlock** screen).
   For encrypted *sync*, use the **same passphrase/keystore** on every device.

### Verify nothing is corrupt (Integrity)

1. Open **Settings → Integrity**.
2. Click **Verify all data** — it re-hashes every stored object (decrypting first
   when encrypted) and reports `OK` or lists any corruption.

### Manage storage, retention & compression

1. Open **Settings → Storage** to see space used (total and per game,
   deduplicated).
2. **Retention:** set "keep newest *N* versions" and optionally "anything from
   the last *D* days," then click **Apply & clean up** to prune older versions
   across all games. **Reclaim unused space** runs garbage collection on its own.
3. **Compress backups** (LZMA2 / 7-Zip codec) shrinks stored backups and synced
   data; it composes with encryption (compress, then encrypt). It can only be
   toggled **before** any backups exist. *(A redirected/symlinked folder holds
   the live files the game reads, so it stays uncompressed.)*

---

## Tutorial: Customize it

### Change the theme

1. Open **Settings → Appearance**.
2. Click a built-in swatch — **Midnight**, **Light**, **Forest**, or **Grape** —
   it applies instantly. (New installs follow your OS light/dark setting until
   you pick one.)
3. Click **More themes…** to open the theme gallery — a grid of every theme with
   its own preview tile: **Auto** (follow your OS), all built-ins, and any custom
   themes you've imported. Click a tile and it applies instantly.

### Import a custom theme

1. Open **Settings → Appearance → More themes… → Import a theme…**.
2. Paste a theme as JSON: a `name` plus a `colors` object with the keys `bg`,
   `panel`, `panel-2`, `border`, `text`, `muted`, `accent`, `accent-hover`,
   `ok`, `err`, `warn`. Optionally add a `swatch` (any CSS background — a gradient
   or a `data:` image URL) to give your theme its own preview tile. (A filled-in
   template is pre-loaded — edit it.)
   - *(Optional)* an `effects` block to restyle surfaces — `"style"` of
     `"glass"` (frosted translucent panels over a gradient), `"neo"`
     (neumorphic extruded panels), or `"skeuo"` (glossy beveled surfaces), plus
     optional `gradient`/`blur`/`opacity`/`highlight`/`shadow`/`glow`. Add
     `"bubbles": true` (with an optional `"bubbleColor"`) to float soft bubbles
     up the background — it composes with any style, or on its own. Themes
     without it are unaffected.
3. Click **Import**. It applies immediately and shows up as a tile in the gallery.
4. To delete a custom theme, click the **×** on its tile in the gallery.

> **Want more themes?** Browse the official
> [GameSync-Themes](https://github.com/nickPisano/GameSync-Themes) gallery and
> import any palette you like.

### Plugins

Extend GameSync with drop-in `.json` files — grab ready-made ones from the
official [GameSync-Plugins](https://github.com/nickPisano/GameSync-Plugins)
catalog, or write your own:

1. Open **Plugins**, then **Open folder** and drop a `.json` plugin in.
2. Click **Reload**. A plugin can add games/emulator save paths to detection,
   run **hooks** before/after backup or restore, or register **file viewers**
   that open matching saves with an external tool.
3. Game/emulator definitions are pure data and always apply. Hooks and viewers
   run shell commands, so they only execute after you tick **"Allow plugins to
   run commands"** (off by default).

See [`docs/PLUGINS.md`](docs/PLUGINS.md) for the file format.

---

## Command-line tool

A headless CLI mirrors the engine — handy for scripting, servers, or quick
checks. Build it with `cargo build -p gamesync-cli` (binary at
`target/debug/gamesync`):

```sh
gamesync scan                          # detect Steam/GOG/Epic games + emulators
gamesync update-list                   # download the ~17k-game community list
gamesync add "My RPG" /path/to/saves   # track a game manually
gamesync list                          # tracked games + status
gamesync backup <game_id>              # snapshot now
gamesync versions <game_id>            # history
gamesync diff <game_id> <v1> <v2>      # what changed
gamesync restore <game_id> <ver_id>    # restore (safety snapshot taken first)
gamesync prune <game_id> --keep 20 --days 30   # retention + reclaim storage
gamesync verify                        # integrity check
gamesync remote set /path/to/shared/folder     # or rclone:<remote>:<path>
gamesync serve-lan                     # host LAN sync on this machine
gamesync sync <game_id>                # push / pull / report conflict
gamesync resolve <game_id> --keep local   # or --keep remote
gamesync help
```

Environment: `GAMESYNC_DATA` overrides the data dir; `GAMESYNC_PASSPHRASE` /
`GAMESYNC_RECOVERY` unlock an encrypted store.

---

## Where your data lives

Backups and metadata live in your OS app-data directory (shown in the status bar
at the bottom of the window):

- **macOS:** `~/Library/Application Support/dev.GameSync.GameSync/`
- **Windows:** `%APPDATA%\GameSync\`
- **Linux:** `~/.local/share/GameSync/`

Your **original save folders are the source of truth** — GameSync only ever
copies *out* of them (for backup) and writes *into* them (on restore, after a
safety snapshot).

---

## Development

```sh
cargo test --workspace      # run the engine test suite
cargo build -p gamesync-cli # build the CLI
cargo run -p gamesync-gui   # build + launch the native GUI
```

Layout:

```
crates/gamesync-core/   # the engine (library): detection, snapshots, sync, crypto
crates/gamesync-cli/    # CLI driver
crates/gamesync-gui/    # native egui/eframe desktop app (no webview)
docs/ARCHITECTURE.md    # design, safety model, sync protocol, roadmap
```

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for how the sync engine and
safety guarantees work.

## License

MIT
