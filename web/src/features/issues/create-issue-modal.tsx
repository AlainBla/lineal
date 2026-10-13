import { useRouterState } from "@tanstack/react-router";
import { CalendarDays, FileText, GitBranch, X } from "lucide-react";
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
  addSavedDraft,
  clearCreateDraft,
  clearCreatePreset,
  draftScope,
  getCreateDraft,
  hasCreateDraft,
  saveCreateDraft,
  useCreatePreset,
  type CreatePreset,
} from "./create-store";
import { useCreateIssue, useIssue, useTeamMeta, useTeams } from "./hooks";
import { Avatar, LabelDot, PriorityIcon, StatusIcon } from "./icons";
import { estimateLabel, priorityLabel, toggleLabel } from "./model";
import { PropertyPicker } from "./picker";
import {
  assigneeOptions,
  cycleOptions,
  estimatePickerOptions,
  labelOptions,
  milestoneOptions,
  NONE,
  priorityOptions,
  projectOptions,
  statusOptions,
  teamOptions,
} from "./property-options";
import { DueDateDialog } from "./due-date-dialog";
import { ParentSearchDialog } from "./parent-dialog";
import { RichEditor } from "./rich-editor";
import { TemplateForm } from "./template-form";
import {
  formFields,
  missingRequired,
  templatePrefill,
  toFormAnswers,
  useIssueTemplates,
  useTeamDefaultTemplates,
  type FormAnswerValue,
} from "./templates";
import { uploadImageAsset } from "./assets";
import { errorMessage, showToast } from "./toast-store";
import { useAssignableUsers, useKnownUsers } from "./users";

