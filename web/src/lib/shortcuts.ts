/**
 * Shortcut-Registry mit Kontexten und Sequenzen (docs/08 §3, docs/11 §4).
 *
 * Notation der Tastenfolgen: Kombinationen mit `+` (`mod+k`, `mod+shift+1`, `shift+v`),
 * Sequenzen mit Leerzeichen (`g i`, `g shift+w`). `mod` = Ctrl oder Cmd. Nach dem ersten
 * Schritt einer Sequenz bleiben `SEQUENCE_TIMEOUT_MS` für den nächsten.
 *
 * Kontexte: `global` gilt immer; `list` und `issue` gelten, solange sie per `pushContext`
 * aktiv sind; ist `modal` aktiv, feuern nur `modal`-Shortcuts und solche mit `allowInModal`.
 * Bei gleicher Tastenfolge gewinnt der spezifischere Kontext (modal > issue > list > global).
 */
import { useEffect, useLayoutEffect, useRef } from "react";

export type ShortcutContext = "global" | "list" | "issue" | "modal";

export const SEQUENCE_TIMEOUT_MS = 1000;

const SPECIFICITY: Record<ShortcutContext, number> = { global: 0, list: 1, issue: 2, modal: 3 };

export interface ShortcutDef {
  /** Eindeutige ID, identisch mit der des Command-Menü-Eintrags. */
  id: string;
  keys: string;
  context?: ShortcutContext;
  description: string;
  /** Wird ausgeführt; Rückgabe `false` lässt das Ereignis unverbraucht. */
  run: () => void | false;
  /** Feuert auch, wenn ein Modal aktiv ist (z. B. `mod+k` zum Schließen). */
  allowInModal?: boolean;
  /** Feuert auch beim Tippen in Eingabefeldern (Default: nur mit Mod/Alt-Kombination). */
  allowInInput?: boolean;
}

export interface Combo {
  key: string;
  mod: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
}

/** Minimale Ereignisform (KeyboardEvent erfüllt sie). */
export interface KeyLike {
  key: string;
  code?: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  target?: EventTarget | null;
  preventDefault?: () => void;
  defaultPrevented?: boolean;
  isComposing?: boolean;
  repeat?: boolean;
}

const KEY_ALIASES: Record<string, string> = {
  esc: "escape",
  return: "enter",
  del: "delete",
  up: "arrowup",
  down: "arrowdown",
  left: "arrowleft",
  right: "arrowright",
  space: " ",
  plus: "+",
};

export function parseCombo(spec: string): Combo {
  const parts = spec.toLowerCase().split("+");
  // "+" selbst als Taste: "mod++" ergibt ["mod", "", ""]
  const last = parts.pop() ?? "";
  const rawKey = last === "" ? "+" : last;
  const combo: Combo = {
    key: KEY_ALIASES[rawKey] ?? rawKey,
    mod: false,
    ctrl: false,
    alt: false,
    shift: false,
  };
  for (const p of parts) {
    if (p === "mod") combo.mod = true;
    else if (p === "ctrl") combo.ctrl = true;
    else if (p === "alt" || p === "option") combo.alt = true;
    else if (p === "shift") combo.shift = true;
    else if (p === "") continue;
    else throw new Error(`Unknown modifier "${p}" in shortcut "${spec}"`);
  }
  return combo;
}

export function parseSequence(spec: string): Combo[] {
  return spec.trim().split(/\s+/).map(parseCombo);
}

const isLetterOrDigit = (k: string) => /^[a-z0-9]$/i.test(k);

/** Taste layoutunabhängig: bei Modifikatoren zählt der physische Code (Shift+1 liefert sonst "!"). */
function effectiveKey(e: KeyLike): string {
  const modified = e.ctrlKey || e.metaKey || e.altKey || e.shiftKey;
  if (modified && e.code) {
    const m = /^(?:Key([A-Z])|Digit([0-9]))$/.exec(e.code);
    if (m) return (m[1] ?? m[2] ?? "").toLowerCase();
  }
  return e.key.toLowerCase();
}

