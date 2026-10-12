/**
 * Account → Password & sessions (`/{ws}/settings/account/security`): Passwort ändern (altes
 * Passwort nötig, danach sind alle anderen Sessions abgemeldet) und aktive Sessions verwalten.
 */
import { useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import { showToast } from "@/features/issues/toast-store";

import {
  describeAgent,
  usePasswordChange,
  useRevokeOtherSessions,
  useRevokeSession,
  useSessions,
  type UserSession,
} from "./account-queries";
import { workspaceSettingsNav } from "./nav";
import { Field, Section, SettingsShell } from "./ui";

const MIN_PASSWORD = 8;

function PasswordForm() {
  const change = usePasswordChange();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [repeat, setRepeat] = useState("");
  const tooShort = next !== "" && next.length < MIN_PASSWORD;
  const mismatch = repeat !== "" && repeat !== next;
  const same = next !== "" && next === current;
  const valid = current !== "" && next.length >= MIN_PASSWORD && repeat === next && !same;

  return (
    <form
      aria-label="Change password"
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!valid) return;
        change.mutate(
          { currentPassword: current, newPassword: next },
          {
            onSuccess: (r) => {
              setCurrent("");
              setNext("");
              setRepeat("");
              showToast({
                message:
                  r.revokedSessions > 0
                    ? `Password changed. ${r.revokedSessions} other session${r.revokedSessions === 1 ? " was" : "s were"} signed out.`
                    : "Password changed.",
              });
            },
          },
        );
      }}
    >
      <Field label="Current password">
        <Input
          type="password"
          autoComplete="current-password"
          aria-label="Current password"
          value={current}
          onChange={(e) => setCurrent(e.target.value)}
        />
      </Field>
      <Field label="New password">
        <Input
          type="password"
          autoComplete="new-password"
          aria-label="New password"
          value={next}
          onChange={(e) => setNext(e.target.value)}
        />
        {tooShort && (
          <span role="alert" className="text-xs text-destructive">
            Use at least {MIN_PASSWORD} characters.
          </span>
        )}
        {same && (
          <span role="alert" className="text-xs text-destructive">
            The new password must differ from the current one.
          </span>
        )}
      </Field>
      <Field label="Repeat new password">
        <Input
          type="password"
          autoComplete="new-password"
          aria-label="Repeat new password"
          value={repeat}
          onChange={(e) => setRepeat(e.target.value)}
        />
        {mismatch && (
          <span role="alert" className="text-xs text-destructive">
            The passwords do not match.
          </span>
        )}
      </Field>
      {change.error != null && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(change.error)}
        </p>
      )}
      <div>
        <Button type="submit" size="sm" disabled={!valid || change.isPending}>
          Change password
        </Button>
      </div>
    </form>
  );
}

const when = (iso: string) => new Date(iso).toISOString().slice(0, 16).replace("T", " ");

function SessionItem({ s }: { s: UserSession }) {
  const revoke = useRevokeSession();
  return (
    <li className="flex items-center gap-3 px-3 py-2 text-sm">
      <div className="min-w-0 flex-1">
        <p className="font-medium">
          {describeAgent(s.userAgent)}
          {s.current && (
            <span className="ml-2 rounded bg-surface-active px-1.5 py-0.5 text-xs font-normal">
              This device
            </span>
          )}
        </p>
        <p className="text-xs text-muted-foreground">
          {s.ipAddress ? `${s.ipAddress} · ` : ""}signed in {when(s.createdAt)} UTC
          {s.lastUsedAt ? ` · last used ${when(s.lastUsedAt)} UTC` : ""}
        </p>
        {revoke.error != null && (
          <p role="alert" className="text-xs text-destructive">
            {errorText(revoke.error)}
          </p>
        )}
      </div>
      {!s.current && (
        <Button
          size="sm"
          variant="outline"
          aria-label={`Sign out ${describeAgent(s.userAgent)}`}
          disabled={revoke.isPending}
          onClick={() => revoke.mutate(s.id)}
        >
          Sign out
        </Button>
      )}
    </li>
  );
}

function SessionsList() {
  const q = useSessions();
  const others = useRevokeOtherSessions();
  const rows = q.data ?? [];
  const otherCount = rows.filter((s) => !s.current).length;
  return (
    <>
      {q.isLoading && <ListSkeleton rows={2} />}
      {q.isError && (
        <ErrorState
          error={q.error}
          title="Could not load your sessions"
          onRetry={() => void q.refetch()}
        />
      )}
      {q.data && (
        <ul
          aria-label="Active sessions"
          className="divide-y divide-border rounded-lg border border-border"
        >
          {rows.map((s) => (
            <SessionItem key={s.id} s={s} />
          ))}
        </ul>
      )}
      {others.error != null && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(others.error)}
        </p>
      )}
      <div>
        <Button
          size="sm"
          variant="outline"
          disabled={otherCount === 0 || others.isPending}
          onClick={() =>
            others.mutate(undefined, {
              onSuccess: (n) =>
                showToast({ message: `Signed out ${n} other session${n === 1 ? "" : "s"}` }),
            })
          }
        >
          Sign out all other sessions
        </Button>
      </div>
    </>
  );
}

export function AccountSecurityPage({ ws }: { ws: string }) {
  const session = useSession();
  return (
    <SettingsShell
      title="Settings"
      nav={workspaceSettingsNav(ws, session, { account: "security" })}
    >
      <Section title="Password" description="Changing your password signs you out everywhere else.">
        <PasswordForm />
      </Section>
      <Section
        title="Active sessions"
        description="Browsers that are currently signed in to your account."
      >
        <SessionsList />
      </Section>
    </SettingsShell>
  );
}
