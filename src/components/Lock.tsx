import { useState } from "react";

interface Props {
  onUnlock: (passphrase: string) => Promise<void>;
}

/** Shown when the store is encrypted and still locked. */
export function Lock({ onUnlock }: Props) {
  const [passphrase, setPassphrase] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await onUnlock(passphrase);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="lock-screen">
      <form className="lock-box" onSubmit={submit}>
        <h1>GameSync</h1>
        <p className="muted">This store is encrypted. Enter your passphrase to unlock.</p>
        <input
          type="password"
          autoFocus
          placeholder="Passphrase"
          value={passphrase}
          onChange={(e) => setPassphrase(e.target.value)}
        />
        {error && <div className="error">{error}</div>}
        <button type="submit" disabled={busy || passphrase.length === 0}>
          {busy ? "Unlocking…" : "Unlock"}
        </button>
      </form>
    </div>
  );
}
