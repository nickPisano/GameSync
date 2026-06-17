import { useEffect, useState } from "react";
import { Modal } from "./Modal";
import { api } from "../api";
import type { Game, SaveFile, ViewerInfo } from "../types";
import { humanSize, timeAgo } from "../format";

interface Props {
  game: Game;
  onClose: () => void;
  notify: (msg: string, kind?: "ok" | "err") => void;
}

/** Lists the actual files in a game's save folder, with reveal-in-Finder and
 *  any matching plugin file-viewers. */
export function FilesModal({ game, onClose, notify }: Props) {
  const [files, setFiles] = useState<SaveFile[] | null>(null);
  const [viewers, setViewers] = useState<Record<string, ViewerInfo[]>>({});

  useEffect(() => {
    (async () => {
      try {
        const fs = await api.listSaveFiles(game.id);
        setFiles(fs);
        // Only query viewers when plugins may actually provide them.
        const plugins = await api.listPlugins();
        if (plugins.commands_allowed && plugins.plugins.some((p) => p.viewers > 0)) {
          const map: Record<string, ViewerInfo[]> = {};
          await Promise.all(
            fs.map(async (f) => {
              const vs = await api.fileViewers(f.abs_path);
              if (vs.length) map[f.rel_path] = vs;
            })
          );
          setViewers(map);
        }
      } catch (e) {
        notify(String(e), "err");
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const total = files?.reduce((n, f) => n + f.size, 0) ?? 0;

  return (
    <Modal title={`Save files — ${game.name}`} onClose={onClose} wide>
      <div className="files-head">
        <code className="files-root" title={game.save_root}>
          {game.save_root}
        </code>
        <button
          className="secondary"
          onClick={() =>
            api.openFolder(game.save_root).catch((e) => notify(String(e), "err"))
          }
        >
          Open save folder
        </button>
      </div>

      {files === null ? (
        <p className="muted">Reading save folder…</p>
      ) : files.length === 0 ? (
        <p className="muted small">
          No save files found in this folder (it may be empty, or everything is
          excluded).
        </p>
      ) : (
        <>
          <div className="versions">
            {files.map((f) => (
              <div className="version-row" key={f.rel_path}>
                <div className="version-info">
                  <span className="file-name">{f.rel_path}</span>
                  <span className="muted">
                    {humanSize(f.size)} · {timeAgo(f.mtime_ms)}
                  </span>
                </div>
                <div className="file-actions">
                  {(viewers[f.rel_path] ?? []).map((v) => (
                    <button
                      key={v.name}
                      className="secondary"
                      title={`${v.plugin}: ${v.command}`}
                      onClick={() =>
                        api
                          .runViewer(v.command, f.abs_path)
                          .catch((e) => notify(String(e), "err"))
                      }
                    >
                      {v.name}
                    </button>
                  ))}
                  <button
                    className="secondary"
                    title="Reveal this file in your file manager"
                    onClick={() =>
                      api.revealFile(f.abs_path).catch((e) => notify(String(e), "err"))
                    }
                  >
                    Reveal
                  </button>
                </div>
              </div>
            ))}
          </div>
          <div className="muted small" style={{ marginTop: 8 }}>
            {files.length} file{files.length === 1 ? "" : "s"} · {humanSize(total)}
          </div>
        </>
      )}
    </Modal>
  );
}
