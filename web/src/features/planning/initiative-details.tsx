import { useNavigate } from "@tanstack/react-router";

import { Button } from "@/components/ui/button";
import { errorText } from "@/features/issues/error-text";
import { useTeams } from "@/features/issues/hooks";
import { useKnownUsers } from "@/features/issues/users";
import type { Initiative } from "@/gql/graphql";

import { PlanningDescription } from "./description-editor";
import { DateField, INITIATIVE_STATUSES, SelectField, TextField } from "./fields";
import { useInitiativeLifecycle, useUpdateInitiative } from "./queries";

/**
 * Bearbeitbare Eigenschaften einer Initiative (docs/04 §4). Änderungen gehen sofort an den Server;
 * seine Regeln und Rechte gelten, Fehler erscheinen unter den Feldern.
 */
export function InitiativeDetails({ initiative: i }: { initiative: Initiative }) {
  const update = useUpdateInitiative();
  const users = useKnownUsers();
  const teams = useTeams();
  const save = (input: Record<string, unknown>) => update.mutate({ id: i.id, input });
  return (
    <div className="flex flex-col gap-4">
      <TextField
        label="Initiative name"
        value={i.name}
        required
        maxLength={255}
        onCommit={(name) => save({ name })}
      />
      <TextField
        label="Summary"
        value={i.summary ?? ""}
        maxLength={255}
        placeholder="A short summary"
        onCommit={(summary) => save({ summary })}
      />
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-3">
        <SelectField
          label="Status"
          value={i.status.toLowerCase()}
          options={INITIATIVE_STATUSES}
          onChange={(status) => save({ status })}
        />
        <SelectField
          label="Owner"
          value={i.owner?.id ?? ""}
          emptyLabel="No owner"
          options={users.map((u) => ({ value: u.id, label: u.name }))}
          onChange={(owner) => save({ owner: owner || null })}
        />
        <SelectField
          label="Lead team"
          value={i.leadTeam?.id ?? ""}
          emptyLabel="No lead team"
          options={(teams.data ?? []).map((t) => ({ value: t.id, label: `${t.name} (${t.key})` }))}
          onChange={(leadTeam) => save({ leadTeam: leadTeam || null })}
        />
      </div>
      <DateField
        label="Target date"
        date={i.targetDate}
        resolution={i.targetDateResolution}
        onChange={(targetDate, targetDateResolution) => save({ targetDate, targetDateResolution })}
      />
      {update.isError && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(update.error)}
        </p>
      )}
      <section aria-label="Description">
        <h2 className="mb-2 text-xs font-medium text-muted-foreground">Description</h2>
        <PlanningDescription
          kind="initiative"
          id={i.id}
          value={i.description ?? ""}
          onSave={(description) => update.mutateAsync({ id: i.id, input: { description } })}
        />
      </section>
    </div>
  );
}

export function InitiativeActions({ initiative: i, ws }: { initiative: Initiative; ws: string }) {
  const { archive, remove } = useInitiativeLifecycle();
  const navigate = useNavigate();
  const archived = !!i.archivedAt;
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
          if (archived || window.confirm(`Archive initiative "${i.name}"?`))
            archive.mutate({ id: i.id, archived: !archived });
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
              `Delete initiative "${i.name}"? It moves to Recently deleted and can be restored for 30 days.`,
            )
          )
            remove.mutate(i.id, {
              onSuccess: () => void navigate({ to: `/${ws}/initiatives/active` as "/" }),
            });
        }}
      >
        Delete
      </Button>
    </>
  );
}
