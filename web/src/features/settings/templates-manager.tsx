/**
 * Template-Verwaltung (docs/06 §2) für Team-Settings und Workspace-Settings: Liste je Typ, anlegen,
 * bearbeiten, löschen und – nur im Team – die Default-Templates. Geerbte Team-Templates und
 * Workspace-Templates sind im Team schreibgeschützt (bearbeitbar an ihrem Ursprung).
 * Der Editor kennt Sub-Issues (Titel, Priorität, Fälligkeit), `dueIn`, bei Projekten Meilensteine,
 * Projekt-Labels, Lead und Mitglieder; weitere Felder bleiben in `templateData` erhalten.
 */
import { Plus } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import { STATE_TYPE_LABEL, STATE_TYPE_ORDER } from "@/features/issues/model";
import { RichEditor } from "@/features/issues/rich-editor";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import type { IssueLabel, Team, Template, WorkflowState } from "@/gql/graphql";

import {
  useTeamLabels,
  useTeamMembers,
  useWorkflowStates,
  useWorkspaceMembers,
  type Member,
} from "./queries";
import { useProjectLabels } from "./team-extra-queries";
import { relativeDays, relativeDaysToIso, relativeDaysValid } from "./team-extra-model";
import {
  useTemplateDefaults,
  useTemplateMutations,
  useTemplates,
  useWorkspaceLabels,
  type DefaultSlot,
  type TemplateKind,
} from "./template-queries";
import { Field, Section, selectClass } from "./ui";

const KINDS: Array<{ kind: TemplateKind; title: string; noun: string }> = [
  { kind: "issue", title: "Issue templates", noun: "issue" },
  { kind: "project", title: "Project templates", noun: "project" },
  { kind: "document", title: "Document templates", noun: "document" },
];

const PRIORITIES = ["No priority", "Urgent", "High", "Medium", "Low"];
const PROJECT_STATUS_TYPES = ["backlog", "planned", "started", "paused", "completed", "canceled"];

type Data = Record<string, unknown>;

/** Setzt einen Wert in `templateData`; leere Werte entfernen den Schlüssel. */
function put(data: Data, key: string, value: unknown): void {
  if (
    value === undefined ||
    value === null ||
    value === "" ||
    (Array.isArray(value) && !value.length)
  )
    delete data[key];
  else data[key] = value;
}

const str = (v: unknown) => (typeof v === "string" ? v : "");

/** Ein Sub-Issue im Editor; `raw` behält Felder, die der Editor nicht kennt (Beschreibung, Kinder …). */
interface SubRow {
  raw: Data;
  title: string;
  priority: string;
  dueDays: string;
  initialDueDays: string;
}

function subRowsOf(v: unknown): SubRow[] {
  if (!Array.isArray(v)) return [];
  return v
    .filter((x): x is Data => typeof x === "object" && x !== null)
    .map((raw) => ({
      raw,
      title: str(raw.title),
      priority: raw.priority === undefined ? "" : String(raw.priority),
      dueDays: relativeDays(raw.dueIn),
      initialDueDays: relativeDays(raw.dueIn),
    }));
}

/** Ein Meilenstein im Projekt-Template. */
interface MilestoneRow {
  raw: Data;
  name: string;
  targetDays: string;
  initialTargetDays: string;
}

function milestoneRowsOf(v: unknown): MilestoneRow[] {
  if (!Array.isArray(v)) return [];
  return v
    .filter((x): x is Data => typeof x === "object" && x !== null)
    .map((raw) => ({
      raw,
      name: str(raw.name),
      targetDays: relativeDays(raw.targetIn),
      initialTargetDays: relativeDays(raw.targetIn),
    }));
}

/** Setzt eine relative Frist nur, wenn der Nutzer sie geändert hat (sonst bleibt der Rohwert). */
function putRelative(d: Data, key: string, days: string, initial: string) {
  if (days === initial) return;
  put(d, key, relativeDaysToIso(days));
}

