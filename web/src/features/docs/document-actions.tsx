/** Aktionen am geöffneten Dokument (F.05): abonnieren, Icon, verschieben, löschen/wiederherstellen. */
import { useNavigate } from "@tanstack/react-router";
import { Bell, BellOff, Settings2, Trash2, Undo2 } from "lucide-react";
import { useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import type { Document } from "@/gql/graphql";

import {
  type DocScope,
  useDocumentSubscription,
  useMoveDocument,
  useSetDocumentIcon,
  useTrashDocument,
} from "./list-queries";

export function SubscribeButton({ documentId }: { documentId: string }) {
  const { subscribed, loading, toggle } = useDocumentSubscription(documentId);
  return (
    <Button
      variant="ghost"
      size="sm"
      disabled={loading || toggle.isPending}
      aria-pressed={subscribed}
      aria-label={subscribed ? "Unsubscribe from this document" : "Subscribe to this document"}
      onClick={() => toggle.mutate(!subscribed)}
    >
      {subscribed ? <BellOff /> : <Bell />} {subscribed ? "Unsubscribe" : "Subscribe"}
    </Button>
  );
}

type ParentKind = "project" | "initiative" | "team" | "issue" | "cycle";

const PARENT_LABEL: Record<ParentKind, string> = {
  project: "Project",
  initiative: "Initiative",
  team: "Team",
  issue: "Issue",
  cycle: "Cycle",
};

const PARENT_HINT: Record<ParentKind, string> = {
  project: "Project name, identifier or slug",
  initiative: "Initiative name or slug",
  team: "Team key (e.g. ENG)",
  issue: "Issue identifier (e.g. ENG-42)",
  cycle: "Cycle number or name",
};

/** Baut die Elternangabe für `documentUpdate` (genau ein Elternparameter). */
export function parentInput(kind: ParentKind, ref: string, teamKey: string): DocScope {
  switch (kind) {
    case "project":
      return { projectId: ref };
    case "initiative":
      return { initiativeId: ref };
    case "team":
      return { teamId: ref };
    case "issue":
      return { issueId: ref };
    case "cycle":
      return { cycleId: ref, ...(teamKey ? { teamId: teamKey } : {}) };
  }
}

/** Panel mit Icon und „Move to…“; die Rechteprüfung für das neue Elternobjekt macht der Server. */
export function DocumentSettingsPanel({ doc, docRef }: { doc: Document; docRef: string }) {
  const session = useSession();
  const move = useMoveDocument(docRef);
  const setIcon = useSetDocumentIcon(docRef);
  const [kind, setKind] = useState<ParentKind>("project");
  const [ref, setRef] = useState("");
  const [teamKey, setTeamKey] = useState("");
  const [icon, setIconText] = useState(doc.icon ?? "");
  return (
    <section
      aria-label="Document settings"
      className="flex flex-col gap-3 border-b border-border px-4 py-3 text-sm"
    >
      <form
        aria-label="Document icon"
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          setIcon.mutate({ id: doc.id, icon: icon.trim() || null });
        }}
      >
        <label className="w-20 text-xs text-muted-foreground" htmlFor="doc-icon">
          Icon
        </label>
        <Input
          id="doc-icon"
          value={icon}
          onChange={(e) => setIconText(e.target.value)}
          placeholder="Icon name"
          className="h-8 max-w-48"
          maxLength={64}
        />
        <Button type="submit" size="sm" variant="ghost" disabled={setIcon.isPending}>
          Save icon
        </Button>
        {setIcon.isError && (
          <span role="alert" className="text-xs text-destructive">
            {errorText(setIcon.error)}
          </span>
        )}
      </form>
      <form
        aria-label="Move document"
        className="flex flex-wrap items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (!ref.trim()) return;
          move.mutate({ id: doc.id, parent: parentInput(kind, ref.trim(), teamKey.trim()) });
        }}
      >
        <label className="w-20 text-xs text-muted-foreground" htmlFor="doc-move-kind">
          Move to
        </label>
        <select
          id="doc-move-kind"
          aria-label="New parent type"
          value={kind}
          onChange={(e) => setKind(e.target.value as ParentKind)}
          className="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
        >
          {(Object.keys(PARENT_LABEL) as ParentKind[]).map((k) => (
            <option key={k} value={k}>
              {PARENT_LABEL[k]}
            </option>
          ))}
        </select>
        {kind === "team" ? (
          <select
            aria-label="New parent"
            value={ref}
            onChange={(e) => setRef(e.target.value)}
            className="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
          >
            <option value="">Choose team…</option>
            {(session?.teams ?? []).map((t) => (
              <option key={t.id} value={t.key}>
                {t.name} ({t.key})
              </option>
            ))}
          </select>
        ) : (
          <Input
            aria-label="New parent"
            value={ref}
            onChange={(e) => setRef(e.target.value)}
            placeholder={PARENT_HINT[kind]}
            className="h-8 max-w-64"
          />
        )}
        {kind === "cycle" && (
          <Input
            aria-label="Cycle team"
            value={teamKey}
            onChange={(e) => setTeamKey(e.target.value)}
            placeholder="Team key"
            className="h-8 w-28"
          />
        )}
        <Button type="submit" size="sm" disabled={!ref.trim() || move.isPending}>
          Move
        </Button>
        {move.isError && (
          <span role="alert" className="text-xs text-destructive">
            {errorText(move.error)}
          </span>
        )}
        {move.isSuccess && <span className="text-xs text-muted-foreground">Moved.</span>}
      </form>
    </section>
  );
}

export function SettingsToggle({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  return (
    <Button variant="ghost" size="sm" aria-pressed={open} onClick={onToggle}>
      <Settings2 /> Settings
    </Button>
  );
}

/** Löschen (Papierkorb) bzw. – im gelöschten Zustand – Wiederherstellen. */
export function TrashButton({ doc }: { doc: Document }) {
  const navigate = useNavigate();
  const session = useSession();
  const trash = useTrashDocument();
  if (doc.trashed) {
    return (
      <div
        role="status"
        className="flex items-center gap-2 border-b border-border bg-surface px-4 py-2 text-sm"
      >
        This document is in Recently deleted.
        <Button
          size="sm"
          disabled={trash.isPending}
          onClick={() => trash.mutate({ id: doc.id, restore: true })}
        >
          <Undo2 /> Restore
        </Button>
        {trash.isError && (
          <span role="alert" className="text-xs text-destructive">
            {errorText(trash.error)}
          </span>
        )}
      </div>
    );
  }
  return (
    <Button
      variant="ghost"
      size="sm"
      disabled={trash.isPending}
      onClick={() => {
        if (!window.confirm(`Move "${doc.title}" to Recently deleted?`)) return;
        trash.mutate(
          { id: doc.id, restore: false },
          {
            onSuccess: () =>
              void navigate({
                to: `/${session?.workspace.urlKey ?? ""}/my-issues/assigned` as "/",
              }),
          },
        );
      }}
    >
      <Trash2 /> Delete
    </Button>
  );
}
