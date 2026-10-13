import { ArrowDown, ArrowUp, Plus } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { useMemo, useState } from "react";

import { useSession } from "@/app/auth";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import {
  STATE_TYPE_LABEL,
  STATE_TYPE_ORDER,
  sortStates,
  type StateType,
} from "@/features/issues/model";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import { StatusIcon } from "@/features/issues/icons";
import type { Team, WorkflowState } from "@/gql/graphql";

import {
  type CycleSettings,
  useMembership,
  useTeamLabels,
  useTeamMembers,
  useUpdateCycleSettings,
  useUpdateTeam,
  useWorkflowMutations,
  useWorkflowStates,
  useWorkspaceMembers,
} from "./queries";
import { estimateLabel } from "./team-config-sections";
import { TeamAccessSection, TeamDangerZone } from "./team-admin-sections";
import { normalizeKey, validateKey } from "./team-key";
import { LabelManager } from "./label-manager";
import { Field, Section, selectClass } from "./ui";

export function SaveRow({
  pending,
  error,
  dirty,
}: {
  pending: boolean;
  error: unknown;
  dirty: boolean;
}) {
  return (
    <div className="flex items-center gap-3">
      <Button type="submit" size="sm" disabled={!dirty || pending}>
        Save
      </Button>
      {error != null && (
        <span role="alert" className="text-xs text-destructive">
          {errorText(error)}
        </span>
      )}
    </div>
  );
}

const ICON_RE = /^[A-Za-z0-9_:-]{1,64}$/;

export function GeneralSection({ team }: { team: Team }) {
  const update = useUpdateTeam(team.id);
  const session = useSession();
  const navigate = useNavigate();
  const [name, setName] = useState(team.name);
  const [key, setKey] = useState(team.key);
  const [description, setDescription] = useState(team.description ?? "");
  const [icon, setIcon] = useState(team.icon ?? "");
  const [color, setColor] = useState(team.color ?? "");
  const [timezone, setTimezone] = useState(team.timezone);
  const [confirmKey, setConfirmKey] = useState(false);
  const keyError = key !== team.key ? validateKey(key) : null;
  const iconError = icon !== "" && !ICON_RE.test(icon) ? "Use letters, digits, _ : - only." : null;
  const colorError =
    color !== "" && !/^#[0-9A-Fa-f]{6}$/.test(color) ? "Use the form #RRGGBB." : null;
  const dirty =
    name !== team.name ||
    key !== team.key ||
    description !== (team.description ?? "") ||
    icon !== (team.icon ?? "") ||
    color !== (team.color ?? "") ||
    timezone !== team.timezone;
  const invalid = !!keyError || !!iconError || !!colorError;
  const save = () => {
    update.mutate(
      {
        ...(name !== team.name ? { name } : {}),
        ...(key !== team.key ? { key } : {}),
        ...(description !== (team.description ?? "") ? { description } : {}),
        // Leeres Feld = explizites null: entfernt Icon bzw. Farbe.
        ...(icon !== (team.icon ?? "") ? { icon: icon === "" ? null : icon } : {}),
        ...(color !== (team.color ?? "") ? { color: color === "" ? null : color } : {}),
        ...(timezone !== team.timezone ? { timezone } : {}),
      },
      {
        onSuccess: () => {
          setConfirmKey(false);
          // Nach einem Key-Wechsel zeigt die URL den neuen Key.
          if (key !== team.key && session)
            void navigate({
              to: `/${session.workspace.urlKey}/settings/teams/${key}/general` as "/",
            });
        },
      },
    );
  };
  return (
    <>
      <Section
        title="General"
        description="Name, identifier, appearance and timezone of this team."
      >
        <form
          className="flex flex-col gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (invalid) return;
            if (key !== team.key) setConfirmKey(true);
            else save();
          }}
        >
          <Field label="Name">
            <Input aria-label="Name" value={name} onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label="Identifier">
            <Input
              aria-label="Identifier"
              value={key}
              maxLength={7}
              aria-invalid={!!keyError}
              onChange={(e) => setKey(normalizeKey(e.target.value))}
            />
            {keyError && (
              <span role="alert" className="text-xs text-destructive">
                {keyError}
              </span>
            )}
          </Field>
          <Field label="Description">
            <Input
              aria-label="Description"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label="Icon">
              <Input
                aria-label="Icon"
                value={icon}
                placeholder="e.g. rocket"
                onChange={(e) => setIcon(e.target.value)}
              />
              {iconError && (
                <span role="alert" className="text-xs text-destructive">
                  {iconError}
                </span>
              )}
            </Field>
            <Field label="Color">
              <Input
                aria-label="Color"
                value={color}
                placeholder="#5e6ad2"
                onChange={(e) => setColor(e.target.value)}
              />
              {colorError && (
                <span role="alert" className="text-xs text-destructive">
                  {colorError}
                </span>
              )}
            </Field>
          </div>
          <Field label="Timezone">
            <Input
              aria-label="Timezone"
              value={timezone}
              onChange={(e) => setTimezone(e.target.value)}
            />
          </Field>
          <p className="text-xs text-muted-foreground">
            Estimates: {estimateLabel(team.issueEstimationType)}
          </p>
          <SaveRow
            pending={update.isPending}
            error={confirmKey ? null : update.error}
            dirty={dirty && !invalid}
          />
        </form>
        <Dialog open={confirmKey} onOpenChange={setConfirmKey}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Change identifier to {key}?</DialogTitle>
              <DialogDescription>
                All issue identifiers of this team switch from {team.key}-… to {key}-…. The old
                identifiers keep resolving to the same issues.
              </DialogDescription>
            </DialogHeader>
            <div className="flex items-center gap-3 p-4">
              <Button size="sm" disabled={update.isPending} onClick={save}>
                Change identifier
              </Button>
              <Button variant="ghost" size="sm" onClick={() => setConfirmKey(false)}>
                Cancel
              </Button>
              {update.isError && (
                <span role="alert" className="text-xs text-destructive">
                  {errorText(update.error)}
                </span>
              )}
            </div>
          </DialogContent>
        </Dialog>
      </Section>
      <TeamDangerZone team={team} />
    </>
  );
}

