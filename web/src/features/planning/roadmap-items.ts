import type { Initiative, Project } from "@/gql/graphql";

import { asResolution, type RoadmapItem } from "./roadmap-model";

const COLOR = /^#[0-9A-Fa-f]{6}$/;

/** Projekte → Roadmap-Elemente (Meilensteine ohne Zieldatum entfallen). */
export function projectItems(projects: Project[], ws: string): RoadmapItem[] {
  return projects
    .filter((p) => !p.archivedAt)
    .map((p) => ({
      id: p.id,
      kind: "project" as const,
      name: p.name,
      href: `/${ws}/project/${p.identifier}/overview`,
      color: COLOR.test(p.color) ? p.color : null,
      status: p.status.name,
      team: p.leadTeam?.name ?? p.teams[0]?.name ?? null,
      initiatives: p.initiatives.map((i) => ({ id: i.id, name: i.name })),
      span: {
        startDate: p.startDate?.slice(0, 10) ?? null,
        targetDate: p.targetDate?.slice(0, 10) ?? null,
        startRes: asResolution(p.startDateResolution),
        targetRes: asResolution(p.targetDateResolution),
      },
      startEditable: true,
      milestones: p.milestones
        .filter((m) => !!m.targetDate)
        .map((m) => ({ id: m.id, name: m.name, targetDate: m.targetDate!.slice(0, 10) })),
    }));
}

/**
 * Initiativen → Roadmap-Elemente. Initiativen haben nur ein Zieldatum (docs/04 §4.1); der Balken
 * beginnt beim ersten Wechsel nach „Active“ (`startedAt`), sonst bei der Anlage.
 */
export function initiativeItems(initiatives: Initiative[], ws: string): RoadmapItem[] {
  return initiatives
    .filter((i) => !i.archivedAt)
    .map((i) => ({
      id: i.id,
      kind: "initiative" as const,
      name: i.name,
      href: `/${ws}/initiative/${i.identifier}/overview`,
      color: COLOR.test(i.color ?? "") ? i.color : null,
      status: i.status,
      team: i.leadTeam?.name ?? null,
      initiatives: i.parentInitiatives.map((p) => ({ id: p.id, name: p.name })),
      span: {
        startDate: i.targetDate ? (i.startedAt ?? i.createdAt).slice(0, 10) : null,
        targetDate: i.targetDate?.slice(0, 10) ?? null,
        startRes: null,
        targetRes: asResolution(i.targetDateResolution),
      },
      startEditable: false,
      milestones: [],
    }));
}
