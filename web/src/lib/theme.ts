/** Hell/dunkel-Umschaltung. Dunkel ist der Default; `system` folgt `prefers-color-scheme`. */
import { useSyncExternalStore } from "react";

export type ThemePreference = "dark" | "light" | "system";
export type ResolvedTheme = "dark" | "light";

export const THEME_STORAGE_KEY = "lineal-theme";
export const DEFAULT_THEME: ThemePreference = "dark";

export function isThemePreference(v: unknown): v is ThemePreference {
  return v === "dark" || v === "light" || v === "system";
}

export function getStoredTheme(storage: Pick<Storage, "getItem"> = localStorage): ThemePreference {
  try {
    const v = storage.getItem(THEME_STORAGE_KEY);
    return isThemePreference(v) ? v : DEFAULT_THEME;
  } catch {
    return DEFAULT_THEME;
  }
}

export function resolveTheme(pref: ThemePreference): ResolvedTheme {
  if (pref !== "system") return pref;
  const dark =
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches;
  return dark ? "dark" : "light";
}

/** Setzt `class="dark"` und `color-scheme` auf <html>. */
export function applyTheme(pref: ThemePreference, root: HTMLElement = document.documentElement) {
  const resolved = resolveTheme(pref);
  root.classList.toggle("dark", resolved === "dark");
  root.style.colorScheme = resolved;
  root.dataset.theme = resolved;
}

type Listener = () => void;
const listeners = new Set<Listener>();
let current: ThemePreference | null = null;

function read(): ThemePreference {
  current ??= getStoredTheme();
  return current;
}

export function getTheme(): ThemePreference {
  return read();
}

export function setTheme(pref: ThemePreference) {
  current = pref;
  try {
    localStorage.setItem(THEME_STORAGE_KEY, pref);
  } catch {
    /* Speicher nicht verfügbar: Einstellung gilt nur für diese Sitzung. */
  }
  applyTheme(pref);
  listeners.forEach((l) => l());
}

/** Test-Hilfe: interner Zustand neu aus dem Speicher lesen. */
export function resetThemeState() {
  current = null;
}

function subscribe(l: Listener) {
  listeners.add(l);
  return () => listeners.delete(l);
}

/** Hängt den Systemwechsel an, solange `system` gewählt ist. Gibt die Abmeldefunktion zurück. */
export function watchSystemTheme(): () => void {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return () => {};
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  const onChange = () => {
    if (read() === "system") applyTheme("system");
  };
  mq.addEventListener?.("change", onChange);
  return () => mq.removeEventListener?.("change", onChange);
}

export function useTheme() {
  const preference = useSyncExternalStore(subscribe, read, () => DEFAULT_THEME);
  return { preference, resolved: resolveTheme(preference), setTheme };
}

/** Nächster Wert beim Umschalten per Button/Shortcut: dunkel ↔ hell. */
export function toggledTheme(pref: ThemePreference): ResolvedTheme {
  return resolveTheme(pref) === "dark" ? "light" : "dark";
}
