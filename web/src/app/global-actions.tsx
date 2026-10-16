import { useNavigate, useRouterState } from "@tanstack/react-router";
import {
  Box,
  CalendarClock,
  CalendarDays,
  Gauge,
  Inbox,
  Keyboard,
  Layers,
  LayoutDashboard,
  ListTodo,
  LogOut,
  Monitor,
  Moon,
  PanelLeft,
  PanelRight,
  RefreshCw,
  Search,
  Settings,
  SquarePen,
  Sun,
  Target,
  Trash2,
  Users,
  FileText,
} from "lucide-react";
import { useMemo } from "react";

import { useActions, type Action } from "@/app/commands";
import { teamKeyFromPath, teamPaths, workspacePaths } from "@/app/nav";
import { buildOpenActions } from "@/app/open-actions";
import {
  setUiState,
  toggleCommandMenu,
  toggleShortcutHelp,
  toggleSidebar,
  getUiState,
} from "@/app/ui-state";
import { buildTeamTree, flattenTeamTree, useSidebarData } from "@/app/use-sidebar-data";
import { setTheme } from "@/lib/theme";

/**
 * Baut die globalen Aktionen (Navigation, Oberfläche, Theme, Konto) aus Workspace, aktuellem Team und
 * Teamliste. Reine Funktion ohne Seiteneffekte; `go` navigiert zu einem Pfad.
 */
