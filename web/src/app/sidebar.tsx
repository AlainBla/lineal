import {
  Box,
  ChevronDown,
  ChevronRight,
  FileText,
  Gauge,
  Inbox,
  Keyboard,
  Layers,
  LayoutDashboard,
  ListTodo,
  LogOut,
  Plus,
  RefreshCw,
  Search,
  Settings,
  SquarePen,
  Target,
  UserPlus,
  Users,
} from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useState, type ReactNode } from "react";

import type { Session } from "@/app/auth";
import { NavLink } from "@/app/nav-link";
import { teamPaths, workspacePaths } from "@/app/nav";
import { setUiState, toggleCommandMenu, toggleShortcutHelp, useUiState } from "@/app/ui-state";
import {
  buildTeamTree,
  useSidebarData,
  type SidebarData,
  type TeamNode,
} from "@/app/use-sidebar-data";
import { Button } from "@/components/ui/button";
import { FavoritesSection } from "@/features/views/favorites-section";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Keys } from "@/components/ui/kbd";
import { Hint } from "@/components/ui/tooltip";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/cn";

export const SIDEBAR_WIDTH = 240;

function Badge({ children, label }: { children: ReactNode; label: string }) {
  return (
    <span
      aria-label={label}
      className="ml-auto rounded-full bg-primary/15 px-1.5 text-[10px] leading-4 font-semibold text-primary tabular-nums"
    >
      {children}
    </span>
  );
}

function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <h2 className="px-2 pt-4 pb-1 text-xs font-medium text-subtle-foreground select-none">
      {children}
    </h2>
  );
}