export function MembersSection({ team }: { team: Team }) {
  const members = useTeamMembers(team.key);
  const { add, remove } = useMembership(team.id);
  const [adding, setAdding] = useState(false);
  const all = useWorkspaceMembers(adding);
  const [search, setSearch] = useState("");
  const candidates = useMemo(() => {
    const ids = new Set((members.data ?? []).map((m) => m.id));
    const q = search.trim().toLowerCase();
    return (all.data ?? [])
      .filter((u) => !ids.has(u.id))
      .filter((u) => !q || `${u.displayName} ${u.name}`.toLowerCase().includes(q))
      .sort((a, b) => (a.displayName || a.name).localeCompare(b.displayName || b.name));
  }, [all.data, members.data, search]);
  const [pick, setPick] = useState("");
  return (
    <>
      <Section title="Members" description="People who belong to this team.">
        {members.isLoading && <ListSkeleton rows={3} />}
        {members.isError && <ErrorState error={members.error} title="Could not load members" />}
        <ul className="divide-y divide-border rounded-lg border border-border">
          {(members.data ?? []).map((m) => (
            <li key={m.id} className="flex items-center gap-3 px-3 py-1.5 text-sm">
              {m.displayName || m.name}
              <Button
                variant="ghost"
                size="sm"
                className="ml-auto"
                aria-label={`Remove ${m.displayName || m.name}`}
                onClick={() => {
                  if (window.confirm(`Remove ${m.displayName || m.name} from ${team.name}?`))
                    remove.mutate(m.id);
                }}
              >
                Remove
              </Button>
            </li>
          ))}
        </ul>
        {remove.isError && (
          <p role="alert" className="text-xs text-destructive">
            {errorText(remove.error)}
          </p>
        )}
        {adding ? (
          <div className="flex items-center gap-2">
            <Input
              aria-label="Search people"
              placeholder="Search people…"
              className="w-48"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
            <select
              aria-label="Add member"
              className={selectClass}
              value={pick}
              onChange={(e) => setPick(e.target.value)}
            >
              <option value="">Choose a person…</option>
              {candidates.map((u) => (
                <option key={u.id} value={u.id}>
                  {u.displayName || u.name}
                </option>
              ))}
            </select>
            <Button
              size="sm"
              disabled={!pick}
              onClick={() =>
                add.mutate(pick, {
                  onSuccess: () => {
                    setPick("");
                    setAdding(false);
                  },
                })
              }
            >
              Add
            </Button>
            {add.isError && (
              <span role="alert" className="text-xs text-destructive">
                {errorText(add.error)}
              </span>
            )}
          </div>
        ) : (
          <Button
            variant="outline"
            size="sm"
            className="self-start"
            onClick={() => setAdding(true)}
          >
            <Plus /> Add a member
          </Button>
        )}
      </Section>
      <TeamAccessSection team={team} />
    </>
  );
}

const COLOR_RE = /^#[0-9A-Fa-f]{6}$/;