export function buildGlobalActions(args: {
  ws: string;
  teams: Array<{ key: string; name: string }>;
  currentTeamKey: string | null;
  go: (path: string) => void;
  logout: () => void;
  /** Gäste sehen keine Workspace-Übersichten (Overview, Initiatives, Workload, Cycles). */
  guest?: boolean;
}): Action[] {
  const { ws, teams, go, logout } = args;
  const w = workspacePaths(ws);
  const teamKey = args.currentTeamKey ?? teams[0]?.key ?? null;
  const t = teamKey ? teamPaths(ws, teamKey) : null;
  const nav = "Navigation";

  const actions: Action[] = [
    {
      id: "palette.toggle",
      title: "Open command menu",
      section: "General",
      keys: "mod+k",
      allowInModal: true,
      icon: Search,
      perform: () => toggleCommandMenu(),
    },
    {
      id: "help.shortcuts",
      title: "Show keyboard shortcuts",
      section: "General",
      keys: "mod+/",
      allowInModal: true,
      icon: Keyboard,
      perform: () => toggleShortcutHelp(),
    },
    {
      id: "help.shortcuts-q",
      title: "Open help menu",
      section: "General",
      keys: "?",
      icon: Keyboard,
      perform: () => toggleShortcutHelp(true),
    },
    {
      id: "search.open",
      title: "Search",
      section: "General",
      keys: "/",
      icon: Search,
      perform: () => go(w.search),
    },
    {
      id: "issue.create",
      title: "Create new issue",
      section: "Issues",
      keys: "c",
      icon: SquarePen,
      perform: () => setUiState({ createIssueOpen: true }),
    },

    {
      id: "team.create",
      title: "Create team",
      section: "Teams",
      keywords: ["new team", "add team"],
      icon: Users,
      perform: () => setUiState({ createTeamOpen: true }),
    },
    {
      id: "ui.sidebar",
      title: "Toggle sidebar",
      section: "Interface",
      keys: "[",
      icon: PanelLeft,
      perform: () => toggleSidebar(),
    },
    {
      id: "ui.details",
      title: "Toggle details sidebar",
      section: "Interface",
      keys: "]",
      icon: PanelRight,
      perform: () => setUiState({ detailsOpen: !getUiState().detailsOpen }),
    },

    {
      id: "nav.inbox",
      title: "Go to Inbox",
      section: nav,
      keys: "g i",
      icon: Inbox,
      perform: () => go(w.inbox),
    },
    {
      id: "nav.my-issues",
      title: "Go to My issues",
      section: nav,
      keys: "g m",
      icon: ListTodo,
      perform: () => go(w.myIssues),
    },
    {
      id: "nav.drafts",
      title: "Go to Drafts",
      section: nav,
      keys: "g d",
      icon: FileText,
      perform: () => go(w.drafts),
    },
    {
      id: "nav.overview",
      title: "Go to company overview",
      section: nav,
      keys: "g o",
      icon: LayoutDashboard,
      keywords: ["workspace", "company"],
      perform: () => go(w.overview),
    },
    // docs/14 §5 nennt `g w`, doch docs/08 belegt `g w` mit „Upcoming cycle“; Workspace-Issues daher `g ⇧W`.
    {
      id: "nav.workspace-issues",
      title: "Go to workspace issues (all teams)",
      section: nav,
      keys: "g shift+w",
      icon: Layers,
      perform: () => go(w.issues),
    },
    {
      id: "nav.workload",
      title: "Go to Workload",
      section: nav,
      icon: Gauge,
      perform: () => go(w.workload),
    },
    {
      id: "nav.workspace-cycles",
      title: "Go to workspace cycles",
      section: nav,
      icon: RefreshCw,
      perform: () => go(w.cycles),
    },
    {
      id: "nav.trash",
      title: "Go to Recently deleted",
      section: nav,
      icon: Trash2,
      perform: () => go(w.trash),
    },
    {
      id: "nav.projects",
      title: "Go to Projects",
      section: nav,
      keys: "g p",
      icon: Box,
      perform: () => go(w.projects),
    },
    {
      id: "nav.initiatives",
      title: "Go to Initiatives",
      section: nav,
      keys: "g n",
      icon: Target,
      perform: () => go(w.initiatives),
    },
    {
      id: "nav.views",
      title: "Go to Views",
      section: nav,
      keys: "g u",
      icon: Layers,
      perform: () => go(w.views),
    },
    {
      id: "nav.settings",
      title: "Go to Settings",
      section: nav,
      keys: "g s",
      icon: Settings,
      perform: () => go(w.settings),
    },

    {
      id: "theme.dark",
      title: "Theme: Dark",
      section: "Preferences",
      icon: Moon,
      keywords: ["appearance"],
      perform: () => setTheme("dark"),
    },
    {
      id: "theme.light",
      title: "Theme: Light",
      section: "Preferences",
      icon: Sun,
      keywords: ["appearance"],
      perform: () => setTheme("light"),
    },
    {
      id: "theme.system",
      title: "Theme: System",
      section: "Preferences",
      icon: Monitor,
      keywords: ["appearance"],
      perform: () => setTheme("system"),
    },
    {
      id: "account.logout",
      title: "Log out",
      section: "Account",
      keys: "alt+shift+q",
      icon: LogOut,
      perform: logout,
    },
  ];

  if (t) {
    actions.push(
      {
        id: "team.triage",
        title: "Go to Triage",
        section: nav,
        keys: "g t",
        icon: Inbox,
        perform: () => go(t.triage),
      },
      {
        id: "team.active",
        title: "Go to Active issues",
        section: nav,
        keys: "g a",
        icon: Layers,
        perform: () => go(t.active),
      },
      {
        id: "team.backlog",
        title: "Go to Backlog",
        section: nav,
        keys: "g b",
        icon: Layers,
        perform: () => go(t.backlog),
      },
      {
        id: "team.all",
        title: "Go to All issues",
        section: nav,
        keys: "g e",
        icon: Layers,
        perform: () => go(t.all),
      },
      {
        id: "team.cycles",
        title: "Go to Cycles",
        section: nav,
        keys: "g c",
        icon: RefreshCw,
        perform: () => go(t.cycles),
      },
      {
        id: "team.cycle-current",
        title: "Go to Current cycle",
        section: nav,
        keys: "g v",
        icon: CalendarDays,
        perform: () => go(t.currentCycle),
      },
      {
        id: "team.cycle-upcoming",
        title: "Go to Upcoming cycle",
        section: nav,
        keys: "g w",
        icon: CalendarClock,
        perform: () => go(t.upcomingCycle),
      },
    );
  }
  teams.forEach((team, i) => {
    actions.push({
      id: `team.open.${team.key}`,
      title: `Go to team ${team.name} (${team.key})`,
      section: "Teams",
      keys: i < 9 ? `mod+shift+${i + 1}` : undefined,
      icon: Users,
      keywords: [team.key, "team"],
      perform: () => go(teamPaths(ws, team.key).home),
    });
  });
  actions.push(...buildOpenActions({ ws, go, guest: args.guest }));
  return args.guest ? actions.filter((a) => !GUEST_HIDDEN.has(a.id)) : actions;
}

const GUEST_HIDDEN = new Set([
  "nav.overview",
  "nav.workload",
  "nav.workspace-cycles",
  "nav.initiatives",
]);

/** Registriert die globalen Aktionen als Shortcuts und Commands (einmal in der Shell). */
export function useGlobalActions(args: { ws: string; onLogout: () => void; guest?: boolean }) {
  const navigate = useNavigate();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { data } = useSidebarData();

  const teams = useMemo(() => (data ? flattenTeamTree(buildTeamTree(data.teams)) : []), [data]);
  const currentTeamKey = teamKeyFromPath(pathname);

  const actions = buildGlobalActions({
    ws: args.ws,
    teams,
    currentTeamKey,
    go: (path) => void navigate({ to: path as "/" }),
    logout: args.onLogout,
    guest: args.guest,
  });
  useActions(actions);
}