function WorkspaceMenu({ session, onLogout }: { session: Session; onLogout: () => void }) {
  const initial = session.workspace.name.charAt(0).toUpperCase();
  const paths = workspacePaths(session.workspace.urlKey);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md px-1.5 text-sm font-medium transition-colors duration-[var(--duration-fast)] hover:bg-surface-hover data-[state=open]:bg-surface-hover"
          aria-label={`Workspace menu, ${session.workspace.name}`}
        >
          <span
            aria-hidden
            className="flex size-5 shrink-0 items-center justify-center rounded bg-primary text-[11px] font-semibold text-primary-foreground"
          >
            {initial}
          </span>
          <span className="truncate">{session.workspace.name}</span>
          <ChevronDown className="size-3.5 shrink-0 text-subtle-foreground" aria-hidden />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-60">
        <DropdownMenuLabel>{session.user.email}</DropdownMenuLabel>
        <DropdownMenuItem asChild>
          <NavLink to={paths.settings} className="h-auto px-2 py-1.5 text-popover-foreground">
            <Settings />
            Settings
            <DropdownMenuShortcut>
              <Keys keys="g s" />
            </DropdownMenuShortcut>
          </NavLink>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <NavLink
            to={`/${session.workspace.urlKey}/settings/members`}
            className="h-auto px-2 py-1.5 text-popover-foreground"
          >
            <UserPlus />
            Invite members
          </NavLink>
        </DropdownMenuItem>
        <DropdownMenuItem disabled>
          <Users />
          Switch workspace
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => toggleShortcutHelp(true)}>
          <Keyboard />
          Keyboard shortcuts
          <DropdownMenuShortcut>
            <Keys keys="mod+/" />
          </DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={onLogout}>
          <LogOut />
          Log out
          <DropdownMenuShortcut>
            <Keys keys="alt+shift+q" />
          </DropdownMenuShortcut>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function TeamEntry({
  node,
  ws,
  depth,
  defaultOpen,
}: {
  node: TeamNode;
  ws: string;
  depth: number;
  defaultOpen: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const { team } = node;
  const p = teamPaths(ws, team.key);
  const panelId = `team-${team.key}-items`;
  return (
    <li>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((o) => !o)}
        className="group flex h-7 w-full items-center gap-1.5 rounded-md px-2 text-left text-sm font-medium text-sidebar-foreground transition-colors duration-[var(--duration-fast)] hover:bg-surface-hover"
      >
        <ChevronRight
          aria-hidden
          className={cn(
            "size-3.5 shrink-0 text-subtle-foreground transition-transform duration-[var(--duration-base)]",
            open && "rotate-90",
          )}
        />
        <span
          aria-hidden
          className="flex h-4 min-w-4 items-center justify-center rounded-[4px] bg-surface-active px-0.5 text-[9px] font-semibold text-muted-foreground"
        >
          {team.key.slice(0, 2)}
        </span>
        <span className="truncate">{team.name}</span>
        <span className="sr-only">({team.key})</span>
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            id={panelId}
            key="panel"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.16, ease: [0.22, 1, 0.36, 1] }}
            className="overflow-hidden"
          >
            <ul
              className={cn(
                "ml-[13px] flex flex-col gap-px border-l border-border pt-0.5 pl-1.5",
                depth > 0 && "ml-[11px]",
              )}
            >
              {team.triageEnabled && (
                <li>
                  <NavLink to={p.triage}>
                    <Inbox />
                    Triage
                    {team.triageCount > 0 && (
                      <Badge label={`${team.triageCount} in triage`}>{team.triageCount}</Badge>
                    )}
                  </NavLink>
                </li>
              )}
              <li>
                <NavLink to={p.all}>
                  <Layers />
                  Issues
                </NavLink>
              </li>
              {team.cyclesEnabled && (
                <li>
                  <NavLink to={p.cycles}>
                    <RefreshCw />
                    Cycles
                  </NavLink>
                  <ul className="ml-[15px] flex flex-col gap-px border-l border-border pl-1.5">
                    <li>
                      <NavLink to={p.currentCycle}>Current</NavLink>
                    </li>
                    <li>
                      <NavLink to={p.upcomingCycle}>Upcoming</NavLink>
                    </li>
                  </ul>
                </li>
              )}
              <li>
                <NavLink to={p.projects}>
                  <Box />
                  Projects
                </NavLink>
              </li>
              <li>
                <NavLink to={p.views}>
                  <LayoutDashboard />
                  Views
                </NavLink>
              </li>
              <li>
                <NavLink to={p.documents}>
                  <FileText />
                  Documents
                </NavLink>
              </li>
              {node.children.length > 0 && (
                <li>
                  <ul
                    className="flex flex-col gap-px pt-0.5"
                    aria-label={`Sub-teams of ${team.name}`}
                  >
                    {node.children.map((child) => (
                      <TeamEntry
                        key={child.team.id}
                        node={child}
                        ws={ws}
                        depth={depth + 1}
                        defaultOpen={false}
                      />
                    ))}
                  </ul>
                </li>
              )}
            </ul>
          </motion.div>
        )}
      </AnimatePresence>
    </li>
  );
}

function TeamsSection({ ws, data, loading }: { ws: string; data?: SidebarData; loading: boolean }) {
  if (loading || !data) {
    return (
      <div className="flex flex-col gap-1.5 px-2 pt-2" aria-busy="true" aria-label="Loading teams">
        <Skeleton className="h-5 w-3/4" />
        <Skeleton className="h-5 w-1/2" />
      </div>
    );
  }
  const tree = buildTeamTree(data.teams);
  return (
    <ul className="flex flex-col gap-px">
      {tree.map((node, i) => (
        <TeamEntry key={node.team.id} node={node} ws={ws} depth={0} defaultOpen={i === 0} />
      ))}
    </ul>
  );
}

