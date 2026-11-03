/**
 * Kontextmenü für Tickets in Listen und Board (Rechtsklick, Kontextmenü-Taste, ⇧F10). Ein Host im
 * `IssuesHost` bedient alle Seiten: Zeilen und Karten melden nur Ziel und Position
 * (`context-menu-store.ts`). Eigenschaften öffnen die vorhandenen Picker (Suche, Häkchen am
 * aktuellen Wert, Ziffern), bei Mehrfachauswahl die Bulk-Picker; Archivieren/Löschen laufen über
 * `BulkActions` (Löschen mit Bestätigung). Tastenkürzel stammen aus der Command-Registry.
 */
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Archive,
  Bell,
  BellOff,
  Calendar,
  CircleDot,
  Copy,
  Gauge,
  Link2Off,
  RefreshCw,
  SignalHigh,
  Star,
  Tag,
  Trash2,
  User,
  UserCheck,
  FolderKanban,
  CornerRightUp,
  Link2,
} from "lucide-react";
import { useCallback, useRef, useState, type ReactNode } from "react";

import { useCommands } from "@/app/commands";
import { Keys } from "@/components/ui/kbd";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { Issue } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { BulkActions, type BulkKind } from "./bulk-actions";
import { closeIssueMenu, useIssueMenuRequest, type IssueMenuRequest } from "./context-menu-store";
import { useTeamMeta } from "./hooks";
import { useIssueActions, type IssueActions } from "./issue-actions";
import { copyText, issueUrl, useIssueOps } from "./issue-extra-shortcuts";
import { branchName, duePresets, type RelationChoice } from "./model";
import { RelationDialog } from "./relation-dialog";
import { IssuePickerHost, type PickerKind, type PickerRequest } from "./picker-host";

function MenuItem({
  icon,
  label,
  actionId,
  disabled,
  onSelect,
}: {
  icon?: ReactNode;
  label: string;
  /** Aktions-ID in der Command-Registry, aus der das Kürzel angezeigt wird. */
  actionId?: string;
  disabled?: boolean;
  onSelect: () => void;
}) {
  const cmds = useCommands();
  const keys = actionId ? cmds.find((c) => c.id === actionId)?.shortcut : undefined;
  return (
    <DropdownMenuItem disabled={disabled} onSelect={onSelect}>
      {icon}
      {label}
      {keys && (
        <DropdownMenuShortcut>
          <Keys keys={keys} />
        </DropdownMenuShortcut>
      )}
    </DropdownMenuItem>
  );
}

function SubmenuItem({
  icon,
  label,
  actionId,
  children,
}: {
  icon?: ReactNode;
  label: string;
  actionId?: string;
  children: ReactNode;
}) {
  const cmds = useCommands();
  const keys = actionId ? cmds.find((c) => c.id === actionId)?.shortcut : undefined;
  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger>
        {icon}
        {label}
        {keys && (
          <DropdownMenuShortcut className="mr-1">
            <Keys keys={keys} />
          </DropdownMenuShortcut>
        )}
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent>{children}</DropdownMenuSubContent>
    </DropdownMenuSub>
  );
}

interface MenuProps {
  req: IssueMenuRequest;
  /** Aktionen und Favorit/Abo-Umschalter leben im Host: sie müssen das Schließen des Menüs überdauern. */
  actions: IssueActions;
  ops: ReturnType<typeof useIssueOps>;
  /** Menü schließen und danach `then` ausführen; mit `defer` erst nach dem Aufräumen (Dialoge). */
  after: (then?: () => void, defer?: boolean) => void;
  onPicker: (kind: PickerKind) => void;
  onBulk: (kind: BulkKind) => void;
  onRelation: (choice: RelationChoice) => void;
}

