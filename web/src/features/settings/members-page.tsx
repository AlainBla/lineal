import { Plus } from "lucide-react";
import { useId, useState, type FormEvent } from "react";

import { useSession, type Session } from "@/app/auth";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { errorText } from "@/features/issues/error-text";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import type { Invitation } from "@/gql/graphql";
import { GraphQLRequestError } from "@/lib/graphql";

import { useInvitationMutations, useInvitations, useMemberList } from "./members-queries";
import { MAX_EMAILS, parseEmails } from "./invite-emails";
import { workspaceSettingsNav } from "./nav";
import { Field, Section, selectClass, SettingsShell } from "./ui";

const isAdmin = (role: string | undefined) => role === "owner" || role === "admin";
/** Gäste dürfen nie einladen; ob die Rolle sonst genügt, entscheidet der Server (`invitationsRole`). */
const mayTryToInvite = (role: string | undefined) => role !== "guest" && role !== "app";

function isForbidden(err: unknown): boolean {
  return err instanceof GraphQLRequestError && err.code === "forbidden";
}

function InviteDialog({
  open,
  onOpenChange,
  session,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  session: Session;
}) {
  const id = useId();
  const { invite } = useInvitationMutations();
  const [text, setText] = useState("");
  const [role, setRole] = useState("user");
  const [teamIds, setTeamIds] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setText("");
    setRole("user");
    setTeamIds([]);
    setError(null);
    invite.reset();
  };

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    const { emails, invalid } = parseEmails(text);
    if (invalid.length > 0) return setError(`Not a valid email address: ${invalid[0]}`);
    if (emails.length === 0) return setError("Enter at least one email address.");
    if (emails.length > MAX_EMAILS)
      return setError(`You can invite at most ${MAX_EMAILS} people at once.`);
    if (role === "guest" && teamIds.length === 0)
      return setError("Choose at least one team for a guest.");
    setError(null);
    invite.mutate(
      { emails, role, teamIds },
      {
        onSuccess: () => {
          reset();
          onOpenChange(false);
        },
      },
    );
  }

  const toggle = (teamId: string) =>
    setTeamIds((cur) =>
      cur.includes(teamId) ? cur.filter((t) => t !== teamId) : [...cur, teamId],
    );
  const shownError = error ?? (invite.isError ? errorText(invite.error) : null);

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) reset();
        onOpenChange(o);
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Invite people</DialogTitle>
          <DialogDescription>
            They receive an email with a link that is valid for 7 days.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={onSubmit} className="flex flex-col gap-3 p-4" aria-label="Invite people">
          <Field label="Email addresses">
            <textarea
              aria-label="Email addresses"
              className="min-h-20 rounded-md border border-input bg-transparent px-2.5 py-1.5 text-sm"
              placeholder="dora@acme.example, erik@acme.example"
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
          </Field>
          <Field label="Role">
            <select
              aria-label="Role"
              className={selectClass}
              value={role}
              onChange={(e) => setRole(e.target.value)}
            >
              <option value="user">Member</option>
              <option value="guest">Guest</option>
              {isAdmin(session.user.role) && <option value="admin">Admin</option>}
            </select>
          </Field>
          <fieldset className="flex flex-col gap-1">
            <legend className="mb-1 text-xs text-muted-foreground">
              Teams{role === "guest" ? " (at least one)" : ""}
            </legend>
            {session.teams.map((t) => (
              <label key={t.id} className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  id={`${id}-${t.id}`}
                  checked={teamIds.includes(t.id)}
                  onChange={() => toggle(t.id)}
                />
                {t.name} <span className="text-xs text-muted-foreground">{t.key}</span>
              </label>
            ))}
          </fieldset>
          {shownError && (
            <p role="alert" className="text-xs text-destructive">
              {shownError}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" size="sm" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={invite.isPending}>
              Send invitations
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

const ROLE_LABEL: Record<string, string> = {
  owner: "Owner",
  admin: "Admin",
  user: "Member",
  guest: "Guest",
};

function statusOf(inv: Invitation, now: number): string {
  if (inv.status === "pending" && Date.parse(inv.expiresAt) <= now) return "expired";
  return inv.status;
}

function formatDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString();
}

function InvitationRow({ inv, session }: { inv: Invitation; session: Session }) {
  const { resend, revoke } = useInvitationMutations();
  const [now] = useState(() => Date.now());
  const status = statusOf(inv, now);
  const teams = inv.teamIds
    .map((t) => session.teams.find((x) => x.id === t)?.name)
    .filter((n): n is string => !!n);
  const err = resend.error ?? revoke.error;
  return (
    <li className="flex flex-col gap-1 px-3 py-2 text-sm">
      <div className="flex items-center gap-3">
        <span className="min-w-0 flex-1 truncate font-medium">{inv.email}</span>
        <span className="text-xs text-muted-foreground">{ROLE_LABEL[inv.role] ?? inv.role}</span>
        <span
          className="rounded bg-surface-hover px-1.5 py-0.5 text-xs capitalize"
          aria-label={`Status of ${inv.email}`}
        >
          {status}
        </span>
        <Button
          variant="ghost"
          size="sm"
          aria-label={`Resend invitation to ${inv.email}`}
          disabled={resend.isPending || (status !== "pending" && status !== "expired")}
          onClick={() => resend.mutate(inv.id)}
        >
          Resend
        </Button>
        <Button
          variant="ghost"
          size="sm"
          aria-label={`Revoke invitation to ${inv.email}`}
          disabled={revoke.isPending || status !== "pending"}
          onClick={() => revoke.mutate(inv.id)}
        >
          Revoke
        </Button>
      </div>
      <div className="text-xs text-muted-foreground">
        {teams.length > 0 ? `Teams: ${teams.join(", ")} · ` : ""}
        {status === "expired" ? "Expired" : "Expires"} {formatDate(inv.expiresAt)}
      </div>
      {err && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(err)}
        </p>
      )}
    </li>
  );
}

