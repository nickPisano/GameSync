# Building & signing releases

`npm run tauri build` produces installable bundles in
`src-tauri/target/release/bundle/` for the OS you run it on:

- **macOS:** `.app` and `.dmg`
- **Windows:** `.msi` (WiX) and/or `.exe` (NSIS)
- **Linux:** `.AppImage` and `.deb`

These work locally **unsigned**, but other machines will warn (or refuse) to run
them. Signing requires *your own* certificates — Claude can't and shouldn't
handle those, so this is the manual step. The icon set is already wired up
(`src-tauri/icons/`, regenerate with `npm run tauri icon src-tauri/icons/icon.png`).

> **Never commit** certificates, `.p12`/`.key` files, passwords, or the updater
> private key. Use environment variables locally and encrypted CI secrets.
> `.gitignore` already excludes the common ones.

## macOS — sign & notarize

You need an Apple Developer account and a **Developer ID Application**
certificate in your Keychain. Tauri reads these env vars at build time:

```sh
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
# notarization (app-specific password from appleid.apple.com):
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="abcd-efgh-ijkl-mnop"
export APPLE_TEAM_ID="TEAMID"

npm run tauri build
```

Tauri signs the `.app`/`.dmg` and submits for notarization automatically when
these are set. Verify afterwards with `spctl -a -vvv "path/to/GameSync.app"`.

## Windows — sign

With a code-signing certificate, set its thumbprint in
`src-tauri/tauri.conf.json` under `bundle.windows`:

```json
"windows": { "certificateThumbprint": "AABBCC…", "digestAlgorithm": "sha256",
             "timestampUrl": "http://timestamp.digicert.com" }
```

or use cloud signing (Azure Trusted Signing) per the Tauri docs. Then
`npm run tauri build`.

## Linux

AppImage/deb are typically distributed unsigned; if you publish an apt repo you
can GPG-sign it. No app-level signing is required to run them.

## Auto-updater signing (when the updater is added)

The updater isn't wired up yet, but when it is, releases must be signed with a
Tauri updater key:

```sh
npm run tauri signer generate -- -w ~/.tauri/gamesync_updater.key
```

This prints a **public key** (put it in `tauri.conf.json` →
`plugins.updater.pubkey`) and writes a **private key** (keep it secret; expose
to the build via `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`).
The updater verifies each release against the public key.

## CI sketch

Build per-OS on a matching runner (GitHub Actions `macos-latest`,
`windows-latest`, `ubuntu-latest`), inject the signing secrets as encrypted
repository secrets, and run `npm ci && npm run tauri build`. `tauri-action`
automates building, signing, and attaching artifacts to a GitHub Release.
