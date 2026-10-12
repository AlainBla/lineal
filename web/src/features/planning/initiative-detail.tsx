import { Link } from "@tanstack/react-router";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { relativeTime } from "@/features/issues/model";
import { RichEditor } from "@/features/issues/rich-editor";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import type { Initiative } from "@/gql/graphql";

import {
  HEALTH_LABEL,
  useInitiative,
  useInitiativeDocuments,
  useInitiativeUpdates,
  usePostInitiativeUpdate,
  type Health,
} from "./queries";
import { RoadmapPage } from "./roadmap-page";
import { HealthBadge, PageHeader, shortDate } from "./ui";

export type InitiativeTab =
  "overview" | "projects" | "updates" | "documents" | "sub-initiatives" | "roadmap";
export const INITIATIVE_TABS: InitiativeTab[] = [
  "overview",
  "projects",
  "updates",
  "documents",
  "sub-initiatives",
  "roadmap",
];
export const isInitiativeTab = (v: string): v is InitiativeTab =>
  (INITIATIVE_TABS as string[]).includes(v);

const TAB_LABEL: Record<InitiativeTab, string> = {
  overview: "Overview",
  projects: "Projects",
  updates: "Updates",
  documents: "Documents",
  "sub-initiatives": "Sub-initiatives",
  roadmap: "Roadmap",
};

function Overview({ i }: { i: Initiative }) {
  const r = i.healthRollup;
  return (
    <div className="flex max-w-3xl flex-col gap-6 overflow-auto p-4">
      {i.summary && <p className="text-sm text-muted-foreground">{i.summary}</p>}
      <dl className="grid grid-cols-2 gap-4 text-sm sm:grid-cols-4">
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
        <div>
          <dt className="text-xs text-muted-foreground">Lead team</dt>
          <dd>{i.leadTeam?.name ?? "–"}</dd>
        </div>
      </dl>
      <section aria-label="Project health">
        <h2 className="mb-2 text-xs font-medium text-muted-foreground">Project health</h2>
        <p className="text-sm tabular-nums">
          {r.onTrack} on track · {r.atRisk} at risk · {r.offTrack} off track · {r.none} no update
        </p>
      </section>
      {i.description && (
        <section aria-label="Description">
          <h2 className="mb-2 text-xs font-medium text-muted-foreground">Description</h2>
          <RichEditor key={i.id} value={i.description} editable={false} ariaLabel="Description" />
        </section>
      )}
    </div>
  );
}

function Projects({ ws, i }: { ws: string; i: Initiative }) {
  if (i.projects.length === 0) return <EmptyState title="No projects" />;
  return (
    <ul className="m-4 max-w-3xl divide-y divide-border rounded-lg border border-border">
      {i.projects.map((p) => (
        <li key={p.id} className="flex items-center gap-3 px-3 py-1.5 text-sm">
          <Link to={`/${ws}/project/${p.identifier}/overview` as "/"} className="hover:underline">
            {p.name}
          </Link>
          <span className="text-xs text-muted-foreground">{p.status}</span>
          <span className="ml-auto">
            <HealthBadge health={p.health} />
          </span>
        </li>
      ))}
    </ul>
  );
}

function UpdateComposer({ initiativeId }: { initiativeId: string }) {
  const [body, setBody] = useState("");
  const [health, setHealth] = useState<Health>("onTrack");
  const post = usePostInitiativeUpdate(initiativeId);
  return (
    <form
      aria-label="Post update"
      className="flex flex-col gap-2 rounded-lg border border-border p-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!body.trim()) return;
        post.mutate({ body: body.trim(), health }, { onSuccess: () => setBody("") });
      }}
    >
      <div role="radiogroup" aria-label="Health" className="flex gap-1">
        {(Object.keys(HEALTH_LABEL) as Health[]).map((h) => (
          <Button
            key={h}
            type="button"
            variant="ghost"
            size="sm"
            role="radio"
            aria-checked={health === h}
            className={health === h ? "bg-surface-active" : undefined}
            onClick={() => setHealth(h)}
          >
            {HEALTH_LABEL[h]}
          </Button>
        ))}
      </div>
      <textarea
        aria-label="Update text"
        value={body}
        onChange={(e) => setBody(e.target.value)}
        rows={3}
        placeholder="Write an initiative update…"
        className="w-full resize-y rounded-md border border-input bg-transparent p-2 text-sm outline-none focus-visible:border-ring"
      />
      {post.isError && (
        <p role="alert" className="text-xs text-destructive">
          Could not post update.
        </p>
      )}
      <div className="flex justify-end">
        <Button type="submit" size="sm" disabled={!body.trim() || post.isPending}>
          Post update
        </Button>
      </div>
    </form>
  );
}

