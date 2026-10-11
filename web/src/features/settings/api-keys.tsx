/**
 * Persönliche API-Keys (`/{ws}/settings/api`): Liste, „Neuer Key“-Dialog mit einmaliger Token-Anzeige
 * und Widerruf mit Bestätigung. Der Klartext-Token lebt nur im Zustand des Dialogs und wird beim
 * Schließen verworfen; er gelangt nie in den Query-Cache.
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Copy } from "lucide-react";
import { useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import { showToast } from "@/features/issues/toast-store";
import type { ApiKey, ApiKeyScope } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { workspaceSettingsNav } from "./nav";
import { Field, Section, SettingsShell } from "./ui";

const KEY_FIELDS = `id label keyPrefix scopes teams { id key name } createdAt lastUsedAt expiresAt revokedAt`;
const API_KEYS_KEY = ["settings", "api-keys"] as const;
export const MAX_LABEL_LENGTH = 100;

export function useApiKeys() {
  return useQuery({
    queryKey: API_KEYS_KEY,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ apiKeys: ApiKey[] }>(
          `query SettingsApiKeys { apiKeys { ${KEY_FIELDS} } }`,
          undefined,
          { signal },
        )
      ).apiKeys,
  });
}

export interface NewKeyInput {
  label: string;
  scopes: ApiKeyScope[];
  teamIds?: string[];
  expiresAt?: string;
}

export function useCreateApiKey() {
  const qc = useQueryClient();
  return useMutation({
    // Das Ergebnis (mit Klartext-Token) wird nicht im Mutations-Cache gehalten.
    gcTime: 0,
    mutationFn: async (input: NewKeyInput) =>
      (
        await gqlRequest<
          { apiKeyCreate: { token: string; apiKey: ApiKey } },
          { input: NewKeyInput }
        >(
          `mutation SettingsApiKeyCreate($input: ApiKeyCreateInput!) {
            apiKeyCreate(input: $input) { success token apiKey { ${KEY_FIELDS} } }
          }`,
          { input },
        )
      ).apiKeyCreate,
    onSuccess: () => void qc.invalidateQueries({ queryKey: API_KEYS_KEY }),
  });
}

export function useRevokeApiKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) =>
      gqlRequest(
        `mutation SettingsApiKeyRevoke($id: String!) { apiKeyRevoke(id: $id) { success } }`,
        { id },
      ),
    onSuccess: () => void qc.invalidateQueries({ queryKey: API_KEYS_KEY }),
  });
}

function copyToken(token: string) {
  void navigator.clipboard
    ?.writeText(token)
    .then(() => showToast({ message: "Token copied" }))
    .catch(() => showToast({ message: "Could not copy token", tone: "error" }));
}

/** Anzeige „Datum, Uhrzeit“ in lokaler Zeit; `null` → Strich. */
export function formatWhen(iso: string | null | undefined): string {
  if (!iso) return "–";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "–" : d.toLocaleDateString();
}

/** Ablaufdatum (`YYYY-MM-DD`, lokales Tagesende) als ISO-Zeitpunkt; ungültig → `undefined`. */
export function endOfDayIso(date: string): string | undefined {
  if (!date) return undefined;
  const d = new Date(`${date}T23:59:59`);
  return Number.isNaN(d.getTime()) ? undefined : d.toISOString();
}

function NewKeyDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const session = useSession();
  const create = useCreateApiKey();
  const [label, setLabel] = useState("");
  const [write, setWrite] = useState(false);
  const [teamIds, setTeamIds] = useState<string[]>([]);
  const [expires, setExpires] = useState("");
  const [token, setToken] = useState<string | null>(null);
  useShortcutContext("modal", open);

  const close = () => {
    setToken(null);
    setLabel("");
    setWrite(false);
    setTeamIds([]);
    setExpires("");
    create.reset();
    onClose();
  };
  const trimmed = label.trim();
  const valid = trimmed.length > 0 && trimmed.length <= MAX_LABEL_LENGTH;

  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent>
        {token === null ? (
          <form
            className="flex flex-col gap-3 p-4"
            onSubmit={(e) => {
              e.preventDefault();
              if (!valid) return;
              create.mutate(
                {
                  label: trimmed,
                  scopes: write ? ["READ", "WRITE"] : ["READ"],
                  ...(teamIds.length > 0 ? { teamIds } : {}),
                  ...(endOfDayIso(expires) ? { expiresAt: endOfDayIso(expires) } : {}),
                },
                { onSuccess: (r) => setToken(r.token) },
              );
            }}
          >
            <DialogTitle className="text-base font-semibold">New API key</DialogTitle>
            <DialogDescription className="text-sm text-muted-foreground">
              Keys act as you. Choose the least access the integration needs.
            </DialogDescription>
            <Field label="Label">
              <Input
                aria-label="Label"
                autoFocus
                maxLength={MAX_LABEL_LENGTH}
                placeholder="e.g. CI pipeline"
                value={label}
                onChange={(e) => setLabel(e.target.value)}
              />
            </Field>
            <fieldset className="flex flex-col gap-1 text-sm">
              <legend className="text-xs text-muted-foreground">Access</legend>
              <label className="flex items-center gap-2">
                <input type="checkbox" aria-label="Read" checked disabled /> Read
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  aria-label="Write"
                  checked={write}
                  onChange={(e) => setWrite(e.target.checked)}
                />
                Write (create and change issues, comments, …)
              </label>
            </fieldset>
            <fieldset className="flex flex-col gap-1 text-sm">
              <legend className="text-xs text-muted-foreground">
                Teams (none selected = all your teams)
              </legend>
              {(session?.teams ?? []).map((t) => (
                <label key={t.id} className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    aria-label={`Team ${t.name}`}
                    checked={teamIds.includes(t.id)}
                    onChange={(e) =>
                      setTeamIds((cur) =>
                        e.target.checked ? [...cur, t.id] : cur.filter((x) => x !== t.id),
                      )
                    }
                  />
                  {t.name} <span className="text-xs text-muted-foreground">{t.key}</span>
                </label>
              ))}
            </fieldset>
            <Field label="Expires (optional)">
              <Input
                type="date"
                aria-label="Expires"
                value={expires}
                onChange={(e) => setExpires(e.target.value)}
              />
            </Field>
            {create.isError && (
              <p role="alert" className="text-xs text-destructive">
                {errorText(create.error)}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button type="button" variant="ghost" onClick={close}>
                Cancel
              </Button>
              <Button type="submit" disabled={!valid || create.isPending}>
                Create key
              </Button>
            </div>
          </form>
        ) : (
          <div className="flex flex-col gap-3 p-4">
            <DialogTitle className="text-base font-semibold">Your new API key</DialogTitle>
            <DialogDescription className="sr-only">
              Copy the token now. It is shown only once.
            </DialogDescription>
            <p
              role="alert"
              className="rounded-md border border-border-strong bg-surface-hover px-3 py-2 text-sm"
            >
              Copy this token now and store it safely. It is shown only once and cannot be retrieved
              later.
            </p>
            <div className="flex items-center gap-2">
              <Input
                readOnly
                aria-label="API token"
                className="font-mono text-xs"
                value={token}
                onFocus={(e) => e.currentTarget.select()}
              />
              <Button type="button" size="sm" onClick={() => copyToken(token)}>
                <Copy className="size-3.5" aria-hidden /> Copy
              </Button>
            </div>
            <div className="flex justify-end">
              <Button type="button" onClick={close}>
                Done
              </Button>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

function RevokeDialog({ apiKey, onClose }: { apiKey: ApiKey | null; onClose: () => void }) {
  const revoke = useRevokeApiKey();
  useShortcutContext("modal", apiKey !== null);
  return (
    <Dialog open={apiKey !== null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-sm">
        <div className="flex flex-col gap-3 p-4">
          <DialogTitle className="text-base font-semibold">Revoke API key?</DialogTitle>
          <DialogDescription className="text-sm text-muted-foreground">
            “{apiKey?.label}” stops working immediately. This cannot be undone.
          </DialogDescription>
          {revoke.isError && (
            <p role="alert" className="text-xs text-destructive">
              {errorText(revoke.error)}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button
              type="button"
              disabled={revoke.isPending}
              onClick={() =>
                apiKey &&
                revoke.mutate(apiKey.id, {
                  onSuccess: () => {
                    showToast({ message: "API key revoked" });
                    onClose();
                  },
                })
              }
            >
              Revoke key
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function keyStatus(k: ApiKey): string {
  if (k.revokedAt) return "Revoked";
  if (k.expiresAt && new Date(k.expiresAt).getTime() <= Date.now()) return "Expired";
  return "Active";
}

function KeyRow({ apiKey, onRevoke }: { apiKey: ApiKey; onRevoke: () => void }) {
  const status = keyStatus(apiKey);
  const scopes = apiKey.scopes.join(", ");
  return (
    <li
      className="flex items-center gap-3 px-3 py-2 text-sm"
      aria-label={`API key ${apiKey.label}`}
    >
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="truncate font-medium">{apiKey.label}</span>
          <code className="text-xs text-muted-foreground">{apiKey.keyPrefix}…</code>
          <span className="text-xs text-muted-foreground">{status}</span>
        </div>
        <p className="text-xs text-muted-foreground">
          {scopes} · {apiKey.teams ? apiKey.teams.map((t) => t.key).join(", ") : "All teams"} ·
          created {formatWhen(apiKey.createdAt)} · last used {formatWhen(apiKey.lastUsedAt)}
          {apiKey.expiresAt ? ` · expires ${formatWhen(apiKey.expiresAt)}` : ""}
        </p>
      </div>
      {status === "Active" && (
        <Button variant="ghost" size="sm" aria-label={`Revoke ${apiKey.label}`} onClick={onRevoke}>
          Revoke
        </Button>
      )}
    </li>
  );
}

/** Persönliche API-Einstellungen (`/{ws}/settings/api`). */
export function ApiSettingsPage({ ws }: { ws: string }) {
  const q = useApiKeys();
  const session = useSession();
  const [creating, setCreating] = useState(false);
  const [revoking, setRevoking] = useState<ApiKey | null>(null);
  const nav = workspaceSettingsNav(ws, session, "api");
  return (
    <SettingsShell title="Settings" nav={nav}>
      <Section
        title="API keys"
        description="Personal keys for scripts and MCP clients. They act as you, within the access you choose."
      >
        <div>
          <Button size="sm" onClick={() => setCreating(true)}>
            New key
          </Button>
        </div>
        {q.isLoading && <ListSkeleton rows={3} />}
        {q.isError && (
          <ErrorState
            error={q.error}
            title="Could not load API keys"
            onRetry={() => void q.refetch()}
          />
        )}
        {q.data && q.data.length === 0 && (
          <p className="text-sm text-muted-foreground">You have no API keys yet.</p>
        )}
        {q.data && q.data.length > 0 && (
          <ul className="divide-y divide-border rounded-lg border border-border">
            {q.data.map((k) => (
              <KeyRow key={k.id} apiKey={k} onRevoke={() => setRevoking(k)} />
            ))}
          </ul>
        )}
      </Section>
      <NewKeyDialog open={creating} onClose={() => setCreating(false)} />
      <RevokeDialog apiKey={revoking} onClose={() => setRevoking(null)} />
    </SettingsShell>
  );
}
