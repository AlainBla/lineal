import { useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { useIssueActions } from "@/features/issues/issue-actions";
import { useIssueLookup } from "@/features/issues/issue-lookup";
import { IssueRow } from "@/features/issues/issue-row";
import { IssuePickerHost, type PickerRequest } from "@/features/issues/picker-host";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import { useFocusNav } from "@/features/issues/use-focus-nav";
import { useIssueShortcuts } from "@/features/issues/use-issue-shortcuts";
import type { CustomView, Issue } from "@/gql/graphql";
import { GraphQLRequestError } from "@/lib/graphql";

import { DisplayMenu } from "./display-menu";
import {
  applyVisibility,
  groupIssues,
  mergeDisplay,
  sortIssues,
  type DisplayOptions,
} from "./display-options";
import { FilterBar } from "./filter-bar";
import { buildFilter, type Filter, type FilterChip } from "./filter-model";
import { combineFilter, filterToChips } from "./filter-parse";
import {
  usePreferences,
  useResetPreferences,
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
}: {
  ws: string;
  view?: CustomView;
  teamId?: string;
  title: string;
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

  const target: PrefTarget | null = view ? { customViewId: view.id } : null;
  const prefs = usePreferences(target);
  const updatePrefs = useUpdatePreferences(target);
  const resetPrefs = useResetPreferences(target);
  const options = useMemo(
    () => ({
      ...mergeDisplay(prefs.data?.values as Record<string, unknown> | undefined),
      ...local,
    }),
    [prefs.data, local],
  );
  const onOptions = (patch: Partial<DisplayOptions>) => {
    setLocal((l) => ({ ...l, ...patch }));
    if (view) updatePrefs.mutate(patch);
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
  const q = useViewIssues({
    customViewId: view && !touched ? view.id : undefined,
    filter: view && !touched ? undefined : scoped,
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
    const groups = groupIssues(sorted, options.grouping, {
      assignee: lookup.assigneeName,
      project: lookup.projectName,
      team: (id) => teamName.get(id) ?? null,
      cycle: (id) => cycleName.get(id) ?? null,
    });
    return { groups, hidden, flat: groups.flatMap((g) => g.issues) };
  }, [q.data, options, teams.data, metas, lookup]);

  const focused: Issue | null = flat.find((i) => i.id === focusedId) ?? null;
  useFocusNav([flat.map((i) => i.id)], focused?.id ?? null, setFocusedId);
  useIssueShortcuts({ context: "list", target: focused, open: setRequest, actions });

  let body;
  if (q.isLoading) body = <ListSkeleton />;
  else if (q.isError) body = <ErrorState error={q.error} onRetry={() => void q.refetch()} />;
  else if (flat.length === 0)
    body = <EmptyState title="No matching issues" description="Adjust the filters to see more." />;
  else
    body = groups.map((g) => (
      <section key={g.key} aria-label={g.label}>
        <h2 className="flex h-8 items-center gap-2 border-b border-border bg-surface px-4 text-sm font-medium">
          {g.label}
          <span className="text-xs text-muted-foreground">{g.issues.length}</span>
        </h2>
        {g.issues.map((issue) => (
          <IssueRow
            key={issue.id}
            issue={issue}
            ws={ws}
            lookup={lookup}
            focused={focused?.id === issue.id}
            onFocus={setFocusedId}
          />
        ))}
      </section>
    ));

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex min-h-10 shrink-0 flex-wrap items-center gap-2 border-b border-border px-3 py-1">
        <h1 className="text-sm font-medium">{title}</h1>
        <FilterBar
          chips={chips}
          onChange={setChips}
          sources={sources}
          advanced={advanced}
          onAdvancedChange={setAdvanced}
        />
        <div className="ml-auto flex items-center gap-1">
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
          <DisplayMenu
            options={options}
            onChange={onOptions}
            onReset={
              view
                ? () => {
                    setLocal({});
                    resetPrefs.mutate();
                  }
                : undefined
            }
          />
        </div>
      </header>
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
