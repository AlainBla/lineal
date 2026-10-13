import { Link, useNavigate } from "@tanstack/react-router";
import { Copy, Link as LinkIcon } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { useActions } from "@/app/commands";
import { useSession } from "@/app/auth";
import { teamPaths } from "@/app/nav";
import { Button } from "@/components/ui/button";
import { NotFound } from "@/components/not-found";
import { Skeleton } from "@/components/ui/skeleton";
import type { Issue } from "@/gql/graphql";
import { GraphQLRequestError } from "@/lib/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { ConnectionStatus, Presence } from "@/features/collab/presence";
import { useCollab } from "@/features/collab/use-collab";

import { DueDateDialog } from "./due-date-dialog";
import { ActivitySection } from "./comments";
import { useIssue, useTeamMeta } from "./hooks";
import { useIssueActions, type IssueActions } from "./issue-actions";
import { useIssueLookup } from "./issue-lookup";
import { issuePath, RELATION_CHOICES, slugify, type RelationChoice } from "./model";
import { IssuePickerHost, type PickerRequest } from "./picker-host";
import { PropertySidebar } from "./property-sidebar";
import { RelationsSection } from "./relations";
import { RichEditor } from "./rich-editor";
import { ErrorState, isNotFound } from "./states";
import { SubIssuesSection } from "./sub-issues";
import { showToast } from "./toast-store";
import { useKnownUsers } from "./users";
import { useIssueShortcuts } from "./use-issue-shortcuts";

const AUTOSAVE_MS = 800;

type SaveStatus = "saved" | "dirty" | "saving";

/**
 * Beschreibung: Echtzeit-Mitbearbeitung (Stufe 2), sobald der Collab-Endpunkt verbunden ist; sonst
 * (nicht erreichbar/beendet) Stufe 1 mit Autosave.
 */
function DescriptionEditor({
  issue,
  actions,
  readOnly = false,
}: {
  issue: Issue;
  actions: IssueActions;
  readOnly?: boolean;
}) {
  const session = useCollab("issue", issue.id);
  if (
    session.status === "connecting" ||
    (session.status === "disconnected" && !session.everConnected)
  ) {
    return <Skeleton className="h-16 w-full" />;
  }
  if (session.status === "fallback" || session.status === "closed" || !session.provider) {
    return <Stage1DescriptionEditor issue={issue} actions={actions} readOnly={readOnly} />;
  }
  return <LiveDescriptionEditor issue={issue} session={session} />;
}

function LiveDescriptionEditor({
  issue,
  session,
}: {
  issue: Issue;
  session: ReturnType<typeof useCollab>;
}) {
  void issue;
  const users = useKnownUsers();
  const me = useSession();
  const mentionUsers = useMemo(
    () =>
      me
        ? [{ id: me.user.id, name: me.user.name }, ...users.filter((u) => u.id !== me.user.id)]
        : users,
    [users, me],
  );
  const provider = session.provider!;
  return (
    <div className="flex flex-col gap-1">
      <RichEditor
        key={`collab:${session.generation}`}
        value=""
        ariaLabel="Description"
        placeholder="Add description…"
        users={mentionUsers}
        collab={{ provider, canWrite: session.canWrite }}
      />
      <div className="flex items-center justify-end gap-2 text-xs text-subtle-foreground">
        <Presence provider={provider} />
        <ConnectionStatus status={session.status} canWrite={session.canWrite} />
      </div>
    </div>
  );
}