function TemplateEditor({
  kind,
  template,
  team,
  labels,
  states,
  members,
  projectMembers,
  onDone,
}: {
  kind: TemplateKind;
  template: Template | null;
  team?: Team;
  labels: IssueLabel[];
  states: WorkflowState[];
  members: Member[];
  projectMembers: Member[];
  onDone: () => void;
}) {
  const projectLabels = useProjectLabels(kind === "project");
  const m = useTemplateMutations();
  const base = (template?.templateData ?? {}) as Data;
  const [name, setName] = useState(template?.name ?? "");
  const [description, setDescription] = useState(template?.description ?? "");
  // templateData-Felder
  const [title, setTitle] = useState(str(kind === "project" ? base.name : base.title));
  const [summary, setSummary] = useState(str(base.summary));
  const [body, setBody] = useState(str(kind === "document" ? base.content : base.description));
  const [priority, setPriority] = useState(
    base.priority === undefined ? "" : String(base.priority),
  );
  const [stateRef, setStateRef] = useState(str(base.stateId) || str(base.stateType));
  const [statusType, setStatusType] = useState(str(base.statusType));
  const [labelIds, setLabelIds] = useState<string[]>(
    Array.isArray(base.labelIds) ? (base.labelIds as string[]) : [],
  );
  const [assignee, setAssignee] = useState(str(base.assigneeId));
  const [estimate, setEstimate] = useState(
    typeof base.estimate === "number" ? String(base.estimate) : "",
  );
  const [dueDays, setDueDays] = useState(relativeDays(base.dueIn));
  const [subs, setSubs] = useState<SubRow[]>(() => subRowsOf(base.subIssues));
  // Projekt: Lead, Mitglieder, Projekt-Labels, Meilensteine
  const [lead, setLead] = useState(str(base.leadId));
  const [memberIds, setMemberIds] = useState<string[]>(
    Array.isArray(base.memberIds) ? (base.memberIds as string[]) : [],
  );
  const [projectLabelIds, setProjectLabelIds] = useState<string[]>(
    kind === "project" && Array.isArray(base.labelIds) ? (base.labelIds as string[]) : [],
  );
  const [milestones, setMilestones] = useState<MilestoneRow[]>(() =>
    milestoneRowsOf(base.milestones),
  );

  function build(): Data {
    const d: Data = { ...base };
    if (kind === "issue") {
      put(d, "title", title.trim() ? title : undefined); // Präfixe wie „Bug: “ behalten ihr Leerzeichen
      put(d, "description", body.trimEnd());
      put(d, "priority", priority === "" ? undefined : Number(priority));
      if (stateRef !== (str(base.stateId) || str(base.stateType))) {
        delete d.stateId;
        delete d.stateType;
        if (team && states.some((s) => s.id === stateRef)) d.stateId = stateRef;
        else put(d, "stateType", stateRef);
      }
      put(d, "labelIds", labelIds);
      put(d, "assigneeId", assignee);
      put(d, "estimate", estimate.trim() === "" ? undefined : Number(estimate));
      if (dueDays !== relativeDays(base.dueIn)) {
        put(d, "dueIn", relativeDaysToIso(dueDays));
        if (dueDays !== "") delete d.dueDate; // dueIn und dueDate schließen sich aus
      }
      put(
        d,
        "subIssues",
        subs
          .filter((r) => r.title.trim() !== "")
          .map((r) => {
            const o: Data = { ...r.raw, title: r.title.trim() };
            put(o, "priority", r.priority === "" ? undefined : Number(r.priority));
            putRelative(o, "dueIn", r.dueDays, r.initialDueDays);
            return o;
          }),
      );
    } else if (kind === "project") {
      put(d, "name", title.trim());
      put(d, "summary", summary.trim());
      put(d, "description", body.trimEnd());
      put(d, "priority", priority === "" ? undefined : Number(priority));
      if (statusType !== str(base.statusType)) {
        delete d.statusId;
        put(d, "statusType", statusType);
      }
      put(d, "leadId", lead);
      put(d, "memberIds", memberIds);
      put(d, "labelIds", projectLabelIds);
      put(
        d,
        "milestones",
        milestones
          .filter((r) => r.name.trim() !== "")
          .map((r) => {
            const o: Data = { ...r.raw, name: r.name.trim() };
            putRelative(o, "targetIn", r.targetDays, r.initialTargetDays);
            return o;
          }),
      );
    } else {
      put(d, "title", title.trim() ? title : undefined); // Präfixe wie „Bug: “ behalten ihr Leerzeichen
      put(d, "content", body.trimEnd());
    }
    return d;
  }

  const estimateBad = estimate.trim() !== "" && !Number.isFinite(Number(estimate));
  const relativeBad =
    !relativeDaysValid(dueDays) ||
    subs.some((r) => !relativeDaysValid(r.dueDays)) ||
    milestones.some((r) => !relativeDaysValid(r.targetDays));
  const valid = name.trim() !== "" && !estimateBad && !relativeBad;
  const error = m.create.error ?? m.update.error;
  const toggleIn = (set: (f: (c: string[]) => string[]) => void, id: string) =>
    set((cur) => (cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id]));
  const toggleLabel = (id: string) => toggleIn(setLabelIds, id);
  const patchSub = (i: number, patch: Partial<SubRow>) =>
    setSubs((cur) => cur.map((r, k) => (k === i ? { ...r, ...patch } : r)));
  const patchMilestone = (i: number, patch: Partial<MilestoneRow>) =>
    setMilestones((cur) => cur.map((r, k) => (k === i ? { ...r, ...patch } : r)));

  return (
    <form
      aria-label={template ? `Edit template ${template.name}` : `New ${kind} template`}
      className="flex flex-col gap-3 rounded-lg border border-border p-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!valid) return;
        const data = build();
        const done = { onSuccess: onDone };
        if (template)
          m.update.mutate(
            {
              id: template.id,
              input: { name: name.trim(), description, templateData: data },
            },
            done,
          );
        else
          m.create.mutate(
            {
              type: kind,
              name: name.trim(),
              ...(description ? { description } : {}),
              ...(team ? { teamId: team.id } : {}),
              templateData: data,
            },
            done,
          );
      }}
    >
      <Field label="Template name">
        <Input aria-label="Template name" value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field label="Description (shown in the template picker)">
        <Input
          aria-label="Template description"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
        />
      </Field>
      <Field label={kind === "project" ? "Project name" : "Title"}>
        <Input aria-label="Title" value={title} onChange={(e) => setTitle(e.target.value)} />
      </Field>
      {kind === "project" && (
        <Field label="Summary">
          <Input
            aria-label="Summary"
            value={summary}
            onChange={(e) => setSummary(e.target.value)}
          />
        </Field>
      )}
      <Field label={kind === "document" ? "Content" : "Description"}>
        <RichEditor
          value={body}
          ariaLabel="Body"
          placeholder={kind === "document" ? "Write the content…" : "Add description…"}
          onChange={setBody}
          className="max-h-72 min-h-24 overflow-y-auto rounded-md border border-input px-2 py-1.5"
        />
      </Field>
      {kind !== "document" && (
        <Field label="Priority">
          <select
            aria-label="Priority"
            className={selectClass}
            value={priority}
            onChange={(e) => setPriority(e.target.value)}
          >
            <option value="">Not preset</option>
            {PRIORITIES.map((p, i) => (
              <option key={p} value={i}>
                {p}
              </option>
            ))}
          </select>
        </Field>
      )}
      {kind === "issue" && (
        <>
          <Field label="Status">
            <select
              aria-label="Status"
              className={selectClass}
              value={stateRef}
              onChange={(e) => setStateRef(e.target.value)}
            >
              <option value="">Not preset</option>
              {team
                ? states.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.name}
                    </option>
                  ))
                : STATE_TYPE_ORDER.filter((t) => t !== "duplicate").map((t) => (
                    <option key={t} value={t}>
                      {STATE_TYPE_LABEL[t]}
                    </option>
                  ))}
            </select>
          </Field>
          <Field label="Assignee">
            <select
              aria-label="Assignee"
              className={selectClass}
              value={assignee}
              onChange={(e) => setAssignee(e.target.value)}
            >
              <option value="">Not preset</option>
              <option value="creator">Creator of the issue</option>
              {members.map((u) => (
                <option key={u.id} value={u.id}>
                  {u.displayName || u.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Estimate">
            <Input
              aria-label="Estimate"
              inputMode="decimal"
              value={estimate}
              onChange={(e) => setEstimate(e.target.value)}
              className="w-24"
            />
          </Field>
          <Field label="Due in (days after creation)">
            <Input
              aria-label="Due in days"
              inputMode="numeric"
              className="w-24"
              value={dueDays}
              onChange={(e) => setDueDays(e.target.value)}
            />
          </Field>
          <fieldset className="flex flex-col gap-2 text-sm">
            <legend className="text-xs text-muted-foreground">Sub-issues</legend>
            {subs.map((r, i) => (
              <div key={i} className="flex flex-wrap items-center gap-2">
                <Input
                  aria-label={`Sub-issue ${i + 1} title`}
                  className="min-w-40 flex-1"
                  value={r.title}
                  onChange={(e) => patchSub(i, { title: e.target.value })}
                />
                <select
                  aria-label={`Sub-issue ${i + 1} priority`}
                  className={selectClass}
                  value={r.priority}
                  onChange={(e) => patchSub(i, { priority: e.target.value })}
                >
                  <option value="">No priority preset</option>
                  {PRIORITIES.map((p, k) => (
                    <option key={p} value={k}>
                      {p}
                    </option>
                  ))}
                </select>
                <Input
                  aria-label={`Sub-issue ${i + 1} due in days`}
                  inputMode="numeric"
                  placeholder="Due in (days)"
                  className="w-32"
                  value={r.dueDays}
                  onChange={(e) => patchSub(i, { dueDays: e.target.value })}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  aria-label={`Remove sub-issue ${i + 1}`}
                  onClick={() => setSubs((cur) => cur.filter((_, k) => k !== i))}
                >
                  Remove
                </Button>
              </div>
            ))}
            <div>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() =>
                  setSubs((cur) => [
                    ...cur,
                    { raw: {}, title: "", priority: "", dueDays: "", initialDueDays: "" },
                  ])
                }
              >
                <Plus /> Add sub-issue
              </Button>
            </div>
          </fieldset>
          <fieldset className="flex flex-col gap-1 text-sm">
            <legend className="text-xs text-muted-foreground">Labels</legend>
            {labels.length === 0 && <span className="text-muted-foreground">No labels yet.</span>}
            <div className="flex flex-wrap gap-x-4 gap-y-1">
              {labels.map((l) => (
                <label key={l.id} className="flex items-center gap-1">
                  <input
                    type="checkbox"
                    aria-label={`Label ${l.name}`}
                    checked={labelIds.includes(l.id)}
                    onChange={() => toggleLabel(l.id)}
                  />
                  {l.name}
                </label>
              ))}
            </div>
          </fieldset>
        </>
      )}
      {kind === "project" && (
        <Field label="Status category">
          <select
            aria-label="Status category"
            className={selectClass}
            value={statusType}
            onChange={(e) => setStatusType(e.target.value)}
          >
            <option value="">Not preset</option>
            {PROJECT_STATUS_TYPES.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        </Field>
      )}
      {kind === "project" && (
        <>
          <Field label="Lead">
            <select
              aria-label="Lead"
              className={selectClass}
              value={lead}
              onChange={(e) => setLead(e.target.value)}
            >
              <option value="">Not preset</option>
              <option value="creator">Creator of the project</option>
              {projectMembers.map((u) => (
                <option key={u.id} value={u.id}>
                  {u.displayName || u.name}
                </option>
              ))}
            </select>
          </Field>
          <fieldset className="flex flex-col gap-1 text-sm">
            <legend className="text-xs text-muted-foreground">Members</legend>
            <div className="flex flex-wrap gap-x-4 gap-y-1">
              {projectMembers.map((u) => (
                <label key={u.id} className="flex items-center gap-1">
                  <input
                    type="checkbox"
                    aria-label={`Member ${u.displayName || u.name}`}
                    checked={memberIds.includes(u.id)}
                    onChange={() => toggleIn(setMemberIds, u.id)}
                  />
                  {u.displayName || u.name}
                </label>
              ))}
            </div>
          </fieldset>
          <fieldset className="flex flex-col gap-1 text-sm">
            <legend className="text-xs text-muted-foreground">Project labels</legend>
            {projectLabels.isError && (
              <span role="alert" className="text-xs text-destructive">
                {errorText(projectLabels.error)}
              </span>
            )}
            {projectLabels.data?.length === 0 && (
              <span className="text-muted-foreground">No project labels yet.</span>
            )}
            <div className="flex flex-wrap gap-x-4 gap-y-1">
              {(projectLabels.data ?? [])
                .filter((l) => !l.isGroup && !l.retiredAt)
                .map((l) => (
                  <label key={l.id} className="flex items-center gap-1">
                    <input
                      type="checkbox"
                      aria-label={`Project label ${l.name}`}
                      checked={projectLabelIds.includes(l.id)}
                      onChange={() => toggleIn(setProjectLabelIds, l.id)}
                    />
                    {l.name}
                  </label>
                ))}
            </div>
          </fieldset>
          <fieldset className="flex flex-col gap-2 text-sm">
            <legend className="text-xs text-muted-foreground">Milestones</legend>
            {milestones.map((r, i) => (
              <div key={i} className="flex flex-wrap items-center gap-2">
                <Input
                  aria-label={`Milestone ${i + 1} name`}
                  className="min-w-40 flex-1"
                  value={r.name}
                  onChange={(e) => patchMilestone(i, { name: e.target.value })}
                />
                <Input
                  aria-label={`Milestone ${i + 1} target in days`}
                  inputMode="numeric"
                  placeholder="Target in (days)"
                  className="w-36"
                  value={r.targetDays}
                  onChange={(e) => patchMilestone(i, { targetDays: e.target.value })}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  aria-label={`Remove milestone ${i + 1}`}
                  onClick={() => setMilestones((cur) => cur.filter((_, k) => k !== i))}
                >
                  Remove
                </Button>
              </div>
            ))}
            <div>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() =>
                  setMilestones((cur) => [
                    ...cur,
                    { raw: {}, name: "", targetDays: "", initialTargetDays: "" },
                  ])
                }
              >
                <Plus /> Add milestone
              </Button>
            </div>
          </fieldset>
        </>
      )}
      <div className="flex items-center gap-3">
        <Button
          type="submit"
          size="sm"
          disabled={!valid || m.create.isPending || m.update.isPending}
        >
          Save template
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onDone}>
          Cancel
        </Button>
        {error != null && (
          <span role="alert" className="text-xs text-destructive">
            {errorText(error)}
          </span>
        )}
      </div>
    </form>
  );
}

