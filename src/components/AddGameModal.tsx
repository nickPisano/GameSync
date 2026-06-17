import { useState } from "react";
import { Modal } from "./Modal";
import { pickFolder } from "../api";

interface Props {
  onClose: () => void;
  onAdd: (name: string, path: string, exePath: string | null) => Promise<void>;
}

export function AddGameModal({ onClose, onAdd }: Props) {
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [exePath, setExePath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      await onAdd(name.trim(), path.trim(), exePath.trim() || null);
      onClose();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  return (
    <Modal title="Add a game" onClose={onClose}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (name && path) submit();
        }}
      >
        <label className="field">
          <span>Name</span>
          <input
            autoFocus
            placeholder="e.g. Dark Souls III"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <label className="field">
          <span>Save folder</span>
          <div className="path-row">
            <input
              placeholder="/Users/you/Documents/MyGame/saves"
              value={path}
              onChange={(e) => setPath(e.target.value)}
            />
            <button
              type="button"
              className="secondary"
              onClick={async () => {
                const p = await pickFolder("Choose the save folder");
                if (p) setPath(p);
              }}
            >
              Browse…
            </button>
          </div>
        </label>
        <label className="field">
          <span>Game folder or app (optional — for auto-backup when it closes)</span>
          <div className="path-row">
            <input
              placeholder="/Applications/MyGame.app  or  the install folder"
              value={exePath}
              onChange={(e) => setExePath(e.target.value)}
            />
            <button
              type="button"
              className="secondary"
              onClick={async () => {
                const p = await pickFolder("Choose the game's folder or app");
                if (p) setExePath(p);
              }}
            >
              Browse…
            </button>
          </div>
        </label>
        <p className="muted small">
          Tip: most automatic games appear via <strong>Scan</strong>. Use this for
          anything not detected. Setting the game folder/app lets GameSync back up
          automatically when you close it.
        </p>
        {error && <div className="error">{error}</div>}
        <div className="modal-foot">
          <button type="button" onClick={onClose} className="secondary">
            Cancel
          </button>
          <button type="submit" disabled={busy || !name || !path}>
            {busy ? "Adding…" : "Add game"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
