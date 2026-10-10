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
  constructor(message: string, status: number, errors: GraphQLErrorItem[] = []) {
    super(message);
    this.name = "GraphQLRequestError";
    this.status = status;
    this.errors = errors;
  }
  /** `extensions.code` des ersten Fehlers (z. B. `unauthenticated`, `not_found`). */
  get code(): string | undefined {
    return this.errors[0]?.extensions?.code;
  }
}

export interface GqlRequestOptions {
  signal?: AbortSignal;
  /** Für Tests; Default: globales `fetch`. */
  fetchImpl?: typeof fetch;
}

export async function gqlRequest<
  TData,
  TVars extends Record<string, unknown> = Record<string, never>,
>(query: string, variables?: TVars, opts: GqlRequestOptions = {}): Promise<TData> {
  const doFetch = opts.fetchImpl ?? fetch;
  const res = await doFetch(GRAPHQL_ENDPOINT, {
    method: "POST",
    credentials: "same-origin",
    headers: { "content-type": "application/json", accept: "application/json" },
    body: JSON.stringify({ query, variables }),
    signal: opts.signal,
  });
  const body = (await res.json().catch(() => null)) as {
    data?: TData;
    errors?: GraphQLErrorItem[];
  } | null;
  if (!res.ok || body?.errors?.length || !body?.data) {
    const errors = body?.errors ?? [];
    throw new GraphQLRequestError(
      errors[0]?.message ?? `GraphQL request failed (${res.status})`,
      res.status,
      errors,
    );
  }
  return body.data;
}
