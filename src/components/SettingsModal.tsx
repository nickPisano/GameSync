import { useEffect, useState, type CSSProperties } from "react";
import { Modal } from "./Modal";
import { api } from "../api";
import type { LanHostInfo, StorageReport, UpdateInfo, VerifyResult } from "../types";
import { humanSize } from "../format";
import { applyTheme, getPreference, BUILTIN, loadCustomThemes, type CustomTheme } from "../theme";
import { ThemeModal } from "./ThemeModal";

interface Props {
  onClose: () => void;
  notify: (msg: string, kind?: "ok" | "err") => void;
  encrypted: boolean;
  onEnableEncryption: () => void;
}

export function SettingsModal({ onClose, notify, encrypted, onEnableEncryption }: Props) {
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
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);

  async function checkForUpdate() {
    setCheckingUpdate(true);
    setUpdate(null);
    setUpdateError(null);
    try {
      setUpdate(await api.checkForUpdate());
    } catch (e) {
      setUpdateError(String(e));
    } finally {
      setCheckingUpdate(false);
    }
  }

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
  const [themeOpen, setThemeOpen] = useState(false);

  function choosePref(p: string) {
    applyTheme(p);
    setPref(p);
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
        <button className="secondary" onClick={() => setThemeOpen(true)}>
          More themes…
        </button>
        <span className="muted small">Auto (follow system), custom imports &amp; the full gallery</span>
      </div>

      {themeOpen && (
        <ThemeModal
          pref={pref}
          customs={customs}
          onChoose={choosePref}
          onCustomsChanged={setCustoms}
          notify={notify}
          onClose={() => setThemeOpen(false)}
        />
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

      <h3 className="settings-h">Encryption</h3>
      {encrypted ? (
        <p className="muted small">
          🔒 Encryption is on — all saved data is encrypted at rest with your
          passphrase.
        </p>
      ) : (
        <>
          <p className="muted small">
            Encrypt all saved data at rest with a passphrase (zero-knowledge).
            You'll get a one-time recovery key when you turn it on.
          </p>
          <button className="secondary" onClick={onEnableEncryption}>
            Enable encryption
          </button>
        </>
      )}

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

      <h3 className="settings-h">Updates</h3>
      <p className="muted small">
        GameSync doesn&apos;t update itself — check here and download the latest
        build from the releases page.
      </p>
      <button className="secondary" onClick={checkForUpdate} disabled={checkingUpdate}>
        {checkingUpdate ? "Checking…" : "Check for updates"}
      </button>
      {update && update.update_available && (
        <div className="verify-result ok">
          Update available: <strong>v{update.latest}</strong> (you have v
          {update.current}).
          <div style={{ marginTop: 8 }}>
            <button className="secondary" onClick={() => api.openUrl(update.url)}>
              Open releases page
            </button>
          </div>
        </div>
      )}
      {update && !update.update_available && (
        <p className="muted small">You&apos;re on the latest version (v{update.current}).</p>
      )}
      {updateError && (
        <div className="verify-result bad">
          {updateError}
          <div style={{ marginTop: 8 }}>
            <button
              className="secondary"
              onClick={() =>
                api.openUrl("https://github.com/nickPisano/GameSync/releases")
              }
            >
              Open releases page
            </button>
          </div>
        </div>
      )}
    </Modal>
  );
}
