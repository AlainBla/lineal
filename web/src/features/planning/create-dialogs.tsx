/**
 * „New project“ (N P) und „New initiative“ (N I) als Dialoge. Der Host hängt einmal in der Shell,
 * registriert die Kürzel samt Command-Menü-Einträgen und zeigt die Dialoge; Buttons der Listen und
 * Detailseiten öffnen sie über `openCreateProject`/`openCreateInitiative`.
 */
import { useNavigate } from "@tanstack/react-router";
import { Box, Target } from "lucide-react";
import { useRef, useState, useSyncExternalStore } from "react";

import { useActions } from "@/app/commands";
import { useSession } from "@/app/auth";
import { teamKeyFromPath } from "@/app/nav";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { errorText } from "@/features/issues/error-text";
import { useTeams } from "@/features/issues/hooks";
import { PRIORITIES } from "@/features/issues/model";
import { useKnownUsers } from "@/features/issues/users";
import { useShortcutContext } from "@/lib/shortcuts";

import { DateField, FieldBox, INITIATIVE_STATUSES, PROJECT_STATUS_TYPES } from "./fields";
import { SelectField, TextField } from "./fields";
import { useCreateInitiative, useCreateProject } from "./queries";

export interface ProjectPreset {
  /** Team-Key aus dem Seitenkontext (Teamseite, Projektliste des Teams). */
  teamKey?: string;
  /** Initiative (UUID/Kennung), der das neue Projekt zugeordnet wird. */
  initiativeRef?: string;
}

interface DialogState {
  project: { open: boolean; preset: ProjectPreset };
  initiative: boolean;
}

let state: DialogState = { project: { open: false, preset: {} }, initiative: false };
const listeners = new Set<() => void>();
const set = (next: DialogState) => {
  state = next;
  listeners.forEach((l) => l());
};
const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};
const snapshot = () => state;

export const openCreateProject = (preset: ProjectPreset = {}) =>
  set({ ...state, project: { open: true, preset } });
export const closeCreateProject = () => set({ ...state, project: { open: false, preset: {} } });
export const openCreateInitiative = () => set({ ...state, initiative: true });
export const closeCreateInitiative = () => set({ ...state, initiative: false });

function NewProjectForm({ preset, onClose }: { preset: ProjectPreset; onClose: () => void }) {
  const session = useSession();
  const navigate = useNavigate();
  const teams = useTeams();
  const users = useKnownUsers();
  const create = useCreateProject();
  const busy = useRef(false);

  const [name, setName] = useState("");
  const [summary, setSummary] = useState("");
  const [picked, setPicked] = useState<string[] | null>(null);
  const [leadTeam, setLeadTeam] = useState("");
  const [status, setStatus] = useState("backlog");
  const [lead, setLead] = useState("");
  const [priority, setPriority] = useState("0");
  const [start, setStart] = useState<{ date: string | null; res: string | null }>({
    date: null,
    res: null,
  });
  const [target, setTarget] = useState<{ date: string | null; res: string | null }>({
    date: null,
    res: null,
  });

  const presetTeam = (teams.data ?? []).find((t) => t.key === preset.teamKey);
  const selected = picked ?? (presetTeam ? [presetTeam.id] : []);
  const effectiveLead = selected.includes(leadTeam) ? leadTeam : (selected[0] ?? "");
  const valid = name.trim() !== "" && selected.length > 0;

  return (
    <form
      className="flex flex-col gap-3 p-4"
      onSubmit={(e) => {
        e.preventDefault();
        if (!valid || create.isPending || busy.current) return;
        busy.current = true;
        create.mutate(
          {
            name: name.trim(),
            ...(summary.trim() ? { summary: summary.trim() } : {}),
            addTeams: selected,
            leadTeam: effectiveLead,
            state: status,
            ...(lead ? { lead } : {}),
            ...(priority !== "0" ? { priority: Number(priority) } : {}),
            ...(start.date ? { startDate: start.date, startDateResolution: start.res } : {}),
            ...(target.date ? { targetDate: target.date, targetDateResolution: target.res } : {}),
            ...(preset.initiativeRef ? { addInitiatives: [preset.initiativeRef] } : {}),
          },
          {
            onError: () => {
              busy.current = false;
            },
            onSuccess: (project) => {
              onClose();
              if (session)
                void navigate({
                  to: `/${session.workspace.urlKey}/project/${project.identifier}/overview` as "/",
                });
            },
          },
        );
      }}
    >
      <FieldBox label="Name">
        <input
          aria-label="Project name"
          autoFocus
          maxLength={255}
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="h-9 rounded-md border border-input bg-transparent px-3 text-sm outline-none focus-visible:border-ring"
        />
      </FieldBox>
      <TextField label="Summary" value={summary} maxLength={255} onCommit={setSummary} />
      <FieldBox label="Teams">
        <ul aria-label="Teams" className="flex max-h-32 flex-col gap-1 overflow-auto text-sm">
          {(teams.data ?? []).map((t) => (
            <li key={t.id}>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={selected.includes(t.id)}
                  onChange={(e) =>
                    setPicked(
                      e.target.checked ? [...selected, t.id] : selected.filter((x) => x !== t.id),
                    )
                  }
                />
                {t.name} ({t.key})
              </label>
            </li>
          ))}
        </ul>
      </FieldBox>
      <div className="grid grid-cols-2 gap-3">
        <SelectField
          label="Lead team"
          value={effectiveLead}
          options={(teams.data ?? [])
            .filter((t) => selected.includes(t.id))
            .map((t) => ({ value: t.id, label: t.name }))}
          onChange={setLeadTeam}
        />
        <SelectField
          label="Status"
          value={status}
          options={PROJECT_STATUS_TYPES}
          onChange={setStatus}
        />
        <SelectField
          label="Lead"
          value={lead}
          emptyLabel="No lead"
          options={users.map((u) => ({ value: u.id, label: u.name }))}
          onChange={setLead}
        />
        <SelectField
          label="Priority"
          value={priority}
          options={PRIORITIES.map((x) => ({ value: String(x.value), label: x.label }))}
          onChange={setPriority}
        />
        <DateField
          label="Start date"
          date={start.date}
          resolution={start.res}
          onChange={(date, res) => setStart({ date, res })}
        />
        <DateField
          label="Target date"
          date={target.date}
          resolution={target.res}
          onChange={(date, res) => setTarget({ date, res })}
        />
      </div>
      <div className="flex items-center gap-3 pt-1">
        <Button type="submit" size="sm" disabled={!valid || create.isPending}>
          Create project
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onClose}>
          Cancel
        </Button>
        {create.isError && (
          <span role="alert" className="text-xs text-destructive">
            {errorText(create.error)}
          </span>
        )}
      </div>
    </form>
  );
}

