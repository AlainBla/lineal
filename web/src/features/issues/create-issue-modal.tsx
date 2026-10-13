import { useRouterState } from "@tanstack/react-router";
import { X } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { useSession } from "@/app/auth";
import { teamKeyFromPath } from "@/app/nav";
import { setUiState, useUiState } from "@/app/ui-state";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Keys } from "@/components/ui/kbd";
import { cn } from "@/lib/cn";
import { useShortcutContext } from "@/lib/shortcuts";

import {
  clearCreateDraft,
  clearCreatePreset,
  draftScope,
  getCreateDraft,
  hasCreateDraft,
  saveCreateDraft,
  useCreatePreset,
  type CreatePreset,
} from "./create-store";
import { useCreateIssue, useTeamMeta, useTeams } from "./hooks";
import { Avatar, LabelDot, PriorityIcon, StatusIcon } from "./icons";
import { estimateLabel, priorityLabel, toggleLabel } from "./model";
import { PropertyPicker } from "./picker";
import {
  assigneeOptions,
  cycleOptions,
  estimatePickerOptions,
  labelOptions,
  NONE,
  priorityOptions,
  projectOptions,
  statusOptions,
  teamOptions,
} from "./property-options";
import { RichEditor } from "./rich-editor";
import { errorMessage, showToast } from "./toast-store";
import { useAssignableUsers, useKnownUsers } from "./users";

type ChipKind =
  "team" | "status" | "priority" | "assignee" | "labels" | "estimate" | "project" | "cycle";

function Chip({
  label,
  onClick,
  children,
  muted,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
  muted?: boolean;
}) {
  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      aria-label={label}
      onClick={onClick}
      className={cn("h-6 gap-1.5 px-2", muted && "text-muted-foreground")}
    >
      {children}
    </Button>
  );
}

interface FormProps {
  preset: CreatePreset | null;
  /** Schließt das Modal endgültig (nach erfolgreichem Anlegen). */
  onClose: () => void;
  /** Schließen durch den Nutzer (X): fragt bei ungespeichertem Inhalt nach. */
  onRequestClose: () => void;
}