/** Workflow-Editor (docs/08 §2.11 „Issue statuses“): Gruppen je Kategorie, anlegen, umbenennen, verschieben, archivieren. */
export function StatusesSection({ team }: { team: Team }) {
  const q = useWorkflowStates(team.id);
  const m = useWorkflowMutations();
  const [type, setType] = useState<StateType>("unstarted");
  const [name, setName] = useState("");
  const [color, setColor] = useState("#5e6ad2");
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [archiving, setArchiving] = useState<WorkflowState | null>(null);
  const [target, setTarget] = useState("");
  const states = useMemo(() => sortStates(q.data ?? []), [q.data]);

  /** Position zwischen den Nachbarn derselben Kategorie (Float-Mittelwert, wie `sort_order::between`). */
  const move = (s: WorkflowState, dir: -1 | 1) => {
    const same = states.filter((x) => x.type === s.type);
    const idx = same.findIndex((x) => x.id === s.id);
    const other = same[idx + dir];
    if (!other) return;
    const beyond = same[idx + dir * 2];
    const position = beyond ? (other.position + beyond.position) / 2 : other.position + dir;
    m.update.mutate({ id: s.id, input: { position } });
  };

  if (q.isLoading) return <ListSkeleton rows={5} />;
  if (q.isError) return <ErrorState error={q.error} title="Could not load statuses" />;
  return (
    <Section title="Issue statuses" description="The workflow of this team, grouped by category.">
      {STATE_TYPE_ORDER.map((t) => {
        const list = states.filter((s) => s.type === t);
        if (list.length === 0) return null;
        return (
          <div
            key={t}
            aria-label={STATE_TYPE_LABEL[t]}
            role="group"
            className="flex flex-col gap-1"
          >
            <h3 className="text-xs font-medium text-muted-foreground">{STATE_TYPE_LABEL[t]}</h3>
            <ul className="divide-y divide-border rounded-lg border border-border">
              {list.map((s, i) => (
                <li key={s.id} className="flex items-center gap-2 px-3 py-1.5 text-sm">
                  <StatusIcon type={s.type} color={s.color} />
                  {editing === s.id ? (
                    <form
                      className="flex flex-1 items-center gap-2"
                      onSubmit={(e) => {
                        e.preventDefault();
                        if (draft.trim())
                          m.update.mutate({ id: s.id, input: { name: draft.trim() } });
                        setEditing(null);
                      }}
                    >
                      <Input
                        aria-label="Status name"
                        value={draft}
                        onChange={(e) => setDraft(e.target.value)}
                        autoFocus
                      />
                      <Button type="submit" size="sm">
                        Save
                      </Button>
                    </form>
                  ) : (
                    <span className="flex-1">
                      {s.name}
                      {s.isDefault && (
                        <span className="ml-2 text-xs text-muted-foreground">Default</span>
                      )}
                    </span>
                  )}
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label={`Move ${s.name} up`}
                    disabled={i === 0}
                    onClick={() => move(s, -1)}
                  >
                    <ArrowUp />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label={`Move ${s.name} down`}
                    disabled={i === list.length - 1}
                    onClick={() => move(s, 1)}
                  >
                    <ArrowDown />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      setEditing(s.id);
                      setDraft(s.name);
                    }}
                  >
                    Rename
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    aria-label={`Archive ${s.name}`}
                    onClick={() => {
                      setArchiving(s);
                      setTarget("");
                    }}
                  >
                    Archive
                  </Button>
                </li>
              ))}
            </ul>
          </div>
        );
      })}
      {archiving && (
        <div
          role="group"
          aria-label="Archive status"
          className="flex flex-wrap items-center gap-2 rounded-lg border border-border p-3 text-sm"
        >
          <span>Archive “{archiving.name}”. Move its issues to:</span>
          <select
            aria-label="Target status"
            className={selectClass}
            value={target}
            onChange={(e) => setTarget(e.target.value)}
          >
            <option value="">(none)</option>
            {states
              .filter((s) => s.id !== archiving.id)
              .map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
          </select>
          <Button
            size="sm"
            onClick={() =>
              m.archive.mutate(
                { id: archiving.id, targetStateId: target || undefined },
                { onSuccess: () => setArchiving(null) },
              )
            }
          >
            Archive
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setArchiving(null)}>
            Cancel
          </Button>
          {m.archive.isError && (
            <span role="alert" className="text-xs text-destructive">
              {errorText(m.archive.error)}
            </span>
          )}
        </div>
      )}
      <form
        aria-label="Create status"
        className="flex flex-wrap items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (!name.trim() || !COLOR_RE.test(color)) return;
          m.create.mutate(
            { teamId: team.id, name: name.trim(), type, color },
            { onSuccess: () => setName("") },
          );
        }}
      >
        <select
          aria-label="Category"
          className={selectClass}
          value={type}
          onChange={(e) => setType(e.target.value as StateType)}
        >
          {STATE_TYPE_ORDER.map((t) => (
            <option key={t} value={t}>
              {STATE_TYPE_LABEL[t]}
            </option>
          ))}
        </select>
        <Input
          aria-label="New status name"
          placeholder="New status"
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="max-w-48"
        />
        <Input
          aria-label="Color"
          value={color}
          onChange={(e) => setColor(e.target.value)}
          className="w-24"
        />
        <Button type="submit" size="sm" disabled={!name.trim() || !COLOR_RE.test(color)}>
          <Plus /> Create status
        </Button>
        {m.create.isError && (
          <span role="alert" className="text-xs text-destructive">
            {errorText(m.create.error)}
          </span>
        )}
      </form>
    </Section>
  );
}

