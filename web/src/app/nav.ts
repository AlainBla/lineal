/**
 * Navigationsziele der Shell. Die Pfade folgen dem URL-Schema aus docs/08 §1.3 (`/{ws}/…`).
 * Die meisten Ziele werden erst von späteren WPs gebaut; bis dahin zeigt die Shell eine Platzhalterseite.
 */

export interface WorkspaceNav {
  inbox: string;
  myIssues: string;
  drafts: string;
  search: string;
  overview: string;
  issues: string;
  workload: string;
  cycles: string;
  projects: string;
  initiatives: string;
  views: string;
  trash: string;
  settings: string;
}

export function workspacePaths(ws: string): WorkspaceNav {
  return {
    inbox: `/${ws}/inbox`,
    myIssues: `/${ws}/my-issues/assigned`,
    drafts: `/${ws}/drafts`,
    search: `/${ws}/search`,
    overview: `/${ws}/overview`,
    issues: `/${ws}/issues`,
    workload: `/${ws}/workload`,
    cycles: `/${ws}/cycles`,
    projects: `/${ws}/projects/all`,
    initiatives: `/${ws}/initiatives/active`,
    views: `/${ws}/views/issues`,
    trash: `/${ws}/trash/issues`,
    settings: `/${ws}/settings/workspace`,
  };
}

export interface TeamNav {
  home: string;
  triage: string;
  active: string;
  backlog: string;
  all: string;
  archive: string;
  cycles: string;
  currentCycle: string;
  upcomingCycle: string;
  projects: string;
  views: string;
  trash: string;
  settings: string;
}

export function teamPaths(ws: string, key: string): TeamNav {
  const base = `/${ws}/team/${key}`;
  return {
    home: `${base}/overview`,
    triage: `${base}/triage`,
    active: `${base}/active`,
    backlog: `${base}/backlog`,
    all: `${base}/all`,
    archive: `${base}/archive`,
    cycles: `${base}/cycles`,
    currentCycle: `${base}/cycle/active`,
    upcomingCycle: `${base}/cycle/upcoming`,
    projects: `${base}/projects/all`,
    views: `${base}/views/issues`,
    trash: `${base}/trash`,
    settings: `/${ws}/settings/teams/${key}/general`,
  };
}

/** Team-Key aus dem aktuellen Pfad (`/acme/team/ENG/all` → `ENG`), sonst `null`. */
export function teamKeyFromPath(pathname: string): string | null {
  return /^\/[^/]+\/team\/([^/]+)/.exec(pathname)?.[1] ?? null;
}

const SEGMENT_TITLES: Record<string, string> = {
  inbox: "Inbox",
  "my-issues": "My issues",
  drafts: "Drafts",
  search: "Search",
  overview: "Overview",
  issues: "Issues",
  workload: "Workload",
  cycles: "Cycles",
  cycle: "Cycle",
  projects: "Projects",
  project: "Project",
  initiatives: "Initiatives",
  initiative: "Initiative",
  views: "Views",
  view: "View",
  settings: "Settings",
  triage: "Triage",
  active: "Active",
  backlog: "Backlog",
  all: "All issues",
  archive: "Archive",
  assigned: "Assigned",
  trash: "Recently deleted",
};

const titleCase = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/-/g, " ");

/** Objektseiten, deren Folgesegmente Kennung und Slug (aus dem Titel) enthalten. */
const OBJECT_SEGMENTS = new Set(["issue", "document", "project", "initiative", "view"]);
/** Kennungen wie `ENG-13`, `P-ENG-3` oder Team-Keys. */
const IDENTIFIER = /^[A-Z][A-Z0-9]*(-[A-Z0-9]+)*$/;

/**
 * Breadcrumb-Segmente für einen Pfad, ohne Workspace-Präfix: `/acme/team/ENG/all` → `["ENG", "All issues"]`.
 * Nach Objektsegmenten (`issue`, `document`, …) erscheinen nur Kennungen und bekannte Tabs, nie der
 * Slug: Er stammt aus dem Titel und würde ihn sonst auch dann zeigen, wenn der Zugriff fehlt.
 */
export function breadcrumbFor(pathname: string): string[] {
  const parts = pathname.split("/").filter(Boolean).slice(1); // Workspace-Präfix weg
  const out: string[] = [];
  let inObject = false;
  for (let i = 0; i < parts.length; i++) {
    const p = parts[i]!;
    if (p === "team") continue;
    const known = SEGMENT_TITLES[p];
    if (inObject) {
      if (known) out.push(known);
      else if (IDENTIFIER.test(p)) out.push(p);
      continue;
    }
    if (OBJECT_SEGMENTS.has(p)) inObject = true;
    out.push(known ?? (IDENTIFIER.test(p) ? p : titleCase(decodeURIComponent(p))));
  }
  return out;
}

/** Seitentitel für Platzhalter und Dokumenttitel. */
export function titleForPath(pathname: string): string {
  const crumbs = breadcrumbFor(pathname);
  return crumbs.length ? crumbs.join(" › ") : "Home";
}

/** In der Navigation verlinkte, aber noch nicht gebaute Seiten: Platzhalter statt 404. */
const PLANNED_PATHS = [
  /^\/[^/]+\/drafts\/?$/,
  /^\/[^/]+\/team\/[^/]+\/(overview|triage|archive)\/?$/,
];

export const isPlannedPath = (pathname: string): boolean =>
  PLANNED_PATHS.some((re) => re.test(pathname));
