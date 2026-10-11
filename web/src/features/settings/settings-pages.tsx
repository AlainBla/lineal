import { useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import type { Organization } from "@/gql/graphql";

import { useSettingsTeam, useUpdateWorkspace, useWorkspaceSettings } from "./queries";
import {
  CyclesSection,
  GeneralSection,
  LabelsSection,
  MembersSection,
  StatusesSection,
  TriageSection,
} from "./team-sections";
import { Field, Section, SettingsShell } from "./ui";

export const TEAM_SECTIONS = [
  ["general", "General"],
  ["members", "Members"],
  ["statuses", "Issue statuses"],
  ["issue-labels", "Issue labels"],
  ["cycles", "Cycles"],
  ["triage", "Triage"],
] as const;
export type TeamSection = (typeof TEAM_SECTIONS)[number][0];
export const isTeamSection = (v: string): v is TeamSection => TEAM_SECTIONS.some(([s]) => s === v);

/** Team-Settings (`/{ws}/settings/teams/{KEY}/{section}`, docs/08 §2.11). */
export function TeamSettingsPage({
  ws,
  teamKey,
  section,
}: {
  ws: string;
  teamKey: string;
  section: TeamSection;
}) {
  const q = useSettingsTeam(teamKey);
  const nav = TEAM_SECTIONS.map(([s, label]) => ({
    label,
    to: `/${ws}/settings/teams/${teamKey}/${s}`,
    active: s === section,
  }));
  let body;
  if (q.isLoading) body = <ListSkeleton rows={4} />;
  else if (q.isError || !q.data)
    body = (
      <ErrorState error={q.error} title="Could not load team" onRetry={() => void q.refetch()} />
    );
  else {
    const team = q.data;
    // `key` setzt lokale Formularzustände zurück, wenn sich die gespeicherten Werte ändern.
    const k = `${team.id}:${team.updatedAt ?? ""}:${team.name}:${team.key}:${team.cyclesEnabled}:${team.triageEnabled}`;
    body = (
      <>
        {section === "general" && <GeneralSection key={k} team={team} />}
        {section === "members" && <MembersSection team={team} />}
        {section === "statuses" && <StatusesSection team={team} />}
        {section === "issue-labels" && <LabelsSection team={team} />}
        {section === "cycles" && <CyclesSection key={k} team={team} />}
        {section === "triage" && <TriageSection team={team} />}
      </>
    );
  }
  return (
    <SettingsShell title={q.data?.name ?? teamKey} nav={nav}>
      {body}
    </SettingsShell>
  );
}

function WorkspaceForm({ ws }: { ws: Organization }) {
  const update = useUpdateWorkspace();
  const [name, setName] = useState(ws.name);
  const [month, setMonth] = useState(ws.fiscalYearStartMonth);
  const [initiatives, setInitiatives] = useState(ws.initiativesEnabled);
  const dirty =
    name !== ws.name || month !== ws.fiscalYearStartMonth || initiatives !== ws.initiativesEnabled;
  return (
    <Section title="Workspace" description="Name and basic settings of your workspace.">
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          update.mutate({
            ...(name !== ws.name ? { name } : {}),
            ...(month !== ws.fiscalYearStartMonth ? { fiscalYearStartMonth: month } : {}),
            ...(initiatives !== ws.initiativesEnabled ? { initiativesEnabled: initiatives } : {}),
          });
        }}
      >
        <Field label="Name">
          <Input
            aria-label="Workspace name"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </Field>
        <Field label="URL">
          <Input aria-label="Workspace URL" value={ws.urlKey} readOnly disabled />
        </Field>
        <Field label="Fiscal year start (month 1–12)">
          <Input
            aria-label="Fiscal year start"
            type="number"
            min={1}
            max={12}
            value={month}
            onChange={(e) => setMonth(Math.min(12, Math.max(1, Number(e.target.value) || 1)))}
          />
        </Field>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            aria-label="Enable initiatives"
            checked={initiatives}
            onChange={(e) => setInitiatives(e.target.checked)}
          />
          Enable initiatives
        </label>
        <div className="flex items-center gap-3">
          <Button type="submit" size="sm" disabled={!dirty || update.isPending}>
            Save
          </Button>
          {update.isError && (
            <span role="alert" className="text-xs text-destructive">
              {errorText(update.error)}
            </span>
          )}
        </div>
      </form>
    </Section>
  );
}

/** Workspace-Settings-Grundgerüst (`/{ws}/settings/workspace`): Stammdaten und Team-Übersicht. */
export function WorkspaceSettingsPage({ ws }: { ws: string }) {
  const q = useWorkspaceSettings();
  const session = useSession();
  const nav = [
    { label: "Workspace", to: `/${ws}/settings/workspace`, active: true },
    { label: "API", to: `/${ws}/settings/api`, active: false },
    ...(session?.teams ?? []).map((t) => ({
      label: `Team · ${t.name}`,
      to: `/${ws}/settings/teams/${t.key}/general`,
      active: false,
    })),
  ];
  return (
    <SettingsShell title="Settings" nav={nav}>
      {q.isLoading && <ListSkeleton rows={4} />}
      {q.isError && (
        <ErrorState
          error={q.error}
          title="Could not load workspace"
          onRetry={() => void q.refetch()}
        />
      )}
      {q.data && <WorkspaceForm key={q.data.updatedAt} ws={q.data} />}
    </SettingsShell>
  );
}
