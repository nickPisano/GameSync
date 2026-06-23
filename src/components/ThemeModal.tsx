import { useState, type CSSProperties } from "react";
import { Modal } from "./Modal";
import {
  BUILTIN,
  importThemeFromJson,
  loadCustomThemes,
  removeCustomTheme,
  themeTemplate,
  type CustomTheme,
  type Effects,
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

function fxTitle(fx: Effects): string {
  const bits = [fx.style ?? "effects", fx.bubbles ? "bubbles" : ""].filter(Boolean);
  return `Effects: ${bits.join(" · ")}`;
}

/** A custom theme's preview swatch. Themes with `effects` get a glow (the
 *  "has effects" cue); themes with `bubbles` also get tiny rising bubbles. */
function CustomSwatch({ t }: { t: CustomTheme }) {
  const fx = t.effects;
  const style: Record<string, string> = t.swatch
    ? { background: t.swatch }
    : { background: t.colors.bg, "--swatch-accent": t.colors.accent };
  if (fx) style["--swatch-glow"] = fx.glow ?? fx.highlight ?? t.colors.accent;
  if (fx?.bubbles) style["--swatch-bubble"] = fx.bubbleColor ?? fx.highlight ?? t.colors.accent;
  return (
    <span
      className={`theme-swatch ${t.swatch ? "theme-swatch-custom" : ""} ${fx ? "fx-glow" : ""}`}
      style={style as CSSProperties}
      title={fx ? fxTitle(fx) : undefined}
    >
      {fx?.bubbles && (
        <span className="swatch-bubbles" aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
      )}
    </span>
  );
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
            <CustomSwatch t={t} />
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
