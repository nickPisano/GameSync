// Mirrors the serde shapes from gamesync-core. Kept in sync by hand.

export type Platform = "steam" | "gog" | "epic" | "emulator" | "manual";
export type SnapshotKind = "manual" | "auto" | "pre_restore";

export interface Game {
  id: string;
  name: string;
  platform: Platform;
  save_root: string;
  install_dir: string | null;
  includes: string[];
  excludes: string[];
  sync_enabled: boolean;
}

export interface GameView {
  game: Game;
  version_count: number;
  last_backup_ms: number | null;
}

export interface FileEntry {
  rel_path: string;
  hash: string;
  size: number;
  mtime_ms: number;
  mode: number;
}

export interface Snapshot {
  version_id: string;
  game_id: string;
  device_id: string;
  created_ms: number;
  label: string | null;
  kind: SnapshotKind;
  parent: string | null;
  vclock: Record<string, number>;
  total_size: number;
  files: FileEntry[];
}

export interface Diff {
  added: string[];
  removed: string[];
  modified: string[];
  unchanged: number;
}

export interface AppStatus {
  encrypted: boolean;
  unlocked: boolean;
  setup_complete: boolean;
  data_dir: string;
}

// SyncOutcome is internally tagged on `status`.
export type SyncOutcome =
  | { status: "in_sync" }
  | { status: "pushed"; version_id: string }
  | { status: "pulled"; version_id: string }
  | { status: "conflict"; local: string; remote: string };

export interface AutoSyncSettings {
  enabled: boolean;
  interval_min: number;
  backup_on_exit: boolean;
}

export interface ConflictInfo {
  game_id: string;
  game_name: string;
  local: string;
  remote: string;
}

export interface AutoSyncReport {
  games: number;
  backed_up: number;
  pushed: number;
  pulled: number;
  in_sync: number;
  skipped_running: number;
  conflicts: ConflictInfo[];
  errors: [string, string][]; // [game_name, message]
}

export interface VerifyResult {
  ok: boolean;
  versions_checked: number;
  objects_checked: number;
  problems: string[];
}

export interface RedirectReport {
  linked_target: string;
  original_backup: string;
}

export interface SaveFile {
  rel_path: string;
  abs_path: string;
  size: number;
  mtime_ms: number;
}

export interface GameStorage {
  game_id: string;
  name: string;
  versions: number;
  bytes: number;
}

export interface StorageReport {
  total_objects: number;
  total_bytes: number;
  compressed: boolean;
  games: GameStorage[];
}

export interface RetentionPolicy {
  keep_last: number;
  keep_days: number | null;
}

export interface PruneResult {
  versions_deleted: number;
  objects_deleted: number;
  bytes_freed: number;
}

export interface GcReport {
  objects_deleted: number;
  bytes_freed: number;
}

export interface PluginInfo {
  id: string;
  name: string;
  enabled: boolean;
  games: number;
  emulators: number;
  hooks: number;
  viewers: number;
}

export interface PluginList {
  dir: string;
  commands_allowed: boolean;
  plugins: PluginInfo[];
  errors: [string, string][]; // [plugin_id, error]
}

export interface ViewerInfo {
  plugin: string;
  name: string;
  command: string;
}

export interface LanHostInfo {
  hosting: boolean;
  ip: string | null;
  port: number | null;
  token: string | null;
  spec: string | null;
}