function NewInitiativeForm({ onClose }: { onClose: () => void }) {
  const session = useSession();
  const navigate = useNavigate();
  const users = useKnownUsers();
  const teams = useTeams();
  const [leadTeam, setLeadTeam] = useState("");
  const create = useCreateInitiative();
  const busy = useRef(false);
  const [name, setName] = useState("");
  const [summary, setSummary] = useState("");
  const [status, setStatus] = useState("planned");
  const [owner, setOwner] = useState("");
  const [target, setTarget] = useState<{ date: string | null; res: string | null }>({
    date: null,
    res: null,
  });
  const valid = name.trim() !== "";
  return (
    <form
      className="flex flex-col gap-3 p-4"
      onSubmit={(e) => {
        e.preventDefault();
        if (!valid || create.isPending || busy.current) return;
        busy.current = true;
        create.mutate(
          {
            name: name.trim(),
            ...(summary.trim() ? { summary: summary.trim() } : {}),
            status,
            ...(owner ? { owner } : {}),
            ...(leadTeam ? { leadTeam } : {}),
            ...(target.date ? { targetDate: target.date, targetDateResolution: target.res } : {}),
          },
          {
            onError: () => {
              busy.current = false;
            },
            onSuccess: (initiative) => {
              onClose();
              if (session)
                void navigate({
                  to: `/${session.workspace.urlKey}/initiative/${initiative.identifier}/overview` as "/",
                });
            },
          },
        );
      }}
    >
      <FieldBox label="Name">
        <input
          aria-label="Initiative name"
          autoFocus
          maxLength={255}
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="h-9 rounded-md border border-input bg-transparent px-3 text-sm outline-none focus-visible:border-ring"
        />
      </FieldBox>
      <TextField label="Summary" value={summary} maxLength={255} onCommit={setSummary} />
      <div className="grid grid-cols-2 gap-3">
        <SelectField
          label="Status"
          value={status}
          options={INITIATIVE_STATUSES}
          onChange={setStatus}
        />
        <SelectField
          label="Owner"
          value={owner}
          emptyLabel="No owner"
          options={users.map((u) => ({ value: u.id, label: u.name }))}
          onChange={setOwner}
        />
        <SelectField
          label="Lead team"
          value={leadTeam}
          emptyLabel="No lead team"
          options={(teams.data ?? []).map((t) => ({ value: t.id, label: `${t.name} (${t.key})` }))}
          onChange={setLeadTeam}
        />
        <DateField
          label="Target date"
          date={target.date}
          resolution={target.res}
          onChange={(date, res) => setTarget({ date, res })}
        />
      </div>
      <div className="flex items-center gap-3 pt-1">
        <Button type="submit" size="sm" disabled={!valid || create.isPending}>
          Create initiative
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onClose}>
          Cancel
        </Button>
        {create.isError && (
          <span role="alert" className="text-xs text-destructive">
            {errorText(create.error)}
          </span>
        )}
      </div>
    </form>
  );
}

/** In der Shell einhängen: Kürzel N P / N I, Command-Menü-Einträge und beide Dialoge. */
export function PlanningDialogHost() {
  const s = useSyncExternalStore(subscribe, snapshot, snapshot);
  useShortcutContext("modal", s.project.open || s.initiative);
  useActions([
    {
      id: "project.create",
      title: "New project",
      section: "Planning",
      keys: "n p",
      icon: Box,
      perform: () => {
        const path = window.location.pathname;
        openCreateProject({
          teamKey: teamKeyFromPath(path) ?? undefined,
          initiativeRef: /^\/[^/]+\/initiative\/([^/]+)/.exec(path)?.[1],
        });
      },
    },
    {
      id: "initiative.create",
      title: "New initiative",
      section: "Planning",
      keys: "n i",
      icon: Target,
      perform: openCreateInitiative,
    },
  ]);
  return (
    <>
      <Dialog open={s.project.open} onOpenChange={(o) => !o && closeCreateProject()}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>New project</DialogTitle>
            <DialogDescription>
              A project groups issues of one or more teams towards a goal.
            </DialogDescription>
          </DialogHeader>
          {s.project.open && (
            <NewProjectForm preset={s.project.preset} onClose={closeCreateProject} />
          )}
        </DialogContent>
      </Dialog>
      <Dialog open={s.initiative} onOpenChange={(o) => !o && closeCreateInitiative()}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>New initiative</DialogTitle>
            <DialogDescription>
              An initiative bundles projects towards a strategic goal.
            </DialogDescription>
          </DialogHeader>
          {s.initiative && <NewInitiativeForm onClose={closeCreateInitiative} />}
        </DialogContent>
      </Dialog>
    </>
  );
}
