import { useState, type CSSProperties } from "react";
import { Modal } from "./Modal";
import {
  BUILTIN,
  importThemeFromJson,
  loadCustomThemes,
  removeCustomTheme,
  themeTemplate,
  type CustomTheme,
} from "../theme";

interface Props {
  /** Current preference: "auto", a built-in id, or "custom:<id>". */
  pref: string;
  customs: CustomTheme[];
  /** Apply + persist a preference (live-previews behind the modal). */
  onChoose: (pref: string) => void;
  /** Notify the parent that the custom-theme list changed. */
  onCustomsChanged: (list: CustomTheme[]) => void;
  notify: (msg: string, kind?: "ok" | "err") => void;
  onClose: () => void;
}

function swatchStyle(bg: string, accent: string): CSSProperties {
  return { background: bg, ["--swatch-accent" as string]: accent } as CSSProperties;
}

/** A gallery of every theme — built-ins, Auto, and custom imports — each shown
 *  with the same micro-preview swatch used in Settings. */
export function ThemeModal({ pref, customs, onChoose, onCustomsChanged, notify, onClose }: Props) {
  const [importOpen, setImportOpen] = useState(false);
  const [importText, setImportText] = useState("");
  const [importErr, setImportErr] = useState<string | null>(null);

  function doImport() {
    try {
      const t = importThemeFromJson(importText);
      onCustomsChanged(loadCustomThemes());
      onChoose(`custom:${t.id}`);
      setImportOpen(false);
      setImportText("");
      setImportErr(null);
      notify(`Imported theme "${t.name}".`);
    } catch (e) {
      setImportErr(e instanceof Error ? e.message : String(e));
    }
  }

  function remove(id: string) {
    removeCustomTheme(id);
    onCustomsChanged(loadCustomThemes());
    if (pref === `custom:${id}`) onChoose("auto");
  }

  return (
    <Modal title="Themes" onClose={onClose}>
      <div className="theme-grid theme-grid-modal">
        <button
          className={`theme-btn ${pref === "auto" ? "active" : ""}`}
          onClick={() => onChoose("auto")}
          title="Follow your system's light/dark setting"
        >
          <span className="theme-swatch theme-swatch-auto" />
          Auto
        </button>

        {BUILTIN.map((t) => (
          <button
            key={t.id}
            className={`theme-btn ${pref === t.id ? "active" : ""}`}
            onClick={() => onChoose(t.id)}
          >
            <span className="theme-swatch" style={swatchStyle(t.bg, t.accent)} />
            {t.name}
          </button>
        ))}

        {customs.map((t) => (
          <button
            key={t.id}
            className={`theme-btn ${pref === `custom:${t.id}` ? "active" : ""}`}
            onClick={() => onChoose(`custom:${t.id}`)}
          >
            <span
              className="theme-del"
              role="button"
              aria-label={`Remove ${t.name}`}
              onClick={(e) => {
                e.stopPropagation();
                remove(t.id);
              }}
            >
              ×
            </span>
            {t.swatch ? (
              <span className="theme-swatch theme-swatch-custom" style={{ background: t.swatch }} />
            ) : (
              <span className="theme-swatch" style={swatchStyle(t.colors.bg, t.colors.accent)} />
            )}
            {t.name}
          </button>
        ))}
      </div>

      {!importOpen ? (
        <button
          className="secondary"
          onClick={() => {
            setImportOpen(true);
            setImportText(themeTemplate());
            setImportErr(null);
          }}
        >
          Import a theme…
        </button>
      ) : (
        <div className="import-theme">
          <p className="muted small">
            Paste a theme as JSON — a <code>name</code> and a <code>colors</code> object (keys:
            bg, panel, panel-2, border, text, muted, accent, accent-hover, ok, err, warn).
            Optionally add a <code>swatch</code> (any CSS background — a gradient or a{" "}
            <code>data:</code> image URL) to give your theme its own preview tile. The template
            below is pre-filled.
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
    </Modal>
  );
}
