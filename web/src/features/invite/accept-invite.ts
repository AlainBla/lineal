/**
 * Einladung annehmen: `POST /invite/accept` (Token + E-Mail + Name + Passwort, setzt das Session-Cookie)
 * und – mit bestehender Konto-Session – die GraphQL-Mutation `acceptInvitation(token)`.
 *
 * Fehlermeldungen sind absichtlich einheitlich: der Server antwortet für unbekannte, abgelaufene,
 * widerrufene und bereits angenommene Einladungen identisch (`404`); die Oberfläche verrät nicht mehr.
 */
import { gqlRequest, GraphQLRequestError } from "@/lib/graphql";

export const MIN_PASSWORD_LEN = 8;

export type AcceptResult =
  | { ok: true; workspace: string | null }
  | {
      ok: false;
      reason:
        | "invalid_link"
        | "bad_input"
        | "account_exists"
        | "rate_limited"
        | "rejected"
        | "server_error"
        | "network";
      /** Nur bei `bad_input`: Feld, das der Server beanstandet. */
      field?: "name" | "password" | "email";
    };

/** Nur harmlose URL-Keys in Pfade übernehmen. */
const URL_KEY_RE = /^[A-Za-z0-9_-]{1,64}$/;
const safeKey = (v: unknown): string | null =>
  typeof v === "string" && URL_KEY_RE.test(v) ? v : null;

export interface AcceptValues {
  token: string;
  email: string;
  name: string;
  password: string;
}

export async function acceptInvite(
  values: AcceptValues,
  fetchImpl: typeof fetch = fetch,
): Promise<AcceptResult> {
  let res: Response;
  try {
    res = await fetchImpl("/invite/accept", {
      method: "POST",
      credentials: "same-origin",
      headers: {
        "content-type": "application/json",
        accept: "application/json",
        "x-lineal-csrf": "1",
      },
      body: JSON.stringify({
        token: values.token,
        email: values.email.trim(),
        name: values.name.trim(),
        password: values.password,
      }),
    });
  } catch {
    return { ok: false, reason: "network" };
  }
  if (res.ok) {
    const body = (await res.json().catch(() => null)) as { workspace?: unknown } | null;
    return { ok: true, workspace: safeKey(body?.workspace) };
  }
  if (res.status === 404) return { ok: false, reason: "invalid_link" };
  if (res.status === 409) return { ok: false, reason: "account_exists" };
  if (res.status === 429) return { ok: false, reason: "rate_limited" };
  if (res.status === 403) return { ok: false, reason: "rejected" };
  if (res.status === 400 || res.status === 422) {
    const body = (await res.json().catch(() => null)) as {
      details?: { field?: unknown };
    } | null;
    const f = body?.details?.field;
    return {
      ok: false,
      reason: "bad_input",
      ...(f === "name" || f === "password" || f === "email" ? { field: f } : {}),
    };
  }
  return { ok: false, reason: "server_error" };
}

/** Annahme mit der Konto-Session des angemeldeten Users (E-Mail muss zur Einladung passen). */
export async function acceptInviteAsAccount(token: string): Promise<AcceptResult> {
  try {
    const data = await gqlRequest<{ acceptInvitation: { urlKey: string } }, { token: string }>(
      `mutation InviteAcceptAsAccount($token: String!) { acceptInvitation(token: $token) { success urlKey } }`,
      { token },
    );
    return { ok: true, workspace: safeKey(data.acceptInvitation.urlKey) };
  } catch (err) {
    if (err instanceof GraphQLRequestError) {
      if (err.code === "not_found") return { ok: false, reason: "invalid_link" };
      if (err.code === "conflict") return { ok: false, reason: "account_exists" };
      if (err.code === "rate_limited") return { ok: false, reason: "rate_limited" };
      if (err.status === 401 || err.code === "unauthenticated")
        return { ok: false, reason: "rejected" };
      return { ok: false, reason: "server_error" };
    }
    return { ok: false, reason: "network" };
  }
}
