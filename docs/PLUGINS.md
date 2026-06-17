# Writing GameSync plugins

Plugins are plain **JSON files** you drop in GameSync's plugins folder — one file
per plugin. Open **Plugins** in the app to see the exact folder path and to
enable/disable each one. A single plugin can declare any combination of:

- **game definitions** — add games/emulators to detection (pure data, always safe);
- **hooks** — run a command before/after a backup or restore;
- **file viewers** — open matching save files with an external program.

> Hooks and viewers run shell commands. They only execute after you turn on
> **"Allow plugins to run commands"** in the Plugins screen (off by default).
> Only install plugins you trust — a plugin command can do anything your user
> account can.

## File format

`<plugins-folder>/my-plugin.json` (the filename is the plugin's id):

```json
{
  "name": "My plugin",

  "games": {
    "steam:391540": {
      "name": "Undertale",
      "paths": ["{APPDATA}/UNDERTALE"]
    }
  },

  "emulators": {
    "yuzu": {
      "name": "yuzu (Switch)",
      "paths": ["{APPDATA}/yuzu/nand", "{XDG_DATA}/yuzu/nand"]
    }
  },

  "hooks": {
    "pre_backup":  "echo backing up {game}",
    "post_backup": "echo done {game}",
    "pre_restore": "echo restoring {game}",
    "post_restore": "notify-send \"Restored {game}\""
  },

  "viewers": [
    { "name": "Hex editor", "match": "*.sl2", "command": "hexedit {file}" }
  ]
}
```

Every field is optional — include only what your plugin needs.

### `games` / `emulators`

Keyed by Steam appid (`"steam:<id>"`) or an emulator id. Each rule has a `name`
and `paths` (tried in order; the first that exists wins). Path templates expand
these placeholders per-OS — unresolved ones are skipped:

`{APPDATA}` `{LOCALAPPDATA}` `{DOCUMENTS}` `{SAVEDGAMES}` `{HOME}`
`{APPSUPPORT}` (macOS) `{XDG_DATA}` `{XDG_CONFIG}` (Linux) `{INSTALL_DIR}`

These merge with GameSync's bundled manifests on the next **Scan** (a plugin
entry overrides the bundled one for the same key).

### `hooks`

Commands run around manual backup/restore. Placeholders: `{game}`, `{game_id}`,
`{save_dir}`, and (restore only) `{version}`. Pre-hooks run before the action;
a failing hook is logged but does not abort the action.

### `viewers`

Each viewer maps a filename glob (`match`) to a `command` containing `{file}`.
In a game's **Files** view, matching files get an "Open with …" button. The
command is launched without waiting (ideal for GUI tools).

## Tips

- Click **Reload** in the Plugins screen after editing a file.
- Malformed files are listed with their parse error instead of being applied.
- Game/emulator definitions need no command permission; only hooks and viewers do.