type ChipKind =
  | "team"
  | "template"
  | "status"
  | "priority"
  | "assignee"
  | "labels"
  | "estimate"
  | "project"
  | "milestone"
  | "cycle";

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
  const [labelIdsTouched, setLabelIdsTouched] = useState(false);
  const [projectId, setProjectId] = useState<string | null | undefined>(undefined);
  const [milestoneId, setMilestoneId] = useState<string | null>(null);
  const [cycleId, setCycleId] = useState<string | null | undefined>(undefined);
  const [estimate, setEstimate] = useState<number | null>(null);
  const [dueDate, setDueDate] = useState<string | null>(null);
  const [parentId, setParentId] = useState<string | null | undefined>(undefined);
  const [templateChoice, setTemplateChoice] = useState<string | null | undefined>(undefined);
  const [answers, setAnswers] = useState<Record<string, FormAnswerValue>>({});
  const [descriptionTouched, setDescriptionTouched] = useState(initialDraft !== null);
  const [templateChosen, setTemplateChosen] = useState(false);
  const [dueOpen, setDueOpen] = useState(false);
  const [parentOpen, setParentOpen] = useState(false);
  const [createMore, setCreateMore] = useState(false);
  // Alt+C öffnet direkt die Template-Auswahl.
  const [picker, setPicker] = useState<ChipKind | null>(preset?.pickTemplate ? "template" : null);
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
  const templates = useIssueTemplates(team?.id);
  const defaults = useTeamDefaultTemplates(team?.id);
  const templateList = useMemo(() => templates.data ?? [], [templates.data]);
  // Default-Template des Teams (Mitglied / Nicht-Mitglied) ist vorausgewählt, abwählbar (docs/06 §2.8).
  const isMember = !!team && !!session?.teams.some((t) => t.id === team.id);
  const defaultTemplateId = isMember ? defaults.data?.forMembersId : defaults.data?.forNonMembersId;
  const template =
    templateList.find(
      (t) => t.id === (templateChoice === undefined ? defaultTemplateId : templateChoice),
    ) ?? null;
  const fields = useMemo(() => (template?.isForm ? formFields(template) : []), [template]);
  const prefill = useMemo(
    () => (template && !template.isForm ? templatePrefill(template) : {}),
    [template],
  );
  const hasTitleField = fields.some((f) => f.kind === "title");
  const [invalidFields, setInvalidFields] = useState<string[]>([]);
  // Nur Personen mit Teamzugang als Assignee anbieten (B4.33).
  const assignable = useAssignableUsers(team?.id);
  const presetApplies = chosenTeamId === null; // Kontextwerte gelten nur fürs Vorgabeteam
  const m = meta.data;
  const states = m?.states ?? [];
  const defaultState = states.find((s) => s.isDefault);
  const explicitStateId = stateId ?? (presetApplies ? preset?.stateId : undefined);
  // Nicht-Mitglied in einem Team mit Triage: das Issue landet in „Triage“ (Anzeige, nicht gesendet).
  const triageApplies = !!m?.team.triageEnabled && !isMember;
  const triageState = states.find((s) => s.type === "triage");
  const effectiveStateId =
    explicitStateId ??
    prefill.stateId ??
    (triageApplies ? triageState?.id : undefined) ??
    defaultState?.id;
  const state = states.find((s) => s.id === effectiveStateId);
  const effectivePriority =
    priority ?? (presetApplies ? preset?.priority : undefined) ?? prefill.priority ?? 0;
  const effectiveAssignee =
    assigneeId !== undefined
      ? assigneeId
      : presetApplies
        ? (preset?.assigneeId ?? prefill.assigneeId ?? null)
        : (prefill.assigneeId ?? null);
  const effectiveProject =
    projectId !== undefined
      ? projectId
      : presetApplies
        ? (preset?.projectId ?? prefill.projectId ?? null)
        : (prefill.projectId ?? null);
  const effectiveCycle =
    cycleId !== undefined ? cycleId : presetApplies ? (preset?.cycleId ?? null) : null;
  const effectiveLabels = labelIdsTouched ? labelIds : (prefill.labelIds ?? labelIds);
  const effectiveEstimate = estimate ?? prefill.estimate ?? null;
  const effectiveParent = parentId !== undefined ? parentId : (preset?.parentId ?? null);
  const parentIssue = useIssue(effectiveParent ?? "");
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

  const titleMissing =
    title.trim() === "" && !(template && !template.isForm && prefill.title) && !hasTitleField;
  const canSubmit = !!team && !titleMissing && !create.isPending;

  const changeTeam = (id: string) => {
    if (id === team?.id) return;
    // Werte des alten Teams passen nicht zum neuen: leeren (docs/08 F3).
    setChosenTeamId(id);
    setStateId(null);
    setLabelIds([]);
    setLabelIdsTouched(false);
    setTemplateChoice(undefined);
    setAnswers({});
    setParentId(undefined);
    setProjectId(null);
    setMilestoneId(null);
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
    if (template?.isForm) {
      const missing = missingRequired(fields, answers);
      setInvalidFields(missing);
      if (missing.length > 0) {
        setError("Fill in the required fields.");
        return;
      }
    }
    setError(null);
    create.mutate(
      {
        teamId: team.id,
        ...(title.trim() ? { title: title.trim() } : {}),
        // Ohne Bearbeitung gilt die Beschreibung des Templates (der Server entfernt Platzhalter).
        ...(description.trim() && (!template || descriptionTouched) && !template?.isForm
          ? { description }
          : {}),
        ...(template ? { templateId: template.id } : {}),
        ...(template?.isForm ? { formAnswers: toFormAnswers(fields, answers) } : {}),
        // Status nur senden, wenn er ausdrücklich gewählt ist: ein Default würde die Triage-Regel
        // (Nicht-Mitglied → Triage, docs/03 §10.2) und den Template-Status überstimmen.
        ...(explicitStateId ? { stateId: explicitStateId } : {}),
        ...(effectivePriority ? { priority: effectivePriority } : {}),
        ...(effectiveAssignee ? { assigneeId: effectiveAssignee } : {}),
        ...(effectiveLabels.length > 0 ? { labelIds: effectiveLabels } : {}),
        ...(effectiveProject ? { projectId: effectiveProject } : {}),
        ...(effectiveProject && milestoneId ? { projectMilestoneId: milestoneId } : {}),
        ...(effectiveCycle ? { cycleId: effectiveCycle } : {}),
        ...(effectiveEstimate !== null ? { estimate: effectiveEstimate } : {}),
        ...(dueDate ? { dueDate } : {}),
        ...(effectiveParent ? { parentId: effectiveParent } : {}),
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
        {(templateList.length > 0 || template) && (
          <Chip
            label={`Template: ${template?.name ?? "none"}`}
            onClick={() => setPicker("template")}
            muted={!template}
          >
            <FileText className="size-3.5" />
            {template?.name ?? "Template"}
          </Chip>
        )}
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
        {!hasTitleField && (
          <input
            ref={titleRef}
            autoFocus
            aria-label="Issue title"
            aria-invalid={submitted && titleMissing}
            aria-describedby={error ? "create-issue-error" : undefined}
            placeholder={prefill.title ?? "Issue title"}
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
        )}
        {!template?.isForm && (
          <RichEditor
            key={`${nonce}:${template?.id ?? ""}`}
            value={
              nonce === 0 && !templateChosen
                ? (initialDraft?.description ?? prefill.description ?? "")
                : (prefill.description ?? "")
            }
            ariaLabel="Description"
            placeholder="Add description…"
            users={[...(me ? [me] : []), ...users.filter((u) => u.id !== me?.id)]}
            onChange={(md) => {
              setDescription(md);
              setDescriptionTouched(true);
            }}
            onSubmit={() => submit()}
            uploadImage={uploadImageAsset}
            onUploadError={(e) =>
              showToast({ message: errorMessage(e, "Could not upload the image."), tone: "error" })
            }
            className="max-h-60 min-h-20 overflow-y-auto"
          />
        )}
      </div>
      {template?.isForm && (
        <TemplateForm
          fields={fields}
          answers={answers}
          labels={labels}
          invalid={invalidFields}
          onChange={(id, v) => {
            setAnswers((a) => ({ ...a, [id]: v }));
            setInvalidFields((f) => f.filter((x) => x !== id));
          }}
        />
      )}
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
          <Chip
            label="Estimate"
            onClick={() => setPicker("estimate")}
            muted={effectiveEstimate === null}
          >
            {estimateLabel(estimationType, effectiveEstimate) ?? "Estimate"}
          </Chip>
        )}
        <Chip
          label="Labels"
          onClick={() => setPicker("labels")}
          muted={effectiveLabels.length === 0}
        >
          {effectiveLabels.length === 0 ? (
            "Labels"
          ) : (
            <>
              {effectiveLabels.slice(0, 2).map((id) => {
                const l = labels.find((x) => x.id === id);
                return l ? <LabelDot key={id} color={l.color} /> : null;
              })}
              {effectiveLabels.length} label{effectiveLabels.length === 1 ? "" : "s"}
            </>
          )}
        </Chip>
        <Chip label="Project" onClick={() => setPicker("project")} muted={!effectiveProject}>
          {m?.projects.find((p) => p.id === effectiveProject)?.name ?? "Project"}
        </Chip>
        {effectiveProject && (
          <Chip label="Milestone" onClick={() => setPicker("milestone")} muted={!milestoneId}>
            {m?.projects
              .find((p) => p.id === effectiveProject)
              ?.milestones?.find((ms) => ms.id === milestoneId)?.name ?? "Milestone"}
          </Chip>
        )}
        {m?.team.cyclesEnabled && (
          <Chip label="Cycle" onClick={() => setPicker("cycle")} muted={!effectiveCycle}>
            {(() => {
              const c = m.cycles.find((x) => x.id === effectiveCycle);
              return c ? `Cycle ${c.number}` : "Cycle";
            })()}
          </Chip>
        )}
        <Chip
          label={`Due date: ${dueDate ?? "none"}`}
          onClick={() => setDueOpen(true)}
          muted={!dueDate}
        >
          <CalendarDays className="size-3.5" />
          {dueDate ?? "Due date"}
        </Chip>
        <Chip
          label={`Parent: ${parentIssue.data?.identifier ?? "none"}`}
          onClick={() => setParentOpen(true)}
          muted={!effectiveParent}
        >
          <GitBranch className="size-3.5" />
          {effectiveParent ? (parentIssue.data?.identifier ?? "Parent") : "Parent"}
        </Chip>
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
        {...pickerProps("template")}
        title="Choose template…"
        options={[
          { id: NONE, label: "No template" },
          ...templateList.map((t) => ({
            id: t.id,
            label: t.name,
            hint: t.isForm ? "Form" : (t.teamKey ?? "Workspace"),
            keywords: t.description ? [t.description] : [],
          })),
        ]}
        selected={template?.id ?? NONE}
        onSelect={(id) => {
          setTemplateChoice(id === NONE ? null : id);
          setTemplateChosen(true);
          setAnswers({});
          setInvalidFields([]);
          if (id === NONE) setDescriptionTouched(false);
        }}
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
        selected={effectiveEstimate === null ? NONE : String(effectiveEstimate)}
        onSelect={(id) => setEstimate(id === NONE ? null : Number(id))}
      />
      <PropertyPicker
        {...pickerProps("labels")}
        multi
        title="Change labels…"
        options={labelOptions(labels)}
        selected={effectiveLabels}
        onSelect={(id) => {
          setLabelIdsTouched(true);
          setLabelIds(toggleLabel(effectiveLabels, id, labels));
        }}
      />
      <PropertyPicker
        {...pickerProps("project")}
        title="Add to project…"
        options={projectOptions(m?.projects ?? [])}
        selected={effectiveProject ?? NONE}
        onSelect={(id) => {
          setProjectId(id === NONE ? null : id);
          // Meilensteine gehören zu genau einem Projekt.
          setMilestoneId(null);
        }}
      />
      <PropertyPicker
        {...pickerProps("milestone")}
        title="Set milestone…"
        options={milestoneOptions(m?.projects ?? [], effectiveProject)}
        selected={milestoneId ?? NONE}
        onSelect={(id) => setMilestoneId(id === NONE ? null : id)}
      />
      <PropertyPicker
        {...pickerProps("cycle")}
        title="Add to cycle…"
        options={cycleOptions(m?.cycles ?? [])}
        selected={effectiveCycle ?? NONE}
        onSelect={(id) => setCycleId(id === NONE ? null : id)}
      />
      {dueOpen && (
        <DueDateDialog
          open
          current={dueDate}
          onClose={() => setDueOpen(false)}
          onSave={setDueDate}
        />
      )}
      {parentOpen && (
        <ParentSearchDialog
          currentParentId={effectiveParent}
          title="Set parent issue"
          onChoose={setParentId}
          onClose={() => setParentOpen(false)}
        />
      )}
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
              variant="outline"
              size="sm"
              onClick={() => {
                const d = scope ? getCreateDraft(scope) : null;
                if (scope && d) {
                  addSavedDraft(scope, d);
                  clearCreateDraft(scope);
                  showToast({
                    message: "Draft saved",
                    href: `/${session?.workspace.urlKey ?? ""}/drafts`,
                  });
                }
                close();
              }}
            >
              Save draft
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
