import { ChevronDown, ChevronRight } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { Button } from "@/components/ui/button";
import { useIssueActions } from "@/features/issues/issue-actions";
import { useIssueLookup } from "@/features/issues/issue-lookup";
import { IssueCardBody, IssueRow } from "@/features/issues/issue-row";
import { PRIORITIES } from "@/features/issues/model";
import { IssuePickerHost, type PickerRequest } from "@/features/issues/picker-host";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import { useFocusNav } from "@/features/issues/use-focus-nav";
import { useIssueShortcuts } from "@/features/issues/use-issue-shortcuts";
import type { CustomView, Issue } from "@/gql/graphql";
import { GraphQLRequestError } from "@/lib/graphql";

import { DisplayMenu } from "./display-menu";
import {
  applySubGrouping,
  applyVisibility,
  flatIssues,
  groupIssues,
  isCustomDisplay,
  mergeDisplay,
  nestIssues,
  sortIssues,
  withEmptyGroups,
  type DisplayOptions,
  type Grouping,
  type IssueGroup,
} from "./display-options";
import { FilterBar, FilterChips, FilterMenu } from "./filter-bar";
import { buildFilter, type Filter, type FilterChip } from "./filter-model";
import { combineFilter, filterToChips } from "./filter-parse";
import {
  usePreferences,
  useResetPreferences,
  useSetDefaultPreferences,
  useUpdatePreferences,
  useUpdateView,
  useViewIssues,
  type PrefTarget,
} from "./queries";
import { useFilterSources } from "./use-filter-sources";
import { TransferOwnerButton, ViewSubscriptionButton } from "./view-actions";
import { ViewBuilderDialog } from "./view-builder";

/**
 * Issue-Ansicht über `viewIssues`: Filter-Bar + Anzeigeoptionen + gruppierte Liste (docs/05 §1.6, §2).
 * Mit `view` sind Filter und Optionen die der gespeicherten View (Optionen persönlich speicherbar);
 * ohne `view` ist es eine Ad-hoc-Liste, die per „Save“ zur View wird.
 */