function Stage1DescriptionEditor({
  issue,
  actions,
  readOnly = false,
}: {
  issue: Issue;
  actions: IssueActions;
  readOnly?: boolean;
}) {
  const users = useKnownUsers();
  const session = useSession();
  const [status, setStatus] = useState<SaveStatus>("saved");
  const pending = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const issueRef = useRef(issue);
  const actionsRef = useRef(actions);
  useEffect(() => {
    issueRef.current = issue;
    actionsRef.current = actions;
  });

  const flush = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    const value = pending.current;
    if (value === null) return;
    pending.current = null;
    setStatus("saving");
    actionsRef.current
      .setDescription(issueRef.current, value)
      .then(() => setStatus(pending.current === null ? "saved" : "dirty"))
      .catch(() => {
        pending.current ??= value; // nicht verlieren: beim nächsten Versuch erneut senden
        setStatus("dirty");
      });
  }, []);

  useEffect(() => () => flush(), [flush]);

  const mentionUsers = useMemo(
    () =>
      session
        ? [
            { id: session.user.id, name: session.user.name },
            ...users.filter((u) => u.id !== session.user.id),
          ]
        : users,
    [users, session],
  );

  return (
    <div className="flex flex-col gap-1">
      <RichEditor
        value={issue.description ?? ""}
        editable={!readOnly}
        ariaLabel="Description"
        placeholder="Add description…"
        users={mentionUsers}
        onChange={(md) => {
          pending.current = md;
          setStatus("dirty");
          if (timer.current) clearTimeout(timer.current);
          timer.current = setTimeout(flush, AUTOSAVE_MS);
        }}
        onBlur={flush}
        onSubmit={flush}
      />
      <div
        className="flex items-center justify-end gap-2 text-xs text-subtle-foreground"
        aria-live="polite"
      >
        {status === "saving" && "Saving…"}
        {status === "saved" && "Saved"}
        {status === "dirty" && "Unsaved changes"}
        <Button variant="ghost" size="sm" onClick={flush} disabled={status !== "dirty"}>
          Save
        </Button>
      </div>
    </div>
  );
}

function TitleField({
  issue,
  actions,
  readOnly = false,
}: {
  issue: Issue;
  actions: IssueActions;
  readOnly?: boolean;
}) {
  const [value, setValue] = useState(issue.title);
  const [syncedTitle, setSyncedTitle] = useState(issue.title);
  // Neuer Titel von außen, der überschrieben würde: Eingabe bleibt, Hinweis erscheint (B4.34).
  const [conflict, setConflict] = useState<string | null>(null);
  const ref = useRef<HTMLTextAreaElement>(null);
  if (issue.title !== syncedTitle) {
    const editing = value !== syncedTitle;
    setSyncedTitle(issue.title);
    if (editing) setConflict(issue.title);
    else setValue(issue.title);
  }
  // Mehrzeilig wachsen statt lange Titel abzuschneiden (B4.37).
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [value]);
  const commit = () => {
    const t = value.trim();
    if (t === "") setValue(issue.title);
    else if (t !== issue.title) actions.setTitle(issue, t);
    setConflict(null);
  };
  return (
    <div className="flex flex-col gap-1">
      <textarea
        ref={ref}
        id="issue-title"
        aria-label="Title"
        rows={1}
        readOnly={readOnly}
        value={value}
        onChange={(e) => setValue(e.target.value.replace(/[\r\n]+/g, " "))}
        onBlur={() => !readOnly && commit()}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            e.currentTarget.blur();
          } else if (e.key === "Escape") {
            setValue(issue.title);
            setConflict(null);
            e.currentTarget.blur();
          }
        }}
        className="w-full resize-none overflow-hidden bg-transparent text-xl font-semibold tracking-tight break-words outline-none placeholder:text-subtle-foreground"
        placeholder="Issue title"
      />
      {conflict !== null && (
        <p role="status" className="text-xs text-muted-foreground">
          The title was changed to “{conflict}” by someone else. Your edit is kept; press Escape to
          discard it.
        </p>
      )}
    </div>
  );
}

function copy(text: string, what: string) {
  void navigator.clipboard
    ?.writeText(text)
    .then(() => showToast({ message: `${what} copied` }))
    .catch(() => showToast({ message: `Could not copy ${what.toLowerCase()}`, tone: "error" }));
}

