/**
 * Team-Settings „SLA“, „Webhooks“ und „Recurring issues“ (WP30) sowie die Workspace-Seite
 * „Webhooks“ (`/{ws}/settings/webhooks`, nur Admins). Rechte, SSRF-Regeln und Validierung liegen im
 * Backend; Fehler werden unverändert (verständlich) angezeigt.
 */
import { Copy, Plus } from "lucide-react";
import { useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import { showToast } from "@/features/issues/toast-store";
import type { RecurringIssue, SlaRule, Team, Webhook, WebhookDelivery } from "@/gql/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { workspaceSettingsNav } from "./nav";
import { useTeamLabels } from "./queries";
import {
  useRecurringIssues,
  useRecurringMutations,
  useSlaMutations,
  useTeamSla,
  useWebhookDeliveries,
  useWebhookMutations,
  useWebhooks,
  type WebhookSecret,
} from "./team-extra-queries";
import {
  conditionsToFilter,
  describeConditions,
  describeDuration,
  describeRhythm,
  DURATION_PRESETS,
  durationToIso,
  filterToConditions,
  formatInZone,
  parseDuration,
  PRIORITY_NAMES,
  timeToMinute,
  WEEKDAYS,
  type DurationUnit,
} from "./team-extra-model";
import { useTemplates } from "./template-queries";
import { Field, Section, SettingsShell, selectClass } from "./ui";

function ErrorLine({ error }: { error: unknown }) {
  if (error == null) return null;
  return (
    <p role="alert" className="text-xs text-destructive">
      {errorText(error)}
    </p>
  );
}

/** Zweistufige Bestätigung für destruktive Aktionen. */
function ConfirmButton({
  label,
  confirmLabel,
  pending,
  onConfirm,
}: {
  label: string;
  confirmLabel: string;
  pending?: boolean;
  onConfirm: () => void;
}) {
  const [confirm, setConfirm] = useState(false);
  if (!confirm)
    return (
      <Button variant="ghost" size="sm" aria-label={label} onClick={() => setConfirm(true)}>
        Delete
      </Button>
    );
  return (
    <>
      <Button
        variant="ghost"
        size="sm"
        aria-label={confirmLabel}
        disabled={pending}
        onClick={() => {
          onConfirm();
          setConfirm(false);
        }}
      >
        Confirm delete
      </Button>
      <Button variant="ghost" size="sm" onClick={() => setConfirm(false)}>
        Cancel
      </Button>
    </>
  );
}

// =====================================================================================
// SLA
// =====================================================================================

type SlaType = "all" | "onlyBusinessDays";

function RuleEditor({
  team,
  rule,
  onDone,
}: {
  team: Team;
  rule: SlaRule | null;
  onDone: () => void;
}) {
  const m = useSlaMutations(team.id);
  const labels = useTeamLabels(team.id);
  const initial = rule ? filterToConditions(rule.filter) : { priorities: [], labelIds: [] };
  const custom = rule !== null && initial === null; // Filter, den der Editor nicht abbilden kann
  const [name, setName] = useState(rule?.name ?? "");
  const [priorities, setPriorities] = useState<number[]>(initial?.priorities ?? []);
  const [labelIds, setLabelIds] = useState<string[]>(initial?.labelIds ?? []);
  const [removes, setRemoves] = useState(rule?.removesSla ?? false);
  const [slaType, setSlaType] = useState<SlaType>((rule?.slaType as SlaType) ?? "all");
  const [startMode, setStartMode] = useState<"issueCreation" | "ruleMatch">(
    (rule?.startMode as "issueCreation" | "ruleMatch") ?? "issueCreation",
  );
  const parsed = parseDuration(rule?.duration);
  const presetIso = DURATION_PRESETS.some((p) => p.iso === rule?.duration)
    ? (rule?.duration ?? "PT24H")
    : rule?.duration
      ? "custom"
      : "PT24H";
  const [preset, setPreset] = useState(presetIso);
  const [amount, setAmount] = useState(
    String(
      parsed
        ? slaType === "onlyBusinessDays" && parsed.unit === "weeks"
          ? parsed.amount * 5
          : parsed.amount
        : 1,
    ),
  );
  const [unit, setUnit] = useState<DurationUnit>(
    parsed && slaType === "all" ? parsed.unit : "days",
  );
  const labelOptions = (labels.data ?? []).filter((l) => !l.isGroup && !l.retiredAt);
  const toggle = <T,>(set: (f: (c: T[]) => T[]) => void, v: T) =>
    set((cur) => (cur.includes(v) ? cur.filter((x) => x !== v) : [...cur, v]));

  const amountNum = Number(amount);
  const amountOk = Number.isInteger(amountNum) && amountNum >= 1 && amountNum <= 3650;
  const needsAmount = !removes && (slaType === "onlyBusinessDays" || preset === "custom");
  const valid = name.trim() !== "" && (!needsAmount || amountOk);

  function duration(): string | undefined {
    if (removes) return undefined;
    if (slaType === "onlyBusinessDays") return durationToIso(amountNum, "days");
    return preset === "custom" ? durationToIso(amountNum, unit) : preset;
  }

  const filter = custom ? rule.filter : conditionsToFilter({ priorities, labelIds });
  const error = m.create.error ?? m.update.error;

  return (
    <form
      aria-label={rule ? `Edit SLA rule ${rule.name}` : "New SLA rule"}
      className="flex flex-col gap-3 rounded-lg border border-border p-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!valid) return;
        const input = {
          name: name.trim(),
          filter: filter,
          removesSla: removes,
          ...(duration() ? { duration: duration() } : {}),
          slaType,
          startMode,
        };
        if (rule) m.update.mutate({ id: rule.id, input }, { onSuccess: onDone });
        else m.create.mutate(input, { onSuccess: onDone });
      }}
    >
      <Field label="Rule name">
        <Input aria-label="Rule name" value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
      {custom ? (
        <p role="note" className="text-sm text-muted-foreground">
          This rule uses conditions that cannot be edited here. They stay unchanged.
        </p>
      ) : (
        <>
          <fieldset className="flex flex-wrap gap-x-4 gap-y-1 text-sm">
            <legend className="text-xs text-muted-foreground">
              Priority (none selected = any priority)
            </legend>
            {PRIORITY_NAMES.map((p, i) => (
              <label key={p} className="flex items-center gap-1">
                <input
                  type="checkbox"
                  aria-label={`Priority ${p}`}
                  checked={priorities.includes(i)}
                  onChange={() => toggle(setPriorities, i)}
                />
                {p}
              </label>
            ))}
          </fieldset>
          <fieldset className="flex flex-wrap gap-x-4 gap-y-1 text-sm">
            <legend className="text-xs text-muted-foreground">
              Has one of these labels (none selected = any labels)
            </legend>
            {labelOptions.length === 0 && (
              <span className="text-muted-foreground">No labels yet.</span>
            )}
            {labelOptions.map((l) => (
              <label key={l.id} className="flex items-center gap-1">
                <input
                  type="checkbox"
                  aria-label={`Label ${l.name}`}
                  checked={labelIds.includes(l.id)}
                  onChange={() => toggle(setLabelIds, l.id)}
                />
                {l.name}
              </label>
            ))}
          </fieldset>
        </>
      )}
      <fieldset className="flex gap-4 text-sm">
        <legend className="text-xs text-muted-foreground">Action</legend>
        <label className="flex items-center gap-1">
          <input
            type="radio"
            name="sla-action"
            aria-label="Set SLA"
            checked={!removes}
            onChange={() => setRemoves(false)}
          />
          Set SLA
        </label>
        <label className="flex items-center gap-1">
          <input
            type="radio"
            name="sla-action"
            aria-label="Remove SLA"
            checked={removes}
            onChange={() => setRemoves(true)}
          />
          Remove SLA
        </label>
      </fieldset>
      {!removes && (
        <>
          <Field label="Counting">
            <select
              aria-label="SLA counting"
              className={selectClass}
              value={slaType}
              onChange={(e) => setSlaType(e.target.value as SlaType)}
            >
              <option value="all">Calendar time</option>
              <option value="onlyBusinessDays">Business days only</option>
            </select>
          </Field>
          {slaType === "all" && (
            <Field label="Time to resolve">
              <select
                aria-label="SLA duration"
                className={selectClass}
                value={preset}
                onChange={(e) => setPreset(e.target.value)}
              >
                {DURATION_PRESETS.map((p) => (
                  <option key={p.iso} value={p.iso}>
                    {p.label}
                  </option>
                ))}
                <option value="custom">Custom…</option>
              </select>
            </Field>
          )}
          {needsAmount && (
            <div className="flex items-end gap-2">
              <Field label={slaType === "all" ? "Custom amount" : "Business days"}>
                <Input
                  aria-label="SLA amount"
                  inputMode="numeric"
                  className="w-24"
                  value={amount}
                  onChange={(e) => setAmount(e.target.value)}
                />
              </Field>
              {slaType === "all" && (
                <select
                  aria-label="SLA unit"
                  className={selectClass}
                  value={unit}
                  onChange={(e) => setUnit(e.target.value as DurationUnit)}
                >
                  <option value="hours">hours</option>
                  <option value="days">days</option>
                  <option value="weeks">weeks</option>
                </select>
              )}
            </div>
          )}
          <Field label="Clock starts">
            <select
              aria-label="SLA start"
              className={selectClass}
              value={startMode}
              onChange={(e) => setStartMode(e.target.value as "issueCreation" | "ruleMatch")}
            >
              <option value="issueCreation">When the issue is created</option>
              <option value="ruleMatch">When the rule matches</option>
            </select>
          </Field>
        </>
      )}
      <div className="flex items-center gap-3">
        <Button
          type="submit"
          size="sm"
          disabled={!valid || m.create.isPending || m.update.isPending}
        >
          Save rule
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onDone}>
          Cancel
        </Button>
        <ErrorLine error={error} />
      </div>
    </form>
  );
}

