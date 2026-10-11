const EMAIL_RE = /^[^\s@,;]+@[^\s@,;]+\.[^\s@,;]+$/;
export const MAX_EMAILS = 50;

/** Zerlegt Freitext (Komma, Semikolon, Leerraum) in eindeutige Adressen; `invalid` enthält Unbrauchbares. */
export function parseEmails(text: string): { emails: string[]; invalid: string[] } {
  const seen = new Set<string>();
  const emails: string[] = [];
  const invalid: string[] = [];
  for (const raw of text.split(/[\s,;]+/)) {
    const e = raw.trim();
    if (!e) continue;
    if (!EMAIL_RE.test(e)) invalid.push(e);
    else if (!seen.has(e.toLowerCase())) {
      seen.add(e.toLowerCase());
      emails.push(e);
    }
  }
  return { emails, invalid };
}