export function Sidebar({
  session,
  onLogout,
  overlay = false,
}: {
  session: Session;
  onLogout: () => void;
  /** Schmaler Viewport: Sidebar liegt über dem Inhalt statt daneben. */
  overlay?: boolean;
}) {
  const { sidebarOpen } = useUiState();
  const { data, isLoading } = useSidebarData();
  const ws = session.workspace.urlKey;
  const p = workspacePaths(ws);
  const guest = session.user.role === "guest";

  return (
    <motion.aside
      aria-label="Sidebar"
      initial={false}
      animate={{ width: sidebarOpen ? SIDEBAR_WIDTH : 0 }}
      transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
      className={cn(
        "shrink-0 overflow-hidden border-r border-border bg-sidebar",
        overlay && "fixed inset-y-0 left-0 z-40 shadow-popover",
      )}
      // Eingeklappt: weder fokussierbar noch für Screenreader sichtbar.
      inert={!sidebarOpen}
      aria-hidden={!sidebarOpen}
      data-open={sidebarOpen}
    >
      <div className="flex h-full flex-col" style={{ width: SIDEBAR_WIDTH }}>
        <div className="flex h-11 items-center gap-1 px-2">
          <WorkspaceMenu session={session} onLogout={onLogout} />
          <Hint label="Search" keys="mod+k">
            <Button
              variant="ghost"
              size="icon"
              aria-label="Search workspace"
              onClick={() => toggleCommandMenu(true)}
            >
              <Search />
            </Button>
          </Hint>
          <Hint label="New issue" keys="c">
            <Button
              variant="ghost"
              size="icon"
              aria-label="Create new issue"
              onClick={() => setUiState({ createIssueOpen: true })}
            >
              <SquarePen />
            </Button>
          </Hint>
        </div>

        <nav aria-label="Main" className="flex-1 overflow-y-auto px-2 pb-4">
          <ul className="flex flex-col gap-px pt-1">
            <li>
              <NavLink to={p.inbox}>
                <Inbox />
                Inbox
                {data && data.inboxUnread > 0 && (
                  <Badge label={`${data.inboxUnread} unread`}>{data.inboxUnread}</Badge>
                )}
              </NavLink>
            </li>
            <li>
              <NavLink to={p.myIssues}>
                <ListTodo />
                My issues
              </NavLink>
            </li>
            <li>
              <NavLink to={p.drafts}>
                <FileText />
                Drafts
                {data && data.draftsCount > 0 && (
                  <span className="ml-auto text-xs text-subtle-foreground tabular-nums">
                    {data.draftsCount}
                  </span>
                )}
              </NavLink>
            </li>
          </ul>

          <SectionLabel>Workspace</SectionLabel>
          <ul className="flex flex-col gap-px">
            {!guest && (
              <li>
                <NavLink to={p.overview}>
                  <LayoutDashboard />
                  Overview
                </NavLink>
              </li>
            )}
            <li>
              <NavLink to={p.issues}>
                <Layers />
                Issues
              </NavLink>
            </li>
            <li>
              <NavLink to={p.projects}>
                <Box />
                Projects
              </NavLink>
            </li>
            {!guest && (
              <li>
                <NavLink to={p.initiatives}>
                  <Target />
                  Initiatives
                </NavLink>
              </li>
            )}
            <li>
              <NavLink to={p.views}>
                <Layers />
                Views
              </NavLink>
            </li>
            {!guest && (
              <li>
                <NavLink to={p.workload}>
                  <Gauge />
                  Workload
                </NavLink>
              </li>
            )}
            {!guest && (
              <li>
                <NavLink to={p.cycles}>
                  <RefreshCw />
                  Cycles
                </NavLink>
              </li>
            )}
          </ul>

          <FavoritesSection ws={ws} />

          <div className="flex items-center justify-between pr-1">
            <SectionLabel>Your teams</SectionLabel>
            {session.user.role !== "guest" && (
              <Hint label="New team">
                <Button
                  variant="ghost"
                  size="icon"
                  className="mt-3 size-5"
                  aria-label="Create new team"
                  onClick={() => setUiState({ createTeamOpen: true })}
                >
                  <Plus />
                </Button>
              </Hint>
            )}
          </div>
          <TeamsSection ws={ws} data={data} loading={isLoading} />
        </nav>
      </div>
    </motion.aside>
  );
}
