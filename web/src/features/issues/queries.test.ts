/**
 * Prüft, dass alle GraphQL-Dokumente der Issue-Screens gegen `web/schema.graphql` gültig sind
 * (Felder, Argumente, Variablentypen) – die Komponententests mocken fetch und würden Abweichungen nicht sehen.
 */
import { readFileSync } from "node:fs";
import path from "node:path";

import { buildSchema, parse, validate } from "graphql";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  createComment,
  createIssue,
  createReaction,
  createRelation,
  deleteReaction,
  fetchComments,
  fetchIssue,
  fetchIssues,
  fetchTeamMeta,
  fetchTeams,
  setSubscription,
  updateIssue,
} from "./queries";

const schema = buildSchema(
  readFileSync(path.resolve(import.meta.dirname, "../../../schema.graphql"), "utf8"),
);

afterEach(() => vi.unstubAllGlobals());

async function documentOf(run: () => Promise<unknown>): Promise<string> {
  let query = "";
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_u: unknown, init?: RequestInit) => {
      query = (JSON.parse(String(init?.body)) as { query: string }).query;
      return new Response(JSON.stringify({ errors: [{ message: "stop" }] }), { status: 200 });
    }),
  );
  await run().catch(() => undefined);
  return query;
}

describe("GraphQL-Dokumente passen zum Schema", () => {
  const cases: Array<[string, () => Promise<unknown>]> = [
    ["fetchIssues", () => fetchIssues({ teamId: "ENG" })],
    ["fetchTeamMeta", () => fetchTeamMeta("ENG")],
    ["fetchTeams", () => fetchTeams()],
    ["fetchIssue", () => fetchIssue("ENG-1")],
    ["fetchComments", () => fetchComments("i1")],
    ["updateIssue", () => updateIssue("i1", { priority: 1 })],
    ["createIssue", () => createIssue({ teamId: "t", title: "x" })],
    ["createComment", () => createComment({ issueId: "i1", body: "x" })],
    ["createReaction", () => createReaction("c1", "👍")],
    ["deleteReaction", () => deleteReaction("r1")],
    ["createRelation", () => createRelation({ issueId: "a", relatedIssueId: "b", type: "blocks" })],
    ["subscribe", () => setSubscription("i1", true)],
    ["unsubscribe", () => setSubscription("i1", false)],
  ];

  it.each(cases)("%s", async (_name, run) => {
    const query = await documentOf(run);
    expect(query).not.toBe("");
    const errors = validate(schema, parse(query));
    expect(errors.map((e) => e.message)).toEqual([]);
  });
});
