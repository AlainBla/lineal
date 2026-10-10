import { Link } from "@tanstack/react-router";
import { useMemo } from "react";

import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import type { Initiative } from "@/gql/graphql";

import { useInitiative, useInitiatives } from "./queries";
import { HealthBadge, PageHeader, shortDate } from "./ui";

export interface InitiativeNode {
  initiative: Initiative;
  depth: number;
}

/**
 * Initiativen als eingerückte Liste (DAG, docs/08 §2.8): Wurzeln = ohne (sichtbaren) Parent; eine
 * Sub-Initiative mit mehreren Eltern erscheint unter jedem. Zyklen werden abgebrochen.
 */
export function buildInitiativeTree(all: Initiative[]): InitiativeNode[] {
  const byId = new Map(all.map((i) => [i.id, i]));
  const children = new Map<string, Initiative[]>();
  const roots: Initiative[] = [];
  for (const i of all) {
    const parents = i.parentInitiatives.filter((p) => byId.has(p.id));
    if (parents.length === 0) roots.push(i);
    for (const p of parents) children.set(p.id, [...(children.get(p.id) ?? []), i]);
  }
  const out: InitiativeNode[] = [];
  const walk = (i: Initiative, depth: number, path: Set<string>) => {
    out.push({ initiative: i, depth });
    if (path.has(i.id)) return;
    const next = new Set(path).add(i.id);
    for (const c of children.get(i.id) ?? []) if (!next.has(c.id)) walk(c, depth + 1, next);
  };
  for (const r of roots) walk(r, 0, new Set());
  return out;
}

export type InitiativesTab = "active" | "planned" | "all";
export const isInitiativesTab = (v: string): v is InitiativesTab =>
  v === "active" || v === "planned" || v === "all";

export function filterInitiatives(all: Initiative[], tab: InitiativesTab): Initiative[] {
  if (tab === "all") return all;
  return all.filter((i) =>
    tab === "active"
      ? i.statusType === "active"
      : i.statusType === "planned" || i.statusType === "proposed",
  );
}

export function InitiativesPage({ ws, tab }: { ws: string; tab: InitiativesTab }) {
  const q = useInitiatives();
  const nodes = useMemo(
    () => buildInitiativeTree(filterInitiatives(q.data ?? [], tab)),
    [q.data, tab],
  );
  let body;
  if (q.isLoading) body = <ListSkeleton />;
  else if (q.isError)
    body = (
      <ErrorState
        error={q.error}
        title="Could not load initiatives"
        onRetry={() => void q.refetch()}
      />
    );
  else if (nodes.length === 0) body = <EmptyState title="No initiatives" />;
  else
    body = (
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-left text-xs text-muted-foreground">
            <th className="px-4 py-2 font-medium">Name</th>
            <th className="px-3 py-2 font-medium">Owner</th>
            <th className="px-3 py-2 font-medium">Target</th>
            <th className="px-3 py-2 font-medium">Projects</th>
            <th className="px-3 py-2 font-medium">Health</th>
          </tr>
        </thead>
        <tbody>
          {nodes.map(({ initiative: i, depth }, n) => {
            const done = i.projects.filter((p) => p.statusType === "completed").length;
            return (
              <tr key={`${i.id}-${n}`} className="border-b border-border hover:bg-surface-hover">
                <td className="px-4 py-1.5" style={{ paddingLeft: 16 + depth * 20 }}>
                  <Link
                    to={`/${ws}/initiative/${i.identifier}/overview` as "/"}
                    className="hover:underline"
                  >
                    {i.name}
                  </Link>
                </td>
                <td className="px-3 py-1.5 text-muted-foreground">{i.owner?.name ?? "–"}</td>
                <td className="px-3 py-1.5 text-muted-foreground">{shortDate(i.targetDate)}</td>
                <td className="px-3 py-1.5 text-muted-foreground tabular-nums">
                  {done} / {i.projects.length}
                </td>
                <td className="px-3 py-1.5">
                  <HealthBadge health={i.health} />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    );
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title="Initiatives"
        docTitle="Initiatives"
        tabs={(["active", "planned", "all"] as const).map((t) => ({
          label: t[0]!.toUpperCase() + t.slice(1),
          to: `/${ws}/initiatives/${t}`,
          active: t === tab,
        }))}
      />
      <div className="min-h-0 flex-1 overflow-auto">{body}</div>
    </div>
  );
}

export function InitiativePage({ ws, initiativeRef }: { ws: string; initiativeRef: string }) {
  const q = useInitiative(initiativeRef);
  const i = q.data;
  if (q.isLoading) return <ListSkeleton rows={4} />;
  if (q.isError || !i)
    return (
      <ErrorState
        error={q.error}
        title="Could not load initiative"
        onRetry={() => void q.refetch()}
      />
    );
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader title={i.name} docTitle={i.name} actions={<HealthBadge health={i.health} />} />
      <div className="flex max-w-3xl flex-col gap-6 overflow-auto p-4">
        {i.summary && <p className="text-sm text-muted-foreground">{i.summary}</p>}
        <dl className="grid grid-cols-3 gap-4 text-sm">
          <div>
            <dt className="text-xs text-muted-foreground">Status</dt>
            <dd>{i.status}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Owner</dt>
            <dd>{i.owner?.name ?? "–"}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Target</dt>
            <dd>{shortDate(i.targetDate)}</dd>
          </div>
        </dl>
        <section aria-label="Projects">
          <h2 className="mb-2 text-xs font-medium text-muted-foreground">Projects</h2>
          {i.projects.length === 0 ? (
            <p className="text-sm text-muted-foreground">No projects.</p>
          ) : (
            <ul className="divide-y divide-border rounded-lg border border-border">
              {i.projects.map((p) => (
                <li key={p.id} className="flex items-center gap-3 px-3 py-1.5 text-sm">
                  <Link
                    to={`/${ws}/project/${p.identifier}/overview` as "/"}
                    className="hover:underline"
                  >
                    {p.name}
                  </Link>
                  <span className="text-xs text-muted-foreground">{p.status}</span>
                  <span className="ml-auto">
                    <HealthBadge health={p.health} />
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>
        <section aria-label="Sub-initiatives">
          <h2 className="mb-2 text-xs font-medium text-muted-foreground">Sub-initiatives</h2>
          {i.subInitiatives.length === 0 ? (
            <p className="text-sm text-muted-foreground">None.</p>
          ) : (
            <ul className="flex flex-col gap-1 text-sm">
              {i.subInitiatives.map((s) => (
                <li key={s.id}>{s.name}</li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}
