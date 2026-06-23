import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, pickFolder } from "./api";
import type {
  AppStatus,
  AutoSyncReport,
  DiscoveredHost,
  Game,
  GameView,
  SyncOutcome,
} from "./types";
import { GameCard } from "./components/GameCard";
import { SyncSpinner } from "./components/SyncSpinner";
import { Lock } from "./components/Lock";
import { AddGameModal } from "./components/AddGameModal";
import { VersionsModal } from "./components/VersionsModal";
import { FilesModal } from "./components/FilesModal";
import { RenameModal } from "./components/RenameModal";
import { EncryptModal } from "./components/EncryptModal";
import { SettingsModal } from "./components/SettingsModal";
import { PluginsModal } from "./components/PluginsModal";
import { ConflictDiffModal } from "./components/ConflictDiffModal";
import { Wizard } from "./components/Wizard";

interface Toast {
  msg: string;
  kind: "ok" | "err";
}

interface Conflict {
  gameId: string;
  gameName: string;
  local: string;
  remote: string;
}

export function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [games, setGames] = useState<GameView[]>([]);
  const [remote, setRemote] = useState<string | null>(null);
  const [remoteInput, setRemoteInput] = useState("");
  const [toast, setToast] = useState<Toast | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  // Number of in-flight sync operations (drives the global spinner); and which
  // single game (if any) is currently syncing (drives its card spinner).
  const [syncCount, setSyncCount] = useState(0);
  const [syncingId, setSyncingId] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [encryptOpen, setEncryptOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [pluginsOpen, setPluginsOpen] = useState(false);
  const [historyGame, setHistoryGame] = useState<Game | null>(null);
  const [filesGame, setFilesGame] = useState<Game | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const [conflict, setConflict] = useState<Conflict | null>(null);
  const [previewOpen, setPreviewOpen] = useState(false);
  // LAN host auto-discovery (peer side): found hosts + in-flight flag.
  const [lanHosts, setLanHosts] = useState<DiscoveredHost[] | null>(null);
  const [discovering, setDiscovering] = useState(false);
  // Library search + sort (revealed by the Filter button).
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [sortKey, setSortKey] = useState<"name" | "recent" | "versions" | "platform">("name");

  // Toggle the filter bar; clearing the search when hiding it avoids a hidden
  // filter silently trimming the list.
  function toggleFilters() {
    setFiltersOpen((open) => {
      if (open) setQuery("");
      return !open;
    });
  }

  const visibleGames = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = q
      ? games.filter(
          (v) =>
            v.game.name.toLowerCase().includes(q) ||
            v.game.platform.toLowerCase().includes(q) ||
            v.game.save_root.toLowerCase().includes(q),
        )
      : [...games];
    list.sort((a, b) => {
      switch (sortKey) {
        case "recent":
          return (b.last_backup_ms ?? -1) - (a.last_backup_ms ?? -1);
        case "versions":
          return b.version_count - a.version_count;
        case "platform":
          return (
            a.game.platform.localeCompare(b.game.platform) ||
            a.game.name.localeCompare(b.game.name)
          );
        default:
          return a.game.name.localeCompare(b.game.name);
      }
    });
    return list;
  }, [games, query, sortKey]);

  const notify = useCallback((msg: string, kind: "ok" | "err" = "ok") => {
    setToast({ msg, kind });
    window.setTimeout(() => setToast(null), 4000);
  }, []);

  // Suppress card hover while the library is scrolling: as cards slide under a
  // stationary cursor, their :hover lifts/shadow transitions would otherwise
  // churn the whole list and stutter. We add `is-scrolling` (which disables
  // pointer events on the cards) on scroll and clear it ~150ms after it stops.
  const gamesRef = useRef<HTMLElement | null>(null);
  const scrollIdle = useRef<number | undefined>(undefined);
  const onGamesScroll = useCallback(() => {
    const el = gamesRef.current;
    if (!el) return;
    el.classList.add("is-scrolling");
    window.clearTimeout(scrollIdle.current);
    scrollIdle.current = window.setTimeout(() => el.classList.remove("is-scrolling"), 150);
  }, []);

  const refresh = useCallback(async () => {
    setGames(await api.listGames());
    const r = await api.remoteStatus();
    setRemote(r);
    setRemoteInput(r ?? "");
  }, []);

  const loadStatus = useCallback(async () => {
    const s = await api.appStatus();
    setStatus(s);
    if (s.unlocked) await refresh();
  }, [refresh]);

  useEffect(() => {
    loadStatus().catch((e) => notify(String(e), "err"));
  }, [loadStatus, notify]);

  async function withBusy(id: string, fn: () => Promise<void>) {
    setBusyId(id);
    try {
      await fn();
    } catch (e) {
      notify(String(e), "err");
    } finally {
      setBusyId(null);
    }
  }

  async function onScan() {
    await withBusy("__scan", async () => {
      const found = await api.scan();
      notify(`Scan complete — ${found.length} detected game(s).`);
      await refresh();
    });
  }

  async function onToggleSync(id: string, enabled: boolean) {
    await withBusy(id, async () => {
      await api.setSyncEnabled(id, enabled);
      await refresh();
    });
  }

  async function onRemove(view: GameView) {
    if (
      !confirm(
        `Remove "${view.game.name}"? This deletes its backup history in GameSync. ` +
          "Your actual save files on disk are not touched."
      )
    )
      return;
    await withBusy(view.game.id, async () => {
      await api.removeGame(view.game.id);
      notify(`Removed ${view.game.name}.`);
      await refresh();
    });
  }

  async function onRedirect(view: GameView) {
    const base = await pickFolder(
      "Choose a synced folder (e.g. your OneDrive or Google Drive folder)"
    );
    if (!base) return;
    if (
      !confirm(
        `Move "${view.game.name}" saves into a subfolder of:\n${base}\n\n` +
          "GameSync backs them up first, moves the folder there, and leaves a symlink so the game still " +
          "finds them. Your original folder is kept (renamed), not deleted.\n\nContinue?"
      )
    )
      return;
    await withBusy(view.game.id, async () => {
      const r = await api.redirectSaveFolder(view.game.id, base);
      notify(`Saves moved to ${r.linked_target}. Original kept at ${r.original_backup}.`);
      await refresh();
    });
  }

  async function onSetExe(view: GameView) {
    const p = await pickFolder("Choose the game's folder or app (to detect when it closes)");
    if (!p) return;
    await withBusy(view.game.id, async () => {
      await api.setGameExe(view.game.id, p);
      notify(`Auto-backup on close enabled for ${view.game.name}.`);
      await refresh();
    });
  }

  async function onBackup(id: string) {
    await withBusy(id, async () => {
      const snap = await api.backup(id);
      notify(`Backed up ${snap.files.length} file(s).`);
      await refresh();
    });
  }

  function describeSync(out: SyncOutcome, name: string, gameId: string) {
    switch (out.status) {
      case "in_sync":
        notify(`${name}: already in sync.`);
        break;
      case "pushed":
        notify(`${name}: pushed to remote.`);
        break;
      case "pulled":
        notify(`${name}: pulled & restored from remote.`);
        break;
      case "conflict":
        setConflict({ gameId, gameName: name, local: out.local, remote: out.remote });
        break;
    }
  }

  async function onSync(view: GameView) {
    setSyncCount((c) => c + 1);
    setSyncingId(view.game.id);
    await withBusy(view.game.id, async () => {
      const out = await api.syncGame(view.game.id);
      describeSync(out, view.game.name, view.game.id);
      await refresh();
    });
    setSyncingId(null);
    setSyncCount((c) => Math.max(0, c - 1));
  }

  // Shared handler for both the "Sync all" button and background auto-sync.
  const handleReport = useCallback(
    (r: AutoSyncReport) => {
      if (r.conflicts.length > 0) {
        const c = r.conflicts[0];
        setConflict({
          gameId: c.game_id,
          gameName: c.game_name,
          local: c.local,
          remote: c.remote,
        });
      }
      const hadIssue = r.conflicts.length > 0 || r.errors.length > 0;
      if (r.games === 0) {
        notify("No games have sync enabled — toggle Sync on a game first.", "err");
      } else {
        notify(
          `Synced ${r.games}: ${r.pushed} pushed, ${r.pulled} pulled, ${r.in_sync} up to date` +
            (r.skipped_running ? `, ${r.skipped_running} skipped (running)` : "") +
            (r.conflicts.length ? `, ${r.conflicts.length} conflict(s)` : "") +
            (r.errors.length ? `, ${r.errors.length} error(s)` : ""),
          hadIssue ? "err" : "ok"
        );
      }
    },
    [notify]
  );

  // Listen for background auto-sync events emitted by the Rust loop / tray:
  // start spins the indicator; the completion/error event stops it.
  useEffect(() => {
    const unsub: Array<() => void> = [];
    listen("auto-sync-start", () => setSyncCount((c) => c + 1)).then((u) => unsub.push(u));
    listen<AutoSyncReport>("auto-sync", (e) => {
      setSyncCount((c) => Math.max(0, c - 1));
      refresh().catch(() => {});
      handleReport(e.payload);
    }).then((u) => unsub.push(u));
    listen<string>("auto-sync-error", (e) => {
      setSyncCount((c) => Math.max(0, c - 1));
      notify(`Auto-sync error: ${e.payload}`, "err");
    }).then((u) => unsub.push(u));
    listen<string>("game-exit-backup", (e) => {
      refresh().catch(() => {});
      notify(`Backed up ${e.payload} after you closed it.`);
    }).then((u) => unsub.push(u));
    return () => unsub.forEach((u) => u());
  }, [refresh, handleReport, notify]);

  async function onSyncAll() {
    setSyncCount((c) => c + 1);
    await withBusy("__syncall", async () => {
      const report = await api.syncAll();
      await refresh();
      handleReport(report);
    });
    setSyncCount((c) => Math.max(0, c - 1));
  }

  async function onResolve(keep: "local" | "remote") {
    if (!conflict) return;
    const c = conflict;
    setConflict(null);
    setPreviewOpen(false);
    await withBusy(c.gameId, async () => {
      await api.resolveConflict(c.gameId, keep);
      notify(`${c.gameName}: conflict resolved (kept ${keep}).`);
      await refresh();
    });
  }

  // Keep both: live save stays, the remote branch is forked into a new game.
  async function onForkConflict() {
    if (!conflict) return;
    const c = conflict;
    setConflict(null);
    setPreviewOpen(false);
    await withBusy(c.gameId, async () => {
      const fork = await api.forkConflict(c.gameId);
      notify(`${c.gameName}: kept both — the other save is now "${fork.name}".`);
      await refresh();
    });
  }

  async function onSaveRemote() {
    await withBusy("__remote", async () => {
      await api.setRemote(remoteInput.trim());
      notify("Remote saved.");
      await refresh();
    });
  }

  // Listen on the LAN for hosts advertising themselves (a couple of seconds).
  async function onDiscoverLan() {
    setDiscovering(true);
    setLanHosts(null);
    try {
      const hosts = await api.discoverLanHosts();
      setLanHosts(hosts);
      if (hosts.length === 0) {
        notify("No LAN hosts found. Start hosting on another device first.", "err");
      }
    } catch (e) {
      notify(String(e), "err");
    } finally {
      setDiscovering(false);
    }
  }

  // Discovery gives the address; the token (shown on the host) is still needed,
  // so we prefill the spec with an empty token for the user to fill in.
  function pickLanHost(h: DiscoveredHost) {
    setRemoteInput(`lan:@${h.addr}:${h.port}`);
    setLanHosts(null);
    notify(`Selected ${h.name} — add the host's token between "lan:" and "@", then Save.`);
  }

  // ---- gating screens --------------------------------------------------

  if (!status) {
    return <div className="loading">Loading…</div>;
  }

  if (status.encrypted && !status.unlocked) {
    return (
      <Lock
        onUnlock={async (p) => {
          await api.unlock(p);
          await loadStatus();
        }}
      />
    );
  }

  if (!status.setup_complete) {
    return <Wizard onDone={() => loadStatus()} notify={notify} />;
  }

  // ---- main app --------------------------------------------------------

  return (
    <div className="app">
      {/* Decorative bubble layer; hidden unless a theme sets data-fx-bubbles. */}
      <div className="fx-bubbles" aria-hidden="true">
        {Array.from({ length: 16 }, (_, i) => {
          const size = 14 + ((i * 37) % 38); // 14–51px
          const left = (i * 61) % 100; // spread across the width
          const dur = 15 + ((i * 7) % 16); // 15–30s rise
          const delay = -((i * 13) % 22); // negative → already mid-flight at load
          return (
            <span
              key={i}
              className="fx-bubble"
              style={{
                left: `${left}%`,
                width: `${size}px`,
                height: `${size}px`,
                animationDuration: `${dur}s`,
                animationDelay: `${delay}s`,
              }}
            />
          );
        })}
      </div>
      <header className="topbar">
        <div className="brand">
          <svg className="brand-logo" viewBox="0 0 32 32" width="28" height="28" aria-hidden="true">
            <rect width="32" height="32" rx="9" fill="var(--accent)" />
            <path
              transform="translate(4 4)"
              fill="#fff"
              d="M12 4V1L8 5l4 4V6c3.31 0 6 2.69 6 6 0 1.01-.25 1.97-.7 2.8l1.46 1.46C19.54 15.03 20 13.57 20 12c0-4.42-3.58-8-8-8zm0 14c-3.31 0-6-2.69-6-6 0-1.01.25-1.97.7-2.8L5.24 7.74C4.46 8.97 4 10.43 4 12c0 4.42 3.58 8 8 8v3l4-4-4-4v3z"
            />
          </svg>
          <span className="brand-name">GameSync</span>
          <span className="tagline">safe save backup &amp; sync</span>
        </div>
        <div className="topbar-actions">
          {syncCount > 0 && (
            <span className="syncing-indicator">
              <SyncSpinner /> Syncing…
            </span>
          )}
          <button
            onClick={onSyncAll}
            disabled={!remote || busyId === "__syncall"}
            title={remote ? "Sync every enabled game" : "Configure a remote first"}
          >
            {busyId === "__syncall" ? (
              <>
                <SyncSpinner /> Syncing…
              </>
            ) : (
              "Sync all"
            )}
          </button>
          <button className="secondary" onClick={onScan} disabled={busyId === "__scan"}>
            {busyId === "__scan" ? "Scanning…" : "Scan"}
          </button>
          <button className="secondary" onClick={() => setAddOpen(true)}>
            Add game
          </button>
          <button
            className={`secondary ${filtersOpen ? "active" : ""}`}
            onClick={toggleFilters}
            disabled={games.length === 0}
            title="Search & sort your games"
            aria-pressed={filtersOpen}
          >
            Filter
          </button>
          <button className="secondary" onClick={() => setPluginsOpen(true)}>
            Plugins
          </button>
          <button className="secondary" onClick={() => setSettingsOpen(true)}>
            Settings
          </button>
          {status.encrypted && (
            <span className="enc-pill" title="Encryption enabled">🔒 Encrypted</span>
          )}
        </div>
      </header>

      <div className="remote-bar">
        <span className="remote-label">Remote</span>
        <input
          placeholder="Path to a shared/cloud folder (e.g. ~/Dropbox/GameSync)"
          value={remoteInput}
          onChange={(e) => setRemoteInput(e.target.value)}
        />
        <button
          className="secondary"
          onClick={async () => {
            const p = await pickFolder("Choose a shared/cloud folder for sync");
            if (p) setRemoteInput(p);
          }}
        >
          Browse…
        </button>
        <button
          className="secondary"
          onClick={onDiscoverLan}
          disabled={discovering}
          title="Find GameSync hosts on your local network"
        >
          {discovering ? "Searching…" : "Find hosts"}
        </button>
        <button onClick={onSaveRemote} disabled={busyId === "__remote"}>
          Save
        </button>
        <span className={`remote-state ${remote ? "ok" : "off"}`}>
          {remote ? "configured" : "not set"}
        </span>
      </div>

      {lanHosts && lanHosts.length > 0 && (
        <div className="lan-hosts">
          <span className="lan-hosts-label">LAN hosts found — pick one, then add its token:</span>
          {lanHosts.map((h) => (
            <button
              key={`${h.addr}:${h.port}`}
              className="lan-host"
              onClick={() => pickLanHost(h)}
              title={`Use ${h.addr}:${h.port}`}
            >
              <strong>{h.name}</strong>
              <span className="muted">
                {h.addr}:{h.port}
              </span>
            </button>
          ))}
        </div>
      )}

      {conflict && (
        <div className="conflict-banner">
          <span>
            <strong>{conflict.gameName}:</strong> your save and the remote save
            diverged. Your live save was not changed.
          </span>
          <div>
            {conflict.local && conflict.remote && (
              <button className="secondary" onClick={() => setPreviewOpen(true)}>
                Preview changes
              </button>
            )}
            <button onClick={() => onResolve("local")}>Keep mine</button>
            <button onClick={() => onResolve("remote")}>Take remote</button>
            <button
              onClick={onForkConflict}
              title="Keep your save and save the other device's as a separate game"
            >
              Keep both
            </button>
            <button className="secondary" onClick={() => setConflict(null)}>
              Later
            </button>
          </div>
        </div>
      )}

      {filtersOpen && games.length > 0 && (
        <div className="games-toolbar">
          <input
            className="games-search"
            type="text"
            autoFocus
            autoComplete="off"
            spellCheck={false}
            placeholder="Search games by name, platform, or path…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="Search games"
          />
          {query && (
            <button
              className="linklike games-clear"
              onClick={() => setQuery("")}
              title="Clear search"
            >
              Clear
            </button>
          )}
          <label className="games-sort">
            Sort
            <select
              value={sortKey}
              onChange={(e) => setSortKey(e.target.value as typeof sortKey)}
            >
              <option value="name">Name (A–Z)</option>
              <option value="recent">Recently backed up</option>
              <option value="versions">Most versions</option>
              <option value="platform">Platform</option>
            </select>
          </label>
          {query && (
            <span className="games-count muted">
              {visibleGames.length} of {games.length}
            </span>
          )}
        </div>
      )}

      <main className="games" ref={gamesRef} onScroll={onGamesScroll}>
        {games.length === 0 ? (
          <div className="empty">
            <p>No games tracked yet.</p>
            <p className="muted">
              Click <strong>Scan</strong> to detect installed games, or{" "}
              <strong>Add game</strong> to point at a save folder.
            </p>
          </div>
        ) : visibleGames.length === 0 ? (
          <div className="empty">
            <p>No games match “{query}”.</p>
            <p className="muted">Try a different search.</p>
          </div>
        ) : (
          visibleGames.map((view) => (
            <GameCard
              key={view.game.id}
              view={view}
              busy={busyId === view.game.id}
              syncing={syncingId === view.game.id}
              remoteConfigured={!!remote}
              onToggleSync={(enabled) => onToggleSync(view.game.id, enabled)}
              onBackup={() => onBackup(view.game.id)}
              onHistory={() => setHistoryGame(view.game)}
              onFiles={() => setFilesGame(view.game)}
              onSync={() => onSync(view)}
              onSetExe={() => onSetExe(view)}
              onRename={() => setRenaming({ id: view.game.id, name: view.game.name })}
              onRemove={() => onRemove(view)}
              onRedirect={() => onRedirect(view)}
            />
          ))
        )}
      </main>

      {addOpen && (
        <AddGameModal
          onClose={() => setAddOpen(false)}
          onAdd={async (name, path, exePath) => {
            const game = await api.addGame(name, path);
            if (exePath) await api.setGameExe(game.id, exePath);
            notify(`Added ${name}.`);
            await refresh();
          }}
        />
      )}

      {encryptOpen && (
        <EncryptModal
          onClose={() => {
            setEncryptOpen(false);
            loadStatus().catch((e) => notify(String(e), "err"));
          }}
          onInit={async (p) => {
            const key = await api.initEncryption(p);
            return key;
          }}
        />
      )}

      {settingsOpen && (
        <SettingsModal
          onClose={() => setSettingsOpen(false)}
          notify={notify}
          encrypted={status.encrypted}
          onEnableEncryption={() => {
            setSettingsOpen(false);
            setEncryptOpen(true);
          }}
        />
      )}

      {pluginsOpen && (
        <PluginsModal onClose={() => setPluginsOpen(false)} notify={notify} />
      )}

      {previewOpen && conflict && (
        <ConflictDiffModal
          gameId={conflict.gameId}
          gameName={conflict.gameName}
          local={conflict.local}
          remote={conflict.remote}
          onClose={() => setPreviewOpen(false)}
          onResolve={(keep) => onResolve(keep)}
          notify={notify}
        />
      )}

      {historyGame && (
        <VersionsModal
          game={historyGame}
          onClose={() => setHistoryGame(null)}
          onChanged={refresh}
          notify={notify}
        />
      )}

      {filesGame && (
        <FilesModal
          game={filesGame}
          onClose={() => setFilesGame(null)}
          notify={notify}
        />
      )}

      {renaming && (
        <RenameModal
          initial={renaming.name}
          onClose={() => setRenaming(null)}
          onSave={async (name) => {
            await api.renameGame(renaming.id, name);
            notify(`Renamed to ${name}.`);
            setRenaming(null);
            await refresh();
          }}
        />
      )}

      <footer className="statusbar">
        <span className={status.encrypted ? "ok" : "muted"}>
          {status.encrypted ? "🔒 Encrypted" : "Unencrypted"}
        </span>
        <span className="muted">·</span>
        <span className="muted">
          {games.length} game{games.length === 1 ? "" : "s"}
        </span>
        <span className="muted">·</span>
        <span className="muted">{remote ? `remote: ${remote}` : "no remote"}</span>
        <span className="statusbar-spacer" />
        <span className="muted statusbar-path" title={status.data_dir}>
          {status.data_dir}
        </span>
      </footer>

      {toast && (
        <div
          className={`toast toast-${toast.kind}`}
          onClick={() => setToast(null)}
          title="Dismiss"
        >
          {toast.msg}
        </div>
      )}
    </div>
  );
}