export function IssueView({
  ws,
  view,
  teamId,
  title,
  scope,
  prefsTarget,
  embedded,
  toolbarSlot,
  trailingControls,
  emptyGroups,
  renderGroupExtra,
}: {
  ws: string;
  view?: CustomView;
  teamId?: string;
  title: string;
  /** Fester Bereichsfilter (z. B. ein Projekt), der mit den Chips per UND verknüpft wird. */
  scope?: Filter;
  /** Speicherort der Anzeigeoptionen ohne View (z. B. `projectIssues` je Projekt). */
  prefsTarget?: PrefTarget;
  /** Ohne Titelzeile: Filter und Anzeige als Symbolknöpfe, Chips nur bei aktivem Filter. */
  embedded?: boolean;
  /** Ziel (Element) für die Symbolknöpfe, z. B. die Tab-Zeile der Seite; sonst die eigene Kopfzeile. */
  toolbarSlot?: HTMLElement | null;
  /** Zusätzliche Knöpfe hinter Filter und Anzeige (z. B. Seitenpanel ein/aus). */
  trailingControls?: ReactNode;
  /** Gruppen, die bei „Show empty groups“ auch leer erscheinen (je Gruppierung). */
  emptyGroups?: Partial<Record<Grouping, Array<{ key: string; label: string }>>>;
  /** Zusatz im Gruppenkopf (Datum, „+“ …). */
  renderGroupExtra?: (g: IssueGroup) => ReactNode;
}) {
  // Gespeicherte Views: filterData → Chips (+ „erweiterter Filter“ für Nicht-Darstellbares).
  const initial = useMemo(() => filterToChips(view?.filterData), [view?.filterData]);
  const [chips, setChipsState] = useState<FilterChip[]>(initial.chips);
  const [advanced, setAdvancedState] = useState<Filter[]>(initial.rest);
  const [touched, setTouched] = useState(false);
  // Version der View, auf der der lokale Entwurf aufbaut: nicht die live nachgeführte, sonst
  // überschreibt „Save“ stille Änderungen aus einem anderen Kontext (B4.04).
  const [baseVersion, setBaseVersion] = useState<number | null>(null);
  const markTouched = () => {
    if (!touched) setBaseVersion(view?.version ?? null);
    setTouched(true);
  };
  const setChips = (c: FilterChip[]) => {
    setChipsState(c);
    markTouched();
  };
  const setAdvanced = (r: Filter[]) => {
    setAdvancedState(r);
    markTouched();
  };
  const updateView = useUpdateView();
  const [local, setLocal] = useState<Partial<DisplayOptions>>({});
  const [saveOpen, setSaveOpen] = useState(false);
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [request, setRequest] = useState<PickerRequest | null>(null);

  const target: PrefTarget | null = view ? { customViewId: view.id } : (prefsTarget ?? null);
  const prefs = usePreferences(target);
  const updatePrefs = useUpdatePreferences(target);
  const resetPrefs = useResetPreferences(target);
  const setDefault = useSetDefaultPreferences(target);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const options = useMemo(
    () => ({
      ...mergeDisplay(prefs.data?.values as Record<string, unknown> | undefined),
      ...local,
    }),
    [prefs.data, local],
  );
  const onOptions = (patch: Partial<DisplayOptions>) => {
    setLocal((l) => ({ ...l, ...patch }));
    if (target) updatePrefs.mutate(patch);
  };

  const filter = useMemo(() => combineFilter(buildFilter(chips), advanced), [chips, advanced]);
  // Unverändert: die View serverseitig anwenden. Bearbeitet: der Chip-Filter ersetzt filterData;
  // der Scope der View (Team, Projekt) wird als UND-Bedingung beibehalten (docs/05 §4.1).
  const scoped = useMemo(() => {
    if (!view || !touched) return filter;
    const scope: Filter[] = [];
    if (view.teamId) scope.push({ team: { id: { in: [view.teamId] } } });
    if (view.projectId) scope.push({ project: { id: { in: [view.projectId] } } });
    return combineFilter(filter, scope);
  }, [view, touched, filter]);
  const withScope = useMemo(
    () => (scope ? combineFilter(scoped, [scope]) : scoped),
    [scoped, scope],
  );
  const q = useViewIssues({
    customViewId: view && !touched ? view.id : undefined,
    filter: view && !touched && !scope ? undefined : withScope,
    teamId,
    includeSubTeams: !!teamId,
  });
  const { sources, metas, teams } = useFilterSources();
  const lookup = useIssueLookup(metas);
  const actions = useIssueActions();

  const { groups, hidden, flat } = useMemo(() => {
    const { visible, hidden } = applyVisibility(q.data?.issues ?? [], options);
    const sorted = sortIssues(visible, options, {
      assignee: (id) => lookup.assigneeName(id),
      statePosition: (id) => lookup.state(id)?.position,
    });
    const teamName = new Map((teams.data ?? []).map((t) => [t.id, t.name]));
    const cycleName = new Map(
      metas.flatMap((m) =>
        (m?.cycles ?? []).map((c) => [c.id, c.name ?? `Cycle ${c.number}`] as const),
      ),
    );
    const names = {
      assignee: lookup.assigneeName,
      project: lookup.projectName,
      team: (id: string) => teamName.get(id) ?? null,
      cycle: (id: string) => cycleName.get(id) ?? null,
      milestone: (id: string | null) =>
        lookup.milestoneName(id) ??
        emptyGroups?.milestone?.find((g) => g.key === id)?.label ??
        null,
    };
    let groups = groupIssues(sorted, options.grouping, names);
    if (options.showEmptyGroups) {
      const all =
        emptyGroups?.[options.grouping] ??
        (options.grouping === "priority"
          ? PRIORITIES.map((p) => ({ key: String(p.value), label: p.label }))
          : []);
      groups = withEmptyGroups(groups, all, options.grouping);
    }
    groups = applySubGrouping(groups, options.subGrouping, names);
    return { groups, hidden, flat: flatIssues(groups) };
  }, [q.data, options, teams.data, metas, lookup, emptyGroups]);

  const focused: Issue | null = flat.find((i) => i.id === focusedId) ?? null;
  useFocusNav([flat.map((i) => i.id)], focused?.id ?? null, setFocusedId);
  useIssueShortcuts({ context: "list", target: focused, open: setRequest, actions });

  const rows = (list: Issue[]) =>
    (options.nestedSubIssues === "hide"
      ? list.map((issue) => ({ issue, depth: 0 }))
      : nestIssues(list)
    ).map(({ issue, depth }) => (
      <IssueRow
        key={issue.id}
        issue={issue}
        ws={ws}
        lookup={lookup}
        focused={focused?.id === issue.id}
        onFocus={setFocusedId}
        display={options.displayProperties}
        depth={depth}
      />
    ));
  const toggle = (key: string) =>
    setCollapsed((c) => {
      const n = new Set(c);
      if (!n.delete(key)) n.add(key);
      return n;
    });
  const header = (g: IssueGroup, sub = false) => (
    <h2
      className={
        sub
          ? "flex h-7 items-center gap-2 border-b border-border px-8 text-xs font-medium text-muted-foreground"
          : "flex h-8 items-center gap-2 border-b border-border bg-surface px-4 text-sm font-medium"
      }
    >
      <button
        type="button"
        aria-label={`${collapsed.has(g.key) ? "Expand" : "Collapse"} ${g.label}`}
        aria-expanded={!collapsed.has(g.key)}
        className="text-muted-foreground"
        onClick={() => toggle(g.key)}
      >
        {collapsed.has(g.key) ? (
          <ChevronRight className="size-3.5" />
        ) : (
          <ChevronDown className="size-3.5" />
        )}
      </button>
      {g.label}
      <span className="text-xs text-muted-foreground">{g.issues.length}</span>
      <span className="ml-auto flex items-center gap-2">{renderGroupExtra?.(g)}</span>
    </h2>
  );

  let body;
  if (q.isLoading) body = <ListSkeleton />;
  else if (q.isError) body = <ErrorState error={q.error} onRetry={() => void q.refetch()} />;
  else if (flat.length === 0 && !options.showEmptyGroups)
    body = <EmptyState title="No matching issues" description="Adjust the filters to see more." />;
  else if (options.layout === "board")
    body = (
      <div className="flex h-full gap-3 overflow-x-auto p-3" aria-label="Board">
        {groups.map((g) => (
          <section
            key={g.key}
            aria-label={g.label}
            className="flex w-72 shrink-0 flex-col gap-2 rounded-lg bg-surface p-2"
          >
            <h2 className="flex items-center gap-2 px-1 text-sm font-medium">
              {g.label}
              <span className="text-xs text-muted-foreground">{g.issues.length}</span>
              <span className="ml-auto flex items-center gap-2">{renderGroupExtra?.(g)}</span>
            </h2>
            {g.issues.map((issue) => (
              <a
                key={issue.id}
                href={`/${ws}/issue/${issue.identifier}`}
                data-issue-id={issue.id}
                className="rounded-lg border border-border bg-card p-2.5 text-sm hover:border-border-strong"
              >
                <IssueCardBody issue={issue} lookup={lookup} />
                <span className="mt-1 block">{issue.title}</span>
              </a>
            ))}
          </section>
        ))}
      </div>
    );
  else
    body = groups.map((g) => (
      <section key={g.key} aria-label={g.label}>
        {header(g)}
        {!collapsed.has(g.key) &&
          (g.subGroups
            ? g.subGroups.map((sg) => (
                <section key={sg.key} aria-label={`${g.label} / ${sg.label}`}>
                  {header({ ...sg, key: `${g.key}/${sg.key}` }, true)}
                  {!collapsed.has(`${g.key}/${sg.key}`) && rows(sg.issues)}
                </section>
              ))
            : rows(g.issues))}
      </section>
    ));

  const filterActive = chips.length + advanced.length > 0;
  const showHeader = !embedded || filterActive || (!!view && touched) || setDefault.isError;
  const displayMenu = (
    <DisplayMenu
      options={options}
      onChange={onOptions}
      compact={embedded}
      showLayout
      customized={
        target
          ? !!prefs.data?.hasPersonal || Object.keys(local).length > 0
          : isCustomDisplay(options)
      }
      onReset={
        target
          ? () => {
              setLocal({});
              resetPrefs.mutate();
            }
          : undefined
      }
      onSetDefault={!target || (view && !view.canEdit) ? undefined : () => setDefault.mutate()}
    />
  );
  const controls = (
    <>
      <FilterMenu
        chips={chips}
        onChange={setChips}
        sources={sources}
        advanced={advanced}
        onAdvancedChange={setAdvanced}
        compact
      />
      {displayMenu}
      {trailingControls}
    </>
  );
  const embeddedControls = embedded
    ? toolbarSlot
      ? createPortal(controls, toolbarSlot)
      : controls
    : null;

  return (
    <div className="flex h-full min-h-0 flex-col">
      {embeddedControls}
      {showHeader && (
        <header className="flex min-h-10 shrink-0 flex-wrap items-center gap-2 border-b border-border px-3 py-1">
          {!embedded && <h1 className="text-sm font-medium">{title}</h1>}
          {embedded ? (
            <div role="group" aria-label="Filters" className="flex flex-wrap items-center gap-1">
              <FilterChips
                chips={chips}
                onChange={setChips}
                sources={sources}
                advanced={advanced}
                onAdvancedChange={setAdvanced}
              />
            </div>
          ) : (
            <FilterBar
              chips={chips}
              onChange={setChips}
              sources={sources}
              advanced={advanced}
              onAdvancedChange={setAdvanced}
            />
          )}
          <div className="ml-auto flex items-center gap-1">
            {setDefault.isError && (
              <span role="alert" className="text-xs text-destructive">
                You cannot set the default for everyone.
              </span>
            )}
            {view && touched && (
              <>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setChipsState(initial.chips);
                    setAdvancedState(initial.rest);
                    setTouched(false);
                    setBaseVersion(null);
                  }}
                >
                  Discard
                </Button>
                {view.canEdit && (
                  <Button
                    size="sm"
                    disabled={updateView.isPending}
                    onClick={() =>
                      updateView.mutate(
                        {
                          id: view.id,
                          input: {
                            filterData: filter,
                            expectedVersion: baseVersion ?? view.version,
                          },
                        },
                        {
                          onSuccess: () => {
                            setTouched(false);
                            setBaseVersion(null);
                          },
                        },
                      )
                    }
                  >
                    Save changes
                  </Button>
                )}
              </>
            )}
            {updateView.isError && (
              <span role="alert" className="text-xs text-destructive">
                {updateView.error instanceof GraphQLRequestError &&
                updateView.error.code === "conflict"
                  ? "The view was changed elsewhere. Discard your changes and apply them again."
                  : "Could not save the view."}
              </span>
            )}
            {view && view.modelName === "Issue" && <ViewSubscriptionButton view={view} />}
            {view && <TransferOwnerButton view={view} />}
            {chips.length > 0 && !view && (
              <Button variant="ghost" size="sm" onClick={() => setSaveOpen(true)}>
                Save view
              </Button>
            )}
            {!embedded && displayMenu}
          </div>
        </header>
      )}
      <div className="min-h-0 flex-1 overflow-auto">
        {body}
        {hidden > 0 && (
          <p className="px-4 py-3 text-xs text-muted-foreground">
            {hidden} issues hidden by display options.
          </p>
        )}
        {q.data?.truncated && (
          <p className="px-4 py-3 text-xs text-muted-foreground">
            Showing the first {q.data.issues.length} issues. Narrow the filter to see the rest.
          </p>
        )}
      </div>
      <IssuePickerHost request={request} onClose={() => setRequest(null)} />
      {saveOpen && (
        <ViewBuilderDialog
          ws={ws}
          initialChips={chips}
          initialOptions={options}
          teamId={teamId}
          onClose={() => setSaveOpen(false)}
        />
      )}
    </div>
  );
}
