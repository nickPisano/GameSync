import { useState } from "react";
import { Modal } from "./Modal";

interface Props {
  onClose: () => void;
  onInit: (passphrase: string) => Promise<string>; // returns recovery key
}

export function EncryptModal({ onClose, onInit }: Props) {
  const [passphrase, setPassphrase] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [recovery, setRecovery] = useState<string | null>(null);

  async function enable() {
    if (passphrase !== confirm) {
      setError("Passphrases do not match.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      setRecovery(await onInit(passphrase));
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  if (recovery) {
    return (
      <Modal title="Save your recovery key" onClose={onClose}>
        <p>
          Encryption is on. Store this recovery key somewhere safe — it can unlock
          your saves if you forget the passphrase. It is shown only once.
        </p>
        <pre className="recovery-key">{recovery}</pre>
        <div className="modal-foot">
          <button onClick={onClose}>I&apos;ve saved it</button>
        </div>
      </Modal>
    );
  }

  return (
    <Modal title="Enable encryption" onClose={onClose}>
      <p className="muted small">
        Encrypts all saved data at rest with a passphrase (zero-knowledge). Only
        available on a store with no data yet.
      </p>
      <label className="field">
        <span>Passphrase (min 8 chars)</span>
        <input
          type="password"
          autoFocus
          value={passphrase}
          onChange={(e) => setPassphrase(e.target.value)}
        />
      </label>
      <label className="field">
        <span>Confirm passphrase</span>
        <input
          type="password"
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
        />
      </label>
      {error && <div className="error">{error}</div>}
      <div className="modal-foot">
        <button onClick={onClose} className="secondary">
          Cancel
        </button>
        <button onClick={enable} disabled={busy || passphrase.length < 8}>
          {busy ? "Enabling…" : "Enable"}
        </button>
      </div>
    </Modal>
  );
}
