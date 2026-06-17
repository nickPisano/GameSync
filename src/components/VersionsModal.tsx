import { useEffect, useState } from "react";
import { Modal } from "./Modal";
import { api } from "../api";
import type { Diff, Game, Snapshot } from "../types";
import { humanSize, shortId, timeAgo } from "../format";

interface Props {
  game: Game;
  onClose: () => void;
  onChanged: () => void;
  notify: (msg: string, kind?: "ok" | "err") => void;
}

export function VersionsModal({ game, onClose, onChanged, notify }: Props) {
  const [versions, setVersions] = useState<Snapshot[]>([]);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [diff, setDiff] = useState<Diff | null>(null);
  const [busy, setBusy] = useState(false);

  async function load() {
    const v = await api.versions(game.id);
    setVersions(v);
    if (v.length >= 2) {
      setFrom(v[1].version_id);
      setTo(v[0].version_id);
    }
  }

  useEffect(() => {
    load().catch((e) => notify(String(e), "err"));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function restore(versionId: string) {
    if (!confirm("Restore this version? Your current save is backed up first.")) return;
    setBusy(true);
    try {
      await api.restore(game.id, versionId);
      notify("Restored. A safety snapshot of the previous save was taken.");
      await load();
      onChanged();
    } catch (e) {
      notify(String(e), "err");
    } finally {
      setBusy(false);
    }
  }

  async function runDiff() {
    try {
      setDiff(await api.diff(game.id, from, to));
    } catch (e) {
      notify(String(e), "err");
    }
  }

  return (
    <Modal title={`History — ${game.name}`} onClose={onClose} wide>
      <div className="versions">
        {versions.map((v) => (
          <div className="version-row" key={v.version_id}>
            <div className="version-info">
              <code>{shortId(v.version_id)}</code>
              <span className={`tag tag-${v.kind}`}>{v.kind}</span>
              <span className="muted">
                {v.files.length} files · {humanSize(v.total_size)} ·{" "}
                {timeAgo(v.created_ms)}
              </span>
              {v.label && <span className="version-label">{v.label}</span>}
            </div>
            <button onClick={() => restore(v.version_id)} disabled={busy}>
              Restore
            </button>
          </div>
        ))}
        {versions.length === 0 && <p className="muted">No versions yet.</p>}
      </div>

      {versions.length >= 2 && (
        <div className="diff-panel">
          <h3>Compare versions</h3>
          <div className="diff-controls">
            <select value={from} onChange={(e) => setFrom(e.target.value)}>
              {versions.map((v) => (
                <option key={v.version_id} value={v.version_id}>
                  {shortId(v.version_id)} · {timeAgo(v.created_ms)}
                </option>
              ))}
            </select>
            <span className="muted">→</span>
            <select value={to} onChange={(e) => setTo(e.target.value)}>
              {versions.map((v) => (
                <option key={v.version_id} value={v.version_id}>
                  {shortId(v.version_id)} · {timeAgo(v.created_ms)}
                </option>
              ))}
            </select>
            <button onClick={runDiff} disabled={from === to}>
              Compare
            </button>
          </div>
          {diff && (
            <div className="diff-output">
              {diff.added.map((p) => (
                <div key={`a${p}`} className="diff-add">+ {p}</div>
              ))}
              {diff.modified.map((p) => (
                <div key={`m${p}`} className="diff-mod">~ {p}</div>
              ))}
              {diff.removed.map((p) => (
                <div key={`r${p}`} className="diff-del">- {p}</div>
              ))}
              <div className="muted small">
                {diff.added.length + diff.modified.length + diff.removed.length}{" "}
                changed, {diff.unchanged} unchanged
              </div>
            </div>
          )}
        </div>
      )}
    </Modal>
  );
}
