/**
 * Daten für Ticket-Chips (`issue-refs.ts`): Team-Schlüssel und Ticket-Kurzinfos aus dem
 * Query-Cache; Fehlendes wird gedrosselt nachgeladen (höchstens 4 Anfragen gleichzeitig).
 * Editoren abonnieren Änderungen und zeichnen ihre Chips dann neu.
 */
import type { QueryClient } from "@tanstack/react-query";

import type { Issue, WorkflowState } from "@/gql/graphql";

import { gqlRequest } from "@/lib/graphql";

import type { IssueRefInfo, ProjectRefInfo } from "./issue-refs";
import { fetchIssue, fetchTeams, keys, type TeamMeta, type TeamRef } from "./queries";

const MAX_PARALLEL = 4;
/** Nicht gefundene Kennungen erst nach dieser Zeit erneut anfragen. */
const MISSING_TTL_MS = 5 * 60_000;

const projectRefKey = (identifier: string) => ["project-ref", identifier] as const;

async function fetchProjectRef(identifier: string, signal?: AbortSignal): Promise<ProjectRefInfo> {
  const res = await gqlRequest<
    { project: { identifier: string; name: string; color: string | null } },
    { id: string }
  >(
    `query ProjectRef($id: String!) { project(id: $id) { identifier name color } }`,
    { id: identifier },
    { signal },
  );
  return res.project;
}

export class IssueRefStore {
  private readonly missing = new Map<string, number>();
  private readonly queue: string[] = [];
  private readonly queued = new Set<string>();
  private active = 0;
  private teamsRequested = false;
  private readonly listeners = new Set<() => void>();
  private notifyScheduled = false;

  constructor(private readonly client: QueryClient) {
    // Statusänderungen (Sync-Patches, Detailansicht) an bekannten Tickets neu zeichnen.
    client.getQueryCache().subscribe((e) => {
      if (e.type === "updated" && e.action.type === "success" && e.query.queryKey[0] === "issue") {
        if (this.listeners.size > 0) this.notify();
      }
    });
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Team-Schlüssel (Großbuchstaben); stößt beim ersten Aufruf das Laden an. */
  keys(): ReadonlySet<string> {
    const teams = this.client.getQueryData<TeamRef[]>(keys.teams);
    if (!teams && !this.teamsRequested) {
      this.teamsRequested = true;
      void this.client
        .fetchQuery({ queryKey: keys.teams, queryFn: ({ signal }) => fetchTeams(signal) })
        .then(() => this.notify())
        .catch(() => {
          this.teamsRequested = false;
        });
    }
    return new Set((teams ?? []).map((t) => t.key.toUpperCase()));
  }

  lookup(identifier: string): IssueRefInfo | null | undefined {
    const issue = this.client.getQueryData<Issue>(keys.issue(identifier));
    if (issue) return this.toInfo(issue);
    const since = this.missing.get(identifier);
    if (since !== undefined && Date.now() - since < MISSING_TTL_MS) return null;
    this.enqueue(identifier);
    return undefined;
  }

  lookupProject(identifier: string): ProjectRefInfo | null | undefined {
    const project = this.client.getQueryData<ProjectRefInfo>(projectRefKey(identifier));
    if (project) return project;
    const since = this.missing.get(identifier);
    if (since !== undefined && Date.now() - since < MISSING_TTL_MS) return null;
    this.enqueue(identifier);
    return undefined;
  }

  private toInfo(issue: Issue): IssueRefInfo {
    const key = issue.identifier.split("-")[0] ?? "";
    let color: string | null = null;
    for (const ref of [issue.teamId, key]) {
      const meta = this.client.getQueryData<TeamMeta>(keys.teamMeta(ref));
      const state = meta?.states.find((s: WorkflowState) => s.id === issue.stateId);
      if (state) {
        color = state.color;
        break;
      }
    }
    return { identifier: issue.identifier, title: issue.title, stateType: issue.stateType, color };
  }

  private enqueue(identifier: string) {
    if (this.queued.has(identifier)) return;
    this.queued.add(identifier);
    this.queue.push(identifier);
    this.pump();
  }

  private pump() {
    while (this.active < MAX_PARALLEL && this.queue.length > 0) {
      const id = this.queue.shift()!;
      this.active += 1;
      // `P-123` ist ein Ticket des Teams P, `P-KOL-3` ein Projekt.
      const load: Promise<unknown> = /^P-[A-Z]/.test(id)
        ? this.client.fetchQuery({
            queryKey: projectRefKey(id),
            queryFn: ({ signal }) => fetchProjectRef(id, signal),
            retry: false,
          })
        : this.client.fetchQuery({
            queryKey: keys.issue(id),
            queryFn: ({ signal }) => fetchIssue(id, signal),
            retry: false,
          });
      void load
        .then(() => this.missing.delete(id))
        .catch(() => this.missing.set(id, Date.now()))
        .finally(() => {
          this.active -= 1;
          this.queued.delete(id);
          this.notify();
          this.pump();
        });
    }
  }

  /** Bündelt Benachrichtigungen (viele Antworten kurz nacheinander → ein Neuzeichnen). */
  private notify() {
    if (this.notifyScheduled) return;
    this.notifyScheduled = true;
    setTimeout(() => {
      this.notifyScheduled = false;
      for (const fn of this.listeners) fn();
    }, 50);
  }
}

const stores = new WeakMap<QueryClient, IssueRefStore>();

/** Ein Store je Query-Client (Tests verwenden eigene Clients). */
export function issueRefStore(client: QueryClient): IssueRefStore {
  let s = stores.get(client);
  if (!s) {
    s = new IssueRefStore(client);
    stores.set(client, s);
  }
  return s;
}
