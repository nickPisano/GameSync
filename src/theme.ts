// Theme manager. A *preference* is one of: a built-in id ("midnight" | "light"
// | "forest" | "grape"), "auto" (follow the OS), or "custom:<id>". Built-in
// palettes live in styles.css under [data-theme]; custom palettes are applied as
// inline CSS variables on :root. Everything persists in localStorage.

export const COLOR_KEYS = [
  "bg",
  "panel",
  "panel-2",
  "border",
  "text",
  "muted",
  "accent",
  "accent-hover",
  "ok",
  "err",
  "warn",
] as const;
export type ColorKey = (typeof COLOR_KEYS)[number];
export type Palette = Record<ColorKey, string>;

export const MIDNIGHT: Palette = {
  bg: "#0f1216",
  panel: "#171b21",
  "panel-2": "#1e242c",
  border: "#2a313b",
  text: "#e6e9ee",
  muted: "#8b95a3",
  accent: "#4f8cff",
  "accent-hover": "#3d7bf0",
  ok: "#3fb950",
  err: "#f85149",
  warn: "#d29922",
};

export const BUILTIN = [
  { id: "midnight", name: "Midnight", accent: "#4f8cff", bg: "#0f1216" },
  { id: "light", name: "Light", accent: "#2f6ae0", bg: "#f6f7f9" },
  { id: "forest", name: "Forest", accent: "#3fb37f", bg: "#0e1512" },
  { id: "grape", name: "Grape", accent: "#b57bff", bg: "#15121d" },
] as const;

export interface CustomTheme {
  id: string;
  name: string;
  colors: Palette;
  /** Optional self-supplied preview: any CSS background value (gradient, solid
   *  color, or a `data:` image URL). Falls back to a generated bg+accent swatch. */
  swatch?: string;
}

const PREF_KEY = "gamesync-theme";
const CUSTOM_KEY = "gamesync-custom-themes";

export function getPreference(): string {
  // Default to following the OS light/dark setting until the user picks a theme.
  return localStorage.getItem(PREF_KEY) ?? "auto";
}

export function loadCustomThemes(): CustomTheme[] {
  try {
    const v = JSON.parse(localStorage.getItem(CUSTOM_KEY) ?? "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

function saveCustomThemes(list: CustomTheme[]) {
  localStorage.setItem(CUSTOM_KEY, JSON.stringify(list));
}

function prefersDark(): boolean {
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? true;
}

function applyBuiltin(id: string) {
  COLOR_KEYS.forEach((k) => document.documentElement.style.removeProperty(`--${k}`));
  document.documentElement.setAttribute("data-theme", id);
}

function applyCustom(colors: Palette) {
  document.documentElement.removeAttribute("data-theme"); // base = :root (Midnight)
  COLOR_KEYS.forEach((k) => document.documentElement.style.setProperty(`--${k}`, colors[k]));
}

/** Apply a preference and persist it. */
export function applyTheme(pref: string) {
  localStorage.setItem(PREF_KEY, pref);
  if (pref === "auto") {
    applyBuiltin(prefersDark() ? "midnight" : "light");
  } else if (pref.startsWith("custom:")) {
    const id = pref.slice("custom:".length);
    const found = loadCustomThemes().find((t) => t.id === id);
    if (found) applyCustom(found.colors);
    else applyBuiltin("midnight");
  } else {
    applyBuiltin(pref);
  }
}

/** Apply the saved preference and keep "auto" in sync with the OS. */
export function initTheme() {
  applyTheme(getPreference());
  window
    .matchMedia?.("(prefers-color-scheme: dark)")
    .addEventListener?.("change", () => {
      if (getPreference() === "auto") applyTheme("auto");
    });
}

function slug(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
}

const REQUIRED: ColorKey[] = ["bg", "panel", "border", "text", "muted", "accent"];

const MAX_SWATCH_LEN = 64 * 1024;

/** A theme may ship its own preview as a CSS background (gradient, solid color,
 *  or a `data:` image URL). Reject anything that would load an external
 *  resource — only `url(data:…)` is allowed, never `url(http…)` / `url(file…)`. */
function isSafeSwatch(s: string): boolean {
  if (s.length > MAX_SWATCH_LEN) return false;
  const urls = s.match(/url\([^)]*\)/gi) ?? [];
  return urls.every((u) => /^url\(\s*['"]?data:/i.test(u));
}

/** Parse + validate an imported theme JSON, store it, and return it. Throws an
 *  Error with a friendly message on bad input. */
export function importThemeFromJson(text: string): CustomTheme {
  let obj: Record<string, unknown>;
  try {
    obj = JSON.parse(text);
  } catch {
    throw new Error("That isn't valid JSON.");
  }
  const name = String((obj.name as string) || "Custom theme").slice(0, 40);
  const src = (obj.colors && typeof obj.colors === "object" ? obj.colors : obj) as Record<
    string,
    unknown
  >;

  const missing = REQUIRED.filter((k) => typeof src[k] !== "string" || !(src[k] as string).trim());
  if (missing.length) {
    throw new Error(`Missing colors: ${missing.join(", ")}`);
  }

  const colors = { ...MIDNIGHT };
  for (const k of COLOR_KEYS) {
    if (typeof src[k] === "string" && (src[k] as string).trim()) {
      colors[k] = (src[k] as string).trim();
    }
  }
  // Sensible fallbacks for optional keys not provided.
  if (typeof src["panel-2"] !== "string") colors["panel-2"] = colors.panel;
  if (typeof src["accent-hover"] !== "string") colors["accent-hover"] = colors.accent;

  const customs = loadCustomThemes();
  const base = slug(name) || "theme";
  let id = base;
  let n = 1;
  while (customs.some((t) => t.id === id)) id = `${base}-${++n}`;

  const theme: CustomTheme = { id, name, colors };
  const swatch = obj.swatch;
  if (typeof swatch === "string" && swatch.trim() && isSafeSwatch(swatch.trim())) {
    theme.swatch = swatch.trim();
  }
  customs.push(theme);
  saveCustomThemes(customs);
  return theme;
}

export function removeCustomTheme(id: string) {
  saveCustomThemes(loadCustomThemes().filter((t) => t.id !== id));
}

/** A ready-to-edit template string for the import box. */
export function themeTemplate(): string {
  return JSON.stringify(
    {
      name: "My theme",
      colors: MIDNIGHT,
      swatch: "linear-gradient(135deg, #4f8cff 0 50%, #0f1216 50% 100%)",
    },
    null,
    2,
  );
}