function originLabel(t: Template, team?: Team): string | null {
  if (!team) return null;
  if (!t.teamId) return "Workspace template";
  return t.teamId === team.id ? null : `Inherited from ${t.teamKey ?? "parent team"}`;
}

function TemplateRow({
  t,
  team,
  defaultFor,
  onEdit,
}: {
  t: Template;
  team?: Team;
  defaultFor: string[];
  onEdit: () => void;
}) {
  const m = useTemplateMutations();
  const [confirm, setConfirm] = useState(false);
  const origin = originLabel(t, team);
  return (
    <li className="flex items-center gap-2 px-3 py-1.5 text-sm">
      <span className="flex-1">
        {t.name}
        {origin && <span className="ml-2 text-xs text-muted-foreground">{origin}</span>}
        {defaultFor.map((d) => (
          <span key={d} className="ml-2 text-xs text-muted-foreground">
            Default {d}
          </span>
        ))}
        {t.description && (
          <span className="block text-xs text-muted-foreground">{t.description}</span>
        )}
      </span>
      {!origin &&
        (confirm ? (
          <>
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Confirm delete ${t.name}`}
              disabled={m.remove.isPending}
              onClick={() => m.remove.mutate(t.id, { onSettled: () => setConfirm(false) })}
            >
              Confirm delete
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setConfirm(false)}>
              Cancel
            </Button>
          </>
        ) : (
          <>
            <Button variant="ghost" size="sm" aria-label={`Edit ${t.name}`} onClick={onEdit}>
              Edit
            </Button>
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Delete ${t.name}`}
              onClick={() => setConfirm(true)}
            >
              Delete
            </Button>
          </>
        ))}
      {m.remove.isError && (
        <span role="alert" className="text-xs text-destructive">
          {errorText(m.remove.error)}
        </span>
      )}
    </li>
  );
}

