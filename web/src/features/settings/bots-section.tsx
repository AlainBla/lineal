/**
 * Bots (LIN-22): Workspace-Admins legen Bots an (User mit Rolle `app`, ohne Login und ohne
 * E-Mail-Benachrichtigungen) und verwalten deren API-Keys. Suspendieren und Teams laufen wie bei
 * allen Mitgliedern über die Mitgliederliste.
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Bot as BotIcon } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import { showToast } from "@/features/issues/toast-store";
import type { ApiKey, Bot } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { BOTS_KEY, KEY_FIELDS, KeyRow, NewKeyDialog, RevokeDialog } from "./api-keys";
import { memberKeys } from "./members-queries";
import { Field, Section } from "./ui";

const BOT_FIELDS = `id name displayName email active createdAt apiKeys { ${KEY_FIELDS} }`;

function useBots() {
  return useQuery({
    queryKey: BOTS_KEY,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ bots: Bot[] }>(
          `query SettingsBots { bots { ${BOT_FIELDS} } }`,
          undefined,
          {
            signal,
          },
        )
      ).bots,
  });
}

function useCreateBot() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: { name: string; email: string }) =>
      (
        await gqlRequest<{ botCreate: { bot: Bot } }, { input: { name: string; email: string } }>(
          `mutation SettingsBotCreate($input: BotCreateInput!) {
            botCreate(input: $input) { success bot { ${BOT_FIELDS} } }
          }`,
          { input },
        )
      ).botCreate.bot,
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: BOTS_KEY });
      void qc.invalidateQueries({ queryKey: memberKeys.adminMembers });
    },
  });
}

function NewBotDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const create = useCreateBot();
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  useShortcutContext("modal", open);
  const close = () => {
    setName("");
    setEmail("");
    create.reset();
    onClose();
  };
  const valid = name.trim().length > 0 && email.trim().includes("@");
  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent>
        <form
          className="flex flex-col gap-3 p-4"
          onSubmit={(e) => {
            e.preventDefault();
            if (!valid) return;
            create.mutate(
              { name: name.trim(), email: email.trim() },
              {
                onSuccess: (bot) => {
                  showToast({ message: `Bot ${bot.displayName} created` });
                  close();
                },
              },
            );
          }}
        >
          <DialogTitle className="text-base font-semibold">New bot</DialogTitle>
          <DialogDescription className="text-sm text-muted-foreground">
            Bots act only through API keys. They cannot sign in and get no email notifications.
          </DialogDescription>
          <Field label="Name">
            <Input
              aria-label="Bot name"
              autoFocus
              maxLength={255}
              placeholder="e.g. Lineal Bot"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </Field>
          <Field label="Email">
            <Input
              aria-label="Bot email"
              type="email"
              placeholder="e.g. lineal-bot@example.com"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
            />
          </Field>
          <p className="text-xs text-muted-foreground">
            The handle for @-mentions is taken from the email (e.g. lineal-bot).
          </p>
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
              Create bot
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function BotRow({
  bot,
  onNewKey,
  onRevoke,
}: {
  bot: Bot;
  onNewKey: () => void;
  onRevoke: (key: ApiKey) => void;
}) {
  return (
    <li className="flex flex-col gap-2 px-3 py-2" aria-label={`Bot ${bot.name}`}>
      <div className="flex items-center gap-3 text-sm">
        <BotIcon className="size-4 text-muted-foreground" aria-hidden />
        <div className="min-w-0 flex-1">
          <span className="truncate font-medium">{bot.name}</span>{" "}
          <span className="text-xs text-muted-foreground">
            @{bot.displayName} · {bot.email}
            {bot.active ? "" : " · Suspended"}
          </span>
        </div>
        <Button
          variant="ghost"
          size="sm"
          disabled={!bot.active}
          aria-label={`New key for ${bot.name}`}
          onClick={onNewKey}
        >
          New key
        </Button>
      </div>
      {bot.apiKeys.length > 0 ? (
        <ul
          aria-label={`API keys of ${bot.name}`}
          className="ml-7 divide-y divide-border rounded-md border border-border"
        >
          {bot.apiKeys.map((k) => (
            <KeyRow key={k.id} apiKey={k} onRevoke={() => onRevoke(k)} />
          ))}
        </ul>
      ) : (
        <p className="ml-7 text-xs text-muted-foreground">No API keys yet.</p>
      )}
    </li>
  );
}

/** Bereich „Bots“ der Mitglieder-Seite (nur für Admins eingehängt). */
export function BotsSection() {
  const q = useBots();
  const [creating, setCreating] = useState(false);
  const [keyFor, setKeyFor] = useState<Bot | null>(null);
  const [revoking, setRevoking] = useState<{ bot: Bot; key: ApiKey } | null>(null);
  return (
    <Section
      title="Bots"
      description="Automation accounts that act through API keys, e.g. for scripts or AI agents. Add them to private teams like any member."
    >
      <div>
        <Button size="sm" onClick={() => setCreating(true)}>
          New bot
        </Button>
      </div>
      {q.isLoading && <ListSkeleton rows={2} />}
      {q.isError && (
        <ErrorState error={q.error} title="Could not load bots" onRetry={() => void q.refetch()} />
      )}
      {q.data && q.data.length === 0 && (
        <p className="text-sm text-muted-foreground">No bots yet.</p>
      )}
      {q.data && q.data.length > 0 && (
        <ul aria-label="Bots" className="divide-y divide-border rounded-lg border border-border">
          {q.data.map((b) => (
            <BotRow
              key={b.id}
              bot={b}
              onNewKey={() => setKeyFor(b)}
              onRevoke={(key) => setRevoking({ bot: b, key })}
            />
          ))}
        </ul>
      )}
      <NewBotDialog open={creating} onClose={() => setCreating(false)} />
      <NewKeyDialog
        open={keyFor !== null}
        onClose={() => setKeyFor(null)}
        bot={keyFor ? { id: keyFor.id, name: keyFor.name } : undefined}
      />
      <RevokeDialog
        apiKey={revoking?.key ?? null}
        onClose={() => setRevoking(null)}
        bot={revoking ? { id: revoking.bot.id, name: revoking.bot.name } : undefined}
      />
    </Section>
  );
}
