// Typed wrappers around the Tauri command layer (src-tauri/src/main.rs).

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  AppStatus,
  AutoSyncReport,
  AutoSyncSettings,
  Diff,
  DiscoveredHost,
  Game,
  GameView,
  GcReport,
  LanHostInfo,
  PluginList,
  PruneResult,
  RedirectReport,
  RetentionPolicy,
  SaveFile,
  Snapshot,
  StorageReport,
  SyncOutcome,
  UpdateInfo,
  VerifyResult,
  ViewerInfo,
} from "./types";

/** Native OS folder picker. Returns the chosen absolute path, or null if cancelled. */
export async function pickFolder(title?: string): Promise<string | null> {
  const result = await open({ directory: true, multiple: false, title });
  return typeof result === "string" ? result : null;
}

export const api = {
  appStatus: () => invoke<AppStatus>("app_status"),
  completeSetup: () => invoke<void>("complete_setup"),
  unlock: (passphrase: string) => invoke<void>("unlock", { passphrase }),
  initEncryption: (passphrase: string) =>
    invoke<string>("init_encryption", { passphrase }),

  listGames: () => invoke<GameView[]>("list_games"),
  scan: () => invoke<Game[]>("scan"),
  updateGameList: () => invoke<number>("update_game_list"),
  knownGameCount: () => invoke<number>("known_game_count"),
  addGame: (name: string, path: string) =>
    invoke<Game>("add_game", { name, path }),
  setSyncEnabled: (id: string, enabled: boolean) =>
    invoke<void>("set_sync_enabled", { id, enabled }),
  setGameExe: (id: string, path: string | null) =>
    invoke<void>("set_game_exe", { id, path }),
  renameGame: (id: string, name: string) =>
    invoke<void>("rename_game", { id, name }),
  removeGame: (id: string) => invoke<void>("remove_game", { id }),
  redirectSaveFolder: (id: string, target: string) =>
    invoke<RedirectReport>("redirect_save_folder", { id, target }),

  backup: (id: string) => invoke<Snapshot>("backup", { id }),
  versions: (id: string) => invoke<Snapshot[]>("versions", { id }),
  restore: (id: string, versionId: string) =>
    invoke<Snapshot>("restore", { id, version: versionId }),
  diff: (id: string, from: string, to: string) =>
    invoke<Diff>("diff", { id, from, to }),

  listSaveFiles: (id: string) => invoke<SaveFile[]>("list_save_files", { id }),
  openFolder: (path: string) => invoke<void>("open_folder", { path }),
  revealFile: (path: string) => invoke<void>("reveal_file", { path }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  checkForUpdate: () => invoke<UpdateInfo>("check_for_update"),

  listPlugins: () => invoke<PluginList>("list_plugins"),
  setPluginEnabled: (id: string, enabled: boolean) =>
    invoke<void>("set_plugin_enabled", { id, enabled }),
  setPluginCommandsAllowed: (allowed: boolean) =>
    invoke<void>("set_plugin_commands_allowed", { allowed }),
  fileViewers: (path: string) => invoke<ViewerInfo[]>("file_viewers", { path }),
  runViewer: (command: string, path: string) =>
    invoke<void>("run_viewer", { command, path }),

  remoteStatus: () => invoke<string | null>("remote_status"),
  setRemote: (path: string) => invoke<void>("set_remote", { path }),
  syncGame: (id: string) => invoke<SyncOutcome>("sync_game", { id }),
  resolveConflict: (id: string, keep: "local" | "remote") =>
    invoke<SyncOutcome>("resolve_conflict", { id, keep }),

  syncAll: () => invoke<AutoSyncReport>("sync_all"),
  getAutoSync: () => invoke<AutoSyncSettings>("get_auto_sync"),
  setAutoSync: (settings: AutoSyncSettings) =>
    invoke<void>("set_auto_sync", { settings }),
  verify: () => invoke<VerifyResult>("verify"),

  storageReport: () => invoke<StorageReport>("storage_report"),
  pruneAll: (policy: RetentionPolicy) => invoke<PruneResult>("prune_all", { policy }),
  runGc: () => invoke<GcReport>("run_gc"),
  setCompression: (enabled: boolean) =>
    invoke<void>("set_compression", { enabled }),

  startLanHost: () => invoke<LanHostInfo>("start_lan_host"),
  stopLanHost: () => invoke<LanHostInfo>("stop_lan_host"),
  lanHostStatus: () => invoke<LanHostInfo>("lan_host_status"),
  discoverLanHosts: () => invoke<DiscoveredHost[]>("discover_lan_hosts"),
};