function DefaultsForm({ team, templates }: { team: Team; templates: Template[] }) {
  const q = useTemplateDefaults(team.id);
  const m = useTemplateMutations();
  const slots: Array<{ slot: DefaultSlot; label: string; kind: TemplateKind; id: string | null }> =
    [
      {
        slot: "forMembers",
        label: "Default issue template for team members",
        kind: "issue",
        id: q.data?.forMembersId ?? null,
      },
      {
        slot: "forNonMembers",
        label: "Default issue template for non-members",
        kind: "issue",
        id: q.data?.forNonMembersId ?? null,
      },
      {
        slot: "project",
        label: "Default project template",
        kind: "project",
        id: q.data?.projectId ?? null,
      },
    ];
  return (
    <div className="flex flex-col gap-2">
      <h3 className="text-sm font-medium">Default templates</h3>
      {q.isError && <ErrorState error={q.error} title="Could not load default templates" />}
      {q.data &&
        slots.map((s) => (
          <Field key={s.slot} label={s.label}>
            <select
              aria-label={s.label}
              className={selectClass}
              value={s.id ?? ""}
              disabled={m.setDefault.isPending}
              onChange={(e) =>
                m.setDefault.mutate({
                  teamId: team.id,
                  slot: s.slot,
                  templateId: e.target.value || null,
                })
              }
            >
              <option value="">None</option>
              {templates
                .filter((t) => t.type === s.kind)
                .map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.name}
                  </option>
                ))}
            </select>
          </Field>
        ))}
      {m.setDefault.isError && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(m.setDefault.error)}
        </p>
      )}
    </div>
  );
}