function CreateIssueForm({ preset, onClose, onRequestClose }: FormProps) {
  const session = useSession();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const teams = useTeams();
  const users = useKnownUsers();
  const create = useCreateIssue();
  const [chosenTeamId, setChosenTeamId] = useState<string | null>(null);
  const scope = session ? draftScope(session.user.id, session.workspace.id) : null;
  const [initialDraft] = useState(() => (scope ? getCreateDraft(scope) : null));
  const [title, setTitle] = useState(initialDraft?.title ?? "");
  const [description, setDescription] = useState(initialDraft?.description ?? "");
  const [nonce, setNonce] = useState(0);
  const [stateId, setStateId] = useState<string | null>(null);
  const [priority, setPriority] = useState<number | null>(null);
  const [assigneeId, setAssigneeId] = useState<string | null | undefined>(undefined);
  const [labelIds, setLabelIds] = useState<string[]>([]);
  const [projectId, setProjectId] = useState<string | null | undefined>(undefined);
  const [cycleId, setCycleId] = useState<string | null | undefined>(undefined);
  const [estimate, setEstimate] = useState<number | null>(null);
  const [createMore, setCreateMore] = useState(false);
  const [picker, setPicker] = useState<ChipKind | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [submitted, setSubmitted] = useState(false);
  const titleRef = useRef<HTMLInputElement>(null);

  const teamList = useMemo(() => teams.data ?? [], [teams.data]);
  const defaultKey = preset?.teamKey ?? teamKeyFromPath(pathname) ?? session?.teams[0]?.key;
  const team =
    teamList.find((t) => t.id === chosenTeamId) ??
    teamList.find((t) => t.key === defaultKey) ??
    teamList[0];
  const meta = useTeamMeta(team?.id);
  // Nur Personen mit Teamzugang als Assignee anbieten (B4.33).
  const assignable = useAssignableUsers(team?.id);
  const presetApplies = chosenTeamId === null; // Kontextwerte gelten nur fürs Vorgabeteam
  const m = meta.data;
  const states = m?.states ?? [];
  const defaultState = states.find((s) => s.isDefault);
  const effectiveStateId =
    stateId ?? (presetApplies ? preset?.stateId : undefined) ?? defaultState?.id;
  const state = states.find((s) => s.id === effectiveStateId);
  const effectivePriority = priority ?? (presetApplies ? preset?.priority : undefined) ?? 0;
  const effectiveAssignee =
    assigneeId !== undefined ? assigneeId : presetApplies ? (preset?.assigneeId ?? null) : null;
  const effectiveProject =
    projectId !== undefined ? projectId : presetApplies ? (preset?.projectId ?? null) : null;
  const effectiveCycle =
    cycleId !== undefined ? cycleId : presetApplies ? (preset?.cycleId ?? null) : null;
  const estimationType = m?.team.issueEstimationType ?? "notUsed";
  const me = session ? { id: session.user.id, name: session.user.name } : null;
  const nameOf = (id: string | null) =>
    id === null
      ? null
      : id === me?.id
        ? me.name
        : (users.find((u) => u.id === id)?.name ?? "Assigned");

  // Entwurf mitschreiben: übersteht Session-Ablauf (Login-Rücksprung) und Reload.
  useEffect(() => {
    if (scope) saveCreateDraft(scope, { title, description });
  }, [scope, title, description]);

  const titleMissing = title.trim() === "";
  const canSubmit = !!team && !titleMissing && !create.isPending;

  const changeTeam = (id: string) => {
    if (id === team?.id) return;
    // Werte des alten Teams passen nicht zum neuen: leeren (docs/08 F3).
    setChosenTeamId(id);
    setStateId(null);
    setLabelIds([]);
    setProjectId(null);
    setCycleId(null);
    setEstimate(null);
    setAssigneeId(effectiveAssignee);
  };

  const submit = (keepOpen = createMore) => {
    setSubmitted(true);
    if (titleMissing) {
      setError("Title is required.");
      titleRef.current?.focus();
      return;
    }
    if (!team || create.isPending) return;
    setError(null);
    create.mutate(
      {
        teamId: team.id,
        title: title.trim(),
        ...(description.trim() ? { description } : {}),
        ...(effectiveStateId ? { stateId: effectiveStateId } : {}),
        ...(effectivePriority ? { priority: effectivePriority } : {}),
        ...(effectiveAssignee ? { assigneeId: effectiveAssignee } : {}),
        ...(labelIds.length > 0 ? { labelIds } : {}),
        ...(effectiveProject ? { projectId: effectiveProject } : {}),
        ...(effectiveCycle ? { cycleId: effectiveCycle } : {}),
        ...(estimate !== null ? { estimate } : {}),
        ...(preset?.parentId ? { parentId: preset.parentId } : {}),
      },
      {
        onSuccess: (issue) => {
          if (scope) clearCreateDraft(scope);
          showToast({
            message: `${issue.identifier} created`,
            href: `/${session?.workspace.urlKey ?? ""}/issue/${issue.identifier}`,
          });
          if (keepOpen) {
            setTitle("");
            setDescription("");
            setNonce((n) => n + 1);
            setSubmitted(false);
            titleRef.current?.focus();
          } else {
            onClose();
          }
        },
        onError: (e) => setError(errorMessage(e, "Could not create the issue.")),
      },
    );
  };

  const pickerProps = (kind: ChipKind) => ({
    open: picker === kind,
    onOpenChange: (o: boolean) => !o && setPicker(null),
  });

  const labels = m?.labels ?? [];

  return (
    <form
      className="flex flex-col"
      noValidate
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
          e.preventDefault();
          submit(e.shiftKey ? true : createMore);
        }
      }}
    >
      <div className="flex items-center gap-2 px-4 pt-3">
        <Chip label={`Team: ${team?.key ?? "none"}`} onClick={() => setPicker("team")}>
          <span className="font-medium">{team?.key ?? "Set team"}</span>
        </Chip>
        <span aria-hidden className="text-muted-foreground">
          ›
        </span>
        <DialogTitle className="text-sm font-normal text-muted-foreground">New issue</DialogTitle>
        <div className="ml-auto flex items-center gap-1">
          <Button
            type="button"
            variant="ghost"
            size="icon"
            aria-label="Close"
            onClick={onRequestClose}
          >
            <X className="size-3.5" />
          </Button>
        </div>
      </div>
      <DialogDescription className="sr-only">
        Create an issue. Press Control or Command plus Enter to submit.
      </DialogDescription>
      <div className="flex flex-col gap-2 px-4 pt-3">
        <input
          ref={titleRef}
          autoFocus
          aria-label="Issue title"
          aria-invalid={submitted && titleMissing}
          aria-describedby={error ? "create-issue-error" : undefined}
          placeholder="Issue title"
          value={title}
          onChange={(e) => {
            setTitle(e.target.value);
            if (error && e.target.value.trim() !== "") setError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.ctrlKey && !e.metaKey) {
              e.preventDefault(); // Enter im Titel springt in die Beschreibung
              document
                .querySelector<HTMLElement>('[role="dialog"] [aria-label="Description"]')
                ?.focus();
            }
          }}
          className="w-full bg-transparent text-lg font-medium outline-none placeholder:text-subtle-foreground"
        />
        <RichEditor
          key={nonce}
          value={nonce === 0 ? (initialDraft?.description ?? "") : ""}
          ariaLabel="Description"
          placeholder="Add description…"
          users={[...(me ? [me] : []), ...users.filter((u) => u.id !== me?.id)]}
          onChange={setDescription}
          onSubmit={() => submit()}
          className="max-h-60 min-h-20 overflow-y-auto"
        />
      </div>
      <div className="flex flex-wrap items-center gap-1.5 px-4 pt-3 pb-3" aria-label="Properties">
        <Chip label={`Status: ${state?.name ?? "default"}`} onClick={() => setPicker("status")}>
          <StatusIcon type={state?.type ?? "backlog"} color={state?.color} />
          {state?.name ?? "Status"}
        </Chip>
        <Chip
          label={`Priority: ${priorityLabel(effectivePriority)}`}
          onClick={() => setPicker("priority")}
        >
          <PriorityIcon priority={effectivePriority} />
          {effectivePriority === 0 ? "Priority" : priorityLabel(effectivePriority)}
        </Chip>
        <Chip
          label={`Assignee: ${nameOf(effectiveAssignee) ?? "none"}`}
          onClick={() => setPicker("assignee")}
          muted={!effectiveAssignee}
        >
          <Avatar name={nameOf(effectiveAssignee) ?? ""} />
          {nameOf(effectiveAssignee) ?? "Assignee"}
        </Chip>
        {estimationType !== "notUsed" && (
          <Chip label="Estimate" onClick={() => setPicker("estimate")} muted={estimate === null}>
            {estimateLabel(estimationType, estimate) ?? "Estimate"}
          </Chip>
        )}
        <Chip label="Labels" onClick={() => setPicker("labels")} muted={labelIds.length === 0}>
          {labelIds.length === 0 ? (
            "Labels"
          ) : (
            <>
              {labelIds.slice(0, 2).map((id) => {
                const l = labels.find((x) => x.id === id);
                return l ? <LabelDot key={id} color={l.color} /> : null;
              })}
              {labelIds.length} label{labelIds.length === 1 ? "" : "s"}
            </>
          )}
        </Chip>
        <Chip label="Project" onClick={() => setPicker("project")} muted={!effectiveProject}>
          {m?.projects.find((p) => p.id === effectiveProject)?.name ?? "Project"}
        </Chip>
        {m?.team.cyclesEnabled && (
          <Chip label="Cycle" onClick={() => setPicker("cycle")} muted={!effectiveCycle}>
            {(() => {
              const c = m.cycles.find((x) => x.id === effectiveCycle);
              return c ? `Cycle ${c.number}` : "Cycle";
            })()}
          </Chip>
        )}
      </div>
      {error && (
        <p id="create-issue-error" role="alert" className="px-4 pb-2 text-xs text-destructive">
          {error}
        </p>
      )}
      <div className="flex items-center gap-3 border-t border-border px-4 py-2.5">
        <label className="flex cursor-default items-center gap-2 text-sm text-muted-foreground">
          <input
            type="checkbox"
            role="switch"
            aria-label="Create more"
            checked={createMore}
            onChange={(e) => setCreateMore(e.target.checked)}
          />
          Create more
        </label>
        <Button type="submit" size="sm" className="ml-auto" disabled={!canSubmit}>
          Create issue
          <Keys keys="mod+enter" className="opacity-70" />
        </Button>
      </div>

      <PropertyPicker
        {...pickerProps("team")}
        title="Set team…"
        options={teamOptions(teamList)}
        selected={team?.id}
        onSelect={changeTeam}
      />
      <PropertyPicker
        {...pickerProps("status")}
        title="Change status…"
        options={statusOptions(states)}
        selected={effectiveStateId}
        onSelect={setStateId}
      />
      <PropertyPicker
        {...pickerProps("priority")}
        title="Set priority…"
        options={priorityOptions()}
        selected={String(effectivePriority)}
        onSelect={(id) => setPriority(Number(id))}
      />
      <PropertyPicker
        {...pickerProps("assignee")}
        title="Assign to…"
        options={assigneeOptions(assignable, me)}
        selected={effectiveAssignee ?? NONE}
        onSelect={(id) => setAssigneeId(id === NONE ? null : id)}
      />
      <PropertyPicker
        {...pickerProps("estimate")}
        title="Set estimate…"
        options={estimatePickerOptions(estimationType)}
        selected={estimate === null ? NONE : String(estimate)}
        onSelect={(id) => setEstimate(id === NONE ? null : Number(id))}
      />
      <PropertyPicker
        {...pickerProps("labels")}
        multi
        title="Change labels…"
        options={labelOptions(labels)}
        selected={labelIds}
        onSelect={(id) => setLabelIds((prev) => toggleLabel(prev, id, labels))}
      />
      <PropertyPicker
        {...pickerProps("project")}
        title="Add to project…"
        options={projectOptions(m?.projects ?? [])}
        selected={effectiveProject ?? NONE}
        onSelect={(id) => setProjectId(id === NONE ? null : id)}
      />
      <PropertyPicker
        {...pickerProps("cycle")}
        title="Add to cycle…"
        options={cycleOptions(m?.cycles ?? [])}
        selected={effectiveCycle ?? NONE}
        onSelect={(id) => setCycleId(id === NONE ? null : id)}
      />
    </form>
  );
}

