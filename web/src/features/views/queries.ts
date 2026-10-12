/**
 * Daten der Views: Custom Views, Favoriten, Anzeigeoptionen, gefilterte Issue-Listen.
 * Alle Zugriffe der Komponenten laufen über diese Datei (WP15 ersetzt die Datenquellen dahinter).
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ISSUE_FIELDS, type IssueList } from "@/features/issues/queries";
import type {
  CustomView,
  CustomViewConnection,
  Favorite,
  IssueConnection,
  ViewPreferencesValues,
} from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

import type { Filter } from "./filter-model";

export const viewKeys = {
  views: (teamId?: string) => ["views", "list", teamId ?? ""] as const,
  view: (ref: string) => ["views", "view", ref] as const,
  favorites: ["views", "favorites"] as const,
  prefs: (target: string) => ["views", "prefs", target] as const,
};

const VIEW_FIELDS = `id slugId name description icon color modelName filterData ownerId ownerName shared teamId teamKey
  sortOrder version canEdit createdAt updatedAt archivedAt`;

export function useCustomViews(opts: { teamId?: string; modelName?: string } = {}) {
  return useQuery({
    queryKey: [...viewKeys.views(opts.teamId), opts.modelName ?? ""],
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ customViews: CustomViewConnection }, Record<string, unknown>>(
          `query ViewsList($teamId: String, $modelName: String) {
            customViews(teamId: $teamId, modelName: $modelName, first: 100) { nodes { ${VIEW_FIELDS} } hasNextPage endCursor }
          }`,
          { teamId: opts.teamId, modelName: opts.modelName },
          { signal },
        )
      ).customViews.nodes,
  });
}

export function useCustomView(ref: string) {
  return useQuery({
    queryKey: viewKeys.view(ref),
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ customView: CustomView }, { id: string }>(
          `query ViewDetail($id: String!) { customView(id: $id) { ${VIEW_FIELDS} } }`,
          { id: ref },
          { signal },
        )
      ).customView,
  });
}

export interface ViewIssuesParams {
  customViewId?: string;
  filter?: Filter;
  teamId?: string;
  includeSubTeams?: boolean;
}

/** `viewIssues(customViewId, filter)`: View UND Filter (docs/05 §4.3). Key unter `["issues"]` für optimistische Patches. */
export function useViewIssues(p: ViewIssuesParams, enabled = true) {
  return useQuery({
    queryKey: ["issues", "view", JSON.stringify(p)] as const,
    enabled,
    queryFn: async ({ signal }): Promise<IssueList> => {
      const res = await gqlRequest<{ viewIssues: IssueConnection }, Record<string, unknown>>(
        `query ViewIssuesList($customViewId: String, $filter: JSON, $teamId: String, $includeSubTeams: Boolean) {
          viewIssues(customViewId: $customViewId, filter: $filter, teamId: $teamId, includeSubTeams: $includeSubTeams, first: 250) {
            nodes { ${ISSUE_FIELDS} } hasNextPage endCursor
          }
        }`,
        {
          customViewId: p.customViewId,
          filter: p.filter && Object.keys(p.filter).length ? p.filter : undefined,
          teamId: p.teamId,
          includeSubTeams: p.includeSubTeams,
        },
        { signal },
      );
      return {
        issues: res.viewIssues.nodes.filter((i) => !i.trashed),
        truncated: res.viewIssues.hasNextPage,
      };
    },
  });
}

export interface ViewInput {
  name: string;
  description?: string;
  modelName: "Issue" | "Project";
  filterData: Filter;
  /** `personal` | `workspace` | Team-ID. */
  saveTo: string;
  preferences?: Record<string, unknown>;
}

