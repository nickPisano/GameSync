import { useEffect, useState } from "react";
import { Modal } from "./Modal";
import { api } from "../api";
import type { PluginList } from "../types";

interface Props {
  onClose: () => void;
  notify: (msg: string, kind?: "ok" | "err") => void;
}

/** Lists drop-in plugins and the opt-in for letting them run commands. */
export function PluginsModal({ onClose, notify }: Props) {
  const [list, setList] = useState<PluginList | null>(null);

  const load = () =>
    api.listPlugins().then(setList).catch((e) => notify(String(e), "err"));

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function toggle(id: string, enabled: boolean) {
    try {
      await api.setPluginEnabled(id, enabled);
      await load();
    } catch (e) {
      notify(String(e), "err");
    }
  }

  async function setCommands(allowed: boolean) {
    try {
      await api.setPluginCommandsAllowed(allowed);
      await load();
    } catch (e) {
      notify(String(e), "err");
    }
  }

  return (
    <Modal title="Plugins" onClose={onClose} wide>
      <p className="muted small">
        Drop <code>.json</code> files in the plugins folder to add games, emulator
        save paths, backup/restore hooks, or file viewers. Click Reload after
        editing.
      </p>
      <div className="files-head">
        <code className="files-root" title={list?.dir}>
          {list?.dir ?? "…"}
        </code>
        <button
          className="secondary"
          onClick={() => list && api.openFolder(list.dir).catch((e) => notify(String(e), "err"))}
        >
          Open folder
        </button>
        <button className="secondary" onClick={load}>
          Reload
        </button>
      </div>

      <label className="toggle-row">
        <input
          type="checkbox"
          checked={list?.commands_allowed ?? false}
          disabled={!list}
          onChange={(e) => setCommands(e.target.checked)}
        />
        <span>
          Allow plugins to run commands (hooks &amp; file viewers).{" "}
          <span className="muted">
            Off by default — this lets plugin files execute programs on your
            computer. Only enable if you trust your plugins.
          </span>
        </span>
      </label>

      {list && list.plugins.length === 0 && (
        <p className="muted small">No plugins installed yet.</p>
      )}

      <div className="versions">
        {list?.plugins.map((p) => (
          <div className="version-row" key={p.id}>
            <div className="version-info">
              <span className="card-title">{p.name}</span>
              <code>{p.id}</code>
              <span className="muted small">
                {p.games} games · {p.emulators} emulators · {p.hooks} hooks ·{" "}
                {p.viewers} viewers
              </span>
            </div>
            <label className="toggle">
              <input
                type="checkbox"
                checked={p.enabled}
                onChange={(e) => toggle(p.id, e.target.checked)}
              />
              <span>{p.enabled ? "On" : "Off"}</span>
            </label>
          </div>
        ))}
      </div>

      {list && list.errors.length > 0 && (
        <div className="verify-result bad">
          Some plugin files could not be loaded:
          {list.errors.map(([id, err]) => (
            <div key={id} className="small">
              {id}.json — {err}
            </div>
          ))}
        </div>
      )}
    </Modal>
  );
}
