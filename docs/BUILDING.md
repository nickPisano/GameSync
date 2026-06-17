# Building & signing releases

`npm run tauri build` produces bundles in **`target/release/bundle/`** (the
workspace-root `target/`, not `src-tauri/`) for the OS you run it on. The bare
executable (the "portable" build) sits one level up at `target/release/`.

| OS | Installer | Portable |
| --- | --- | --- |
| **macOS** | `.dmg` | `.app` (drag-to-run, shipped inside the `.dmg`) |
| **Windows** | `.msi` (WiX) + `.exe` (NSIS setup) | `target/release/GameSync.exe` (needs WebView2, preinstalled on Win10+) |
| **Linux** | `.deb` + `.rpm` | `.AppImage` |

`bundle.targets` is `"all"` in `tauri.conf.json`, so every format for the host OS
is emitted. These work locally **unsigned**, but other machines will warn (or
refuse) to run them. Signing requires *your own* certificates — Claude can't and
shouldn't handle those, so this is the manual step. The icon set is already wired
up (`src-tauri/icons/`, regenerate with `npm run tauri icon src-tauri/icons/icon.png`).

## Targets & architectures

Releases cover **macOS (universal), Windows x64 + arm64, and Linux x64 + arm64**.
You build each architecture on a matching machine — the
[`Release` workflow](../.github/workflows/release.yml) does this automatically on
a tag push (`git tag v0.1.0 && git push origin v0.1.0`), using GitHub's native
ARM runners (`windows-11-arm`, `ubuntu-22.04-arm`) so nothing is cross-compiled.

To build a specific target **locally**:

```sh
# macOS universal (one app/dmg that runs on Apple Silicon AND Intel):
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm run tauri build -- --target universal-apple-darwin

# Windows arm64, cross-compiled from an x64 Windows host (MSVC supports this):
rustup target add aarch64-pc-windows-msvc
npm run tauri build -- --target aarch64-pc-windows-msvc

# Linux arm64: build on real arm64 hardware (or the ubuntu-22.04-arm CI runner).
# Cross-compiling Linux arm64 + AppImage from x64 is painful (webkit2gtk multiarch,
# AppImage tooling) — prefer a native arm64 box.
```

Omit `--target` to build for the host architecture (the common case).

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

## CI (automated releases)

[`.github/workflows/release.yml`](../.github/workflows/release.yml) runs on a
`v*` tag push and builds all five targets in parallel via `tauri-action`, which
compiles, (optionally) signs, and attaches the installers to a **draft** GitHub
release; an extra step uploads the portable Windows `.exe`. To ship:

1. Add any signing values as encrypted **repository secrets** (Settings → Secrets
   and variables → Actions): `APPLE_SIGNING_IDENTITY`, `APPLE_ID`,
   `APPLE_PASSWORD`, `APPLE_TEAM_ID` (and the Windows cert thumbprint in
   `tauri.conf.json`). Unset secrets simply produce unsigned builds.
2. Bump `version` in `src-tauri/tauri.conf.json`, then
   `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. Wait for the matrix to finish, review the draft release, and **Publish**.

> The ARM runners (`windows-11-arm`, `ubuntu-22.04-arm`) are free for public
> repos; on a private repo they may require a paid plan — swap arm64 entries for
> cross-compilation or self-hosted runners if so.
