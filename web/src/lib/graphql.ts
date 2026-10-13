/**
 * Schlanker GraphQL-Client (POST /graphql, Session-Cookie). Die Schema-Typen werden von
 * `graphql-codegen` aus `web/schema.graphql` nach `src/gql/graphql.ts` erzeugt (`npm run codegen`).
 *
 * Verwendung: `const data = await gqlRequest<{ viewer: Viewer }>(gql\`query { viewer { id } }\`)`.
 */

/** Von graphql-codegen erzeugte Schema-Typen (Wurzeltypen). */
export type { Query, Mutation } from "@/gql/graphql";

export const GRAPHQL_ENDPOINT = "/graphql";

/** Markiert Query-Strings für Editor-Highlighting und spätere Codegen-Dokumente. */
export function gql(strings: TemplateStringsArray, ...values: unknown[]): string {
  return strings.reduce((acc, s, i) => acc + s + (i < values.length ? String(values[i]) : ""), "");
}

export interface GraphQLErrorItem {
  message: string;
  path?: (string | number)[];
  extensions?: { code?: string; [k: string]: unknown };
}

export class GraphQLRequestError extends Error {
  readonly status: number;
  readonly errors: GraphQLErrorItem[];
  /** Wartezeit aus dem `Retry-After`-Header (Sekunden), falls der Server sie nennt. */
  readonly retryAfterSeconds?: number;
  constructor(
    message: string,
    status: number,
    errors: GraphQLErrorItem[] = [],
    retryAfterSeconds?: number,
  ) {
    super(message);
    this.name = "GraphQLRequestError";
    this.status = status;
    this.errors = errors;
    this.retryAfterSeconds = retryAfterSeconds;
  }
  /** `extensions.code` des ersten Fehlers (z. B. `unauthenticated`, `not_found`). */
  get code(): string | undefined {
    return this.errors[0]?.extensions?.code;
  }
  /** Der Server war nicht erreichbar (kein HTTP-Status). */
  get isNetwork(): boolean {
    return this.status === 0;
  }
}

export const NETWORK_ERROR_MESSAGE = "Could not reach the server. Check your connection.";

/** `Retry-After` als Sekunden (nur die Zahlenform; Datumsform wird ignoriert). */
export function parseRetryAfter(value: string | null | undefined): number | undefined {
  if (!value) return undefined;
  const n = Number(value.trim());
  return Number.isFinite(n) && n >= 0 ? n : undefined;
}

/** Obergrenze für die automatische Wartezeit vor dem einmaligen 429-Wiederholversuch. */
export const MAX_RETRY_WAIT_MS = 10_000;
const DEFAULT_RETRY_WAIT_MS = 1_000;

/** Wartezeit vor dem Wiederholversuch: `Retry-After`, sonst 1 s, höchstens 10 s. */
export function retryDelayMs(retryAfterSeconds: number | undefined): number {
  const ms = retryAfterSeconds === undefined ? DEFAULT_RETRY_WAIT_MS : retryAfterSeconds * 1000;
  return Math.min(Math.max(ms, 0), MAX_RETRY_WAIT_MS);
}

const sleep = (ms: number, signal?: AbortSignal) =>
  new Promise<void>((resolve, reject) => {
    const t = setTimeout(resolve, ms);
    signal?.addEventListener(
      "abort",
      () => {
        clearTimeout(t);
        reject(signal.reason ?? new DOMException("Aborted", "AbortError"));
      },
      { once: true },
    );
  });

type UnauthenticatedHandler = () => void;
let onUnauthenticated: UnauthenticatedHandler | null = null;

/** Die App registriert hier den Rücksprung zum Login bei abgelaufener Session (HTTP 401). */
export function setUnauthenticatedHandler(handler: UnauthenticatedHandler | null): void {
  onUnauthenticated = handler;
}

export interface GqlRequestOptions {
  signal?: AbortSignal;
  /** Für Tests; Default: globales `fetch`. */
  fetchImpl?: typeof fetch;
  /** Für Tests: Wartefunktion des 429-Wiederholversuchs. */
  sleepImpl?: (ms: number) => Promise<void>;
}

async function send(
  doFetch: typeof fetch,
  query: string,
  variables: unknown,
  signal: AbortSignal | undefined,
): Promise<Response> {
  try {
    return await doFetch(GRAPHQL_ENDPOINT, {
      method: "POST",
      credentials: "same-origin",
      headers: {
        "content-type": "application/json",
        accept: "application/json",
        // CSRF-Schutz für Cookie-Sessions (Backend verlangt Header + erlaubten Origin).
        "x-lineal-csrf": "1",
      },
      body: JSON.stringify({ query, variables }),
      signal,
    });
  } catch (e) {
    if (signal?.aborted || (e instanceof DOMException && e.name === "AbortError")) throw e;
    throw new GraphQLRequestError(NETWORK_ERROR_MESSAGE, 0);
  }
}

export async function gqlRequest<
  TData,
  TVars extends Record<string, unknown> = Record<string, never>,
>(query: string, variables?: TVars, opts: GqlRequestOptions = {}): Promise<TData> {
  const doFetch = opts.fetchImpl ?? fetch;
  let res = await send(doFetch, query, variables, opts.signal);
  if (res.status === 429) {
    // Die Anfrage wurde nicht verarbeitet: einmal nach Retry-After wiederholen (docs/08 F4).
    const wait = retryDelayMs(parseRetryAfter(res.headers.get("retry-after")));
    await (opts.sleepImpl ? opts.sleepImpl(wait) : sleep(wait, opts.signal));
    res = await send(doFetch, query, variables, opts.signal);
  }
  const body = (await res.json().catch(() => null)) as {
    data?: TData;
    errors?: GraphQLErrorItem[];
  } | null;
  if (!res.ok || body?.errors?.length || !body?.data) {
    const errors = body?.errors ?? [];
    const err = new GraphQLRequestError(
      errors[0]?.message ?? `GraphQL request failed (${res.status})`,
      res.status,
      errors,
      parseRetryAfter(res.headers.get("retry-after")),
    );
    if (res.status === 401 || err.code === "unauthenticated") onUnauthenticated?.();
    throw err;
  }
  return body.data;
}

/**
 * Verständlicher Text zu einem Fehler (Netzwerk, 429, Berechtigung, Session); sonst die
 * Servermeldung bzw. `fallback`.
 */
export function describeError(err: unknown, fallback = "Something went wrong."): string {
  if (err instanceof GraphQLRequestError) {
    if (err.isNetwork) return NETWORK_ERROR_MESSAGE;
    if (err.status === 429 || err.code === "rate_limited") {
      const s = err.retryAfterSeconds;
      return s !== undefined
        ? `Too many requests. Try again in ${Math.max(1, Math.ceil(s))} seconds.`
        : "Too many requests. Try again in a moment.";
    }
    if (err.status === 401 || err.code === "unauthenticated")
      return "Your session has expired. Sign in again.";
    if (err.code === "not_found") return "This page does not exist or you do not have access.";
    if (err.code === "forbidden" || /^Not allowed to perform/.test(err.message))
      return "You do not have permission to do this.";
    return err.message || fallback;
  }
  if (err instanceof TypeError && /fetch/i.test(err.message)) return NETWORK_ERROR_MESSAGE;
  return err instanceof Error && err.message ? err.message : fallback;
}
