/** Audit-Log des Workspace (GraphQL `auditLog`, nur Owner/Admin mit Scope admin). */
import { useInfiniteQuery } from "@tanstack/react-query";

import { gqlRequest } from "@/lib/graphql";

/** Seitengröße; der Server akzeptiert höchstens 100. */
export const AUDIT_PAGE_SIZE = 50;

export interface AuditEntry {
  id: string;
  occurredAt: string;
  actorKind: string;
  actorId: string | null;
  actorName: string | null;
  credentialKind: string | null;
  action: string;
  targetType: string;
  targetId: string | null;
  /** JSON-Text der Details (ohne Geheimnisse). */
  details: string;
  ip: string | null;
  requestId: string;
}

interface AuditVariables extends Record<string, unknown> {
  first: number;
  after: string | null;
  action: string | null;
  actorId: string | null;
  since: string | null;
}

interface AuditLogData {
  auditLog: {
    nodes: AuditEntry[];
    pageInfo: { hasNextPage: boolean; endCursor: string | null };
  };
}

export type AuditPeriod = "all" | "24h" | "7d" | "30d";

export interface AuditFilters {
  /** Aktion aus `AUDIT_ACTIONS`; leer = alle. */
  action: string;
  /** User-ID; leer = alle Akteure. */
  actorId: string;
  period: AuditPeriod;
}

export const EMPTY_AUDIT_FILTERS: AuditFilters = { action: "", actorId: "", period: "all" };

/** Bekannte Aktionen mit lesbarer Beschriftung (Reihenfolge = Auswahl im Filter). */
export const AUDIT_ACTIONS: ReadonlyArray<{ value: string; label: string }> = [
  { value: "auth.login_succeeded", label: "Signed in" },
  { value: "auth.login_failed", label: "Failed sign-in attempt" },
  { value: "auth.logout", label: "Signed out" },
  { value: "workspace.created", label: "Workspace created" },
  { value: "workspace.updated", label: "Workspace settings changed" },
  { value: "workspace.security_settings_updated", label: "Security settings changed" },
  { value: "user.role_changed", label: "Role changed" },
  { value: "user.password_changed", label: "Password changed" },
  { value: "user.password_change_failed", label: "Failed password change" },
  { value: "user.profile_updated", label: "Profile updated" },
  { value: "user.session_revoked", label: "Session signed out" },
  { value: "user.sessions_revoked", label: "Other sessions signed out" },
  { value: "user.suspended", label: "Member suspended" },
  { value: "user.reactivated", label: "Member reactivated" },
  { value: "invitation.created", label: "Invitation sent" },
  { value: "invitation.resent", label: "Invitation resent" },
  { value: "invitation.revoked", label: "Invitation revoked" },
  { value: "invitation.accepted", label: "Invitation accepted" },
  { value: "api_key.created", label: "API key created" },
  { value: "api_key.revoked", label: "API key revoked" },
  { value: "oauth_app.authorized", label: "OAuth app authorized" },
  { value: "oauth_app.revoked", label: "OAuth app revoked" },
  { value: "import.started", label: "Import started" },
  { value: "import.completed", label: "Import completed" },
  { value: "team.created", label: "Team created" },
  { value: "team.archived", label: "Team archived" },
  { value: "team.deleted", label: "Team deleted" },
  { value: "team.restored", label: "Team restored" },
  { value: "team.updated", label: "Team settings changed" },
  { value: "team.visibility_changed", label: "Team visibility changed" },
  { value: "team.member_added", label: "Team member added" },
  { value: "team.member_removed", label: "Team member removed" },
  { value: "team.owner_changed", label: "Team owner changed" },
  { value: "webhook.created", label: "Webhook created" },
  { value: "webhook.deleted", label: "Webhook deleted" },
];

const CREDENTIAL_LABELS: Record<string, string> = {
  session: "browser session",
  api_key: "API key",
  oauth: "OAuth",
  cli: "command line",
};

/** Lesbare Beschriftung einer Aktion; unbekannte Aktionen bleiben als Kennung sichtbar. */
export function auditActionLabel(action: string): string {
  return AUDIT_ACTIONS.find((a) => a.value === action)?.label ?? action;
}

/** Beschriftung der Token-Art (`via …`), `null` ohne Zugangsdaten. */
export function auditCredentialLabel(kind: string | null): string | null {
  return kind ? (CREDENTIAL_LABELS[kind] ?? kind) : null;
}

const PERIOD_HOURS: Record<Exclude<AuditPeriod, "all">, number> = {
  "24h": 24,
  "7d": 24 * 7,
  "30d": 24 * 30,
};

/** Beginn des gewählten Zeitraums als ISO-Zeitpunkt; `null` = unbegrenzt. */
export function periodSince(period: AuditPeriod, now: Date): string | null {
  if (period === "all") return null;
  return new Date(now.getTime() - PERIOD_HOURS[period] * 3_600_000).toISOString();
}

/** Relative Zeitangabe („5 minutes ago“) relativ zu `now`. */
export function relativeTime(iso: string, now: Date): string {
  const seconds = (new Date(iso).getTime() - now.getTime()) / 1000;
  const rtf = new Intl.RelativeTimeFormat("en", { numeric: "auto" });
  const units: Array<[Intl.RelativeTimeFormatUnit, number]> = [
    ["year", 31_536_000],
    ["month", 2_592_000],
    ["week", 604_800],
    ["day", 86_400],
    ["hour", 3_600],
    ["minute", 60],
  ];
  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) return rtf.format(Math.round(seconds / size), unit);
  }
  return rtf.format(Math.round(seconds), "second");
}

const FIELDS =
  "id occurredAt actorKind actorId actorName credentialKind action targetType targetId details ip requestId";

export const auditKey = (filters: AuditFilters) => ["settings", "audit-log", filters] as const;

/** Seiten des Audit-Logs (Cursor-Pagination über „Mehr laden“). */
export function useAuditLog(filters: AuditFilters, enabled = true) {
  return useInfiniteQuery({
    queryKey: auditKey(filters),
    enabled,
    retry: false,
    initialPageParam: null as string | null,
    queryFn: async ({ pageParam, signal }) =>
      (
        await gqlRequest<AuditLogData, AuditVariables>(
          `query SettingsAuditLog($first: Int, $after: String, $action: String, $actorId: String, $since: String) { auditLog(first: $first, after: $after, action: $action, actorId: $actorId, since: $since) { nodes { ${FIELDS} } pageInfo { hasNextPage endCursor } } }`,
          {
            first: AUDIT_PAGE_SIZE,
            after: pageParam,
            action: filters.action || null,
            actorId: filters.actorId || null,
            since: periodSince(filters.period, new Date()),
          },
          { signal },
        )
      ).auditLog,
    getNextPageParam: (last) => (last.pageInfo.hasNextPage ? last.pageInfo.endCursor : undefined),
  });
}
