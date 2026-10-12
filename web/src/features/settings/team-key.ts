/** Team-Key: Vorschlag aus dem Namen und Validierung (docs/02 §2.2). */

export const TEAM_KEY_RE = /^[A-Z][A-Z0-9]{0,6}$/;

/** Reserviert: `P`, `I` sowie alles mit `P-`/`I-` (kollidiert mit Projekt-/Initiativ-Kennungen). */
export function isReservedKey(key: string): boolean {
  return key === "P" || key === "I" || key.startsWith("P-") || key.startsWith("I-");
}

/** Normalisiert die Eingabe: trimmen, Großbuchstaben. */
export const normalizeKey = (s: string) => s.trim().toUpperCase();

/** Fehlertext für einen Key oder `null`, wenn er gültig ist (Eindeutigkeit prüft der Server). */
export function validateKey(key: string): string | null {
  if (key === "") return "Enter an identifier.";
  if (!TEAM_KEY_RE.test(key))
    return "Use 1–7 characters: capital letters and digits, starting with a letter.";
  if (isReservedKey(key)) return "This identifier is reserved.";
  return null;
}

const TRANSLITERATE: Record<string, string> = { Ä: "AE", Ö: "OE", Ü: "UE", ß: "SS" };

/** Schlägt einen Key aus dem Namen vor; vermeidet Kollisionen mit `taken` (Keys, case-insensitiv). */
export function suggestTeamKey(name: string, taken: Iterable<string> = []): string {
  const used = new Set([...taken].map((k) => k.toUpperCase()));
  const upper = name
    .replace(/ß/g, "SS")
    .toUpperCase()
    .replace(/[ÄÖÜ]/g, (c) => TRANSLITERATE[c] ?? c);
  const words = upper
    .split(/\s+/)
    .map((w) => w.replace(/[^A-Z0-9]/g, ""))
    .filter((w) => w !== "");
  let base: string;
  if (words.length === 0) base = "TEAM";
  else if (words.length === 1) base = words[0]!.slice(0, 3);
  else
    base = words
      .slice(0, 4)
      .map((w) => w[0])
      .join("");
  if (/^[0-9]/.test(base)) base = `T${base}`;
  let candidate = base;
  for (let i = 2; isReservedKey(candidate) || used.has(candidate); i++) {
    candidate = base.slice(0, 7 - String(i).length) + String(i);
  }
  return candidate;
}

/** Erlaubte Sichtbarkeiten eines Teams unter einem Parent (docs/02 §5.2). */
export function allowedVisibilities(parent: { visibility: string } | undefined): string[] {
  if (!parent) return ["public", "private"];
  if (parent.visibility === "public") return ["public", "restricted", "private"];
  return ["restricted", "private"];
}