function IssueDetail({ issue, ws }: { issue: Issue; ws: string }) {
  const meta = useTeamMeta(issue.teamId);
  const metas = useMemo(() => [meta.data], [meta.data]);
  const lookup = useIssueLookup(metas);
  const actions = useIssueActions();
  const navigate = useNavigate();
  const parent = useIssue(issue.parentId ?? "");
  // Geteiltes Issue ohne Teamzugriff (Gast): nur lesen; andere Ladefehler (429, Netz): Retry (B4.16/B4.27).
  const metaCode = meta.error instanceof GraphQLRequestError ? meta.error.code : undefined;
  const readOnly = meta.isError && (metaCode === "not_found" || metaCode === "forbidden");
  const metaError = meta.isError && !readOnly;
  const parentGone =
    parent.isError &&
    parent.error instanceof GraphQLRequestError &&
    parent.error.code === "not_found";
  const [request, setRequest] = useState<PickerRequest | null>(null);
  const [relationDialog, setRelationDialog] = useState<RelationChoice | null>(null);
  const [addingSub, setAddingSub] = useState(false);
  const [dueOpen, setDueOpen] = useState(false);
  const closePicker = useCallback(() => setRequest(null), []);
  useShortcutContext("issue");

  useIssueShortcuts({
    context: "issue",
    target: readOnly || metaError ? null : issue,
    open: setRequest,
    actions,
  });
  useActions([
    {
      id: "issue.rename",
      title: "Rename issue",
      section: "Issues",
      keys: "shift+r",
      context: "issue",
      perform: () => document.getElementById("issue-title")?.focus(),
    },
    {
      id: "issue.due",
      title: "Set due date…",
      section: "Issues",
      keys: "shift+d",
      context: "issue",
      perform: () => setDueOpen(true),
    },
    {
      id: "issue.due-clear",
      title: "Remove due date",
      section: "Issues",
      keys: "mod+shift+d",
      context: "issue",
      enabled: issue.dueDate !== null,
      perform: () => actions.setDueDate(issue, null),
    },
    {
      id: "issue.sub-issue",
      title: "Add sub-issue",
      section: "Issues",
      keys: "mod+shift+o",
      context: "issue",
      perform: () => setAddingSub(true),
    },
    {
      id: "issue.focus-description",
      title: "Focus description",
      section: "Issues",
      keys: "mod+shift+i",
      context: "issue",
      enabled: !readOnly,
      perform: () => document.querySelector<HTMLElement>('[aria-label="Description"]')?.focus(),
    },
    {
      id: "issue.open-parent",
      title: "Open parent issue",
      section: "Issues",
      keys: "mod+shift+arrowup",
      context: "issue",
      enabled: !!parent.data,
      perform: () =>
        parent.data && void navigate({ to: issuePath(ws, parent.data.identifier) as "/" }),
    },
    {
      id: "issue.comment",
      title: "Comment on issue",
      section: "Issues",
      keys: "mod+m",
      context: "issue",
      perform: () => document.querySelector<HTMLElement>('[aria-label="Leave a comment"]')?.focus(),
    },
    ...RELATION_CHOICES.map((c) => ({
      id: `issue.relation.${c.id}`,
      title: `Mark as ${c.label.toLowerCase()}…`,
      section: "Issues",
      keys: c.keys,
      context: "issue" as const,
      perform: () => setRelationDialog(c.id),
    })),
  ]);

  const teamKey = meta.data?.team.key;
  const identifierUrl = `${window.location.origin}${issuePath(ws, issue.identifier)}`;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-4 text-sm">
        <nav
          aria-label="Issue breadcrumb"
          className="flex items-center gap-1.5 text-muted-foreground"
        >
          {teamKey ? (
            <Link to={teamPaths(ws, teamKey).all as "/"} className="hover:text-foreground">
              {meta.data?.team.name}
            </Link>
          ) : meta.isError ? (
            <span>{issue.identifier.split("-")[0]}</span>
          ) : (
            <Skeleton className="h-3 w-16" />
          )}
          <span aria-hidden>›</span>
          <span className="text-foreground">{issue.identifier}</span>
        </nav>
        <div className="ml-auto flex items-center gap-1">
          <Button variant="ghost" size="sm" onClick={() => copy(issue.identifier, "Issue ID")}>
            <Copy /> Copy ID
          </Button>
          <Button variant="ghost" size="sm" onClick={() => copy(identifierUrl, "Issue URL")}>
            <LinkIcon /> Copy URL
          </Button>
        </div>
      </header>
      <div className="flex min-h-0 flex-1 overflow-hidden">
        <article
          className="min-w-0 flex-1 overflow-y-auto"
          aria-label={`Issue ${issue.identifier}`}
        >
          <div className="mx-auto flex max-w-3xl flex-col gap-6 px-6 py-6">
            {parentGone && (
              <span className="w-fit rounded-md border border-border px-2 py-0.5 text-xs text-muted-foreground">
                Parent issue was deleted
              </span>
            )}
            {parent.data && (
              <Link
                to={issuePath(ws, parent.data.identifier) as "/"}
                className="w-fit rounded-md border border-border px-2 py-0.5 text-xs text-muted-foreground hover:bg-surface-hover"
              >
                {parent.data.identifier} {parent.data.title}
              </Link>
            )}
            <TitleField
              key={`title-${issue.id}`}
              issue={issue}
              actions={actions}
              readOnly={readOnly}
            />
            <DescriptionEditor
              key={`desc-${issue.id}`}
              issue={issue}
              actions={actions}
              readOnly={readOnly}
            />
            <SubIssuesSection
              issue={issue}
              ws={ws}
              lookup={lookup}
              adding={addingSub}
              onAddingChange={setAddingSub}
            />
            <RelationsSection
              issue={issue}
              ws={ws}
              dialog={relationDialog}
              onDialog={setRelationDialog}
            />
            <ActivitySection issue={issue} lookup={lookup} />
          </div>
        </article>
        <aside
          aria-label="Issue details"
          className="hidden w-72 shrink-0 overflow-y-auto border-l border-border p-3 md:block"
        >
          <PropertySidebar
            issue={issue}
            meta={meta.data}
            lookup={lookup}
            ws={ws}
            onOpen={(kind) => setRequest({ kind, issue })}
            onEditDueDate={() => setDueOpen(true)}
            readOnly={readOnly}
            metaError={metaError}
            onRetryMeta={() => void meta.refetch()}
          />
          {!issue.dueDate && !readOnly && !metaError && (
            <Button
              variant="ghost"
              size="sm"
              className="mt-1 ml-[5.5rem]"
              onClick={() => setDueOpen(true)}
            >
              Set due date
            </Button>
          )}
        </aside>
      </div>
      <IssuePickerHost request={request} onClose={closePicker} />
      <DueDateDialog
        key={`${issue.id}:${dueOpen}`}
        open={dueOpen}
        current={issue.dueDate}
        onClose={() => setDueOpen(false)}
        onSave={(d) => actions.setDueDate(issue, d)}
      />
    </div>
  );
}

