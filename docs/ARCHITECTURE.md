# GameSync — Architecture

This document describes how the engine is built and where it's going. For the
product framing, see the project README.

## Guiding principle

> **Game saves are not general files.** A save is a *set of interdependent files*
> that is only consistent when the game is **not running**. So the primitive is
> not "sync files as they change" (Syncthing's model) but **"atomically snapshot
> a save set at a safe moment, version it, and replicate the snapshot."** This
> avoids the #1 cause of save corruption and makes version history and conflict
> handling tractable.

## Layers

```
┌─────────────────────────────────────────────────────────┐
│  UI (Phase 1, next): Tauri v2 + React/TS                 │
│  CLI (now): crates/gamesync-cli                          │
├─────────────────────────────────────────────────────────┤
│  Engine: crates/gamesync-core  (Engine = entry point)    │
│   detection │ snapshot │ restore │ process │ verify      │
├──────────────┬───────────────────────┬──────────────────┤
│  CAS (BLAKE3)│  SQLite metadata (db) │  Remote (Phase 3) │
└──────────────┴───────────────────────┴──────────────────┘
```

The CLI and the future Tauri UI are both thin layers over `Engine`. Keeping all
stateful logic in `gamesync-core` means the UI is just presentation + IPC.

## Module map (`gamesync-core`)

| Module | Responsibility |
| --- | --- |
| `model` | Serializable domain types (`Game`, `Snapshot`, `FileEntry`, …). The IPC surface for the UI. |
| `error` | One `Error`/`Result`; `GameRunning` and `Integrity` are first-class. |
| `util` | Atomic write, id/time helpers, portable relative-path formatting. |
| `cas` | Content-addressed store. BLAKE3-keyed, sharded, deduplicated, verifiable. Optional LZMA2 compression and XChaCha20-Poly1305 encryption at rest (compress → encrypt). |
| `crypto` | Zero-knowledge keystore: random DEK wrapped by an Argon2id passphrase key and by a recovery key (envelope scheme). |
| `db` | SQLite: games, versions (manifest stored as JSON), engine metadata. WAL. |
| `snapshot` | Walk a save set, store contents in CAS, write an immutable version. |
| `restore` | Safety snapshot → stage + verify → atomic swap with rollback. |
| `diff` | Compare two snapshots (added / removed / modified / unchanged). |
| `retention` | Keep-last-N / keep-within-days pruning + reference-counted GC. |
| `process` | Is the game running? Which install dirs are running (exit watcher)? Has the folder gone quiescent? |
| `detection` | `vdf` parser, Steam discovery, `gog` + `epic` store discovery, `emulators`, per-OS `paths` expansion, bundled save-path `manifest` (+ `name_index` for store-agnostic title matching), and `ludusavi` (downloads/translates the ~17k-game community manifest). |
| `vclock` | Version-vector compare / merge / bump — the basis of conflict detection. |
| `remote` | `Remote` trait + transports: `FolderRemote` (a folder), `RcloneRemote` (40+ cloud backends via rclone), `LanRemote`/`lan::serve` (peer-to-peer over TCP). |
| `plugins` | Drop-in JSON plugins: game/emulator defs (merged into detection), hooks, file viewers. Command execution is opt-in. |
| `engine` | Orchestration: scan, backup, restore, diff, prune/gc, verify, encryption, sync/resolve, plugins. |

## Data model

- **Save Set** — the file group for one game (`save_root` + include/exclude
  globs). The unit we snapshot.
- **Snapshot / Version** — an immutable manifest of
  `{rel_path → blake3 hash, size, mtime, mode}` plus
  `{device_id, created_ms, kind, parent}`. Contents live once in the CAS; the
  manifest is stored as JSON in the `versions` row.
- **`kind`** — `manual`, `auto`, or `pre_restore` (the automatic safety net).
- **`parent`** — previous version id on this device → a simple causal chain that
  Phase 3's version vectors build on for conflict detection.

## Safety model (implemented)

These are the guarantees the tests exercise (`crates/gamesync-core/tests/`):

1. **Never snapshot mid-write.** `backup` refuses if the game's process is
   running (`is_running`), with an optional `--wait` quiescence settle.
2. **Append-only history.** Snapshotting only ever *adds* a version; nothing
   mutates or deletes prior versions or their objects.
3. **Every restore is undoable.** A `pre_restore` snapshot of the current state
   is captured before any overwrite.
4. **Verify before you trust.** Restored files are re-hashed in staging before
   the live folder is touched; `verify` re-hashes the whole store on demand.
5. **Crash-safe writes.** Temp-write + fsync + rename for objects and the
   restore swap; on swap failure the previous folder is rolled back.
6. **Cheap history.** Content addressing means unchanged files cost nothing per
   version (the roundtrip test stores 4 objects for 6 file-references).

## Sync engine (implemented)

Replication rides the `Remote` trait — an object store + per-game version log +
a head pointer, with advisory locking. `FolderRemote` targets a plain directory,
so cross-device sync rides any folder the user already syncs (Dropbox/Drive/
OneDrive/network share) with no per-provider integration. Objects are stored
content-addressed on the remote too, so they dedup across devices.

Each snapshot carries a **version vector** (`device_id → counter`). The engine
tracks a per-game *head* — the version the live save currently reflects — and a
new snapshot's vector is the head's, bumped for this device. `sync_game` compares
the local head's vector against the remote head's:

| Relation | Action |
| --- | --- |
| Equal | in sync |
| Local dominates | **push** (fast-forward up) |
| Remote dominates | **pull** + restore (fast-forward down, safety snapshot first) |
| Concurrent | **conflict** — pull remote into history for inspection, leave the live save untouched, ask the user |

`resolve_conflict` captures the chosen side as a new version whose vector is the
**merge** of both (so it dominates each), then pushes it; the other device then
fast-forwards onto it. Nothing is ever auto-discarded — both branches remain in
history.

The configured remote spec selects the transport: `rclone:<target>` (e.g.
`rclone:gdrive:GameSync`) → `RcloneRemote`; `lan:<token>@<host:port>` →
`LanRemote` (one device runs `lan::serve` to host a `FolderRemote` store over
TCP; peers connect with a shared token); anything else is a folder path
(`FolderRemote`). On conflict the remote version is pulled into local history, so
the UI's **diff preview** (`diff(local, remote)`) works offline before the user
resolves. Still to come: LAN host **auto-discovery** (mDNS/UDP) — today the peer
enters the host's address — and "keep both as a fork".

## Roadmap

- **Phase 0 (done):** core engine + CLI — safe snapshot/restore, CAS dedup,
  Steam detection, integrity.
- **Phase 1 (done):** ✅ Tauri v2 + React desktop UI (library, version
  history/restore, diff, remote config, conflict banner, settings, encryption
  unlock), ✅ `FolderRemote` cross-device sync, ✅ conflict detection + manual
  resolution, ✅ **system tray** (close-to-tray, Open/Sync/Quit menu), and
  ✅ **background auto-sync** (timer-driven `auto_sync_pass`: back up changed
  saves, sync enabled games, skip running games, surface conflicts),
  ✅ **native folder picker** (`@tauri-apps/plugin-dialog` on Add Game + Remote +
  wizard), and ✅ **first-run setup wizard** (mode → remote → encryption → scan &
  select games). Phase 1 is feature-complete.
- **Phase 2 (engine done; rest pending):** ✅ client-side encryption,
  ✅ emulator detection, ✅ **GOG (Galaxy) + Epic detection** (name-matched into
  the manifest), ✅ retention/GC, ✅ version diff. Remaining: signed auto-update,
  Linux/macOS packaging, signed manifest auto-update — all need the app shell.
- **Phase 3 (mostly done):** ✅ version-vector conflict model + resolution,
  ✅ rclone provider support (`RcloneRemote`), ✅ LAN peer-to-peer transport
  (`LanRemote` + `lan::serve`, host UI in Settings), ✅ conflict **diff preview**
  before resolving. Remaining: LAN host auto-discovery, "keep both as a fork".
- **Phase 4 (partly done):** ✅ **fault-injection tests** (a corrupt, missing, or
  tampered CAS object aborts restore at the verify gate without touching the live
  save, leaves no temp dirs, keeps the pre-restore safety snapshot recoverable,
  and is flagged by `verify`), ✅ **larger manifest** + community game-list import
  (Ludusavi). Remaining: security review, opt-in telemetry, performance.

## Encryption design (Phase 2)

Optional and zero-knowledge. A random 256-bit **data encryption key (DEK)**
encrypts objects (XChaCha20-Poly1305, random 192-bit nonce per object). The DEK
is wrapped twice in the keystore (`keystore.json`):

- by a key derived from the user's passphrase via **Argon2id** + random salt, and
- by a randomly generated **recovery key** shown once at setup.

Either unlocks the DEK, so a forgotten passphrase is recoverable via the recovery
key — but losing both means the data is unrecoverable (the point of
zero-knowledge). Object keys remain BLAKE3 of the *plaintext*, so dedup still
works; on disk an object is `GSE1 || nonce || ciphertext`. Encryption is set at
store-init on a fresh store; transparent re-encryption of an existing store is a
later refinement.

## Retention & GC (Phase 2)

Pruning is the only operation that deletes version records, and it is policy
driven (keep newest N, keep within D days, never delete the sole newest
version). Byte reclamation is a **separate, reference-counted GC pass**: collect
every hash referenced by any surviving version, then delete only store objects
outside that live set. Separating the two means a pruning mistake can never
orphan-delete bytes another version still needs.

## Notes / known limitations

- Detection covers Steam, GOG (Galaxy), Epic, emulators, and manual add. Steam
  resolves saves by appid; GOG/Epic carry no appid locally, so they're matched
  into the (appid-keyed) manifest by normalized title via `manifest::name_index`.
  The bundled manifests (`crates/gamesync-core/manifests/{saves,emulators}.json`)
  are small starter sets — `update-list` layers the ~17k-game community manifest
  on top, which also widens GOG/Epic coverage.
- `is_running` is a heuristic (process exe under the game's install dir); it
  returns false when the install dir is unknown (e.g. manually-added games and
  emulators), so quiescence settling is the backstop there.
- Encrypted objects are sealed whole-file in memory (fine for save-sized files);
  chunked streaming AEAD for very large objects is a later refinement.
- Sync currently has one transport (`FolderRemote`). For encrypted sync, devices
  must share the same keystore/DEK (object keys are plaintext hashes, so dedup
  matches across devices, but decryption needs the shared key). rclone and LAN
  transports are future `Remote` implementations.
- Manual-game ids are derived from the game name so they match across devices;
  two genuinely different games sharing a name would collide (rename to fix).
