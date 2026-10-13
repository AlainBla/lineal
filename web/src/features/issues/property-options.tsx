/** Optionslisten der Property-Picker (Status, Priority, Assignee, Labels, Estimate, Projekt, Cycle, Team). */
import type { Cycle, IssueLabel, WorkflowState } from "@/gql/graphql";

import { Avatar, LabelDot, PriorityIcon, StatusIcon } from "./icons";
import { estimateOptions, PRIORITIES, sortStates } from "./model";
import type { PickerOption } from "./picker";
import type { TeamMeta } from "./queries";
import type { KnownUser } from "./users";

export const NONE = "__none__";

export function statusOptions(states: WorkflowState[]): PickerOption[] {
  return sortStates(states)
    .filter((s) => s.type !== "duplicate" && s.type !== "triage")
    .map((s) => ({
      id: s.id,
      label: s.name,
      icon: <StatusIcon type={s.type} color={s.color} />,
    }));
}

export function priorityOptions(): PickerOption[] {
  return PRIORITIES.map((p) => ({
    id: String(p.value),
    label: p.label,
    icon: <PriorityIcon priority={p.value} />,
    digit: String(p.value),
  }));
}

export function assigneeOptions(users: KnownUser[], me: KnownUser | null): PickerOption[] {
  const rest = users.filter((u) => u.id !== me?.id);
  return [
    { id: NONE, label: "No assignee", icon: <Avatar name="" /> },
    ...(me
      ? [{ id: me.id, label: `${me.name} (assign to me)`, icon: <Avatar name={me.name} /> }]
      : []),
    ...rest.map((u) => ({ id: u.id, label: u.name, icon: <Avatar name={u.name} /> })),
  ];
}

/** Labels; Gruppen sind nicht wählbar, ihre Kinder erscheinen als „Gruppe/Label“. */
export function labelOptions(labels: IssueLabel[]): PickerOption[] {
  const byId = new Map(labels.map((l) => [l.id, l]));
  return labels
    .filter((l) => !l.isGroup)
    .map((l) => {
      const parent = l.parentId ? byId.get(l.parentId) : undefined;
      return {
        id: l.id,
        label: parent ? `${parent.name}/${l.name}` : l.name,
        icon: <LabelDot color={l.color} />,
        keywords: [l.name],
      };
    });
}

export function estimatePickerOptions(type: string): PickerOption[] {
  return [
    { id: NONE, label: "No estimate" },
    ...estimateOptions(type).map((o) => ({ id: String(o.value), label: o.label })),
  ];
}

export function projectOptions(projects: TeamMeta["projects"]): PickerOption[] {
  return [
    { id: NONE, label: "No project" },
    ...projects.map((p) => ({
      id: p.id,
      label: p.name,
      icon: <LabelDot color={p.color} />,
    })),
  ];
}

/** Aktive und künftige Cycles (abgeschlossene sind nicht wählbar, docs/08 F4). */
export function cycleOptions(cycles: Cycle[]): PickerOption[] {
  return [
    { id: NONE, label: "No cycle" },
    ...cycles
      .filter((c) => !c.isPast)
      .map((c) => ({
        id: c.id,
        label: c.name ? `Cycle ${c.number} · ${c.name}` : `Cycle ${c.number}`,
        hint: c.isActive ? "Current" : c.isNext ? "Upcoming" : undefined,
      })),
  ];
}

export function teamOptions(
  teams: Array<{ id: string; key: string; name: string }>,
): PickerOption[] {
  return teams.map((t) => ({
    id: t.id,
    label: `${t.name} (${t.key})`,
    keywords: [t.key],
  }));
}