function IssueMenu({ req, actions, ops, after, onPicker, onBulk, onRelation }: MenuProps) {
  const { issue, targets } = req;
  const multi = targets.length > 1;
  const meta = useTeamMeta(issue.teamId);
  const { toggleFavorite, toggleSubscription, isFavorite } = ops;
  useShortcutContext("modal", true);

  const subscribed = useQuery({
    queryKey: ["issue-subscribed", issue.id],
    enabled: !multi,
    staleTime: 0,
    queryFn: async () =>
      (
        await gqlRequest<{ issue: { subscribed: boolean } | null }, { id: string }>(
          `query IssueSubscribed($id: String!) { issue(id: $id) { subscribed } }`,
          { id: issue.id },
        )
      ).issue?.subscribed === true,
  });

  const estimatesOff = meta.data?.team.issueEstimationType === "notUsed";
  const cyclesOff = meta.data !== undefined && !meta.data.team.cyclesEnabled;
  const refocus = useRef(true);

  // Eigenschaft: Picker (Einzel) bzw. Bulk-Picker (Mehrfachauswahl).
  const prop = (kind: Exclude<PickerKind, "team" | "due" | "parent" | "milestone">) => () =>
    after(() => (multi ? onBulk(kind) : onPicker(kind)), true);
  const label = multi ? `${targets.length} issues` : issue.identifier;

  const copyAll = (what: string, text: (i: Issue) => string) => () =>
    copyText(targets.map(text).join("\n"), multi ? `${what}s` : what);

  return (
    <DropdownMenu open modal onOpenChange={(o) => !o && closeIssueMenu()}>
      <DropdownMenuTrigger asChild>
        <span
          aria-hidden
          data-testid="issue-menu-anchor"
          style={{ position: "fixed", left: req.x, top: req.y, width: 0, height: 0 }}
        />
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        side="bottom"
        sideOffset={2}
        aria-label={`Actions for ${label}`}
        className="max-h-[80vh] w-64 overflow-y-auto"
        onCloseAutoFocus={(e) => {
          e.preventDefault();
          // Fokus zurück auf die Zeile/Karte (nicht auf den unsichtbaren Anker).
          if (refocus.current)
            document.querySelector<HTMLElement>(`[data-issue-id="${issue.id}"]`)?.focus();
        }}
      >
        <DropdownMenuLabel>{label}</DropdownMenuLabel>
        <MenuItem
          icon={<CircleDot />}
          label="Status…"
          actionId="issue.status"
          onSelect={prop("status")}
        />
        <MenuItem
          icon={<SignalHigh />}
          label="Priority…"
          actionId="issue.priority"
          onSelect={prop("priority")}
        />
        <MenuItem
          icon={<User />}
          label="Assignee…"
          actionId="issue.assignee"
          onSelect={prop("assignee")}
        />
        <MenuItem
          icon={<UserCheck />}
          label="Assign to me"
          actionId="issue.assign-me"
          onSelect={() => after(() => targets.forEach((t) => actions.assignToMe(t)))}
        />
        <MenuItem
          icon={<Tag />}
          label="Labels…"
          actionId="issue.labels"
          onSelect={prop("labels")}
        />
        <MenuItem
          icon={<FolderKanban />}
          label="Project…"
          actionId="issue.project"
          onSelect={prop("project")}
        />
        {!cyclesOff && (
          <MenuItem
            icon={<RefreshCw />}
            label="Cycle…"
            actionId="issue.cycle"
            onSelect={prop("cycle")}
          />
        )}
        <SubmenuItem icon={<Calendar />} label="Due date" actionId="issue.due-list">
          {duePresets(new Date()).map((p) => (
            <MenuItem
              key={p.id}
              label={p.label}
              onSelect={() => after(() => targets.forEach((t) => actions.setDueDate(t, p.date)))}
            />
          ))}
          {!multi && (
            <MenuItem label="Pick a date…" onSelect={() => after(() => onPicker("due"), true)} />
          )}
          <DropdownMenuSeparator />
          <MenuItem
            label="Clear due date"
            disabled={targets.every((t) => !t.dueDate)}
            onSelect={() => after(() => targets.forEach((t) => actions.setDueDate(t, null)))}
          />
        </SubmenuItem>
        {!estimatesOff && (
          <MenuItem
            icon={<Gauge />}
            label="Estimate…"
            actionId="issue.estimate"
            onSelect={prop("estimate")}
          />
        )}

        {!multi && (
          <>
            <DropdownMenuSeparator />
            <MenuItem
              icon={<CornerRightUp />}
              label="Set parent issue…"
              actionId="issue.parent"
              onSelect={() => after(() => onPicker("parent"), true)}
            />
            {issue.parentId && (
              <MenuItem
                icon={<Link2Off />}
                label="Remove parent issue"
                actionId="issue.parent-remove"
                onSelect={() => after(() => actions.setParent(issue, null))}
              />
            )}
          </>
        )}

        <DropdownMenuSeparator />
        <SubmenuItem icon={<Link2 />} label="Relations">
          <MenuItem label="Blocks…" onSelect={() => after(() => onRelation("blocking"), true)} />
          <MenuItem
            label="Blocked by…"
            onSelect={() => after(() => onRelation("blocked-by"), true)}
          />
          <MenuItem label="Related to…" onSelect={() => after(() => onRelation("related"), true)} />
          {!multi && (
            <MenuItem
              label="Mark as duplicate of…"
              onSelect={() => after(() => onRelation("duplicate"), true)}
            />
          )}
        </SubmenuItem>

        <SubmenuItem icon={<Copy />} label="Copy">
          <MenuItem
            label="ID"
            actionId="issue.copy-id"
            onSelect={copyAll("Issue ID", (i) => i.identifier)}
          />
          <MenuItem
            label="Link"
            actionId="issue.copy-url"
            onSelect={copyAll("Issue URL", (i) => issueUrl(i.identifier))}
          />
          <MenuItem
            label="Title"
            actionId="issue.copy-title"
            onSelect={copyAll("Issue title", (i) => i.title)}
          />
          <MenuItem
            label="Git branch name"
            onSelect={copyAll("Branch name", (i) => branchName(i.identifier, i.title))}
          />
        </SubmenuItem>
        {!multi && (
          <>
            <MenuItem
              icon={<Star />}
              label={isFavorite ? "Remove from favorites" : "Add to favorites"}
              actionId="issue.favorite"
              onSelect={() => after(toggleFavorite)}
            />
            <MenuItem
              icon={subscribed.data ? <BellOff /> : <Bell />}
              label={
                subscribed.data === undefined
                  ? "Subscribe / unsubscribe"
                  : subscribed.data
                    ? "Unsubscribe"
                    : "Subscribe"
              }
              actionId="issue.subscribe"
              onSelect={() => after(toggleSubscription)}
            />
          </>
        )}

        <DropdownMenuSeparator />
        <MenuItem
          icon={<Archive />}
          label="Archive"
          onSelect={() => after(() => onBulk("archive"), true)}
        />
        <MenuItem
          icon={<Trash2 />}
          label="Delete…"
          actionId="issue.delete"
          onSelect={() => {
            refocus.current = false;
            after(() => onBulk("delete"), true);
          }}
        />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** Einmal in der Shell: Menü plus die Picker/Dialoge, die es öffnet. */
export function IssueContextMenuHost() {
  const req = useIssueMenuRequest();
  const [picker, setPicker] = useState<PickerRequest | null>(null);
  const [bulk, setBulk] = useState<{
    kind: BulkKind;
    issues: Issue[];
    clear?: () => void;
  } | null>(null);
  const [relation, setRelation] = useState<{ choice: RelationChoice; issues: Issue[] } | null>(
    null,
  );
  const closePicker = useCallback(() => setPicker(null), []);
  const noop = useCallback(() => undefined, []);

  // Erst nach dem Schließen des (modalen) Menüs weitermachen: sonst bleibt `pointer-events: none`
  // am Body und der folgende Dialog fängt den Fokus nicht sauber.
  const after = useCallback((then?: () => void, defer = false) => {
    closeIssueMenu();
    if (!then) return;
    if (defer) setTimeout(then, 0);
    else then();
  }, []);

  // Letzte Anfrage behalten: Favorit/Abo/Änderungen laufen nach dem Schließen des Menüs weiter.
  const [sticky, setSticky] = useState(req);
  if (req && req !== sticky) setSticky(req);
  const actions = useIssueActions();
  const qc = useQueryClient();
  const stickyIssue = sticky && sticky.targets.length === 1 ? sticky.issue : null;
  const rawOps = useIssueOps(stickyIssue);
  const ops = {
    ...rawOps,
    toggleSubscription: () =>
      rawOps
        .toggleSubscription()
        .then(() => qc.invalidateQueries({ queryKey: ["issue-subscribed", stickyIssue?.id] })),
  };

  return (
    <>
      {req && (
        <IssueMenu
          key={`${req.issue.id}:${req.x}:${req.y}`}
          req={req}
          actions={actions}
          ops={ops}
          after={after}
          onPicker={(kind) => setPicker({ kind, issue: req.issue })}
          onRelation={(choice) => setRelation({ choice, issues: req.targets })}
          onBulk={(kind) =>
            setBulk({
              kind,
              issues: req.targets,
              clear: req.targets.length > 1 ? req.clearSelection : undefined,
            })
          }
        />
      )}
      <IssuePickerHost request={picker} onClose={closePicker} />
      {relation && (
        <RelationDialog
          issues={relation.issues}
          choice={relation.choice}
          onChoice={(choice) => setRelation((r) => r && { ...r, choice })}
          onClose={() => setRelation(null)}
        />
      )}
      <BulkActions
        hideBar
        issues={bulk?.issues ?? []}
        request={bulk?.kind ?? null}
        onRequest={(k) => setBulk((b) => (b && k ? { ...b, kind: k } : null))}
        onClear={bulk?.clear ?? noop}
      />
    </>
  );
}
