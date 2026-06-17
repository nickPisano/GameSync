import { useState } from "react";
import { Modal } from "./Modal";

interface Props {
  initial: string;
  onClose: () => void;
  onSave: (name: string) => Promise<void>;
}

export function RenameModal({ initial, onClose, onSave }: Props) {
  const [name, setName] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    const trimmed = name.trim();
    if (!trimmed) return;
    setBusy(true);
    try {
      await onSave(trimmed);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <Modal title="Rename game" onClose={onClose}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <label className="field">
          <span>Name</span>
          <input autoFocus value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        {error && <div className="error">{error}</div>}
        <div className="modal-foot">
          <button type="button" className="secondary" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" disabled={busy || !name.trim()}>
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