function InvitationsSection({ session }: { session: Session }) {
  const [open, setOpen] = useState(false);
  const allowed = mayTryToInvite(session.user.role);
  const q = useInvitations(allowed);
  const forbidden = !allowed || isForbidden(q.error);
  return (
    <Section
      title="Invitations"
      description="People who have been invited but have not joined yet."
    >
      {forbidden ? (
        <p className="text-sm text-muted-foreground">
          You do not have permission to invite people to this workspace.
        </p>
      ) : (
        <>
          {q.isLoading && <ListSkeleton rows={2} />}
          {q.isError && (
            <ErrorState
              error={q.error}
              title="Could not load invitations"
              onRetry={() => void q.refetch()}
            />
          )}
          {q.data && q.data.length === 0 && (
            <p className="text-sm text-muted-foreground">No pending invitations.</p>
          )}
          {q.data && q.data.length > 0 && (
            <ul
              aria-label="Pending invitations"
              className="divide-y divide-border rounded-lg border border-border"
            >
              {q.data.map((inv) => (
                <InvitationRow key={inv.id} inv={inv} session={session} />
              ))}
            </ul>
          )}
          <Button size="sm" className="self-start" onClick={() => setOpen(true)}>
            <Plus /> Invite people
          </Button>
          <InviteDialog open={open} onOpenChange={setOpen} session={session} />
        </>
      )}
    </Section>
  );
}

/** Workspace-Mitglieder und Einladungen (`/{ws}/settings/members`, docs/08 F1/F16). */
export function MembersSettingsPage({ ws }: { ws: string }) {
  const session = useSession();
  const members = useMemberList();
  return (
    <SettingsShell title="Settings" nav={workspaceSettingsNav(ws, session, "members")}>
      <Section title="Members" description="People in this workspace you can see.">
        {members.isLoading && <ListSkeleton rows={3} />}
        {members.isError && (
          <ErrorState
            error={members.error}
            title="Could not load members"
            onRetry={() => void members.refetch()}
          />
        )}
        {members.data && (
          <ul
            aria-label="Members"
            className="divide-y divide-border rounded-lg border border-border"
          >
            {members.data.map((m) => (
              <li key={m.id} className="flex items-center gap-3 px-3 py-1.5 text-sm">
                <span className="min-w-0 flex-1 truncate">{m.displayName || m.name}</span>
                {m.id === session?.user.id && session.user.role && (
                  <span className="text-xs text-muted-foreground">
                    You · {ROLE_LABEL[session.user.role] ?? session.user.role}
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
      </Section>
      {session && <InvitationsSection session={session} />}
    </SettingsShell>
  );
}