/** Daten für den Editor (Labels, Status, Mitglieder) – Team oder Workspace. */
function useEditorData(team?: Team) {
  const teamLabels = useTeamLabels(team?.id);
  const wsLabels = useWorkspaceLabels();
  const states = useWorkflowStates(team?.id);
  const members = useTeamMembers(team?.key ?? "");
  const wsMembers = useWorkspaceMembers(!team);
  const all = team ? teamLabels.data : wsLabels.data;
  return {
    projectMembers: (team ? members.data : wsMembers.data) ?? [],
    labels: (all ?? []).filter((l) => !l.isGroup && !l.retiredAt),
    states: states.data ?? [],
    members: team ? (members.data ?? []) : [],
  };
}

export function TemplatesManager({ team }: { team?: Team }) {
  const q = useTemplates(team?.id ?? null);
  const defaults = useTemplateDefaults(team?.id);
  const editor = useEditorData(team);
  const [editing, setEditing] = useState<{ kind: TemplateKind; template: Template | null } | null>(
    null,
  );
  const [newKind, setNewKind] = useState<TemplateKind>("issue");
  if (q.isLoading) return <ListSkeleton rows={4} />;
  if (q.isError)
    return (
      <ErrorState
        error={q.error}
        title="Could not load templates"
        onRetry={() => void q.refetch()}
      />
    );
  const templates = q.data ?? [];
  const defaultFor = (t: Template) => {
    const d = team ? defaults.data : undefined;
    if (!d) return [];
    return [
      ...(d.forMembersId === t.id ? ["for members"] : []),
      ...(d.forNonMembersId === t.id ? ["for non-members"] : []),
      ...(d.projectId === t.id ? ["for projects"] : []),
    ];
  };
  return (
    <Section
      title="Templates"
      description={
        team
          ? "Templates for issues, projects and documents of this team. Sub-teams can use them too."
          : "Workspace templates are available in all teams. They cannot contain team-specific values."
      }
    >
      {KINDS.map(({ kind, title }) => {
        const rows = templates.filter((t) => t.type === kind);
        return (
          <div key={kind} role="group" aria-label={title} className="flex flex-col gap-1">
            <h3 className="text-sm font-medium">{title}</h3>
            <ul className="divide-y divide-border rounded-lg border border-border">
              {rows.length === 0 && (
                <li className="px-3 py-2 text-sm text-muted-foreground">No templates yet.</li>
              )}
              {rows.map((t) => (
                <TemplateRow
                  key={t.id}
                  t={t}
                  team={team}
                  defaultFor={defaultFor(t)}
                  onEdit={() => setEditing({ kind, template: t })}
                />
              ))}
            </ul>
          </div>
        );
      })}
      {editing ? (
        <TemplateEditor
          key={editing.template?.id ?? `new-${editing.kind}`}
          kind={editing.kind}
          template={editing.template}
          team={team}
          {...editor}
          onDone={() => setEditing(null)}
        />
      ) : (
        <div className="flex items-center gap-2">
          <select
            aria-label="New template type"
            className={selectClass}
            value={newKind}
            onChange={(e) => setNewKind(e.target.value as TemplateKind)}
          >
            {KINDS.map((k) => (
              <option key={k.kind} value={k.kind}>
                {k.noun}
              </option>
            ))}
          </select>
          <Button size="sm" onClick={() => setEditing({ kind: newKind, template: null })}>
            <Plus /> New template
          </Button>
        </div>
      )}
      {team && <DefaultsForm team={team} templates={templates} />}
    </Section>
  );
}