/** Issue-Detail unter `/{ws}/issue/{KEY-n}[/{slug}]` (docs/08 §1.3); alte Kennungen leiten um. */
export function IssueDetailPage({
  ws,
  splat,
  embedded = false,
}: {
  ws: string;
  splat: string;
  /** Eingebettet (z. B. Inbox-Split): keine URL-Normalisierung, kein Titelwechsel (B4.01). */
  embedded?: boolean;
}) {
  const parts = splat.split("/").filter(Boolean);
  const reference = decodeURIComponent(parts[0] ?? "");
  const slug = parts[1];
  const issue = useIssue(reference);
  const navigate = useNavigate();
  // Nach einem Fehler (z. B. Freigabe entzogen) behält TanStack Query die alten Daten; sie gelten nicht mehr.
  const data = issue.isError ? undefined : issue.data;

  useEffect(() => {
    if (!data || embedded) return;
    const wanted = `${issuePath(ws, data.identifier)}/${slugify(data.title)}`;
    if (parts[0] !== data.identifier || !slug) {
      void navigate({ to: wanted as "/", replace: true });
    }
    // `parts` ist aus `splat` abgeleitet.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data?.identifier, splat, ws]);

  useEffect(() => {
    if (!data || embedded) return;
    const prev = document.title;
    document.title = `${data.identifier} ${data.title} · Lineal`;
    return () => {
      document.title = prev;
    };
  }, [data]);

  if (reference === "") {
    return <NotFound title="Issue not found" description="No issue was specified in this URL." />;
  }
  if (issue.isLoading) {
    return (
      <div role="status" aria-label="Loading issue" className="flex flex-col gap-4 p-6">
        <Skeleton className="h-7 w-2/3" />
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-4 w-1/3" />
      </div>
    );
  }
  if (issue.isError || !data) {
    if (isNotFound(issue.error)) {
      return (
        <NotFound
          title={issue.data ? "This issue was deleted" : "Issue not found"}
          description="It does not exist, was deleted, or you do not have access to it."
        />
      );
    }
    return (
      <ErrorState
        title="Could not load the issue"
        error={issue.error}
        onRetry={() => void issue.refetch()}
      />
    );
  }
  return <IssueDetail issue={data} ws={ws} />;
}
