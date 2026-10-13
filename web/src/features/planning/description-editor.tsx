/**
 * Beschreibung von Projekt und Initiative: Echtzeit-Mitbearbeitung über den Collab-Endpunkt (wie bei
 * Issues), sonst Stufe 1 mit Autosave über die reguläre Mutation.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { ConnectionStatus, Presence } from "@/features/collab/presence";
import { useCollab } from "@/features/collab/use-collab";
import { RichEditor } from "@/features/issues/rich-editor";
import { useKnownUsers } from "@/features/issues/users";

const AUTOSAVE_MS = 800;

type SaveStatus = "saved" | "dirty" | "saving";

function useMentionUsers() {
  const users = useKnownUsers();
  const session = useSession();
  return useMemo(
    () =>
      session
        ? [
            { id: session.user.id, name: session.user.name },
            ...users.filter((u) => u.id !== session.user.id),
          ]
        : users,
    [users, session],
  );
}

function Stage1Editor({
  value,
  readOnly,
  onSave,
}: {
  value: string;
  readOnly: boolean;
  onSave: (markdown: string) => Promise<unknown>;
}) {
  const mentionUsers = useMentionUsers();
  const [status, setStatus] = useState<SaveStatus>("saved");
  const [error, setError] = useState<string | null>(null);
  const pending = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const saveRef = useRef(onSave);
  useEffect(() => {
    saveRef.current = onSave;
  });

  const flush = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    const next = pending.current;
    if (next === null) return;
    pending.current = null;
    setStatus("saving");
    saveRef
      .current(next)
      .then(() => {
        setError(null);
        setStatus(pending.current === null ? "saved" : "dirty");
      })
      .catch((e: unknown) => {
        pending.current ??= next; // nicht verlieren: beim nächsten Versuch erneut senden
        setError(e instanceof Error ? e.message : "Could not save the description.");
        setStatus("dirty");
      });
  }, []);
  useEffect(() => () => flush(), [flush]);

  return (
    <div className="flex flex-col gap-1">
      <RichEditor
        value={value}
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
        {error && (
          <span role="alert" className="text-destructive">
            {error}
          </span>
        )}
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

export function PlanningDescription({
  kind,
  id,
  value,
  onSave,
}: {
  kind: "project" | "initiative";
  id: string;
  value: string;
  onSave: (markdown: string) => Promise<unknown>;
}) {
  const session = useCollab(kind, id);
  const mentionUsers = useMentionUsers();
  if (
    session.status === "connecting" ||
    (session.status === "disconnected" && !session.everConnected)
  ) {
    return <Skeleton className="h-16 w-full" />;
  }
  if (session.status === "fallback" || session.status === "closed" || !session.provider) {
    return <Stage1Editor value={value} readOnly={false} onSave={onSave} />;
  }
  return (
    <div className="flex flex-col gap-1">
      <RichEditor
        key={`collab:${session.generation}`}
        value=""
        ariaLabel="Description"
        placeholder="Add description…"
        users={mentionUsers}
        collab={{ provider: session.provider, canWrite: session.canWrite }}
      />
      <div className="flex items-center justify-end gap-2 text-xs text-subtle-foreground">
        <Presence provider={session.provider} />
        <ConnectionStatus status={session.status} canWrite={session.canWrite} />
      </div>
    </div>
  );
}
