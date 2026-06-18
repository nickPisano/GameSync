import type { GameView } from "../types";
import { timeAgo } from "../format";
import { SyncSpinner } from "./SyncSpinner";

interface Props {
  view: GameView;
  busy: boolean;
  syncing: boolean;
  remoteConfigured: boolean;
  onToggleSync: (enabled: boolean) => void;
  onBackup: () => void;
  onHistory: () => void;
  onFiles: () => void;
  onSync: () => void;
  onSetExe: () => void;
  onRename: () => void;
  onRemove: () => void;
  onRedirect: () => void;
}

export function GameCard({
  view,
  busy,
  syncing,
  remoteConfigured,
  onToggleSync,
  onBackup,
  onHistory,
  onFiles,
  onSync,
  onSetExe,
  onRename,
  onRemove,
  onRedirect,
}: Props) {
  const { game, version_count, last_backup_ms } = view;
  return (
    <div className="card">
      <div className="card-main">
        <div className="card-title-row">
          <span className="card-title">{game.name}</span>
          <span className={`badge badge-${game.platform}`}>{game.platform}</span>
        </div>
        <div className="card-path" title={game.save_root}>
          {game.save_root}
        </div>
        <div className="card-meta">
          {version_count} version{version_count === 1 ? "" : "s"} · last backup{" "}
          {timeAgo(last_backup_ms)}
        </div>
        <div className="card-meta">
          {game.install_dir ? (
            <span title={game.install_dir}>↻ backs up automatically when it closes</span>
          ) : (
            <button className="linklike" onClick={onSetExe} disabled={busy}>
              + set up auto-backup on close
            </button>
          )}
        </div>
        <div className="card-meta card-secondary">
          <button className="linklike" onClick={onRename} disabled={busy}>
            Rename
          </button>
          <span className="muted">·</span>
          <button
            className="linklike"
            onClick={onRedirect}
            disabled={busy}
            title="Move this save folder into a synced folder (e.g. OneDrive) and leave a symlink"
          >
            Redirect to synced folder
          </button>
          <span className="muted">·</span>
          <button className="linklike linklike-danger" onClick={onRemove} disabled={busy}>
            Remove
          </button>
        </div>
      </div>

      <div className="card-actions">
        <label className="toggle" title="Enable automatic sync for this game">
          <input
            type="checkbox"
            checked={game.sync_enabled}
            disabled={busy}
            onChange={(e) => onToggleSync(e.target.checked)}
          />
          <span className="switch" aria-hidden="true" />
          <span>Sync</span>
        </label>
        <button onClick={onBackup} disabled={busy}>
          Back up
        </button>
        <button onClick={onFiles} disabled={busy}>
          Files
        </button>
        <button onClick={onHistory} disabled={busy || version_count === 0}>
          History
        </button>
        <button
          onClick={onSync}
          disabled={busy || !remoteConfigured}
          title={remoteConfigured ? "Sync with remote" : "Configure a remote first"}
        >
          {syncing ? (
            <>
              <SyncSpinner /> Syncing…
            </>
          ) : (
            "Sync now"
          )}
        </button>
      </div>
    </div>
  );
}
