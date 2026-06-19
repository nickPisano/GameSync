# Games detected on install

GameSync ships with a small **curated save-path manifest** baked into the app.
The moment you click **Scan**, it recognizes these **72 games** and **10
emulators** automatically — locating their save folders (and matching the same
games across Steam, GOG, and Epic) and adding them to your library with zero
setup.

> **This is only the starter set.** You don't need to reinstall to get more —
> inside the app, **Settings → Game detection → Update game list** pulls the
> full community database of **~17,000 games** (PCGamingWiki via
> [Ludusavi](https://github.com/mtkennerly/ludusavi)). After it updates, run
> **Scan** again to pick up the newly recognized titles. Anything still missing
> you can always [add manually](../README.md#add-a-game-manually).

Detection keys are Steam app IDs; GOG/Epic copies are matched to the same rules
by title (those stores carry no Steam app id locally).

## Games (72)

### FromSoftware & action RPGs
- Dark Souls: Prepare to Die Edition
- Dark Souls: Remastered
- Dark Souls II: Scholar of the First Sin
- Dark Souls III
- Sekiro: Shadows Die Twice
- Elden Ring
- Armored Core VI
- Nioh
- NieR: Automata

### Bethesda RPGs
- The Elder Scrolls III: Morrowind
- The Elder Scrolls IV: Oblivion
- The Elder Scrolls V: Skyrim
- Skyrim Special Edition
- Skyrim VR
- Fallout 3
- Fallout: New Vegas
- Fallout 4
- Fallout 4 VR
- Starfield

### CD Projekt Red
- The Witcher 2
- The Witcher 3: Wild Hunt
- Cyberpunk 2077

### CRPGs (Larian & Obsidian)
- Baldur's Gate 3
- Divinity: Original Sin 2
- Pillars of Eternity
- Pillars of Eternity II: Deadfire

### Paradox grand strategy
- Crusader Kings II
- Crusader Kings III
- Europa Universalis IV
- Hearts of Iron IV
- Stellaris
- Victoria 3
- Imperator: Rome

### Strategy & simulation
- Sid Meier's Civilization V
- Sid Meier's Civilization VI
- XCOM: Enemy Unknown
- XCOM 2
- Space Engineers
- Cities: Skylines
- Cities: Skylines II

### Survival & sandbox
- Stardew Valley
- Terraria
- Starbound
- Factorio
- RimWorld
- Valheim
- The Forest
- Sons of the Forest
- ARK: Survival Evolved
- Kenshi
- Don't Starve
- Don't Starve Together
- Project Zomboid

### Roguelikes & indies
- Hades
- Noita
- Slay the Spire
- Cuphead
- Celeste
- Undertale
- Deltarune
- Enter the Gungeon
- Repentance (Binding of Isaac)
- Risk of Rain 2
- Hollow Knight
- Dead Cells
- Geometry Dash

### Co-op & other
- Borderlands GOTY
- Borderlands 2
- Borderlands: The Pre-Sequel
- Borderlands 3
- Castle Crashers
- Amnesia: A Machine for Pigs

## Emulators (10)

On **Scan**, GameSync also auto-detects these emulators' save directories:

- Dolphin (GameCube/Wii)
- PCSX2 (PlayStation 2)
- RPCS3 (PlayStation 3)
- PPSSPP (PSP)
- DuckStation (PlayStation 1)
- RetroArch
- Citra (3DS)
- yuzu (Switch)
- Ryujinx (Switch)
- ScummVM

## Standalone games (2)

Non-store titles (free / itch.io / modpacks) have no Steam/GOG/Epic id, so they're
detected by probing their fixed save folder — added automatically on **Scan** when
that folder exists:

- Voices of the Void — `%LOCALAPPDATA%\VotV\Saved\SaveGames`
- S.T.A.L.K.E.R. G.A.M.M.A. — best-effort: common install paths such as
  `C:\G.A.M.M.A\Anomaly\appdata\savedgames`. GAMMA installs to a folder *you*
  pick, so if yours lives elsewhere (another drive, etc.) it won't auto-detect —
  just use **Add game** and point at your `…\Anomaly\appdata\savedgames`.

## Don't see your game?

- **Pull the big list (in-app):** **Settings → Game detection → Update game
  list** (~17,000 games), then **Scan** again. *(CLI: `gamesync update-list`.)*
- **Add it manually:** **Add game** → name it and point at its save folder. See
  [Add a game manually](../README.md#add-a-game-manually).
- **Contribute a rule:** the bundled rules live in
  [`crates/gamesync-core/manifests/saves.json`](../crates/gamesync-core/manifests/saves.json)
  (games) and
  [`emulators.json`](../crates/gamesync-core/manifests/emulators.json)
  (emulators); plugins can add more without touching the app.
