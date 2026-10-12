/**
 * Security-Seite (`/{ws}/settings/security`, nur Owner/Admin): Mindestrolle je Recht und erlaubte
 * Einladungs-Domains. Speichern fragt nach, zeigt die Änderungen und die Serverfehler.
 */
import { useMemo, useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { errorText } from "@/features/issues/error-text";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import { showToast } from "@/features/issues/toast-store";
import { useShortcutContext } from "@/lib/shortcuts";

import { workspaceSettingsNav } from "./nav";
import {
  parseDomains,
  ROLE_FIELDS,
  ROLE_OPTIONS,
  ROLE_QUESTIONS,
  useSecuritySettings,
  useUpdateSecuritySettings,
  type RoleField,
  type SecuritySettings,
  type SecurityUpdate,
} from "./security-queries";
import { Section, selectClass, SettingsShell } from "./ui";

const sameList = (a: string[], b: string[]) =>
  a.length === b.length && a.every((x, i) => x.toLowerCase() === b[i]!.toLowerCase());

interface Draft {
  roles: Record<RoleField, string>;
  domains: string;
}

function toDraft(s: SecuritySettings): Draft {
  const roles = Object.fromEntries(ROLE_FIELDS.map((f) => [f, s[f]])) as Record<RoleField, string>;
  return { roles, domains: s.allowedInviteDomains.join("\n") };
}

function diff(saved: SecuritySettings, draft: Draft): SecurityUpdate {
  const update: SecurityUpdate = {};
  for (const f of ROLE_FIELDS) if (draft.roles[f] !== saved[f]) update[f] = draft.roles[f];
  const domains = parseDomains(draft.domains);
  if (!sameList(domains, saved.allowedInviteDomains)) update.allowedInviteDomains = domains;
  return update;
}

function ConfirmDialog({
  open,
  update,
  saved,
  onCancel,
  onConfirm,
  pending,
  error,
}: {
  open: boolean;
  update: SecurityUpdate;
  saved: SecuritySettings;
  onCancel: () => void;
  onConfirm: () => void;
  pending: boolean;
  error: unknown;
}) {
  useShortcutContext("modal", open);
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onCancel()}>
      <DialogContent className="max-w-md">
        <div className="flex flex-col gap-3 p-4">
          <DialogTitle className="text-base font-semibold">Save security settings?</DialogTitle>
          <DialogDescription className="text-sm text-muted-foreground">
            The new rules apply to everyone in the workspace immediately.
          </DialogDescription>
          <ul aria-label="Changes" className="flex flex-col gap-1 text-sm">
            {ROLE_FIELDS.filter((f) => update[f] !== undefined).map((f) => (
              <li key={f}>
                {ROLE_QUESTIONS[f]} {ROLE_OPTIONS[saved[f]] ?? saved[f]} →{" "}
                <strong>{ROLE_OPTIONS[update[f]!] ?? update[f]}</strong>
              </li>
            ))}
            {update.allowedInviteDomains !== undefined && (
              <li>
                Allowed invite domains:{" "}
                <strong>
                  {update.allowedInviteDomains.length > 0
                    ? update.allowedInviteDomains.join(", ")
                    : "all domains"}
                </strong>
              </li>
            )}
          </ul>
          {error != null && (
            <p role="alert" className="text-xs text-destructive">
              {errorText(error)}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
            <Button type="button" disabled={pending} onClick={onConfirm}>
              Save changes
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function SecurityForm({ saved, isOwner }: { saved: SecuritySettings; isOwner: boolean }) {
  const update = useUpdateSecuritySettings();
  const [draft, setDraft] = useState<Draft>(() => toDraft(saved));
  const [confirming, setConfirming] = useState(false);
  const changes = useMemo(() => diff(saved, draft), [saved, draft]);
  const dirty = Object.keys(changes).length > 0;

  return (
    <form
      className="flex flex-col gap-6"
      aria-label="Security settings"
      onSubmit={(e) => {
        e.preventDefault();
        if (dirty) {
          update.reset();
          setConfirming(true);
        }
      }}
    >
      <Section
        title="Permissions"
        description="Choose the minimum role needed for each action. Owners can always do everything."
      >
        <ul className="divide-y divide-border rounded-lg border border-border">
          {ROLE_FIELDS.map((f) => {
            const locked = f === "adminManagementRole" && !isOwner;
            return (
              <li key={f} className="flex items-center gap-3 px-3 py-2 text-sm">
                <span className="min-w-0 flex-1">
                  {ROLE_QUESTIONS[f]}
                  {locked && (
                    <span className="block text-xs text-muted-foreground">
                      Only owners can change this.
                    </span>
                  )}
                </span>
                <select
                  aria-label={ROLE_QUESTIONS[f]}
                  className={selectClass}
                  disabled={locked}
                  value={draft.roles[f]}
                  onChange={(e) =>
                    setDraft((d) => ({ ...d, roles: { ...d.roles, [f]: e.target.value } }))
                  }
                >
                  {Object.entries(ROLE_OPTIONS)
                    .filter(([v]) => f !== "adminManagementRole" || v !== "user")
                    .map(([v, label]) => (
                      <option key={v} value={v}>
                        {label}
                      </option>
                    ))}
                </select>
              </li>
            );
          })}
        </ul>
      </Section>
      <Section
        title="Invitation domains"
        description="Only email addresses from these domains can be invited or added. Leave empty to allow every domain."
      >
        <textarea
          aria-label="Allowed invite domains"
          className="min-h-20 rounded-md border border-input bg-transparent px-2.5 py-1.5 text-sm"
          placeholder="acme.example"
          value={draft.domains}
          onChange={(e) => setDraft((d) => ({ ...d, domains: e.target.value }))}
        />
        <p className="text-xs text-muted-foreground">
          One domain per line, without @. Subdomains must be listed separately.
        </p>
      </Section>
      <div className="flex gap-2">
        <Button type="submit" size="sm" disabled={!dirty}>
          Save…
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={!dirty}
          onClick={() => setDraft(toDraft(saved))}
        >
          Reset
        </Button>
      </div>
      <ConfirmDialog
        open={confirming}
        update={changes}
        saved={saved}
        pending={update.isPending}
        error={update.error}
        onCancel={() => setConfirming(false)}
        onConfirm={() =>
          update.mutate(changes, {
            onSuccess: (next) => {
              setDraft(toDraft(next));
              setConfirming(false);
              showToast({ message: "Security settings saved" });
            },
          })
        }
      />
    </form>
  );
}

/** Security-Einstellungen des Workspace. Für Nicht-Admins nur ein Hinweis. */
export function SecuritySettingsPage({ ws }: { ws: string }) {
  const session = useSession();
  const role = session?.user.role;
  const admin = role === "owner" || role === "admin";
  const q = useSecuritySettings(admin);
  return (
    <SettingsShell title="Settings" nav={workspaceSettingsNav(ws, session, "security")}>
      {!admin ? (
        <p className="text-sm text-muted-foreground">
          Only workspace owners and admins can change security settings.
        </p>
      ) : (
        <>
          {q.isLoading && <ListSkeleton rows={4} />}
          {q.isError && (
            <ErrorState
              error={q.error}
              title="Could not load security settings"
              onRetry={() => void q.refetch()}
            />
          )}
          {q.data && <SecurityForm saved={q.data} isOwner={role === "owner"} />}
        </>
      )}
    </SettingsShell>
  );
}
