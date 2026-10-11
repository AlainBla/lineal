/**
 * Darstellung des Aktivitätsverlaufs (`Issue.history`, docs/03 §8). Reine Logik ohne React: aus einem Eintrag
 * werden kurze Sätze („changed status from Todo to Done“). IDs werden über einen `HistoryResolver` in Namen
 * aufgelöst; was dort nicht bekannt ist, erscheint als neutraler Platzhalter („another status“), nie als UUID.
 * Der Server schwärzt Verweise auf unsichtbare Teams, Eltern, Projekte, Cycles und Meilensteine zu `null`
 * (docs/15). Ein `null` auf Verweisfeldern ist daher mehrdeutig: fehlen beide Seiten, zeigen wir „ein privates
 * Element“; fehlt nur eine Seite, lesen wir es als „nicht gesetzt“ (der Normalfall).
 */
import type { IssueHistoryEntry } from "@/gql/graphql";

export interface HistoryResolver {
  user: (id: string) => string | null;
  state: (id: string) => string | null;
  label: (id: string) => string | null;
  project: (id: string) => string | null;
  cycle: (id: string) => string | null;
  team: (id: string) => string | null;
}

export interface DescribedEntry {
  id: string;
  createdAt: string;
  /** Anzeigename des Akteurs (bei Automationen/Systemquellen der Quellenname). */
  actor: string;
  /** Quelle, wenn nicht von Hand („API“), sonst `null`. */
  via: string | null;
  phrases: string[];
}

const PRIORITY_NAMES = ["No priority", "Urgent", "High", "Medium", "Low"];

const SOURCE_LABEL: Record<string, string> = {
  automation: "Automation",
  api: "API",
  mcp: "MCP",
  import: "Import",
  system: "Lineal",
};

const FLAG_PHRASE: Record<string, string> = {
  created: "created the issue",
  descriptionUpdated: "edited the description",
  trashed: "moved the issue to the trash",
  restored: "restored the issue",
  archived: "archived the issue",
  unarchived: "unarchived the issue",
  slaBreached: "SLA breached",
  autoClosed: "closed the issue automatically",
  autoArchived: "archived the issue automatically",
  triageResponsibilityAutoAssigned: "auto-assigned via triage responsibility",
};

const RELATION_PHRASE: Record<string, [added: string, removed: string]> = {
  blocks: ["marked as blocking", "removed blocking relation to"],
  blockedBy: ["marked as blocked by", "removed blocked-by relation to"],
  relatedTo: ["marked as related to", "removed related relation to"],
  duplicateOf: ["marked as duplicate of", "removed duplicate relation to"],
  duplicates: ["marked as original of", "removed original relation to"],
};

interface Side {
  from: unknown;
  to: unknown;
}

const sideOf = (changes: Record<string, unknown>, field: string): Side | null => {
  const v = changes[field];
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  return { from: o.from ?? null, to: o.to ?? null };
};

const text = (v: unknown): string | null => (typeof v === "string" && v !== "" ? v : null);

interface Verbs {
  set: string;
  unset: string;
  change: string;
}

/** „gesetzt / entfernt / geändert“ bei Verweisfeldern; beide Seiten `null` = geschwärzt. */
function reference(
  noun: string,
  { from, to }: Side,
  name: (id: string) => string | null,
  unknown: string,
  verbs: Verbs,
): string {
  const f = text(from);
  const t = text(to);
  const show = (id: string) => name(id) ?? unknown;
  if (f === null && t === null) return `changed the ${noun} (a private item)`;
  if (f === null && t !== null) return `${verbs.set} ${show(t)}`;
  if (f !== null && t === null) return `${verbs.unset} ${show(f)}`;
  return `${verbs.change} ${show(f!)} to ${show(t!)}`;
}

function scalar(noun: string, s: Side): string {
  if (s.to === null) return `removed the ${noun}`;
  if (s.from === null) return `set ${noun} to ${String(s.to)}`;
  return `changed ${noun} from ${String(s.from)} to ${String(s.to)}`;
}