export function useCreateView() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: ViewInput): Promise<CustomView> => {
      const input = {
        name: v.name,
        description: v.description || undefined,
        modelName: v.modelName,
        filterData: v.filterData,
        shared: v.saveTo !== "personal",
        teamId: v.saveTo === "personal" || v.saveTo === "workspace" ? undefined : v.saveTo,
        preferences: v.preferences,
      };
      const res = await gqlRequest<
        { customViewCreate: { success: boolean; customView: CustomView | null } },
        { input: typeof input }
      >(
        `mutation ViewCreate($input: CustomViewCreateInput!) {
          customViewCreate(input: $input) { success customView { ${VIEW_FIELDS} } }
        }`,
        { input },
      );
      if (!res.customViewCreate.customView) throw new Error("Could not create view");
      return res.customViewCreate.customView;
    },
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["views"] }),
  });
}

export function useDeleteView() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) =>
      gqlRequest(`mutation ViewDelete($id: String!) { customViewDelete(id: $id) { success } }`, {
        id,
      }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["views"] }),
  });
}

// ---------- Favoriten ----------

const FAV_FIELDS = `id type parentId folderName sortOrder targetId predefinedViewType predefinedViewTeamId`;

export function useFavorites() {
  return useQuery({
    queryKey: viewKeys.favorites,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ favorites: Favorite[] }>(
          `query ViewsFavorites { favorites { ${FAV_FIELDS} } }`,
          undefined,
          { signal },
        )
      ).favorites,
    staleTime: 30_000,
  });
}

export function useToggleFavorite() {
  const qc = useQueryClient();
  return {
    add: useMutation({
      mutationFn: async (input: { type: string; targetId?: string }) =>
        gqlRequest(
          `mutation FavoriteCreate($input: FavoriteCreateInput!) { favoriteCreate(input: $input) { success } }`,
          { input },
        ),
      onSuccess: () => void qc.invalidateQueries({ queryKey: viewKeys.favorites }),
    }),
    remove: useMutation({
      mutationFn: async (id: string) =>
        gqlRequest(
          `mutation FavoriteDelete($id: String!) { favoriteDelete(id: $id) { success } }`,
          { id },
        ),
      // Optimistisch: Eintrag sofort ausblenden.
      onMutate: async (id) => {
        await qc.cancelQueries({ queryKey: viewKeys.favorites });
        const prev = qc.getQueryData<Favorite[]>(viewKeys.favorites);
        qc.setQueryData<Favorite[]>(viewKeys.favorites, (old) => old?.filter((f) => f.id !== id));
        return { prev };
      },
      onError: (_e, _id, ctx) => qc.setQueryData(viewKeys.favorites, ctx?.prev),
      onSettled: () => void qc.invalidateQueries({ queryKey: viewKeys.favorites }),
    }),
  };
}

// ---------- Anzeigeoptionen ----------

export interface PrefTarget {
  customViewId?: string;
  viewType?: string;
  teamId?: string;
}

export function usePreferences(t: PrefTarget | null) {
  return useQuery({
    queryKey: viewKeys.prefs(JSON.stringify(t)),
    enabled: !!t,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ viewPreferences: ViewPreferencesValues }, Record<string, unknown>>(
          `query ViewPrefs($customViewId: String, $viewType: String, $teamId: String) {
            viewPreferences(customViewId: $customViewId, viewType: $viewType, teamId: $teamId) { values hasPersonal hasDefault }
          }`,
          { ...t },
          { signal },
        )
      ).viewPreferences,
  });
}

/** Persönliche Änderung (nur die genannten Schlüssel); optimistisch im Cache. */
export function useUpdatePreferences(t: PrefTarget | null) {
  const qc = useQueryClient();
  const key = viewKeys.prefs(JSON.stringify(t));
  return useMutation({
    mutationFn: async (preferences: Record<string, unknown>) =>
      gqlRequest(
        `mutation ViewPrefsUpdate($customViewId: String, $viewType: String, $teamId: String, $preferences: JSON!) {
          viewPreferencesUpdate(customViewId: $customViewId, viewType: $viewType, teamId: $teamId, preferences: $preferences) { hasPersonal }
        }`,
        { ...t, preferences },
      ),
    onMutate: async (preferences) => {
      await qc.cancelQueries({ queryKey: key });
      const prev = qc.getQueryData<ViewPreferencesValues>(key);
      if (prev) {
        qc.setQueryData<ViewPreferencesValues>(key, {
          ...prev,
          hasPersonal: true,
          values: { ...(prev.values as Record<string, unknown>), ...preferences },
        });
      }
      return { prev };
    },
    onError: (_e, _p, ctx) => ctx?.prev && qc.setQueryData(key, ctx.prev),
    onSettled: () => void qc.invalidateQueries({ queryKey: key }),
  });
}