/** Passt das Tastaturereignis auf die Kombination? */
export function matchesCombo(e: KeyLike, c: Combo): boolean {
  if (effectiveKey(e) !== c.key) return false;
  const ctrlOrMeta = e.ctrlKey || e.metaKey;
  if (c.mod) {
    if (!ctrlOrMeta) return false;
  } else if (c.ctrl) {
    if (!e.ctrlKey) return false;
  } else if (ctrlOrMeta) {
    return false;
  }
  if (c.alt !== e.altKey) return false;
  // Symbole wie "?", "#" oder "[" entstehen layoutabhängig mit/ohne Shift: Shift ignorieren.
  if (isLetterOrDigit(c.key) || c.key.length > 1) {
    if (c.shift !== e.shiftKey) return false;
  }
  return true;
}

function isEditableTarget(t: EventTarget | null | undefined): boolean {
  if (!t || typeof (t as HTMLElement).tagName !== "string") return false;
  const el = t as HTMLElement;
  const tag = el.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return el.isContentEditable === true || el.getAttribute?.("contenteditable") === "true";
}

const isModifierOnly = (key: string) =>
  ["shift", "control", "alt", "meta"].includes(key.toLowerCase());

interface Entry {
  def: ShortcutDef;
  seq: Combo[];
}

type Listener = () => void;

export class ShortcutRegistry {
  private entries = new Map<string, Entry>();
  private contexts: ShortcutContext[] = [];
  private pending: Combo[] = [];
  private timer: ReturnType<typeof setTimeout> | null = null;
  private listeners = new Set<Listener>();
  private version = 0;

  register(def: ShortcutDef): () => void {
    const entry: Entry = { def, seq: parseSequence(def.keys) };
    this.entries.set(def.id, entry);
    this.bump();
    return () => {
      if (this.entries.get(def.id) === entry) {
        this.entries.delete(def.id);
        this.bump();
      }
    };
  }

  list(): ShortcutDef[] {
    return [...this.entries.values()].map((e) => e.def);
  }

  /** Aktiviert einen Kontext (Stapel); gibt die Funktion zum Entfernen zurück. */
  pushContext(ctx: ShortcutContext): () => void {
    this.contexts.push(ctx);
    this.bump();
    return () => {
      const i = this.contexts.lastIndexOf(ctx);
      if (i >= 0) this.contexts.splice(i, 1);
      this.bump();
    };
  }

  activeContexts(): ShortcutContext[] {
    return ["global", ...new Set(this.contexts)];
  }

  /** Teilfolge der aktuell wartenden Sequenz (leer = keine). Für UI-Hinweise. */
  pendingKeys(): string[] {
    return this.pending.map((c) => c.key);
  }

  subscribe(l: Listener): () => void {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  }

  getVersion(): number {
    return this.version;
  }

  reset(): void {
    this.clearPending();
    this.contexts = [];
    this.entries.clear();
    this.bump();
  }

  private bump() {
    this.version++;
    this.listeners.forEach((l) => l());
  }

