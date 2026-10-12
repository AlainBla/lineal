/**
 * Audit-Log (`/{ws}/settings/audit`, nur Owner/Admin): wer hat wann was geändert. Zeitpunkt
 * relativ mit absoluter Angabe als Tooltip; Filter nach Aktion, Akteur und Zeitraum; „Mehr laden“.
 */
import { useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { errorText } from "@/features/issues/error-text";
import { ErrorState, ListSkeleton } from "@/features/issues/states";

import {
  AUDIT_ACTIONS,
  EMPTY_AUDIT_FILTERS,
  auditActionLabel,
  auditCredentialLabel,
  relativeTime,
  useAuditLog,
  type AuditEntry,
  type AuditFilters,
  type AuditPeriod,
} from "./audit-queries";
import { useMemberList } from "./members-queries";
import { workspaceSettingsNav } from "./nav";
import { Field, Section, selectClass, SettingsShell } from "./ui";

const PERIODS: Array<{ value: AuditPeriod; label: string }> = [
  { value: "all", label: "All time" },
  { value: "24h", label: "Last 24 hours" },
  { value: "7d", label: "Last 7 days" },
  { value: "30d", label: "Last 30 days" },
];

function actorText(e: AuditEntry): string {
  if (e.actorName) return e.actorName;
  if (e.actorKind === "system") return "System";
  if (e.actorKind === "app") return "App";
  return e.actorId ?? "—";
}

function targetText(e: AuditEntry): string {
  if (!e.targetId) return e.targetType;
  return `${e.targetType} · ${e.targetId.slice(0, 8)}`;
}

function AuditRow({ entry, now }: { entry: AuditEntry; now: Date }) {
  const absolute = new Date(entry.occurredAt).toLocaleString();
  const via = auditCredentialLabel(entry.credentialKind);
  return (
    <tr className="border-b border-border last:border-0">
      <td className="px-3 py-2 align-top whitespace-nowrap text-muted-foreground">
        <time dateTime={entry.occurredAt} title={absolute}>
          {relativeTime(entry.occurredAt, now)}
        </time>
      </td>
      <td className="px-3 py-2 align-top">
        <span>{actorText(entry)}</span>
        {via && <span className="block text-xs text-muted-foreground">via {via}</span>}
      </td>
      <td className="px-3 py-2 align-top" title={entry.details}>
        {auditActionLabel(entry.action)}
      </td>
      <td className="px-3 py-2 align-top text-muted-foreground">{targetText(entry)}</td>
      <td className="px-3 py-2 align-top font-mono text-xs whitespace-nowrap text-muted-foreground">
        {entry.ip ?? "—"}
      </td>
    </tr>
  );
}

function AuditTable() {
  const [filters, setFilters] = useState<AuditFilters>(EMPTY_AUDIT_FILTERS);
  const members = useMemberList();
  const log = useAuditLog(filters);
  // Zeitbezug für die relativen Angaben: je Abruf festgehalten, damit die Liste stabil bleibt.
  const [now] = useState(() => new Date());
  const update = (patch: Partial<AuditFilters>) => setFilters((f) => ({ ...f, ...patch }));
  const entries = log.data?.pages.flatMap((p) => p.nodes) ?? [];

  return (
    <>
      <form
        aria-label="Audit filters"
        className="flex flex-wrap items-end gap-3"
        onSubmit={(e) => e.preventDefault()}
      >
        <Field label="Action">
          <select
            aria-label="Action"
            className={selectClass}
            value={filters.action}
            onChange={(e) => update({ action: e.target.value })}
          >
            <option value="">All actions</option>
            {AUDIT_ACTIONS.map((a) => (
              <option key={a.value} value={a.value}>
                {a.label}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Actor">
          <select
            aria-label="Actor"
            className={selectClass}
            value={filters.actorId}
            onChange={(e) => update({ actorId: e.target.value })}
          >
            <option value="">All actors</option>
            {(members.data ?? []).map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Period">
          <select
            aria-label="Period"
            className={selectClass}
            value={filters.period}
            onChange={(e) => update({ period: e.target.value as AuditPeriod })}
          >
            {PERIODS.map((p) => (
              <option key={p.value} value={p.value}>
                {p.label}
              </option>
            ))}
          </select>
        </Field>
      </form>

      {log.isPending && <ListSkeleton rows={6} />}
      {log.isError && <ErrorState error={log.error} onRetry={() => void log.refetch()} />}
      {log.isSuccess && entries.length === 0 && (
        <p className="text-sm text-muted-foreground">No entries match these filters.</p>
      )}
      {log.isSuccess && entries.length > 0 && (
        <div className="overflow-x-auto rounded-lg border border-border">
          <table aria-label="Audit log" className="w-full text-left text-sm">
            <thead className="border-b border-border bg-surface text-xs text-muted-foreground">
              <tr>
                <th scope="col" className="px-3 py-2 font-medium">
                  When
                </th>
                <th scope="col" className="px-3 py-2 font-medium">
                  Actor
                </th>
                <th scope="col" className="px-3 py-2 font-medium">
                  Action
                </th>
                <th scope="col" className="px-3 py-2 font-medium">
                  Target
                </th>
                <th scope="col" className="px-3 py-2 font-medium">
                  IP
                </th>
              </tr>
            </thead>
            <tbody>
              {entries.map((e) => (
                <AuditRow key={e.id} entry={e} now={now} />
              ))}
            </tbody>
          </table>
        </div>
      )}
      {log.isSuccess && log.hasNextPage && (
        <div>
          <Button
            type="button"
            variant="ghost"
            disabled={log.isFetchingNextPage}
            onClick={() => void log.fetchNextPage()}
          >
            {log.isFetchingNextPage ? "Loading…" : "Load more"}
          </Button>
        </div>
      )}
      {log.isFetchNextPageError && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(log.error)}
        </p>
      )}
    </>
  );
}

/** Settings-Seite „Audit log“. Gäste, Mitglieder und Apps sehen nur einen Hinweis. */
export function AuditLogPage({ ws }: { ws: string }) {
  const session = useSession();
  const role = session?.user.role;
  const admin = role === "owner" || role === "admin";
  return (
    <SettingsShell title="Settings" nav={workspaceSettingsNav(ws, session, "audit")}>
      <Section
        title="Audit log"
        description="Security-relevant changes in this workspace: sign-ins, settings, roles, invitations, keys, apps, imports, teams and webhooks. Entries are kept for the retention period set in the security settings."
      >
        {admin ? (
          <AuditTable />
        ) : (
          <p className="text-sm text-muted-foreground">
            Only workspace owners and admins can view the audit log.
          </p>
        )}
      </Section>
    </SettingsShell>
  );
}