export function SlaSection({ team }: { team: Team }) {
  const q = useTeamSla(team.id);
  const labels = useTeamLabels(team.id);
  const m = useSlaMutations(team.id);
  const [editing, setEditing] = useState<SlaRule | "new" | null>(null);
  if (q.isLoading) return <ListSkeleton rows={3} />;
  if (q.isError || !q.data)
    return (
      <ErrorState error={q.error} title="Could not load SLAs" onRetry={() => void q.refetch()} />
    );
  const sla = q.data;
  const rules = [...sla.rules].sort((a, b) => a.position - b.position);
  const labelName = (id: string) => labels.data?.find((l) => l.id === id)?.name ?? "Unknown label";
  const move = (index: number, dir: -1 | 1) => {
    const a = rules[index];
    const b = rules[index + dir];
    if (!a || !b) return;
    // Positionen tauschen: erst a, dann b. Fehler zeigt `m.update.error`.
    void (async () => {
      try {
        await m.update.mutateAsync({ id: a.id, input: { position: b.position } });
        await m.update.mutateAsync({ id: b.id, input: { position: a.position } });
      } catch {
        /* wird über m.update.error angezeigt */
      }
    })();
  };
  return (
    <Section
      title="SLA"
      description="Service-level agreements set a deadline on issues that match a rule. The first matching rule applies. Changes affect issues from their next update on."
    >
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          aria-label="Enable SLAs"
          checked={sla.enabled}
          disabled={m.settings.isPending}
          onChange={(e) => m.settings.mutate({ enabled: e.target.checked })}
        />
        Enable SLAs for this team
      </label>
      <Field label="Work week (for business-day SLAs, in the team time zone)">
        <select
          aria-label="Work week"
          className={selectClass}
          value={sla.workWeek}
          disabled={m.settings.isPending}
          onChange={(e) => m.settings.mutate({ workWeek: e.target.value })}
        >
          <option value="monToFri">Monday to Friday</option>
          <option value="sunToThu">Sunday to Thursday</option>
        </select>
      </Field>
      <ErrorLine error={m.settings.error ?? m.remove.error ?? m.update.error} />
      <ul aria-label="SLA rules" className="divide-y divide-border rounded-lg border border-border">
        {rules.length === 0 && (
          <li className="px-3 py-2 text-sm text-muted-foreground">
            No rules yet. Turn SLAs on to get the default rules, or add your own.
          </li>
        )}
        {rules.map((r, i) => (
          <li
            key={r.id}
            aria-label={`SLA rule ${r.name}`}
            className="flex items-center gap-2 px-3 py-1.5 text-sm"
          >
            <span className="flex-1">
              <span className="font-medium">{r.name}</span>
              <span className="block text-xs text-muted-foreground">
                {describeConditions(r.filter, labelName)} →{" "}
                {r.removesSla
                  ? "remove SLA"
                  : `${describeDuration(r.duration, r.slaType === "onlyBusinessDays")}, clock starts ${
                      r.startMode === "ruleMatch" ? "when the rule matches" : "at creation"
                    }`}
              </span>
            </span>
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Move ${r.name} up`}
              disabled={i === 0 || m.update.isPending}
              onClick={() => move(i, -1)}
            >
              Up
            </Button>
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Move ${r.name} down`}
              disabled={i === rules.length - 1 || m.update.isPending}
              onClick={() => move(i, 1)}
            >
              Down
            </Button>
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Edit ${r.name}`}
              onClick={() => setEditing(r)}
            >
              Edit
            </Button>
            <ConfirmButton
              label={`Delete ${r.name}`}
              confirmLabel={`Confirm delete ${r.name}`}
              pending={m.remove.isPending}
              onConfirm={() => m.remove.mutate(r.id)}
            />
          </li>
        ))}
      </ul>
      {editing ? (
        <RuleEditor
          key={editing === "new" ? "new" : editing.id}
          team={team}
          rule={editing === "new" ? null : editing}
          onDone={() => setEditing(null)}
        />
      ) : (
        <div>
          <Button size="sm" onClick={() => setEditing("new")}>
            <Plus /> New rule
          </Button>
        </div>
      )}
    </Section>
  );
}

// =====================================================================================
// Recurring issues
// =====================================================================================

function RecurringForm({ team, onDone }: { team: Team; onDone: () => void }) {
  const templates = useTemplates(team.id);
  const m = useRecurringMutations(team.id);
  const issueTemplates = (templates.data ?? []).filter((t) => t.type === "issue");
  const [templateId, setTemplateId] = useState("");
  const [frequency, setFrequency] = useState<"daily" | "weekly" | "monthly">("weekly");
  const [interval, setInterval] = useState("1");
  const [weekday, setWeekday] = useState(1);
  const [dayOfMonth, setDayOfMonth] = useState("1");
  const [time, setTime] = useState("09:00");
  const [startDate, setStartDate] = useState("");
  const minute = timeToMinute(time);
  const intervalNum = Number(interval);
  const maxInterval = frequency === "daily" ? 365 : frequency === "weekly" ? 52 : 12;
  const intervalOk =
    Number.isInteger(intervalNum) && intervalNum >= 1 && intervalNum <= maxInterval;
  const dom = Number(dayOfMonth);
  const domOk = frequency !== "monthly" || (Number.isInteger(dom) && dom >= 1 && dom <= 31);
  const valid = templateId !== "" && minute !== null && intervalOk && domOk;
  return (
    <form
      aria-label="New recurring issue"
      className="flex flex-col gap-3 rounded-lg border border-border p-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!valid || minute === null) return;
        m.create.mutate(
          {
            templateId,
            frequency,
            interval: intervalNum,
            ...(frequency === "weekly" ? { weekday } : {}),
            ...(frequency === "monthly" ? { dayOfMonth: dom } : {}),
            minuteOfDay: minute,
            ...(startDate ? { startDate } : {}),
          },
          { onSuccess: onDone },
        );
      }}
    >
      <Field label="Issue template (title, sub-issues and fields come from it)">
        <select
          aria-label="Issue template"
          className={selectClass}
          value={templateId}
          onChange={(e) => setTemplateId(e.target.value)}
        >
          <option value="">Choose a template…</option>
          {issueTemplates.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
      </Field>
      {templates.isSuccess && issueTemplates.length === 0 && (
        <p role="note" className="text-sm text-muted-foreground">
          This team has no issue templates yet. Create one in the Templates section first.
        </p>
      )}
      <div className="flex flex-wrap items-end gap-3">
        <Field label="Repeat">
          <select
            aria-label="Frequency"
            className={selectClass}
            value={frequency}
            onChange={(e) => setFrequency(e.target.value as "daily" | "weekly" | "monthly")}
          >
            <option value="daily">Daily</option>
            <option value="weekly">Weekly</option>
            <option value="monthly">Monthly</option>
          </select>
        </Field>
        <Field
          label={`Every (${frequency === "daily" ? "days" : frequency === "weekly" ? "weeks" : "months"})`}
        >
          <Input
            aria-label="Interval"
            inputMode="numeric"
            className="w-20"
            value={interval}
            onChange={(e) => setInterval(e.target.value)}
          />
        </Field>
        {frequency === "weekly" && (
          <Field label="Weekday">
            <select
              aria-label="Weekday"
              className={selectClass}
              value={weekday}
              onChange={(e) => setWeekday(Number(e.target.value))}
            >
              {WEEKDAYS.map((d, i) => (
                <option key={d} value={i + 1}>
                  {d}
                </option>
              ))}
            </select>
          </Field>
        )}
        {frequency === "monthly" && (
          <Field label="Day of month (short months use their last day)">
            <Input
              aria-label="Day of month"
              inputMode="numeric"
              className="w-20"
              value={dayOfMonth}
              onChange={(e) => setDayOfMonth(e.target.value)}
            />
          </Field>
        )}
        <Field label={`Time (${team.timezone})`}>
          <Input
            aria-label="Time of day"
            type="time"
            className="w-28"
            value={time}
            onChange={(e) => setTime(e.target.value)}
          />
        </Field>
      </div>
      <Field label="Start date (optional, in the team time zone)">
        <Input
          aria-label="Start date"
          type="date"
          className="w-44"
          value={startDate}
          onChange={(e) => setStartDate(e.target.value)}
        />
      </Field>
      <div className="flex items-center gap-3">
        <Button type="submit" size="sm" disabled={!valid || m.create.isPending}>
          Create recurring issue
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onDone}>
          Cancel
        </Button>
        <ErrorLine error={m.create.error} />
      </div>
    </form>
  );
}

function RecurringRow({ r, team }: { r: RecurringIssue; team: Team }) {
  const m = useRecurringMutations(team.id);
  return (
    <li
      aria-label={`Recurring issue ${r.templateName}`}
      className="flex flex-col gap-1 px-3 py-2 text-sm"
    >
      <div className="flex items-center gap-2">
        <span className="flex-1">
          <span className="font-medium">{r.templateName}</span>
          {r.paused && <span className="ml-2 text-xs text-muted-foreground">Paused</span>}
          <span className="block text-xs text-muted-foreground">
            {describeRhythm(r)} ({team.timezone})
          </span>
          <span className="block text-xs text-muted-foreground">
            {r.paused ? "Not scheduled" : `Next: ${formatInZone(r.nextRunAt, team.timezone)}`}
            {r.lastRunAt ? ` · last created ${formatInZone(r.lastRunAt, team.timezone)}` : ""}
          </span>
        </span>
        <Button
          variant="ghost"
          size="sm"
          aria-label={`${r.paused ? "Resume" : "Pause"} ${r.templateName}`}
          disabled={m.update.isPending}
          onClick={() => m.update.mutate({ id: r.id, input: { paused: !r.paused } })}
        >
          {r.paused ? "Resume" : "Pause"}
        </Button>
        <ConfirmButton
          label={`Delete ${r.templateName}`}
          confirmLabel={`Confirm delete ${r.templateName}`}
          pending={m.remove.isPending}
          onConfirm={() => m.remove.mutate(r.id)}
        />
      </div>
      {r.lastError && (
        <p role="alert" className="text-xs text-destructive">
          The last run was skipped: {r.lastError}
        </p>
      )}
      <ErrorLine error={m.update.error ?? m.remove.error} />
    </li>
  );
}

export function RecurringSection({ team }: { team: Team }) {
  const q = useRecurringIssues(team.id);
  const [creating, setCreating] = useState(false);
  if (q.isLoading) return <ListSkeleton rows={3} />;
  if (q.isError)
    return (
      <ErrorState
        error={q.error}
        title="Could not load recurring issues"
        onRetry={() => void q.refetch()}
      />
    );
  const rows = q.data ?? [];
  return (
    <Section
      title="Recurring issues"
      description="Create issues from an issue template on a schedule. Missed runs are not made up."
    >
      <ul
        aria-label="Recurring issues"
        className="divide-y divide-border rounded-lg border border-border"
      >
        {rows.length === 0 && (
          <li className="px-3 py-2 text-sm text-muted-foreground">No recurring issues yet.</li>
        )}
        {rows.map((r) => (
          <RecurringRow key={r.id} r={r} team={team} />
        ))}
      </ul>
      {creating ? (
        <RecurringForm team={team} onDone={() => setCreating(false)} />
      ) : (
        <div>
          <Button size="sm" onClick={() => setCreating(true)}>
            <Plus /> New recurring issue
          </Button>
        </div>
      )}
    </Section>
  );
}

// =====================================================================================
// Webhooks
// =====================================================================================

export const WEBHOOK_RESOURCE_TYPES = [
  "Issue",
  "Comment",
  "Attachment",
  "IssueLabel",
  "Reaction",
  "Project",
  "ProjectUpdate",
  "Cycle",
  "Document",
  "Initiative",
  "InitiativeUpdate",
  "User",
  "IssueSLA",
] as const;

function copySecret(secret: string) {
  void navigator.clipboard
    ?.writeText(secret)
    .then(() => showToast({ message: "Secret copied" }))
    .catch(() => showToast({ message: "Could not copy secret", tone: "error" }));
}

/** Einmalige Anzeige des Signing-Secrets; es lebt nur im Zustand des Dialogs. */
function SecretDialog({ secret, onClose }: { secret: WebhookSecret | null; onClose: () => void }) {
  useShortcutContext("modal", secret !== null);
  return (
    <Dialog open={secret !== null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <div className="flex flex-col gap-3 p-4">
          <DialogTitle className="text-base font-semibold">Signing secret</DialogTitle>
          <DialogDescription className="sr-only">
            Copy the secret now. It is shown only once.
          </DialogDescription>
          <p
            role="alert"
            className="rounded-md border border-border-strong bg-surface-hover px-3 py-2 text-sm"
          >
            Copy this secret now and store it safely. It is shown only once and cannot be retrieved
            later. Use it to verify the signature of each delivery.
          </p>
          <div className="flex items-center gap-2">
            <Input
              readOnly
              aria-label="Webhook secret"
              className="font-mono text-xs"
              value={secret?.secret ?? ""}
              onFocus={(e) => e.currentTarget.select()}
            />
            <Button type="button" size="sm" onClick={() => secret && copySecret(secret.secret)}>
              <Copy className="size-3.5" aria-hidden /> Copy
            </Button>
          </div>
          <div className="flex justify-end">
            <Button type="button" onClick={onClose}>
              Done
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function WebhookForm({
  team,
  webhook,
  onSecret,
  onDone,
}: {
  /** Festes Team (Team-Settings); ohne Team wählt das Formular „alle öffentlichen Teams“ oder ein Team. */
  team?: Team;
  webhook: Webhook | null;
  onSecret: (s: WebhookSecret) => void;
  onDone: () => void;
}) {
  const session = useSession();
  const m = useWebhookMutations();
  const [label, setLabel] = useState(webhook?.label ?? "");
  const [url, setUrl] = useState(webhook?.url ?? "");
  const [types, setTypes] = useState<string[]>(webhook?.resourceTypes ?? ["Issue"]);
  const [scope, setScope] = useState<string>(team ? team.id : "all"); // "all" | Team-ID
  const valid = url.trim() !== "" && types.length > 0;
  const toggle = (t: string) =>
    setTypes((cur) => (cur.includes(t) ? cur.filter((x) => x !== t) : [...cur, t]));
  const error = m.create.error ?? m.update.error;
  return (
    <form
      aria-label={webhook ? "Edit webhook" : "New webhook"}
      className="flex flex-col gap-3 rounded-lg border border-border p-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!valid) return;
        if (webhook)
          m.update.mutate(
            {
              id: webhook.id,
              input: {
                ...(label.trim() !== (webhook.label ?? "") ? { label: label.trim() } : {}),
                ...(url.trim() !== webhook.url ? { url: url.trim() } : {}),
                resourceTypes: types,
              },
            },
            { onSuccess: onDone },
          );
        else
          m.create.mutate(
            {
              ...(label.trim() ? { label: label.trim() } : {}),
              url: url.trim(),
              resourceTypes: types,
              allPublicTeams: scope === "all",
              ...(scope !== "all" ? { teamId: scope } : {}),
            },
            {
              onSuccess: (r) => {
                onSecret(r);
                onDone();
              },
            },
          );
      }}
    >
      <Field label="Label (optional)">
        <Input
          aria-label="Webhook label"
          maxLength={100}
          value={label}
          onChange={(e) => setLabel(e.target.value)}
        />
      </Field>
      <Field label="URL (HTTPS; must be publicly reachable)">
        <Input
          aria-label="Webhook URL"
          type="url"
          placeholder="https://example.com/hooks/lineal"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
        />
      </Field>
      {!webhook && !team && (
        <Field label="Teams">
          <select
            aria-label="Webhook teams"
            className={selectClass}
            value={scope}
            onChange={(e) => setScope(e.target.value)}
          >
            <option value="all">All public teams</option>
            {(session?.teams ?? []).map((t) => (
              <option key={t.id} value={t.id}>
                {t.name} only
              </option>
            ))}
          </select>
        </Field>
      )}
      <fieldset className="flex flex-wrap gap-x-4 gap-y-1 text-sm">
        <legend className="text-xs text-muted-foreground">Resource types</legend>
        {WEBHOOK_RESOURCE_TYPES.map((t) => (
          <label key={t} className="flex items-center gap-1">
            <input
              type="checkbox"
              aria-label={`Resource ${t}`}
              checked={types.includes(t)}
              onChange={() => toggle(t)}
            />
            {t}
          </label>
        ))}
      </fieldset>
      <div className="flex items-center gap-3">
        <Button
          type="submit"
          size="sm"
          disabled={!valid || m.create.isPending || m.update.isPending}
        >
          {webhook ? "Save webhook" : "Create webhook"}
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onDone}>
          Cancel
        </Button>
        <ErrorLine error={error} />
      </div>
    </form>
  );
}

const DELIVERY_STATUS: Record<string, string> = {
  pending: "Pending",
  succeeded: "Delivered",
  given_up: "Failed",
};

function Deliveries({ webhookId }: { webhookId: string }) {
  const q = useWebhookDeliveries(webhookId, true);
  const m = useWebhookMutations();
  if (q.isLoading) return <ListSkeleton rows={2} />;
  if (q.isError)
    return (
      <ErrorState
        error={q.error}
        title="Could not load deliveries"
        onRetry={() => void q.refetch()}
      />
    );
  const rows: WebhookDelivery[] = q.data ?? [];
  return (
    <div className="flex flex-col gap-1">
      <ul
        aria-label="Webhook deliveries"
        className="divide-y divide-border rounded-md border border-border"
      >
        {rows.length === 0 && (
          <li className="px-3 py-2 text-xs text-muted-foreground">No deliveries yet.</li>
        )}
        {rows.map((d) => (
          <li
            key={d.id}
            aria-label={`Delivery ${d.resourceType} ${d.action}`}
            className="flex items-center gap-2 px-3 py-1 text-xs"
          >
            <span className="flex-1">
              {d.resourceType} {d.action} ·{" "}
              <span className={d.status === "given_up" ? "text-destructive" : undefined}>
                {DELIVERY_STATUS[d.status] ?? d.status}
              </span>
              {d.responseCode ? ` (HTTP ${d.responseCode})` : ""} · attempt {d.attempt} ·{" "}
              {formatInZone(d.createdAt, undefined)}
            </span>
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Resend ${d.resourceType} ${d.action}`}
              disabled={m.retry.isPending}
              onClick={() => m.retry.mutate({ webhookId, deliveryId: d.id })}
            >
              Resend
            </Button>
          </li>
        ))}
      </ul>
      <ErrorLine error={m.retry.error} />
    </div>
  );
}

