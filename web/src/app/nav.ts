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
  trash: "Trash",
};

const titleCase = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/-/g, " ");

/** Breadcrumb-Segmente für einen Pfad, ohne Workspace-Präfix: `/acme/team/ENG/all` → `["ENG", "All issues"]`. */
export function breadcrumbFor(pathname: string): string[] {
  const parts = pathname.split("/").filter(Boolean).slice(1); // Workspace-Präfix weg
  const out: string[] = [];
  for (let i = 0; i < parts.length; i++) {
    const p = parts[i]!;
    if (p === "team") continue;
    out.push(
      SEGMENT_TITLES[p] ?? (/^[A-Z][A-Z0-9]*$/.test(p) ? p : titleCase(decodeURIComponent(p))),
    );
  }
  return out;
}

/** Seitentitel für Platzhalter und Dokumenttitel. */
export function titleForPath(pathname: string): string {
  const crumbs = breadcrumbFor(pathname);
  return crumbs.length ? crumbs.join(" › ") : "Home";
}
