import { useState } from "react";
import { api, pickFolder } from "../api";
import type { Game } from "../types";

interface Props {
  onDone: () => void;
  notify: (msg: string, kind?: "ok" | "err") => void;
}

/** First-run guided setup: mode → remote → storage → pick games. */
export function Wizard({ onDone, notify }: Props) {
  const [step, setStep] = useState(1);
  const [mode, setMode] = useState<"local" | "cloud" | null>(null);
  const [remotePath, setRemotePath] = useState("");
  const [compress, setCompress] = useState(false);
  const [scanned, setScanned] = useState<Game[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function chooseMode(m: "local" | "cloud") {
    setMode(m);
    setError(null);
    setStep(m === "cloud" ? 2 : 3);
  }

  async function browseRemote() {
    const p = await pickFolder("Choose a shared/cloud folder for sync");
    if (p) setRemotePath(p);
  }

  async function saveRemote() {
    if (!remotePath.trim()) {
      setError("Pick a folder, or go Back and choose Local-only.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await api.setRemote(remotePath.trim());
      setStep(3);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function gotoGames() {
    setStep(4);
    setBusy(true);
    setError(null);
    try {
      const games = await api.scan();
      setScanned(games);
      setSelected(new Set(games.map((g) => g.id)));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  // Apply the compression choice (store is still empty here) then continue.
  async function storageContinue() {
    try {
      await api.setCompression(compress);
    } catch (e) {
      setError(String(e));
      return;
    }
    await gotoGames();
  }

  function toggleGame(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  }

  async function finish() {
    setBusy(true);
    setError(null);
    try {
      for (const g of scanned) {
        if (selected.has(g.id)) await api.setSyncEnabled(g.id, true);
      }
      await api.completeSetup();
      notify("Setup complete — welcome to GameSync!");
      onDone();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  async function skip() {
    try {
      await api.completeSetup();
    } catch {
      /* ignore */
    }
    onDone();
  }

  return (
    <div className="wizard">
      <div className="wizard-card">
        <div className="stepper">Setup · step {step} of 4</div>

        {step === 1 && (
          <>
            <h1>Welcome to GameSync</h1>
            <p className="muted">How do you want to use it?</p>
            <div className="mode-grid">
              <button className="mode-option" onClick={() => chooseMode("local")}>
                <strong>Local backups only</strong>
                <span className="muted small">
                  Keep versioned backups of your saves on this machine.
                </span>
              </button>
              <button className="mode-option" onClick={() => chooseMode("cloud")}>
                <strong>Sync across devices</strong>
                <span className="muted small">
                  Back up <em>and</em> sync saves through a shared/cloud folder.
                </span>
              </button>
            </div>
          </>
        )}

        {step === 2 && (
          <>
            <h1>Choose a sync folder</h1>
            <p className="muted small">
              Point GameSync at a folder you already sync (Dropbox, Google Drive,
              OneDrive, a network share). Use the <strong>same folder</strong> on
              each device.
            </p>
            <div className="path-row">
              <input
                placeholder="/Users/you/Dropbox/GameSync"
                value={remotePath}
                onChange={(e) => setRemotePath(e.target.value)}
              />
              <button className="secondary" onClick={browseRemote}>
                Browse…
              </button>
            </div>
            {error && <div className="error">{error}</div>}
            <div className="wizard-foot">
              <button className="secondary" onClick={() => setStep(1)}>
                Back
              </button>
              <button onClick={saveRemote} disabled={busy}>
                Continue
              </button>
            </div>
          </>
        )}

        {step === 3 && (
          <>
            <h1>Storage options</h1>
            <p className="muted small">
              Compression can only be enabled now, on a fresh store. Encryption is
              optional and can be turned on later from <strong>Settings</strong>.
            </p>
            <label className="toggle-row">
              <input
                type="checkbox"
                checked={compress}
                onChange={(e) => setCompress(e.target.checked)}
              />
              <span>
                Compress backups (LZMA2 / 7-Zip)
                <span className="muted"> — smaller backups and uploads</span>
              </span>
            </label>
            {error && <div className="error">{error}</div>}
            <div className="wizard-foot">
              <button className="secondary" onClick={() => setStep(mode === "cloud" ? 2 : 1)}>
                Back
              </button>
              <button onClick={storageContinue} disabled={busy}>
                Continue
              </button>
            </div>
          </>
        )}

        {step === 4 && (
          <>
            <h1>Add your games</h1>
            {busy ? (
              <p className="muted">Scanning for installed games…</p>
            ) : scanned.length === 0 ? (
              <p className="muted small">
                No games auto-detected. You can add them manually after setup with
                the <strong>Add game</strong> button.
              </p>
            ) : (
              <>
                <p className="muted small">
                  Detected {scanned.length}. Pick which to track
                  {mode === "cloud" ? " & sync" : ""}:
                </p>
                <div className="game-pick">
                  {scanned.map((g) => (
                    <label key={g.id} className="game-pick-row">
                      <input
                        type="checkbox"
                        checked={selected.has(g.id)}
                        onChange={() => toggleGame(g.id)}
                      />
                      <span>{g.name}</span>
                      <span className={`badge badge-${g.platform}`}>{g.platform}</span>
                    </label>
                  ))}
                </div>
              </>
            )}
            {error && <div className="error">{error}</div>}
            <div className="wizard-foot">
              <button className="secondary" onClick={() => setStep(3)} disabled={busy}>
                Back
              </button>
              <button onClick={finish} disabled={busy}>
                Finish
              </button>
            </div>
          </>
        )}

        <button className="link-btn" onClick={skip}>
          Skip setup
        </button>
      </div>
    </div>
  );
}