/** Create-Issue-Modal (docs/08 §2.3); öffnet über `createIssueOpen` (`c`, Stift, „+“ in Gruppen). */
export function CreateIssueModal() {
  const { createIssueOpen } = useUiState();
  const preset = useCreatePreset();
  const session = useSession();
  const scope = session ? draftScope(session.user.id, session.workspace.id) : null;
  const [confirming, setConfirming] = useState(false);
  useShortcutContext("modal", createIssueOpen);

  const close = () => {
    setConfirming(false);
    setUiState({ createIssueOpen: false });
    clearCreatePreset();
  };
  // Esc, Backdrop-Klick und X: bei ungespeichertem Inhalt erst nachfragen (B4.26).
  const requestClose = () => {
    if (scope && hasCreateDraft(scope)) setConfirming(true);
    else close();
  };

  return (
    <Dialog open={createIssueOpen} onOpenChange={(o) => !o && requestClose()}>
      <DialogContent className="max-w-2xl">
        {createIssueOpen && (
          <CreateIssueForm preset={preset} onClose={close} onRequestClose={requestClose} />
        )}
        {createIssueOpen && confirming && (
          <div
            role="alertdialog"
            aria-label="Discard this issue?"
            className="flex items-center gap-3 border-t border-border bg-surface px-4 py-2.5 text-sm"
          >
            <span className="mr-auto">
              Discard this issue? Your title and description are lost.
            </span>
            <Button type="button" variant="outline" size="sm" onClick={() => setConfirming(false)}>
              Keep editing
            </Button>
            <Button
              type="button"
              variant="destructive"
              size="sm"
              onClick={() => {
                if (scope) clearCreateDraft(scope);
                close();
              }}
            >
              Discard
            </Button>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
