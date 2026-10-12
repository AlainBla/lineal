/**
 * Account → Notifications (`/{ws}/settings/account/notifications`): Kanäle je Kategorie (Inbox
 * immer an, E-Mail schaltbar), E-Mail-Optionen, Priority-Kategorien und Objekt-Abos (docs/05 §6.9).
 */
import { useSession } from "@/app/auth";
import { errorText } from "@/features/issues/error-text";
import { ErrorState, ListSkeleton } from "@/features/issues/states";

import {
  emailEnabledFor,
  NOTIFICATION_CATEGORIES,
  useNotificationSettings,
  useNotificationSubscriptions,
  useToggleSubscription,
  useUpdateNotificationSettings,
  type NotificationSettings,
  type ObjectSubscription,
} from "./account-queries";
import { workspaceSettingsNav } from "./nav";
import { Section, selectClass, SettingsShell } from "./ui";

function Toggle({
  label,
  hint,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex items-start gap-2 text-sm">
      <input
        type="checkbox"
        className="mt-1"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>
        {label}
        {hint && <span className="block text-xs text-muted-foreground">{hint}</span>}
      </span>
    </label>
  );
}

function ChannelMatrix({ s }: { s: NotificationSettings }) {
  const update = useUpdateNotificationSettings();
  return (
    <table aria-label="Notification channels" className="w-full text-sm">
      <thead>
        <tr className="text-left text-xs text-muted-foreground">
          <th className="py-1 font-normal">Category</th>
          <th className="w-20 py-1 font-normal">In-app</th>
          <th className="w-20 py-1 font-normal">Email</th>
        </tr>
      </thead>
      <tbody className="divide-y divide-border">
        {NOTIFICATION_CATEGORIES.map((c) => (
          <tr key={c.id}>
            <td className="py-1.5">{c.label}</td>
            <td>
              <input
                type="checkbox"
                aria-label={`${c.label} in-app`}
                checked
                disabled
                title="The inbox is always on"
                readOnly
              />
            </td>
            <td>
              <input
                type="checkbox"
                aria-label={`${c.label} email`}
                checked={emailEnabledFor(s, c.id)}
                disabled={!s.emailEnabled || update.isPending}
                onChange={(e) =>
                  update.mutate({
                    channels: [{ category: c.id, channel: "email", enabled: e.target.checked }],
                  })
                }
              />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function EmailOptions({ s }: { s: NotificationSettings }) {
  const update = useUpdateNotificationSettings();
  const set = (patch: Parameters<typeof update.mutate>[0]) => update.mutate(patch);
  return (
    <div className="flex flex-col gap-3">
      <Toggle
        label="Send email notifications"
        hint="Main switch for all email."
        checked={s.emailEnabled}
        onChange={(v) => set({ emailEnabled: v })}
      />
      <label className="flex items-center gap-2 text-sm">
        <span>Email format</span>
        <select
          aria-label="Email format"
          className={selectClass}
          value={s.emailFormat}
          disabled={!s.emailEnabled}
          onChange={(e) => set({ emailFormat: e.target.value })}
        >
          <option value="digest">Digest</option>
          <option value="immediate">Immediately</option>
        </select>
      </label>
      <Toggle
        label="Only priority notifications by email"
        checked={s.emailOnlyPriority}
        disabled={!s.emailEnabled}
        onChange={(v) => set({ emailOnlyPriority: v })}
      />
      <Toggle
        label="Hold low-urgency email outside work hours"
        hint="Sent the next working day at 08:00 in your time zone."
        checked={s.emailHoldOutsideWorkHours}
        disabled={!s.emailEnabled}
        onChange={(v) => set({ emailHoldOutsideWorkHours: v })}
      />
      <Toggle
        label="Notify immediately for urgent issues and SLA breaches"
        checked={s.emailImmediateUrgent}
        disabled={!s.emailEnabled}
        onChange={(v) => set({ emailImmediateUrgent: v })}
      />
      <Toggle
        label="Due date reminders"
        hint="For issues assigned to you."
        checked={s.dueDateReminders}
        onChange={(v) => set({ dueDateReminders: v })}
      />
      {update.error != null && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(update.error)}
        </p>
      )}
    </div>
  );
}

function PriorityCategories({ s }: { s: NotificationSettings }) {
  const update = useUpdateNotificationSettings();
  const has = (id: string) => s.priorityCategories.includes(id);
  return (
    <div role="group" aria-label="Priority categories" className="grid grid-cols-2 gap-x-4 gap-y-2">
      {NOTIFICATION_CATEGORIES.map((c) => (
        <Toggle
          key={c.id}
          label={c.label}
          checked={has(c.id)}
          disabled={update.isPending}
          onChange={(v) =>
            update.mutate({
              priorityCategories: v
                ? [...s.priorityCategories, c.id]
                : s.priorityCategories.filter((x) => x !== c.id),
            })
          }
        />
      ))}
    </div>
  );
}

function SubscriptionRow({ sub, teamName }: { sub: ObjectSubscription; teamName: string | null }) {
  const toggle = useToggleSubscription();
  const kind = sub.targetType[0]!.toUpperCase() + sub.targetType.slice(1);
  const label = teamName ?? `${kind} ${sub.targetId.slice(0, 8)}`;
  return (
    <li className="flex items-center gap-3 px-3 py-2 text-sm">
      <span className="min-w-0 flex-1 truncate">
        {label}
        <span className="ml-2 text-xs text-muted-foreground">{kind}</span>
      </span>
      <Toggle
        label={`Notify me about ${label}`}
        checked={sub.active}
        disabled={toggle.isPending}
        onChange={(active) => toggle.mutate({ sub, active })}
      />
    </li>
  );
}

function Subscriptions() {
  const q = useNotificationSubscriptions();
  const session = useSession();
  const teams = new Map((session?.teams ?? []).map((t) => [t.id, t.name]));
  if (q.isLoading) return <ListSkeleton rows={2} />;
  if (q.isError) return null;
  const rows = q.data ?? [];
  if (rows.length === 0) {
    return <p className="text-sm text-muted-foreground">You have no object subscriptions yet.</p>;
  }
  return (
    <ul
      aria-label="Subscriptions"
      className="divide-y divide-border rounded-lg border border-border"
    >
      {rows.map((s) => (
        <SubscriptionRow
          key={s.id}
          sub={s}
          teamName={s.targetType === "team" ? (teams.get(s.targetId) ?? null) : null}
        />
      ))}
    </ul>
  );
}

export function AccountNotificationsPage({ ws }: { ws: string }) {
  const session = useSession();
  const q = useNotificationSettings();
  return (
    <SettingsShell
      title="Settings"
      nav={workspaceSettingsNav(ws, session, { account: "notifications" })}
    >
      {q.isLoading && <ListSkeleton rows={4} />}
      {q.isError && (
        <ErrorState
          error={q.error}
          title="Could not load notification settings"
          onRetry={() => void q.refetch()}
        />
      )}
      {q.data && (
        <>
          <Section
            title="Channels"
            description="Choose where you are notified for each kind of activity. The inbox is always on."
          >
            <ChannelMatrix s={q.data} />
          </Section>
          <Section title="Email" description="When and how email notifications are sent.">
            <EmailOptions s={q.data} />
          </Section>
          <Section
            title="Priority inbox"
            description="Notifications of these categories show up under Priority in your inbox."
          >
            <PriorityCategories s={q.data} />
          </Section>
        </>
      )}
      <Section
        title="Subscriptions"
        description="Teams, projects, initiatives, cycles and labels you follow."
      >
        <Subscriptions />
      </Section>
    </SettingsShell>
  );
}
