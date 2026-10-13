import { Link, useNavigate } from "@tanstack/react-router";

import { Button } from "@/components/ui/button";
import { errorText } from "@/features/issues/error-text";
import { useTeams } from "@/features/issues/hooks";
import { PRIORITIES } from "@/features/issues/model";
import { useKnownUsers } from "@/features/issues/users";
import type { Project } from "@/gql/graphql";

import { PlanningDescription } from "./description-editor";
import {
  DateField,
  FieldBox,
  PROJECT_STATUS_TYPES,
  RefList,
  SelectField,
  TextField,
} from "./fields";
import { useProjectLabels, useProjectLifecycle, useUpdateProject } from "./queries";

/**
 * Bearbeitbare Eigenschaften eines Projekts (docs/08 §2.7, F10). Jede Änderung geht sofort als
 * `projectUpdate` an den Server; dessen Regeln (Teams, Lead-Team, Status, Daten) und Rechte gelten,
 * Fehler erscheinen direkt unter den Feldern.
 */
export function ProjectDetails({ project: p, ws }: { project: Project; ws: string }) {
  const update = useUpdateProject();
  const teams = useTeams();
  const users = useKnownUsers();
  const labels = useProjectLabels();
  const save = (input: Record<string, unknown>) => update.mutate({ id: p.id, input });
  const teamOptions = (teams.data ?? []).map((t) => ({
    value: t.id,
    label: `${t.name} (${t.key})`,
  }));
  const userOptions = users.map((u) => ({ value: u.id, label: u.name }));

  return (
    <div className="flex flex-col gap-4">
      <TextField
        label="Project name"
        value={p.name}
        required
        maxLength={255}
        onCommit={(name) => save({ name })}
      />
      <TextField
        label="Summary"
        value={p.summary ?? ""}
        maxLength={255}
        placeholder="A short summary"
        onCommit={(summary) => save({ summary })}
      />
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <SelectField
          label="Status"
          value={p.status.type}
          options={PROJECT_STATUS_TYPES}
          onChange={(state) => save({ state })}
        />
        <SelectField
          label="Priority"
          value={String(p.priority)}
          options={PRIORITIES.map((x) => ({ value: String(x.value), label: x.label }))}
          onChange={(v) => save({ priority: Number(v) })}
        />
        <SelectField
          label="Lead"
          value={p.lead?.id ?? ""}
          emptyLabel="No lead"
          options={userOptions}
          onChange={(lead) => save({ lead: lead || null })}
        />
        <SelectField
          label="Lead team"
          value={p.leadTeam?.id ?? ""}
          options={p.teams.map((t) => ({ value: t.id, label: t.name }))}
          emptyLabel={p.leadTeam ? undefined : "No access to the lead team"}
          onChange={(leadTeam) => leadTeam && save({ leadTeam })}
        />
      </div>
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <DateField
          label="Start date"
          date={p.startDate}
          resolution={p.startDateResolution}
          onChange={(startDate, startDateResolution) => save({ startDate, startDateResolution })}
        />
        <DateField
          label="Target date"
          date={p.targetDate}
          resolution={p.targetDateResolution}
          onChange={(targetDate, targetDateResolution) =>
            save({ targetDate, targetDateResolution })
          }
        />
      </div>
      <RefList
        label="Teams"
        items={p.teams}
        choices={teamOptions}
        addLabel="Add team…"
        busy={update.isPending}
        onAdd={(id) => save({ addTeams: [id] })}
        onRemove={(id) => save({ removeTeams: [id] })}
      />
      <RefList
        label="Labels"
        items={p.labels}
        choices={(labels.data ?? []).map((l) => ({ value: l.id, label: l.name }))}
        addLabel="Add label…"
        busy={update.isPending}
        onAdd={(id) => save({ labels: [...p.labels.map((l) => l.id), id] })}
        onRemove={(id) => save({ labels: p.labels.filter((l) => l.id !== id).map((l) => l.id) })}
      />
      <RefList
        label="Members"
        items={p.members}
        choices={userOptions}
        addLabel="Add member…"
        busy={update.isPending}
        onAdd={(id) => save({ addMembers: [id] })}
        onRemove={(id) => save({ removeMembers: [id] })}
      />
      <FieldBox label="Initiatives">
        {p.initiatives.length === 0 ? (
          <span className="text-sm text-muted-foreground">None</span>
        ) : (
          <ul className="flex flex-wrap gap-2 text-sm">
            {p.initiatives.map((i) => (
              <li key={i.id}>
                <Link to={`/${ws}/initiative/${i.id}/overview` as "/"} className="hover:underline">
                  {i.name}
                </Link>
              </li>
            ))}
          </ul>
        )}
      </FieldBox>
      {update.isError && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(update.error)}
        </p>
      )}
      <section aria-label="Description">
        <h2 className="mb-2 text-xs font-medium text-muted-foreground">Description</h2>
        <PlanningDescription
          kind="project"
          id={p.id}
          value={p.description ?? ""}
          onSave={(description) => update.mutateAsync({ id: p.id, input: { description } })}
        />
      </section>
    </div>
  );
}

/** Archivieren und Löschen im Seitenkopf; beides fragt nach, der Server prüft die Rechte. */
export function ProjectActions({ project: p, ws }: { project: Project; ws: string }) {
  const { archive, remove } = useProjectLifecycle();
  const navigate = useNavigate();
  const archived = !!p.archivedAt;
  const error = archive.error ?? remove.error;
  return (
    <>
      {error && (
        <span role="alert" className="text-xs text-destructive">
          {errorText(error)}
        </span>
      )}
      <Button
        variant="ghost"
        size="sm"
        disabled={archive.isPending}
        onClick={() => {
          if (archived || window.confirm(`Archive project "${p.name}"? It becomes read-only.`))
            archive.mutate({ id: p.id, archived: !archived });
        }}
      >
        {archived ? "Unarchive" : "Archive"}
      </Button>
      <Button
        variant="ghost"
        size="sm"
        disabled={remove.isPending}
        onClick={() => {
          if (
            window.confirm(
              `Delete project "${p.name}"? It moves to Recently deleted and can be restored for 30 days.`,
            )
          )
            remove.mutate(p.id, {
              onSuccess: () => void navigate({ to: `/${ws}/projects/all` as "/" }),
            });
        }}
      >
        Delete
      </Button>
    </>
  );
}
