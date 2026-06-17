import { useEffect, useState, type CSSProperties } from "react";
import { Modal } from "./Modal";
import { api } from "../api";
import type { LanHostInfo, StorageReport, VerifyResult } from "../types";
import { humanSize } from "../format";
import {
  applyTheme,
  getPreference,
  BUILTIN,
  loadCustomThemes,
  importThemeFromJson,
  removeCustomTheme,
  themeTemplate,
  type CustomTheme,
} from "../theme";

interface Props {
  onClose: () => void;
  notify: (msg: string, kind?: "ok" | "err") => void;
}

export function SettingsModal({ onClose, notify }: Props) {
  const [enabled, setEnabled] = useState(false);
  const [interval, setIntervalMin] = useState(15);
  const [backupOnExit, setBackupOnExit] = useState(true);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verifyResult, setVerifyResult] = useState<VerifyResult | null>(null);
  const [lan, setLan] = useState<LanHostInfo | null>(null);
  const [lanBusy, setLanBusy] = useState(false);
  const [storage, setStorage] = useState<StorageReport | null>(null);
  const [keepLast, setKeepLast] = useState(20);
  const [keepDaysOn, setKeepDaysOn] = useState(true);
  const [keepDays, setKeepDays] = useState(30);
  const [storageBusy, setStorageBusy] = useState(false);
  const [gameCount, setGameCount] = useState<number | null>(null);
  const [updatingList, setUpdatingList] = useState(false);

  async function updateList() {
    setUpdatingList(true);
    try {
      const n = await api.updateGameList();
      setGameCount(n);
      notify(`Game list updated — ${n.toLocaleString()} games now auto-detectable.`);
    } catch (e) {
      notify(String(e), "err");
    } finally {
      setUpdatingList(false);
    }
  }

  const loadStorage = () =>
    api.storageReport().then(setStorage).catch((e) => notify(String(e), "err"));

  async function applyRetention() {
    if (
      !confirm(
        `Keep the newest ${keepLast} version(s)` +
          (keepDaysOn ? ` (and anything from the last ${keepDays} days)` : "") +
          " of every game and delete the rest? This can't be undone."
      )
    )
      return;
    setStorageBusy(true);
    try {
      const r = await api.pruneAll({ keep_last: keepLast, keep_days: keepDaysOn ? keepDays : null });
      notify(`Removed ${r.versions_deleted} version(s); reclaimed ${humanSize(r.bytes_freed)}.`);
      await loadStorage();
    } catch (e) {
      notify(String(e), "err");
    } finally {
      setStorageBusy(false);
    }
  }

  async function toggleCompression(on: boolean) {
    setStorageBusy(true);
    try {
      await api.setCompression(on);
      notify(on ? "Compression enabled for new backups." : "Compression disabled.");
      await loadStorage();
    } catch (e) {
      notify(String(e), "err");
    } finally {
      setStorageBusy(false);
    }
  }

  async function reclaim() {
    setStorageBusy(true);
    try {
      const r = await api.runGc();
      notify(`Reclaimed ${humanSize(r.bytes_freed)} from ${r.objects_deleted} object(s).`);
      await loadStorage();
    } catch (e) {
      notify(String(e), "err");
    } finally {
      setStorageBusy(false);
    }
  }
  const [pref, setPref] = useState<string>(getPreference());
  const [customs, setCustoms] = useState<CustomTheme[]>(loadCustomThemes());
  const [importOpen, setImportOpen] = useState(false);
  const [importText, setImportText] = useState("");
  const [importErr, setImportErr] = useState<string | null>(null);

  function choosePref(p: string) {
    applyTheme(p);
    setPref(p);
  }

  function doImport() {
    try {
      const t = importThemeFromJson(importText);
      setCustoms(loadCustomThemes());
      choosePref(`custom:${t.id}`);
      setImportOpen(false);
      setImportText("");
      setImportErr(null);
      notify(`Imported theme "${t.name}".`);
    } catch (e) {
      setImportErr(e instanceof Error ? e.message : String(e));
    }
  }

  function deleteCustom(id: string) {
    removeCustomTheme(id);
    setCustoms(loadCustomThemes());
    if (pref === `custom:${id}`) choosePref("midnight");
  }

  useEffect(() => {
    api
      .getAutoSync()
      .then((s) => {
        setEnabled(s.enabled);
        setIntervalMin(s.interval_min);
        setBackupOnExit(s.backup_on_exit);
        setLoaded(true);
      })
      .catch((e) => notify(String(e), "err"));
    api.lanHostStatus().then(setLan).catch(() => {});
    api.knownGameCount().then(setGameCount).catch(() => {});
    loadStorage();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function toggleHost() {
    setLanBusy(true);
    try {
      setLan(lan?.hosting ? await api.stopLanHost() : await api.startLanHost());
    } catch (e) {
      notify(String(e), "err");
    } finally {
      setLanBusy(false);
    }
  }

  async function save() {
    setSaving(true);
    try {
      await api.setAutoSync({
        enabled,
        interval_min: Math.max(1, Math.floor(interval)),
        backup_on_exit: backupOnExit,
      });
      notify("Settings saved.");
      onClose();
    } catch (e) {
      notify(String(e), "err");
      setSaving(false);
    }
  }

  async function runVerify() {
    setVerifying(true);
    setVerifyResult(null);
    try {
      setVerifyResult(await api.verify());
    } catch (e) {
      notify(String(e), "err");
    } finally {
      setVerifying(false);
    }
  }

  return (
    <Modal title="Settings" onClose={onClose}>
      <h3 className="settings-h">Appearance</h3>
      <div className="theme-grid">
        {BUILTIN.map((t) => (
          <button
            key={t.id}
            className={`theme-btn ${pref === t.id ? "active" : ""}`}
            onClick={() => choosePref(t.id)}
          >
            <span
              className="theme-swatch"
              style={{ background: t.bg, ["--swatch-accent" as string]: t.accent } as CSSProperties}
            />
            {t.name}
          </button>
        ))}
      </div>

      <div className="more-themes">
        <label className="muted small" htmlFor="more-theme-select">
          More
        </label>
        <select
          id="more-theme-select"
          value={pref === "auto" || pref.startsWith("custom:") ? pref : ""}
          onChange={(e) => e.target.value && choosePref(e.target.value)}
        >
          <option value="">More themes…</option>
          <option value="auto">Auto (system light/dark)</option>
          {customs.length > 0 && (
            <optgroup label="Custom">
              {customs.map((t) => (
                <option key={t.id} value={`custom:${t.id}`}>
                  {t.name}
                </option>
              ))}
            </optgroup>
          )}
        </select>
        <button
          className="secondary"
          onClick={() => {
            setImportOpen(true);
            setImportText(themeTemplate());
            setImportErr(null);
          }}
        >
          Import…
        </button>
        {pref.startsWith("custom:") && (
          <button className="secondary" onClick={() => deleteCustom(pref.slice("custom:".length))}>
            Remove
          </button>
        )}
      </div>

      {importOpen && (
        <div className="import-theme">
          <p className="muted small">
            Paste a theme as JSON — a <code>name</code> and a <code>colors</code>{" "}
            object (keys: bg, panel, panel-2, border, text, muted, accent,
            accent-hover, ok, err, warn). The template below is pre-filled.
          </p>
          <textarea
            rows={10}
            value={importText}
            onChange={(e) => setImportText(e.target.value)}
            spellCheck={false}
          />
          {importErr && <div className="error">{importErr}</div>}
          <div className="modal-foot">
            <button className="secondary" onClick={() => setImportOpen(false)}>
              Cancel
            </button>
            <button onClick={doImport}>Import</button>
          </div>
        </div>
      )}

      <h3 className="settings-h">Game detection</h3>
      <p className="muted small">
        GameSync auto-detects{" "}
        <strong>{gameCount === null ? "…" : gameCount.toLocaleString()}</strong> games.
        Update to pull the latest community list (thousands of games, from
        PCGamingWiki via Ludusavi).
      </p>
      <button className="secondary" onClick={updateList} disabled={updatingList}>
        {updatingList ? "Updating…" : "Update game list"}
      </button>

      <h3 className="settings-h">Automatic sync</h3>
      <label className="toggle-row">
        <input
          type="checkbox"
          checked={enabled}
          disabled={!loaded}
          onChange={(e) => setEnabled(e.target.checked)}
        />
        <span>Automatically back up &amp; sync enabled games in the background</span>
      </label>
      <label className="field">
        <span>Sync every (minutes)</span>
        <input
          type="number"
          min={1}
          value={interval}
          disabled={!enabled}
          onChange={(e) => setIntervalMin(Number(e.target.value))}
        />
      </label>
      <p className="muted small">
        Runs while GameSync is open (including minimized to the tray). Games that
        look like they&apos;re running are skipped, and conflicts are never
        auto-resolved.
      </p>
      <label className="toggle-row">
        <input
          type="checkbox"
          checked={backupOnExit}
          disabled={!loaded}
          onChange={(e) => setBackupOnExit(e.target.checked)}
        />
        <span>
          Back up automatically when a game closes
          <span className="muted">
            {" "}
            — works for detected games (e.g. Steam); manual/emulator games use the
            timer or manual backup.
          </span>
        </span>
      </label>
      <div className="modal-foot">
        <button className="secondary" onClick={onClose}>
          Cancel
        </button>
        <button onClick={save} disabled={!loaded || saving}>
          {saving ? "Saving…" : "Save"}
        </button>
      </div>

      <h3 className="settings-h">Integrity</h3>
      <p className="muted small">
        Re-hash every stored object (decrypting first when encrypted) to confirm
        nothing is corrupt.
      </p>
      <button className="secondary" onClick={runVerify} disabled={verifying}>
        {verifying ? "Checking…" : "Verify all data"}
      </button>
      {verifyResult && (
        <div className={`verify-result ${verifyResult.ok ? "ok" : "bad"}`}>
          {verifyResult.ok
            ? `OK — ${verifyResult.objects_checked} object(s) across ${verifyResult.versions_checked} version(s) verified.`
            : `${verifyResult.problems.length} problem(s) found:`}
          {!verifyResult.ok &&
            verifyResult.problems.map((p, i) => (
              <div key={i} className="small">
                {p}
              </div>
            ))}
        </div>
      )}

      <h3 className="settings-h">Storage</h3>
      {storage ? (
        <>
          <p className="muted small">
            Backups use <strong>{humanSize(storage.total_bytes)}</strong> across{" "}
            {storage.total_objects} object(s) (deduplicated).
          </p>
          {storage.games.length > 0 && (
            <div className="versions">
              {storage.games.map((g) => (
                <div className="version-row" key={g.game_id}>
                  <div className="version-info">
                    <span>{g.name}</span>
                    <span className="muted small">{g.versions} versions</span>
                  </div>
                  <span className="muted small">{humanSize(g.bytes)}</span>
                </div>
              ))}
            </div>
          )}
        </>
      ) : (
        <p className="muted small">Calculating…</p>
      )}

      <label className="toggle-row">
        <input
          type="checkbox"
          checked={storage?.compressed ?? false}
          disabled={!storage || storageBusy || storage.total_objects > 0}
          onChange={(e) => toggleCompression(e.target.checked)}
        />
        <span>
          Compress backups (LZMA2 / 7-Zip codec)
          <span className="muted">
            {" "}
            — shrinks stored backups and what's uploaded to your synced folder.
            {storage && storage.total_objects > 0
              ? " Can only be changed before any backups exist."
              : ""}
          </span>
        </span>
      </label>

      <div className="retention-row">
        <span className="muted small">Keep newest</span>
        <input
          type="number"
          min={1}
          value={keepLast}
          onChange={(e) => setKeepLast(Math.max(1, Number(e.target.value)))}
        />
        <span className="muted small">versions</span>
      </div>
      <label className="toggle-row">
        <input type="checkbox" checked={keepDaysOn} onChange={(e) => setKeepDaysOn(e.target.checked)} />
        <span>
          Also keep anything from the last{" "}
          <input
            type="number"
            min={1}
            value={keepDays}
            disabled={!keepDaysOn}
            onChange={(e) => setKeepDays(Math.max(1, Number(e.target.value)))}
            style={{ width: 64 }}
          />{" "}
          days
        </span>
      </label>
      <div className="modal-foot">
        <button className="secondary" onClick={reclaim} disabled={storageBusy}>
          Reclaim unused space
        </button>
        <button onClick={applyRetention} disabled={storageBusy}>
          Apply &amp; clean up
        </button>
      </div>

      <h3 className="settings-h">LAN sync (this network)</h3>
      <p className="muted small">
        Make this device the hub for peer-to-peer sync over your local network —
        no cloud or shared folder. Hosting sets this device&apos;s remote to a
        local shared folder.
      </p>
      <button className="secondary" onClick={toggleHost} disabled={lanBusy}>
        {lan?.hosting ? "Stop hosting" : "Host on this network"}
      </button>
      {lan?.hosting && lan.spec && (
        <div className="verify-result ok">
          On your other device, set the remote to:
          <pre className="recovery-key">{lan.spec}</pre>
          <span className="small muted">
            Keep GameSync running here while syncing. The address changes each
            time you start hosting.
          </span>
        </div>
      )}
    </Modal>
  );
}