  private clearPending() {
    this.pending = [];
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  private applicable(entry: Entry): boolean {
    const ctx = entry.def.context ?? "global";
    const active = this.contexts;
    if (active.includes("modal")) return ctx === "modal" || entry.def.allowInModal === true;
    return ctx === "global" || active.includes(ctx);
  }

  /**
   * Verarbeitet ein Tastaturereignis. Gibt `true` zurück, wenn es verbraucht wurde
   * (Shortcut ausgeführt oder Sequenz begonnen).
   */
  handleKeyDown(e: KeyLike): boolean {
    if (e.isComposing || isModifierOnly(e.key)) return false;
    const editable = isEditableTarget(e.target);
    const hasMod = e.ctrlKey || e.metaKey || e.altKey;

    const step = this.pending;
    const candidates = [...this.entries.values()].filter((en) => {
      if (!this.applicable(en)) return false;
      if (editable && !(hasMod || en.def.allowInInput)) return false;
      return true;
    });

    const tryMatch = (done: Combo[]): { exact: Entry[]; prefix: boolean } => {
      const exact: Entry[] = [];
      let prefix = false;
      const i = done.length;
      for (const en of candidates) {
        if (en.seq.length <= i) continue;
        if (!done.every((c, idx) => sameCombo(en.seq[idx]!, c))) continue;
        const next = en.seq[i]!;
        if (!matchesCombo(e, next)) continue;
        if (en.seq.length === i + 1) exact.push(en);
        else prefix = true;
      }
      return { exact, prefix };
    };

    let { exact, prefix } = tryMatch(step);
    if (step.length > 0 && exact.length === 0 && !prefix) {
      // Sequenz abgebrochen: Taste als Neustart interpretieren.
      this.clearPending();
      ({ exact, prefix } = tryMatch([]));
    }

    const pressed = comboFromEvent(e);
    if (exact.length > 0) {
      this.clearPending();
      const best = exact.reduce((a, b) =>
        SPECIFICITY[b.def.context ?? "global"] > SPECIFICITY[a.def.context ?? "global"] ? b : a,
      );
      const result = best.def.run();
      if (result === false) return false;
      e.preventDefault?.();
      return true;
    }
    if (prefix) {
      this.pending = [...this.pending, pressed];
      if (this.timer) clearTimeout(this.timer);
      this.timer = setTimeout(() => this.clearPending(), SEQUENCE_TIMEOUT_MS);
      e.preventDefault?.();
      return true;
    }
    this.clearPending();
    return false;
  }
}

function comboFromEvent(e: KeyLike): Combo {
  return {
    key: effectiveKey(e),
    mod: e.ctrlKey || e.metaKey,
    ctrl: false,
    alt: e.altKey,
    shift: e.shiftKey,
  };
}

/** Vergleicht eine gespeicherte Soll-Kombination mit einer bereits gedrückten Ist-Kombination. */
function sameCombo(want: Combo, got: Combo): boolean {
  const modOk = want.mod || want.ctrl ? got.mod : !got.mod;
  const shiftOk =
    isLetterOrDigit(want.key) || want.key.length > 1 ? want.shift === got.shift : true;
  return want.key === got.key && modOk && want.alt === got.alt && shiftOk;
}

/** Globale Registry der Anwendung. */
export const shortcuts = new ShortcutRegistry();

/** Hängt die Registry an `document` (einmal, in der Shell). */
export function attachShortcuts(target: Document = document, reg = shortcuts): () => void {
  const onKey = (e: KeyboardEvent) => {
    if (e.defaultPrevented) return;
    reg.handleKeyDown(e);
  };
  target.addEventListener("keydown", onKey);
  return () => target.removeEventListener("keydown", onKey);
}

/** Registriert Shortcuts für die Lebensdauer der Komponente. `run` darf Closures über State sein. */
export function useShortcuts(defs: ShortcutDef[], reg: ShortcutRegistry = shortcuts): void {
  const ref = useRef(defs);
  useLayoutEffect(() => {
    ref.current = defs;
  });
  const signature = defs.map((d) => `${d.id}|${d.keys}|${d.context ?? "global"}`).join(",");
  useEffect(() => {
    const offs = ref.current.map((d, i) =>
      reg.register({ ...d, run: () => ref.current[i]?.run() }),
    );
    return () => offs.forEach((off) => off());
  }, [signature, reg]);
}

/** Aktiviert einen Kontext, solange `active` wahr ist (z. B. `modal`, solange ein Dialog offen ist). */
export function useShortcutContext(
  ctx: ShortcutContext,
  active = true,
  reg: ShortcutRegistry = shortcuts,
): void {
  useEffect(() => {
    if (!active) return;
    return reg.pushContext(ctx);
  }, [ctx, active, reg]);
}

const isMac = () => typeof navigator !== "undefined" && /mac|iphone|ipad/i.test(navigator.platform);

const KEY_LABELS: Record<string, string> = {
  escape: "Esc",
  enter: "↵",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
  " ": "Space",
};

/** Anzeigeform einer Tastenfolge: `["Ctrl", "K"]` bzw. `["⌘", "K"]`; Sequenzen als `[["G"], ["I"]]`. */
export function formatKeys(spec: string, mac = isMac()): string[][] {
  return parseSequence(spec).map((c) => {
    const out: string[] = [];
    if (c.mod) out.push(mac ? "⌘" : "Ctrl");
    if (c.ctrl) out.push(mac ? "⌃" : "Ctrl");
    if (c.alt) out.push(mac ? "⌥" : "Alt");
    if (c.shift) out.push("⇧");
    out.push(KEY_LABELS[c.key] ?? c.key.toUpperCase());
    return out;
  });
}