function changePhrases(changes: Record<string, unknown>, r: HistoryResolver): string[] {
  const out: string[] = [];
  let sla = false;
  for (const field of Object.keys(changes)) {
    const s = sideOf(changes, field);
    if (!s) continue;
    const f = text(s.from);
    const t = text(s.to);
    switch (field) {
      case "title":
        out.push(`renamed the issue from “${String(s.from)}” to “${String(s.to)}”`);
        break;
      case "stateId": {
        const state = (id: string | null) => (id && r.state(id)) || "another status";
        out.push(`changed status from ${state(f)} to ${state(t)}`);
        break;
      }
      case "priority": {
        const name = (v: unknown) => PRIORITY_NAMES[Number(v)] ?? "an unknown priority";
        out.push(`changed priority from ${name(s.from)} to ${name(s.to)}`);
        break;
      }
      case "estimate":
        out.push(scalar("estimate", s));
        break;
      case "assigneeId": {
        const who = (id: string | null) => (id && r.user(id)) || "another user";
        out.push(
          f === null && t !== null
            ? `assigned to ${who(t)}`
            : f !== null && t === null
              ? `removed the assignee ${who(f)}`
              : `changed assignee from ${who(f)} to ${who(t)}`,
        );
        break;
      }
      case "delegateId":
        out.push("changed the delegate");
        break;
      case "teamId": {
        const team = (id: string | null) =>
          id ? (r.team(id) ?? "another team") : "a private team";
        out.push(`moved the issue from ${team(f)} to ${team(t)}`);
        break;
      }
      case "parentId":
        out.push(
          reference("parent", s, () => null, "a parent issue", {
            set: "made this a sub-issue of",
            unset: "removed the parent",
            change: "changed the parent from",
          }),
        );
        break;
      case "projectId":
        out.push(
          reference("project", s, r.project, "another project", {
            set: "added to project",
            unset: "removed from project",
            change: "moved from project",
          }),
        );
        break;
      case "cycleId":
        out.push(
          reference("cycle", s, r.cycle, "another cycle", {
            set: "added to",
            unset: "removed from",
            change: "moved from",
          }),
        );
        break;
      case "projectMilestoneId":
        out.push(
          reference("milestone", s, () => null, "a milestone", {
            set: "set the milestone to",
            unset: "removed the milestone",
            change: "changed the milestone from",
          }),
        );
        break;
      case "dueDate":
        out.push(scalar("due date", s));
        break;
      case "identifier":
        break; // Begleitfeld des Teamwechsels
      default:
        if (field.startsWith("sla")) sla = true;
        else out.push(`updated ${field.replace(/([A-Z])/g, " $1").toLowerCase()}`);
    }
  }
  if (sla) out.push("updated the SLA");
  return out;
}

function labelPhrase(verb: string, ids: string[], r: HistoryResolver): string[] {
  if (ids.length === 0) return [];
  const names = ids.map((id) => r.label(id) ?? "a label");
  return [`${verb} ${ids.length === 1 ? "label" : "labels"} ${names.join(", ")}`];
}

function relationPhrases(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  const out: string[] = [];
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const o = item as Record<string, unknown>;
    const identifier = text(o.identifier);
    if (!identifier) continue;
    const pair = RELATION_PHRASE[String(o.type)];
    const added = o.action !== "removed";
    out.push(
      pair
        ? `${pair[added ? 0 : 1]} ${identifier}`
        : `${added ? "added" : "removed"} relation to ${identifier}`,
    );
  }
  return out;
}

/** Beschreibt einen Verlaufseintrag; `phrases` ist leer, wenn nichts Darstellbares übrig bleibt. */
export function describeHistory(entry: IssueHistoryEntry, r: HistoryResolver): DescribedEntry {
  const changes =
    entry.changes && typeof entry.changes === "object"
      ? (entry.changes as Record<string, unknown>)
      : {};
  const phrases = [
    ...entry.flags.flatMap((f) => (FLAG_PHRASE[f] ? [FLAG_PHRASE[f]] : [])),
    ...changePhrases(changes, r),
    ...labelPhrase("added", entry.addedLabelIds, r),
    ...labelPhrase("removed", entry.removedLabelIds, r),
    ...relationPhrases(entry.relationChanges),
  ];
  const sourceLabel = SOURCE_LABEL[entry.source] ?? null;
  const actorName = entry.actorId ? r.user(entry.actorId) : null;
  return {
    id: entry.id,
    createdAt: entry.createdAt,
    actor: actorName ?? (entry.actorId ? "Someone" : (sourceLabel ?? "Someone")),
    via: sourceLabel && actorName ? sourceLabel : null,
    phrases,
  };
}