function WebhookRow({
  w,
  teamName,
  onEdit,
  onSecret,
}: {
  w: Webhook;
  teamName: string | null;
  onEdit: () => void;
  onSecret: (s: WebhookSecret) => void;
}) {
  const m = useWebhookMutations();
  const [open, setOpen] = useState(false);
  const [rotate, setRotate] = useState(false);
  const name = w.label || w.url;
  return (
    <li aria-label={`Webhook ${name}`} className="flex flex-col gap-1 px-3 py-2 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1">
          <span className="font-medium">{name}</span>
          <span className="ml-2 text-xs text-muted-foreground">
            {w.enabled ? "Enabled" : "Disabled"}
          </span>
          <span className="block truncate text-xs text-muted-foreground">
            {w.url} · {teamName ?? "All public teams"} · {w.resourceTypes.join(", ")}
          </span>
          {!w.enabled && w.disabledReason && (
            <span className="block text-xs text-destructive">
              Disabled automatically: {w.disabledReason}
            </span>
          )}
          {w.enabled && w.consecutiveFailures > 0 && (
            <span className="block text-xs text-destructive">
              {w.consecutiveFailures} failed deliveries in a row
            </span>
          )}
        </span>
        <Button
          variant="ghost"
          size="sm"
          aria-label={`${open ? "Hide" : "Show"} deliveries of ${name}`}
          onClick={() => setOpen((o) => !o)}
        >
          Deliveries
        </Button>
        <Button variant="ghost" size="sm" aria-label={`Edit ${name}`} onClick={onEdit}>
          Edit
        </Button>
        <Button
          variant="ghost"
          size="sm"
          aria-label={`${w.enabled ? "Disable" : "Enable"} ${name}`}
          disabled={m.update.isPending}
          onClick={() => m.update.mutate({ id: w.id, input: { enabled: !w.enabled } })}
        >
          {w.enabled ? "Disable" : "Enable"}
        </Button>
        {rotate ? (
          <>
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Confirm rotate secret of ${name}`}
              disabled={m.rotate.isPending}
              onClick={() =>
                m.rotate.mutate(w.id, {
                  onSuccess: onSecret,
                  onSettled: () => setRotate(false),
                })
              }
            >
              Confirm rotate
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setRotate(false)}>
              Cancel
            </Button>
          </>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            aria-label={`Rotate secret of ${name}`}
            onClick={() => setRotate(true)}
          >
            Rotate secret
          </Button>
        )}
        <ConfirmButton
          label={`Delete ${name}`}
          confirmLabel={`Confirm delete ${name}`}
          pending={m.remove.isPending}
          onConfirm={() => m.remove.mutate(w.id)}
        />
      </div>
      {rotate && (
        <p className="text-xs text-muted-foreground">
          A new secret is generated. The old one keeps working for 24 hours.
        </p>
      )}
      <ErrorLine error={m.update.error ?? m.remove.error ?? m.rotate.error} />
      {open && <Deliveries webhookId={w.id} />}
    </li>
  );
}

/** Webhook-Verwaltung: ohne `team` alle verwaltbaren Webhooks, sonst nur die des Teams. */
export function WebhooksManager({ team }: { team?: Team }) {
  const q = useWebhooks();
  const session = useSession();
  const [editing, setEditing] = useState<Webhook | "new" | null>(null);
  const [secret, setSecret] = useState<WebhookSecret | null>(null);
  if (q.isLoading) return <ListSkeleton rows={3} />;
  if (q.isError)
    return (
      <ErrorState
        error={q.error}
        title="Could not load webhooks"
        onRetry={() => void q.refetch()}
      />
    );
  const rows = (q.data ?? []).filter((w) => !team || w.teamId === team.id);
  const teamName = (id: string | null | undefined) =>
    id ? (session?.teams.find((t) => t.id === id)?.name ?? "One team") : null;
  return (
    <Section
      title="Webhooks"
      description={
        team
          ? "Send events of this team to your own HTTPS endpoint. Each delivery is signed with the webhook secret."
          : "Send events to your own HTTPS endpoint. Each delivery is signed with the webhook secret."
      }
    >
      <ul aria-label="Webhooks" className="divide-y divide-border rounded-lg border border-border">
        {rows.length === 0 && (
          <li className="px-3 py-2 text-sm text-muted-foreground">No webhooks yet.</li>
        )}
        {rows.map((w) => (
          <WebhookRow
            key={w.id}
            w={w}
            teamName={teamName(w.teamId)}
            onEdit={() => setEditing(w)}
            onSecret={setSecret}
          />
        ))}
      </ul>
      {editing ? (
        <WebhookForm
          key={editing === "new" ? "new" : editing.id}
          team={team}
          webhook={editing === "new" ? null : editing}
          onSecret={setSecret}
          onDone={() => setEditing(null)}
        />
      ) : (
        <div>
          <Button size="sm" onClick={() => setEditing("new")}>
            <Plus /> New webhook
          </Button>
        </div>
      )}
      <SecretDialog secret={secret} onClose={() => setSecret(null)} />
    </Section>
  );
}

export function WebhooksSection({ team }: { team: Team }) {
  return <WebhooksManager team={team} />;
}

/** Workspace-Settings „Webhooks“ (`/{ws}/settings/webhooks`). */
export function WorkspaceWebhooksPage({ ws }: { ws: string }) {
  const session = useSession();
  return (
    <SettingsShell title="Settings" nav={workspaceSettingsNav(ws, session, "webhooks")}>
      <WebhooksManager />
    </SettingsShell>
  );
}
