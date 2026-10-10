/**
 * Session-Zugriff gegen `/auth/login`, `/auth/logout`, `/auth/me` (Cookie-Session, WP02).
 *
 * Erwartete Antwort von `GET /auth/me` (200): `{ user: { id, name?, displayName?, email, avatarUrl? },
 * organization | workspace: { id?, name, urlKey } }`. Der Parser ist bewusst tolerant, damit kleine
 * Abweichungen des Servers die Shell nicht brechen. 401/403 bedeuten „nicht angemeldet“.
 */
import { useQuery, type QueryClient } from "@tanstack/react-query";

import { queryClient as defaultClient } from "@/app/query-client";

export interface SessionUser {
  id: string;
  name: string;
  email: string;
  avatarUrl: string | null;
}

export interface SessionWorkspace {
  id: string;
  name: string;
  /** `urlKey` des Workspace, Präfix der Screen-Routen (`/acme/inbox`). */
  urlKey: string;
}

/** Team, in dem der User Mitglied ist (aus `/auth/me`). */
export interface SessionTeam {
  id: string;
  key: string;
  name: string;
  parentId: string | null;
  cyclesEnabled: boolean;
  triageEnabled: boolean;
}

export interface Session {
  user: SessionUser;
  workspace: SessionWorkspace;
  teams: SessionTeam[];
}

export const SESSION_QUERY_KEY = ["session"] as const;

const str = (v: unknown): string | null => (typeof v === "string" && v.length > 0 ? v : null);
const rec = (v: unknown): Record<string, unknown> | null =>
  v && typeof v === "object" ? (v as Record<string, unknown>) : null;

/** Wandelt die Antwort von `/auth/me` in eine `Session`; `null`, wenn Pflichtfelder fehlen. */
function parseTeams(raw: unknown): SessionTeam[] {
  if (!Array.isArray(raw)) return [];
  return raw.flatMap((t): SessionTeam[] => {
    const o = rec(t);
    const id = str(o?.id);
    const key = str(o?.key);
    if (!o || !id || !key) return [];
    return [
      {
        id,
        key,
        name: str(o.name) ?? key,
        parentId: str(o.parentId),
        cyclesEnabled: o.cyclesEnabled === true,
        triageEnabled: o.triageEnabled === true,
      },
    ];
  });
}

export function normalizeSession(raw: unknown): Session | null {
  const root = rec(raw);
  // `/auth/me` (WP02) liefert den User auf oberster Ebene: { id, email, …, workspace }.
  const u = rec(root?.user ?? root?.viewer) ?? (typeof root?.email === "string" ? root : null);
  const w = rec(root?.organization ?? root?.workspace);
  if (!u) return null;
  const email = str(u.email);
  const id = str(u.id) ?? email;
  if (!id || !email) return null;
  const urlKey = str(w?.urlKey) ?? str(w?.url_key) ?? "app";
  return {
    user: {
      id,
      email,
      name: str(u.displayName) ?? str(u.name) ?? email.split("@")[0] ?? email,
      avatarUrl: str(u.avatarUrl) ?? null,
    },
    workspace: { id: str(w?.id) ?? urlKey, name: str(w?.name) ?? urlKey, urlKey },
    teams: parseTeams(root?.teams),
  };
}

export class AuthError extends Error {
  readonly status: number;
  constructor(message: string, status: number) {
    super(message);
    this.name = "AuthError";
    this.status = status;
  }
}

/** Lädt die aktuelle Session; `null` = nicht angemeldet, Fehler bei Server-/Netzproblemen. */
export async function fetchSession(fetchImpl: typeof fetch = fetch): Promise<Session | null> {
  const res = await fetchImpl("/auth/me", {
    credentials: "same-origin",
    headers: { accept: "application/json" },
  });
  if (res.status === 401 || res.status === 403) return null;
  if (!res.ok) throw new AuthError(`Session check failed (${res.status})`, res.status);
  return normalizeSession(await res.json().catch(() => null));
}

/** Mit Cache (60 s); vom Auth-Guard und von `useSession` genutzt. */
export function ensureSession(client: QueryClient = defaultClient): Promise<Session | null> {
  return client.ensureQueryData({
    queryKey: SESSION_QUERY_KEY,
    queryFn: () => fetchSession(),
    staleTime: 60_000,
  });
}

/** Aktuelle Session aus dem Cache (vom Auth-Guard vorbefüllt); `undefined` solange sie lädt. */
export function useSession(): Session | null | undefined {
  return useQuery({
    queryKey: SESSION_QUERY_KEY,
    queryFn: () => fetchSession(),
    staleTime: 60_000,
  }).data;
}

export type LoginResult =
  | { ok: true }
  | { ok: false; reason: "invalid_credentials" | "rate_limited" | "server_error" | "network" };

export async function login(
  email: string,
  password: string,
  fetchImpl: typeof fetch = fetch,
): Promise<LoginResult> {
  let res: Response;
  try {
    res = await fetchImpl("/auth/login", {
      method: "POST",
      credentials: "same-origin",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({ email, password }),
    });
  } catch {
    return { ok: false, reason: "network" };
  }
  if (res.ok) return { ok: true };
  if (res.status === 401 || res.status === 400 || res.status === 403)
    return { ok: false, reason: "invalid_credentials" };
  if (res.status === 429) return { ok: false, reason: "rate_limited" };
  return { ok: false, reason: "server_error" };
}

export async function logout(fetchImpl: typeof fetch = fetch): Promise<void> {
  try {
    await fetchImpl("/auth/logout", { method: "POST", credentials: "same-origin" });
  } catch {
    /* Cookie läuft ohnehin ab; Client-Zustand wird trotzdem verworfen. */
  }
}

export interface LoginValues {
  email: string;
  password: string;
}
export type LoginErrors = Partial<Record<keyof LoginValues, string>>;

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** Validierung des Login-Formulars (Client-Seite; der Server entscheidet endgültig). */
export function validateLogin(values: LoginValues): LoginErrors {
  const errors: LoginErrors = {};
  const email = values.email.trim();
  if (!email) errors.email = "Enter your email address.";
  else if (!EMAIL_RE.test(email)) errors.email = "Enter a valid email address.";
  if (!values.password) errors.password = "Enter your password.";
  return errors;
}

/** Nur Pfade innerhalb der App zulassen (kein Open Redirect). */
/**
 * Nur Ziele auf derselben Origin zulassen (Schutz vor Open Redirect).
 * Lehnt u. a. `//host`, `/\\host`, Steuerzeichen und absolute URLs ab.
 */
export function safeRedirect(target: unknown): string {
  if (typeof target !== "string" || !target.startsWith("/")) return "/";
  // eslint-disable-next-line no-control-regex
  if (/[\\\u0000-\u001f\u007f]/.test(target) || target.startsWith("//")) return "/";
  try {
    const origin = window.location.origin;
    const url = new URL(target, origin);
    if (url.origin !== origin) return "/";
    return url.pathname + url.search + url.hash;
  } catch {
    return "/";
  }
}
