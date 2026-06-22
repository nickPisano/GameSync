# Cloud sync with Google Drive & OneDrive

An in-depth guide to syncing your game saves across devices through **Google
Drive** or **OneDrive**. For the quick version, see the
[Sync across devices](../README.md#tutorial-sync-across-devices) tutorial in the
README — this document goes deeper, with per-provider steps, the gotchas that
trip people up, and troubleshooting.

> **First, the one-sentence mental model.** GameSync does **not** live-sync your
> save files. It takes versioned **snapshots** and replicates them as
> content-addressed objects into a **shared folder** that each device can reach.
> Google Drive / OneDrive is simply *how that shared folder gets to your other
> device.* GameSync writes snapshots into the cloud folder; the cloud client
> uploads them; your other device's cloud client downloads them; GameSync there
> reads them back. Nothing is ever overwritten without a safety snapshot, and
> divergence becomes a **conflict** you resolve, never a silent loss.

## Contents

- [Pick your method](#pick-your-method)
- [Before you start (read this)](#before-you-start-read-this)
- [Method A — Cloud desktop client + folder remote (recommended)](#method-a--cloud-desktop-client--folder-remote-recommended)
  - [Google Drive (Drive for desktop)](#a1-google-drive-drive-for-desktop)
  - [OneDrive](#a2-onedrive)
- [Method B — rclone direct (advanced / headless)](#method-b--rclone-direct-advanced--headless)
  - [Google Drive via rclone](#b1-google-drive-via-rclone)
  - [OneDrive via rclone](#b2-onedrive-via-rclone)
- [Adding your other device](#adding-your-other-device)
- [Encryption + cloud sync](#encryption--cloud-sync)
- [Good habits to avoid conflicts](#good-habits-to-avoid-conflicts)
- [Troubleshooting](#troubleshooting)
- [Redirect ≠ sync (don't confuse them)](#redirect--sync-dont-confuse-them)

---

## Pick your method

| | **Method A — Folder remote** | **Method B — rclone** |
| --- | --- | --- |
| **How** | Point GameSync at a folder the official Drive/OneDrive **desktop app** keeps synced | Point GameSync at `rclone:<remote>:<path>`; rclone talks to the cloud API directly |
| **Best for** | Most people. Windows/macOS with the cloud client installed | Linux, headless boxes, or no desktop client; power users |
| **Extra software** | The cloud desktop app (you probably already have it) | [rclone](https://rclone.org) installed + configured once |
| **Setup difficulty** | Easy — it's just "Browse… → pick a folder" | Moderate — a one-time `rclone config` OAuth flow |

You can mix methods across devices (e.g. Method A on your laptop, Method B on a
Linux server) **as long as they point at the same cloud folder** — the objects
are identical either way.

---

## Before you start (read this)

A few rules that prevent 90% of problems:

1. **Use the *same* cloud folder on every device.** Create one subfolder, e.g.
   `GameSync`, inside your Drive/OneDrive and use it everywhere. Don't sync to the
   *root* of your cloud drive.
2. **Make the folder *really* present on disk, not a placeholder.** Both Drive
   ("streaming" mode) and OneDrive ("Files On-Demand") can keep files as
   online-only stubs. GameSync needs to *read* the objects, so mark the GameSync
   folder **"Available offline" / "Always keep on this device."** Steps are in
   each method below.
3. **Don't put GameSync's own data directory in the cloud.** Only the **remote
   folder** goes in Drive/OneDrive. GameSync's local store (see [Where your data
   lives](../README.md#where-your-data-lives)) must stay local — syncing a live
   SQLite database through a cloud client *will* corrupt it.
4. **Let the cloud finish syncing before you sync on the other machine.** Wait
   for the cloud client's "up to date" check mark on device 1 before clicking
   **Sync now** on device 2. Racing the upload/download is the #1 cause of
   avoidable conflicts.
5. **Turn the per-game `Sync` toggle on.** Configuring the remote is only half of
   it — each game has its own **Sync** switch on its card.

---

## Method A — Cloud desktop client + folder remote (recommended)

The idea: the cloud's own desktop app mirrors a folder between your computer and
the cloud. You tell GameSync to use that folder as its remote. GameSync never
talks to Google/Microsoft — it just reads and writes local files, and the cloud
app does the uploading.

### A1. Google Drive (Drive for desktop)

**1. Install & sign in to Drive for desktop.**
Get it from <https://www.google.com/drive/download/>. Sign in with the Google
account you'll use on all your devices.

**2. Find your Drive folder on disk.**

- **Windows:** Drive mounts as a virtual drive, usually `G:` → your files are
  under `G:\My Drive`. (If you chose "mirror" during setup, it's a normal folder
  in your home directory instead.)
- **macOS:** `~/Library/CloudStorage/GoogleDrive-<your-email>/My Drive`

**3. Create a sync folder.** Inside *My Drive*, make a new folder named
`GameSync`. (Full path example, Windows: `G:\My Drive\GameSync`.)

**4. Make it available offline.** Right-click the `GameSync` folder → **Offline
access → Available offline.** This forces real files onto disk so GameSync can
read them and the cloud won't evict them.

**5. Point GameSync at it.**
   1. In GameSync's **Remote** bar, click **Browse…** and select the
      `…/My Drive/GameSync` folder (or paste its path).
   2. Click **Save**. The status pill flips to **configured**.

**6. Enable sync per game** and back up:
   1. Flip the **Sync** toggle on each game card you want to sync.
   2. Click **Sync now** on a card (or **Sync all** in the toolbar). GameSync
      pushes your snapshots into `…/My Drive/GameSync`, and Drive uploads them.

**7. Wait for Drive to show "up to date,"** then set up your [other
device](#adding-your-other-device).

### A2. OneDrive

**1. Sign in to OneDrive.** It's built into Windows (sign in from the taskbar
cloud icon); on macOS install it from the App Store and sign in.

**2. Find your OneDrive folder on disk.**

- **Windows:** `%USERPROFILE%\OneDrive` (personal) or
  `%USERPROFILE%\OneDrive - <Company>` (work/school).
- **macOS:** `~/Library/CloudStorage/OneDrive-Personal` (newer macOS) or
  `~/OneDrive`.

**3. Create a sync folder.** Make a `GameSync` folder inside OneDrive
(e.g. `C:\Users\<you>\OneDrive\GameSync`).

**4. Keep it on the device.** Right-click the `GameSync` folder → **Always keep
on this device.** This stops OneDrive's Files On-Demand from turning the objects
into online-only placeholders GameSync can't read.

**5. Point GameSync at it.** Same as Drive: **Remote** bar → **Browse…** → pick
`…\OneDrive\GameSync` → **Save** (status → **configured**).

**6. Enable `Sync` per game**, then **Sync now** / **Sync all**.

**7. Wait for OneDrive's green check,** then go to [Adding your other
device](#adding-your-other-device).

---

## Method B — rclone direct (advanced / headless)

[rclone](https://rclone.org) speaks the Google Drive and OneDrive APIs directly,
so you don't need the desktop client at all. This is ideal on Linux, on a server,
or whenever you'd rather not run the official app. GameSync runs `rclone` under
the hood when the remote spec starts with `rclone:`.

**Requirements:** `rclone` must be installed and on your `PATH` (GameSync calls
the `rclone` binary; override the binary path with the `GAMESYNC_RCLONE`
environment variable if needed), and you must configure a remote once.

### B1. Google Drive via rclone

**1. Install rclone** — <https://rclone.org/install/> (or `brew install rclone`,
`winget install Rclone.Rclone`, your distro's package, etc.).

**2. Create the remote:**
```sh
rclone config
```
- `n` for **New remote**
- **name:** `gdrive` (remember this — it becomes part of the GameSync spec)
- **storage:** choose **Google Drive** (`drive`)
- Leave client id/secret blank for the defaults, accept the scope, and complete
  the browser **OAuth** sign-in rclone opens.

**3. Verify it works:**
```sh
rclone mkdir gdrive:GameSync
rclone lsd  gdrive:
```

**4. Point GameSync at it.** In the **Remote** bar type:
```
rclone:gdrive:GameSync
```
Click **Save**, flip on each game's **Sync** toggle, then **Sync now**.

### B2. OneDrive via rclone

**1. Install rclone** (as above).

**2. Create the remote:**
```sh
rclone config
```
- `n` → **name:** `onedrive`
- **storage:** **Microsoft OneDrive** (`onedrive`)
- Complete the **OAuth** sign-in; when prompted pick your account type
  (**OneDrive Personal** or **Business**) and the drive.

**3. Verify:**
```sh
rclone mkdir onedrive:GameSync
rclone lsd  onedrive:
```

**4. In GameSync's Remote bar:**
```
rclone:onedrive:GameSync
```
**Save** → enable per-game **Sync** → **Sync now**.

> The CLI equivalent on any device:
> `gamesync remote set rclone:onedrive:GameSync` then `gamesync sync <game_id>`.

---

## Adding your other device

Sync needs at least two devices pointed at the **same** cloud folder.

1. Install GameSync on device 2 and run through first-time setup (or **Skip
   setup**).
2. Make sure the **same** Drive/OneDrive account is signed in and the `GameSync`
   folder has finished downloading (and is marked offline / always-keep).
3. Configure the **same remote**:
   - **Method A:** Browse to that device's local path to the *same* cloud folder
     (the path differs per OS — e.g. `G:\My Drive\GameSync` on Windows vs
     `~/Library/CloudStorage/GoogleDrive-…/My Drive/GameSync` on macOS — but it's
     the same cloud folder).
   - **Method B:** enter the same `rclone:gdrive:GameSync` /
     `rclone:onedrive:GameSync` spec (after running `rclone config` on this
     device too).
4. Add the **same games** and turn their **Sync** toggles on.
   - Auto-detected (Steam/GOG/Epic) games agree on identity automatically.
   - For a **manually-added** game, give it the **exact same name** on both
     devices so they're recognized as the same game.
5. Click **Sync now**. The device that's behind **pulls and restores** the latest
   save (after taking its own safety snapshot first).

**Typical flow once set up:** Sync **before** you play (pull the latest) and
**after** you play (push your new save). Background auto-sync
([Settings → Automatic sync](../README.md#automatic-background-sync)) can do this
for you on an interval.

---

## Encryption + cloud sync

GameSync's encryption is **zero-knowledge**: if you enable it, everything written
to the remote is **ciphertext**, so Google/Microsoft only ever store encrypted
blobs. Object names stay BLAKE3 hashes of the *plaintext*, so de-duplication
still works across devices — but **decryption needs the shared key.**

That means for encrypted *cross-device* sync, **every device must use the same
keystore** (the same data-encryption key), not just the same passphrase. If each
device independently enables encryption, they generate **different** keys and
won't be able to read each other's objects.

To share one keystore (advanced):

1. Enable encryption on **device 1** (toolbar → **Enable encryption** → set a
   passphrase → **save the recovery key**). Do this *before* backing up games —
   encryption can only be turned on while the store is empty.
2. Copy that device's **`keystore.json`** from its GameSync data directory (see
   [Where your data lives](../README.md#where-your-data-lives)) to the **same
   location** in device 2's data directory, *before* enabling encryption there.
3. On device 2, unlock with the same passphrase. Both devices now share the key,
   so synced objects decrypt on either side.

If you don't need the cloud provider to be blind to your saves, you can skip
encryption entirely — sync works the same, just unencrypted.

---

## Good habits to avoid conflicts

A "conflict" means the same game changed on two devices without a sync in
between. GameSync handles it safely (it never overwrites your live save and keeps
both branches in history), but it's nicer to avoid them:

- **Quit the game before backing up / syncing.** GameSync refuses to snapshot a
  game it detects as running, but quitting also ensures the save is fully flushed.
- **Wait for the cloud "up to date" indicator** on the sending device before
  syncing on the receiving one.
- **Sync at the end of a session** so the next device starts from your latest
  save.
- **One device at a time** for the same game, when you can.
- If a conflict does appear, click **Preview changes**, then **Keep mine** or
  **Take remote** — see [Resolve a
  conflict](../README.md#resolve-a-conflict).

---

## Troubleshooting

**The Remote says "configured" but nothing syncs.**
The per-game **Sync** toggle is probably off. Flip it on for each game, then
**Sync now**. Also confirm a remote is actually set (the pill says *configured*,
not *not set*).

**My other device doesn't see any backups.**
- The cloud client hasn't finished downloading the `GameSync` folder yet — wait
  for "up to date."
- The files are **online-only placeholders.** Mark the folder **Available
  offline** (Drive) / **Always keep on this device** (OneDrive).
- The two devices point at **different** folders. Double-check both paths resolve
  to the *same* cloud folder/account.

**Every sync turns into a conflict.**
You're racing the cloud — device 2 is syncing against a half-downloaded folder.
Wait for both clients to show "up to date" between syncs. Keep one device's
session fully synced before switching.

**rclone: "remote not found" / "didn't recognize the remote."**
The name in your spec must match the name you gave during `rclone config`
(`rclone:gdrive:…` needs a remote literally named `gdrive`). List them with
`rclone listremotes`. Also make sure `rclone` is on your `PATH` (or set
`GAMESYNC_RCLONE` to its full path).

**Saves seem stale / partially uploaded.**
Cloud clients upload asynchronously. Give it a moment and re-check the client's
status before concluding something's wrong. Never edit the contents of the
`GameSync` remote folder by hand.

**Encrypted, but the other device can't read the saves.**
The devices don't share a keystore. See [Encryption + cloud
sync](#encryption--cloud-sync) — copy `keystore.json` across before enabling
encryption on the second device.

---

## Redirect ≠ sync (don't confuse them)

GameSync has a separate **Redirect to synced folder** feature (on each game
card). That one **moves the live save folder** into your Drive/OneDrive and
leaves a symlink behind, so the **cloud client** continuously syncs the *live*
files — there are no GameSync snapshots involved in the transfer.

- Use **this guide's snapshot sync** (Remote + per-game **Sync**) when you want
  versioned history, conflict detection, and safe restores.
- Use **Redirect** when you just want your cloud client to mirror the live save
  with no versioning.

They're independent; most people want snapshot sync. See [Live-sync via your
cloud client](../README.md#live-sync-via-your-cloud-client-redirect-to-synced-folder)
for the Redirect details (and the Windows Developer Mode requirement for
symlinks).
