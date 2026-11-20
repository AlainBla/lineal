/**
 * Self-Service-Passwort-Reset (LIN-58): `POST /auth/forgot-password` und `POST /auth/reset-password`.
 *
 * Enumeration-Schutz: Die Anfrage antwortet IMMER mit `200` — ob ein Konto existiert, nicht
 * existiert oder das Limit erreicht ist. Die Oberfläche zeigt darum stets dieselbe Bestätigung und
 * verrät nie, ob zur eingegebenen Adresse ein Konto gehört. Nach einem erfolgreichen Reset gibt es
 * KEINE Session — der Nutzer meldet sich frisch an.
 */

/** Mindestlänge (Server: `validate_password`, min 12). */
export const MIN_PASSWORD_LEN = 12;

const CSRF_HEADERS = {
  "content-type": "application/json",
  accept: "application/json",
  "x-lineal-csrf": "1",
} as const;

export type RequestResult =
  { kind: "sent" } | { kind: "error"; reason: "network" | "server_error" };

/** Fordert einen Reset-Link an. `urlKey` grenzt auf einen Workspace ein (bei Adresse in mehreren). */
export async function requestPasswordReset(
  email: string,
  urlKey: string | undefined,
  fetchImpl: typeof fetch = fetch,
): Promise<RequestResult> {
  let res: Response;
  try {
    res = await fetchImpl("/auth/forgot-password", {
      method: "POST",
      credentials: "same-origin",
      headers: CSRF_HEADERS,
      body: JSON.stringify({
        email: email.trim(),
        ...(urlKey && urlKey.trim() ? { urlKey: urlKey.trim() } : {}),
      }),
    });
  } catch {
    return { kind: "error", reason: "network" };
  }
  // Der Server antwortet immer neutral (200); auch ein Limit (429) führt zur selben Bestätigung,
  // damit die Oberfläche keine Existenz/Frequenz verrät.
  if (res.ok || res.status === 429) return { kind: "sent" };
  return { kind: "error", reason: "server_error" };
}

export type ResetResult =
  | { kind: "ok" }
  | {
      kind: "error";
      reason: "invalid_link" | "weak_password" | "rate_limited" | "server_error" | "network";
    };

/** Löst ein Reset-Token ein und setzt das neue Passwort. */
export async function resetPassword(
  token: string,
  newPassword: string,
  fetchImpl: typeof fetch = fetch,
): Promise<ResetResult> {
  let res: Response;
  try {
    res = await fetchImpl("/auth/reset-password", {
      method: "POST",
      credentials: "same-origin",
      headers: CSRF_HEADERS,
      body: JSON.stringify({ token, newPassword }),
    });
  } catch {
    return { kind: "error", reason: "network" };
  }
  if (res.ok) return { kind: "ok" };
  if (res.status === 429) return { kind: "error", reason: "rate_limited" };
  if (res.status === 400 || res.status === 422) {
    const body = (await res.json().catch(() => null)) as {
      details?: { field?: unknown; reason?: unknown };
    } | null;
    if (body?.details?.field === "password") return { kind: "error", reason: "weak_password" };
    // Alles andere (unbekanntes/abgelaufenes/verbrauchtes Token): neutraler „Link ungültig".
    return { kind: "error", reason: "invalid_link" };
  }
  return { kind: "error", reason: "server_error" };
}

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export interface ResetErrors {
  password?: string;
  confirm?: string;
}

/** Client-Validierung des neuen Passworts (der Server entscheidet endgültig). */
export function validateNewPassword(password: string, confirm: string): ResetErrors {
  const e: ResetErrors = {};
  if (password.length < MIN_PASSWORD_LEN)
    e.password = `Use at least ${MIN_PASSWORD_LEN} characters.`;
  if (confirm !== password) e.confirm = "The passwords do not match.";
  return e;
}

/** Client-Validierung der E-Mail im „Passwort vergessen"-Formular. */
export function validateForgotEmail(email: string): string | undefined {
  const e = email.trim();
  if (!e) return "Enter your email address.";
  if (!EMAIL_RE.test(e)) return "Enter a valid email address.";
  return undefined;
}
