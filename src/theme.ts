// Theme manager. A *preference* is one of: a built-in id ("midnight" | "light"
// | "forest" | "grape"), "auto" (follow the OS), or "custom:<id>". Built-in
// palettes live in styles.css under [data-theme]; custom palettes are applied as
// inline CSS variables on :root. Everything persists in localStorage.

import { invoke } from "@tauri-apps/api/core";

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

/** Optional surface treatment a theme can request. Gated entirely behind the
 *  `[data-style]` / `[data-fx-bubbles]` attributes in CSS, so a theme without
 *  effects looks unchanged. A block is valid if it has a known `style` and/or
 *  `bubbles` — so bubbles can ride on a surface style or stand alone. */
export interface Effects {
  /** Surface style. Optional so `bubbles` can be used without restyling panels. */
  style?: "glass" | "neo" | "skeuo";
  /** 2–4 `#rrggbb` stops, top→bottom (glass backdrop / skeuo surface). */
  gradient?: string[];
  /** px, 0–40 — glass frost radius / neo shadow softness. */
  blur?: number;
  /** 0–1 — glass panel translucency. */
  opacity?: number;
  /** Light edge: glass rim / neo top-left glow / skeuo gloss. */
  highlight?: string;
  /** Dark edge: drop shadow / neo bottom-right / skeuo base. */
  shadow?: string;
  /** Optional accent glow on the primary button. */
  glow?: string;
  /** Float soft bubbles up the background (composes with any style). */
  bubbles?: boolean;
  /** `#rrggbb` tint for the bubbles (defaults to highlight, then accent). */
  bubbleColor?: string;
}

export const FX_STYLES = ["glass", "neo", "skeuo"] as const;

export interface CustomTheme {
  id: string;
  name: string;
  colors: Palette;
  /** Optional self-supplied preview: any CSS background value (gradient, solid
   *  color, or a `data:` image URL). Falls back to a generated bg+accent swatch. */
  swatch?: string;
  /** Optional glass/neo/skeuo surface treatment. */
  effects?: Effects;
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

// CSS custom properties driven by a theme's `effects` block.
const FX_VARS = [
  "--fx-gradient",
  "--fx-blur",
  "--fx-opacity",
  "--fx-highlight",
  "--fx-shadow",
  "--fx-glow",
  "--fx-bubble",
] as const;

/** Remove every trace of a previous effect (the `data-style`/`data-fx-bubbles`
 *  attributes and all `--fx-*` vars), so switching themes fully clears it. */
function clearEffects() {
  document.documentElement.removeAttribute("data-style");
  document.documentElement.removeAttribute("data-fx-bubbles");
  FX_VARS.forEach((v) => document.documentElement.style.removeProperty(v));
}

/** Apply a theme's effects (or clear them when none). Only sets the `--fx-*`
 *  vars actually provided; CSS supplies fallbacks for the rest. */
function applyEffects(effects?: Effects) {
  clearEffects();
  if (!effects) return;
  const root = document.documentElement;
  if (effects.style) root.setAttribute("data-style", effects.style);
  if (effects.gradient && effects.gradient.length) {
    root.style.setProperty("--fx-gradient", `linear-gradient(180deg, ${effects.gradient.join(", ")})`);
  }
  if (typeof effects.blur === "number") root.style.setProperty("--fx-blur", `${effects.blur}px`);
  if (typeof effects.opacity === "number") root.style.setProperty("--fx-opacity", String(effects.opacity));
  if (effects.highlight) root.style.setProperty("--fx-highlight", effects.highlight);
  if (effects.shadow) root.style.setProperty("--fx-shadow", effects.shadow);
  if (effects.glow) root.style.setProperty("--fx-glow", effects.glow);
  if (effects.bubbleColor) root.style.setProperty("--fx-bubble", effects.bubbleColor);
  if (effects.bubbles) root.setAttribute("data-fx-bubbles", "");
}

function applyBuiltin(id: string) {
  COLOR_KEYS.forEach((k) => document.documentElement.style.removeProperty(`--${k}`));
  document.documentElement.setAttribute("data-theme", id);
  clearEffects(); // built-ins never carry effects
}

function applyCustom(colors: Palette, effects?: Effects) {
  document.documentElement.removeAttribute("data-theme"); // base = :root (Midnight)
  COLOR_KEYS.forEach((k) => document.documentElement.style.setProperty(`--${k}`, colors[k]));
  applyEffects(effects);
}

/** Rough perceived-luminance test on a `#rrggbb` color. */
function isDarkColor(hex: string): boolean {
  const h = hex.replace("#", "").trim();
  if (h.length < 6) return true;
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return 0.299 * r + 0.587 * g + 0.114 * b < 140;
}

/** Recolor the native window title bar to match the active theme (Tauri command;
 *  no-op outside the app). Reads the resolved CSS vars so it works for built-ins
 *  and custom themes alike. */
function syncTitlebar() {
  const cs = getComputedStyle(document.documentElement);
  const bg = cs.getPropertyValue("--panel").trim();
  const text = cs.getPropertyValue("--text").trim();
  const base = cs.getPropertyValue("--bg").trim() || bg;
  if (!bg) return;
  invoke("set_titlebar", { bg, text, dark: isDarkColor(base) }).catch(() => {});
}

/** Apply a preference and persist it. */
export function applyTheme(pref: string) {
  localStorage.setItem(PREF_KEY, pref);
  if (pref === "auto") {
    applyBuiltin(prefersDark() ? "midnight" : "light");
  } else if (pref.startsWith("custom:")) {
    const id = pref.slice("custom:".length);
    const found = loadCustomThemes().find((t) => t.id === id);
    if (found) applyCustom(found.colors, found.effects);
    else applyBuiltin("midnight");
  } else {
    applyBuiltin(pref);
  }
  syncTitlebar();
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

function isHex6(v: unknown): v is string {
  return typeof v === "string" && /^#[0-9a-fA-F]{6}$/.test(v.trim());
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n));
}

/** Defensively parse an `effects` block. Returns `undefined` (block ignored)
 *  unless it requests something real — a valid `style` and/or `bubbles`;
 *  individual malformed fields are dropped, not fatal. */
function parseEffects(raw: unknown): Effects | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const e = raw as Record<string, unknown>;
  const out: Effects = {};
  if (FX_STYLES.includes(e.style as (typeof FX_STYLES)[number])) {
    out.style = e.style as Effects["style"];
  }
  if (Array.isArray(e.gradient)) {
    const stops = e.gradient.filter(isHex6).map((s) => s.trim());
    if (stops.length >= 2 && stops.length <= 4) out.gradient = stops;
  }
  if (typeof e.blur === "number" && Number.isFinite(e.blur)) out.blur = clamp(e.blur, 0, 40);
  if (typeof e.opacity === "number" && Number.isFinite(e.opacity)) out.opacity = clamp(e.opacity, 0, 1);
  if (isHex6(e.highlight)) out.highlight = e.highlight.trim();
  if (isHex6(e.shadow)) out.shadow = e.shadow.trim();
  if (isHex6(e.glow)) out.glow = e.glow.trim();
  if (e.bubbles === true) out.bubbles = true;
  if (isHex6(e.bubbleColor)) out.bubbleColor = e.bubbleColor.trim();
  // Only keep the block if it actually does something.
  if (!out.style && !out.bubbles) return undefined;
  return out;
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
  const effects = parseEffects(obj.effects);
  if (effects) theme.effects = effects;
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
