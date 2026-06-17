import { useEffect, useState } from "react";
import { Modal } from "./Modal";
import { api } from "../api";
import type { Diff } from "../types";

interface Props {
  gameId: string;
  gameName: string;
  local: string;
  remote: string;
  onClose: () => void;
  onResolve: (keep: "local" | "remote") => void;
  notify: (msg: string, kind?: "ok" | "err") => void;
}

/** Shows what differs between this device's save and the other device's, then
 *  lets the user resolve the conflict from the same view. */
export function ConflictDiffModal({
  gameId,
  gameName,
  local,
  remote,
  onClose,
  onResolve,
  notify,
}: Props) {
  const [diff, setDiff] = useState<Diff | null>(null);

  useEffect(() => {
    // diff(local -> remote): "added" = on the other device only, "removed" =
    // on yours only, "modified" = different on each.
    api
      .diff(gameId, local, remote)
      .then(setDiff)
      .catch((e) => notify(String(e), "err"));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <Modal title={`Conflict — ${gameName}`} onClose={onClose} wide>
      <p className="muted small">
        Your save and the other device&apos;s diverged. This compares them so you
        can choose. Nothing changes until you pick.
      </p>

      {diff === null ? (
        <p className="muted">Comparing…</p>
      ) : diff.added.length + diff.modified.length + diff.removed.length === 0 ? (
        <p className="muted small">
          The file lists are identical ({diff.unchanged} files) — the difference
          is in edit history/timing. Either side is safe to keep.
        </p>
      ) : (
        <>
          <div className="conflict-legend small muted">
            <span className="diff-add">＋ only on the other device</span>
            <span className="diff-mod">～ different on each</span>
            <span className="diff-del">－ only on this device</span>
          </div>
          <div className="diff-output">
            {diff.added.map((p) => (
              <div key={`a${p}`} className="diff-add">＋ {p}</div>
            ))}
            {diff.modified.map((p) => (
              <div key={`m${p}`} className="diff-mod">～ {p}</div>
            ))}
            {diff.removed.map((p) => (
              <div key={`r${p}`} className="diff-del">－ {p}</div>
            ))}
            <div className="muted small">{diff.unchanged} unchanged</div>
          </div>
        </>
      )}

      <div className="modal-foot">
        <button className="secondary" onClick={onClose}>
          Decide later
        </button>
        <button onClick={() => onResolve("local")}>Keep mine</button>
        <button onClick={() => onResolve("remote")}>Take other device</button>
      </div>
    </Modal>
  );
}
