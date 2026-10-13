import { Link } from "@tanstack/react-router";
import { useMemo } from "react";

import { Plus } from "lucide-react";

import { Button } from "@/components/ui/button";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import type { Initiative } from "@/gql/graphql";

import { openCreateInitiative } from "./create-dialogs";
import { useInitiatives } from "./queries";
import { DateText, HealthBadge, PageHeader } from "./ui";

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
                <td className="px-3 py-1.5 text-muted-foreground">
                  <DateText iso={i.targetDate} resolution={i.targetDateResolution} />
                </td>
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
        tabs={[
          ...(["active", "planned", "all"] as const).map((t) => ({
            label: t[0]!.toUpperCase() + t.slice(1),
            to: `/${ws}/initiatives/${t}`,
            active: t === tab,
          })),
          { label: "Roadmap", to: `/${ws}/roadmap`, active: false },
        ]}
        actions={
          <Button size="sm" onClick={openCreateInitiative}>
            <Plus /> New initiative
          </Button>
        }
      />
      <div className="min-h-0 flex-1 overflow-auto">{body}</div>
    </div>
  );
}