function Updates({ initiativeId }: { initiativeId: string }) {
  const q = useInitiativeUpdates(initiativeId);
  return (
    <div className="flex max-w-3xl flex-col gap-4 p-4">
      <UpdateComposer initiativeId={initiativeId} />
      {q.isLoading ? (
        <ListSkeleton rows={3} />
      ) : q.isError ? (
        <ErrorState error={q.error} title="Could not load updates" />
      ) : !q.data?.length ? (
        <p className="text-sm text-muted-foreground">No updates yet.</p>
      ) : (
        <ul className="flex flex-col gap-3">
          {q.data.map((u) => (
            <li key={u.id} className="rounded-lg border border-border p-3">
              <div className="mb-1 flex items-center gap-2 text-xs text-muted-foreground">
                <HealthBadge health={u.health} />
                <span>{u.user?.name ?? "Someone"}</span>
                <span>{relativeTime(u.createdAt)}</span>
              </div>
              <RichEditor key={u.id} value={u.body} editable={false} ariaLabel="Update" />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Documents({ ws, initiativeId }: { ws: string; initiativeId: string }) {
  const q = useInitiativeDocuments(initiativeId);
  if (q.isLoading) return <ListSkeleton rows={3} />;
  if (q.isError) return <ErrorState error={q.error} title="Could not load documents" />;
  if (!q.data?.length) return <EmptyState title="No documents" />;
  return (
    <ul className="m-4 max-w-3xl divide-y divide-border rounded-lg border border-border">
      {q.data.map((d) => (
        <li key={d.id} className="flex items-center gap-3 px-3 py-1.5 text-sm">
          <Link to={`/${ws}/document/${d.slugId}` as "/"} className="hover:underline">
            {d.title}
          </Link>
          <span className="ml-auto text-xs text-muted-foreground">{relativeTime(d.updatedAt)}</span>
        </li>
      ))}
    </ul>
  );
}

function SubInitiatives({ ws, i }: { ws: string; i: Initiative }) {
  if (i.subInitiatives.length === 0) return <EmptyState title="No sub-initiatives" />;
  return (
    <ul className="m-4 max-w-3xl divide-y divide-border rounded-lg border border-border">
      {i.subInitiatives.map((s) => (
        <li key={s.id} className="px-3 py-1.5 text-sm">
          <Link to={`/${ws}/initiative/${s.id}/overview` as "/"} className="hover:underline">
            {s.name}
          </Link>
        </li>
      ))}
    </ul>
  );
}

/** Initiative mit Tabs (docs/04 §4): Übersicht, Projekte, Updates, Dokumente, Sub-Initiativen, Roadmap. */
export function InitiativeDetail({
  ws,
  initiativeRef,
  tab,
}: {
  ws: string;
  initiativeRef: string;
  tab: InitiativeTab;
}) {
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
  const base = `/${ws}/initiative/${initiativeRef}`;
  let body;
  switch (tab) {
    case "overview":
      body = <Overview i={i} />;
      break;
    case "projects":
      body = <Projects ws={ws} i={i} />;
      break;
    case "updates":
      body = <Updates initiativeId={i.id} />;
      break;
    case "documents":
      body = <Documents ws={ws} initiativeId={i.id} />;
      break;
    case "sub-initiatives":
      body = <SubInitiatives ws={ws} i={i} />;
      break;
    case "roadmap":
      body = <RoadmapPage ws={ws} scope={{ kind: "initiative", ref: initiativeRef }} embedded />;
      break;
  }
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title={i.name}
        docTitle={i.name}
        tabs={INITIATIVE_TABS.map((t) => ({
          label: TAB_LABEL[t],
          to: `${base}/${t}`,
          active: t === tab,
        }))}
        actions={<HealthBadge health={i.health} />}
      />
      <div className="flex min-h-0 flex-1 flex-col overflow-auto">{body}</div>
    </div>
  );
}