export function LabelsSection({ team }: { team: Team }) {
  const q = useTeamLabels(team.id);
  if (q.isLoading) return <ListSkeleton rows={4} />;
  if (q.isError) return <ErrorState error={q.error} title="Could not load labels" />;
  return (
    <LabelManager
      title="Issue labels"
      description="Labels and label groups of this team."
      teamId={team.id}
      labels={(q.data ?? []).filter((l) => l.teamId === team.id)}
    />
  );
}

const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

export function CyclesSection({ team }: { team: Team }) {
  const update = useUpdateCycleSettings(team.id);
  const [enabled, setEnabled] = useState(team.cyclesEnabled);
  const [duration, setDuration] = useState(team.cycleDuration);
  const [cooldown, setCooldown] = useState(team.cycleCooldownTime);
  const [startDay, setStartDay] = useState(team.cycleStartDay);
  const [upcoming, setUpcoming] = useState(team.upcomingCycleCount);
  return (
    <Section title="Cycles" description="Time-boxed iterations for this team.">
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          // Nur geänderte Werte senden: ein unverändertes Speichern ändert nichts (und kollidiert
          // nicht mit einem von der Elternebene geerbten Zeitplan).
          const input: CycleSettings = {};
          if (enabled !== team.cyclesEnabled) input.cyclesEnabled = enabled;
          if (enabled) {
            if (duration !== team.cycleDuration) input.cycleDuration = duration;
            if (cooldown !== team.cycleCooldownTime) input.cycleCooldownTime = cooldown;
            if (startDay !== team.cycleStartDay) input.cycleStartDay = startDay;
            if (upcoming !== team.upcomingCycleCount) input.upcomingCycleCount = upcoming;
          }
          if (Object.keys(input).length === 0) return;
          update.mutate(input);
        }}
      >
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
            aria-label="Enable cycles"
          />
          Enable cycles for this team
        </label>
        {enabled && (
          <>
            <Field label="Cycle duration (weeks)">
              <select
                aria-label="Cycle duration"
                className={selectClass}
                value={duration}
                onChange={(e) => setDuration(Number(e.target.value))}
              >
                {[1, 2, 3, 4, 5, 6, 7, 8].map((w) => (
                  <option key={w} value={w}>
                    {w} {w === 1 ? "week" : "weeks"}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Cooldown (weeks)">
              <select
                aria-label="Cooldown"
                className={selectClass}
                value={cooldown}
                onChange={(e) => setCooldown(Number(e.target.value))}
              >
                {[0, 1, 2, 3, 4, 5, 6, 7, 8].map((w) => (
                  <option key={w} value={w}>
                    {w === 0 ? "None" : `${w} ${w === 1 ? "week" : "weeks"}`}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Cycle starts on">
              <select
                aria-label="Start day"
                className={selectClass}
                value={startDay}
                onChange={(e) => setStartDay(Number(e.target.value))}
              >
                {WEEKDAYS.map((d, i) => (
                  <option key={d} value={i}>
                    {d}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Upcoming cycles to create">
              <select
                aria-label="Upcoming cycles"
                className={selectClass}
                value={upcoming}
                onChange={(e) => setUpcoming(Number(e.target.value))}
              >
                {Array.from({ length: 15 }, (_, i) => i + 1).map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </Field>
          </>
        )}
        <SaveRow pending={update.isPending} error={update.error} dirty />
      </form>
    </Section>
  );
}

export function TriageSection({ team }: { team: Team }) {
  const update = useUpdateTeam(team.id);
  return (
    <Section
      title="Triage"
      description="Incoming issues wait in triage until someone accepts them."
    >
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          aria-label="Enable triage"
          checked={team.triageEnabled}
          disabled={update.isPending}
          onChange={(e) => update.mutate({ triageEnabled: e.target.checked })}
        />
        Enable triage for this team
      </label>
      {update.isError && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(update.error)}
        </p>
      )}
    </Section>
  );
}
