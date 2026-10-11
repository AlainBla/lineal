import { Link, useNavigate } from "@tanstack/react-router";
import { Copy, Link as LinkIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useActions } from "@/app/commands";
import { useSession } from "@/app/auth";
import { teamPaths } from "@/app/nav";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import type { Issue } from "@/gql/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { ActivitySection } from "./comments";
import { useIssue, useTeamMeta } from "./hooks";
import { useIssueActions, type IssueActions } from "./issue-actions";
import { useIssueLookup } from "./issue-lookup";
import { issuePath, RELATION_CHOICES, slugify, type RelationChoice } from "./model";
import { IssuePickerHost, type PickerRequest } from "./picker-host";
import { PropertySidebar } from "./property-sidebar";
import { RelationsSection } from "./relations";
import { RichEditor } from "./rich-editor";
import { ErrorState } from "./states";
import { SubIssuesSection } from "./sub-issues";
import { showToast } from "./toast-store";
import { useKnownUsers } from "./users";
import { useIssueShortcuts } from "./use-issue-shortcuts";

const AUTOSAVE_MS = 800;

type SaveStatus = "saved" | "dirty" | "saving";

function DescriptionEditor({ issue, actions }: { issue: Issue; actions: IssueActions }) {
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

function TitleField({ issue, actions }: { issue: Issue; actions: IssueActions }) {
  const [value, setValue] = useState(issue.title);
  const [syncedTitle, setSyncedTitle] = useState(issue.title);
  // Serverseitige Änderungen übernehmen, solange nicht getippt wird.
  if (issue.title !== syncedTitle) {
    setSyncedTitle(issue.title);
    setValue(issue.title);
  }
  const commit = () => {
    const t = value.trim();
    if (t === "") setValue(issue.title);
    else if (t !== issue.title) actions.setTitle(issue, t);
  };
  return (
    <input
      id="issue-title"
      aria-label="Title"
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          e.currentTarget.blur();
        } else if (e.key === "Escape") {
          setValue(issue.title);
          e.currentTarget.blur();
        }
      }}
      className="w-full bg-transparent text-xl font-semibold tracking-tight outline-none placeholder:text-subtle-foreground"
      placeholder="Issue title"
    />
  );
}

function DueDateDialog({
  open,
  current,
  onClose,
  onSave,
}: {
  open: boolean;
  current: string | null;
  onClose: () => void;
  onSave: (d: string | null) => void;
}) {
  const [value, setValue] = useState(current ?? "");
  useShortcutContext("modal", open);
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-xs">
        <form
          className="flex flex-col gap-3 p-4"
          onSubmit={(e) => {
            e.preventDefault();
            onSave(value || null);
            onClose();
          }}
        >
          <DialogTitle className="text-base font-semibold">Set due date</DialogTitle>
          <DialogDescription className="sr-only">Pick a date or clear it.</DialogDescription>
          <Input
            type="date"
            autoFocus
            aria-label="Due date"
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
          <div className="flex justify-between">
            <Button
              type="button"
              variant="ghost"
              onClick={() => {
                onSave(null);
                onClose();
              }}
            >
              Clear
            </Button>
            <Button type="submit">Save</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
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
  const parent = useIssue(issue.parentId ?? "");
  const [request, setRequest] = useState<PickerRequest | null>(null);
  const [relationDialog, setRelationDialog] = useState<RelationChoice | null>(null);
  const [addingSub, setAddingSub] = useState(false);
  const [dueOpen, setDueOpen] = useState(false);
  const closePicker = useCallback(() => setRequest(null), []);
  useShortcutContext("issue");

  useIssueShortcuts({ context: "issue", target: issue, open: setRequest, actions });
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
            {parent.data && (
              <Link
                to={issuePath(ws, parent.data.identifier) as "/"}
                className="w-fit rounded-md border border-border px-2 py-0.5 text-xs text-muted-foreground hover:bg-surface-hover"
              >
                {parent.data.identifier} {parent.data.title}
              </Link>
            )}
            <TitleField key={`title-${issue.id}`} issue={issue} actions={actions} />
            <DescriptionEditor key={`desc-${issue.id}`} issue={issue} actions={actions} />
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
          />
          {!issue.dueDate && (
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
export function IssueDetailPage({ ws, splat }: { ws: string; splat: string }) {
  const parts = splat.split("/").filter(Boolean);
  const reference = decodeURIComponent(parts[0] ?? "");
  const slug = parts[1];
  const issue = useIssue(reference);
  const navigate = useNavigate();
  const data = issue.data;

  useEffect(() => {
    if (!data) return;
    const wanted = `${issuePath(ws, data.identifier)}/${slugify(data.title)}`;
    if (parts[0] !== data.identifier || !slug) {
      void navigate({ to: wanted as "/", replace: true });
    }
    // `parts` ist aus `splat` abgeleitet.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data?.identifier, splat, ws]);

  useEffect(() => {
    if (!data) return;
    const prev = document.title;
    document.title = `${data.identifier} ${data.title} · Lineal`;
    return () => {
      document.title = prev;
    };
  }, [data]);

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
