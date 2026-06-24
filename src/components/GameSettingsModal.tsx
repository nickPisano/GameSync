import { useState } from "react";
import { Modal } from "./Modal";
import { api, pickFolder } from "../api";
import type { Game } from "../types";

interface Props {
  game: Game;
  onClose: () => void;
  notify: (msg: string, kind?: "ok" | "err") => void;
  /** Refresh the library after a change. */
  onChanged: () => Promise<void> | void;
}

/** Per-game settings: extra folders to back up, and the location watched to
 *  auto-back-up when the game closes. */
export function GameSettingsModal({ game, onClose, notify, onChanged }: Props) {
  const [roots, setRoots] = useState<string[]>(game.extra_roots ?? []);
  const [watched, setWatched] = useState<string | null>(game.install_dir);
  const [busy, setBusy] = useState(false);

  async function saveRoots(next: string[], msg?: string) {
    setBusy(true);
    try {
      await api.setExtraRoots(game.id, next);
      setRoots(next);
      await onChanged();
      if (msg) notify(msg);
    } catch (e) {
      notify(String(e), "err");
    } finally {
      setBusy(false);
    }
  }

  async function addFolder() {
    const p = await pickFolder("Choose an extra folder to back up with this game");
    if (!p) return;
    if (p === game.save_root || roots.includes(p)) {
      notify("That folder is already backed up for this game.", "err");
      return;
    }
    await saveRoots([...roots, p], "Extra folder added — included from the next backup.");
  }

  async function setWatchedLocation() {
    const p = await pickFolder("Choose the game's folder or app (to detect when it closes)");
    if (!p) return;
    setBusy(true);
    try {
      await api.setGameExe(game.id, p);
      setWatched(p);
      await onChanged();
      notify("Auto-backup-on-close location updated.");
    } catch (e) {
      notify(String(e), "err");
    } finally {
      setBusy(false);
    }
  }

  async function clearWatched() {
    setBusy(true);
    try {
      await api.setGameExe(game.id, null);
      setWatched(null);
      await onChanged();
    } catch (e) {
      notify(String(e), "err");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title={`${game.name} — settings`} onClose={onClose}>
      <div className="settings-section">
        <h3 className="settings-h">Extra backup folders</h3>
        <p className="muted small">
          Backed up and restored together with the main save folder, as one version.
        </p>
        <ul className="root-list">
          <li className="root-item">
            <code>{game.save_root}</code>
            <span className="root-tag">main</span>
          </li>
          {roots.map((r) => (
            <li key={r} className="root-item">
              <code>{r}</code>
              <button
                className="linklike linklike-danger"
                onClick={() => saveRoots(roots.filter((x) => x !== r))}
                disabled={busy}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
        <button className="secondary" onClick={addFolder} disabled={busy}>
          Add folder…
        </button>
      </div>

      <div className="settings-section">
        <h3 className="settings-h">Auto-backup on close</h3>
        <p className="muted small">
          The folder or app GameSync watches to detect when this game closes, so it can back up
          automatically. (Steam games are detected for you.)
        </p>
        {watched ? (
          <div className="root-item">
            <code>{watched}</code>
            <button className="linklike" onClick={clearWatched} disabled={busy}>
              Clear
            </button>
          </div>
        ) : (
          <p className="muted small">Not set.</p>
        )}
        <button className="secondary" onClick={setWatchedLocation} disabled={busy}>
          {watched ? "Change…" : "Choose folder or app…"}
        </button>
      </div>

      <div className="modal-foot">
        <button onClick={onClose}>Done</button>
      </div>
    </Modal>
  );
}