export function useResetPreferences(t: PrefTarget | null) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () =>
      gqlRequest(
        `mutation ViewPrefsReset($customViewId: String, $viewType: String, $teamId: String) {
          viewPreferencesReset(customViewId: $customViewId, viewType: $viewType, teamId: $teamId) { hasPersonal }
        }`,
        { ...t },
      ),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["views", "prefs"] }),
  });
}

// ---------- Personen (Assignee-Filter, Picker) ----------

// ---------- Favoriten sortieren (WP31) ----------

/** Verschiebt einen Favoriten (Mittelwert-`sortOrder`, optional in/aus einem Ordner); optimistisch. */
export function useMoveFavorite() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { id: string; sortOrder: number; parentId: string | null }) =>
      gqlRequest(
        `mutation FavoriteMove($id: String!, $sortOrder: Float!, $parentId: String) {
          favoriteUpdate(id: $id, sortOrder: $sortOrder, parentId: $parentId) { success }
        }`,
        v,
      ),
    onMutate: async (v) => {
      await qc.cancelQueries({ queryKey: viewKeys.favorites });
      const prev = qc.getQueryData<Favorite[]>(viewKeys.favorites);
      qc.setQueryData<Favorite[]>(viewKeys.favorites, (old) =>
        old?.map((f) =>
          f.id === v.id ? { ...f, sortOrder: v.sortOrder, parentId: v.parentId } : f,
        ),
      );
      return { prev };
    },
    onError: (_e, _v, ctx) => ctx?.prev && qc.setQueryData(viewKeys.favorites, ctx.prev),
    onSettled: () => void qc.invalidateQueries({ queryKey: viewKeys.favorites }),
  });
}

// ---------- Views ändern, Eigentümer, Abos (WP31) ----------

/** `customViewUpdate`: Filter speichern, Eigentümer wechseln (`ownerId`) u. a. */
export function useUpdateView() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { id: string; input: Record<string, unknown> }) =>
      gqlRequest(
        `mutation ViewUpdate($id: String!, $input: CustomViewUpdateInput!) {
          customViewUpdate(id: $id, input: $input) { success }
        }`,
        v,
      ),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["views"] });
      void qc.invalidateQueries({ queryKey: ["issues", "view"] });
    },
  });
}

export const VIEW_SUBSCRIPTION_TYPES = [
  { value: "issueAddedToView", label: "New issue in this view" },
  { value: "issueStatusChanged", label: "Issue completed or canceled" },
  { value: "issueStatusChangedAll", label: "Every status change" },
] as const;

export interface ViewSubscriptionState {
  active: boolean;
  notificationSubscriptionTypes: string[];
}

export function useViewSubscription(viewId: string) {
  return useQuery({
    queryKey: ["views", "subscription", viewId] as const,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ viewSubscription: ViewSubscriptionState | null }, { id: string }>(
          `query ViewSubscriptionGet($id: String!) {
            viewSubscription(customViewId: $id) { active notificationSubscriptionTypes }
          }`,
          { id: viewId },
          { signal },
        )
      ).viewSubscription,
  });
}

export function useSetViewSubscription(viewId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { active: boolean; types?: string[] }) =>
      gqlRequest(
        `mutation ViewSubscriptionSet($id: String!, $active: Boolean!, $types: [String!]) {
          viewSubscriptionUpdate(customViewId: $id, active: $active, notificationSubscriptionTypes: $types) { active }
        }`,
        { id: viewId, active: v.active, types: v.types },
      ),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["views", "subscription", viewId] }),
  });
}
