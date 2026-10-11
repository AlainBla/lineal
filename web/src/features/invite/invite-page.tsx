import { Link } from "@tanstack/react-router";
import { useEffect, useId, useState, type FormEvent } from "react";

import type { Session } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

import {
  acceptInvite,
  acceptInviteAsAccount,
  MIN_PASSWORD_LEN,
  type AcceptResult,
} from "./accept-invite";

type Failure = Extract<AcceptResult, { ok: false }>;

const MESSAGES: Record<Failure["reason"], string> = {
  invalid_link:
    "This invitation link is no longer valid. It may have expired, been revoked, already been used, or was sent to a different email address. Ask the person who invited you for a new one.",
  bad_input: "Please check your details and try again.",
  account_exists: "An account with this email already exists. Log in instead.",
  rate_limited: "Too many attempts. Please wait a moment and try again.",
  rejected: "The request was rejected. Reload the page and try again.",
  server_error: "Something went wrong on our side. Please try again.",
  network: "Cannot reach the server. Check your connection and try again.",
};

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

interface Errors {
  email?: string;
  name?: string;
  password?: string;
  confirm?: string;
}

export function validateAccept(v: {
  email: string;
  name: string;
  password: string;
  confirm: string;
}): Errors {
  const e: Errors = {};
  if (!EMAIL_RE.test(v.email.trim())) e.email = "Enter the email address you were invited with.";
  const name = v.name.trim();
  if (!name) e.name = "Enter your name.";
  else if (name.length > 255) e.name = "Use at most 255 characters.";
  if (v.password.length < MIN_PASSWORD_LEN)
    e.password = `Use at least ${MIN_PASSWORD_LEN} characters.`;
  if (v.confirm !== v.password) e.confirm = "The passwords do not match.";
  return e;
}

function Card({ children }: { children: React.ReactNode }) {
  return (
    <main className="flex min-h-dvh items-center justify-center bg-background p-4">
      <div className="w-full max-w-sm rounded-xl border border-border-strong bg-card p-6 shadow-popover">
        <div
          aria-hidden
          className="mb-4 flex size-8 items-center justify-center rounded-lg bg-primary text-sm font-bold text-primary-foreground"
        >
          L
        </div>
        {children}
      </div>
    </main>
  );
}

function FieldError({ id, text }: { id: string; text?: string }) {
  return text ? (
    <p id={id} className="text-xs text-destructive">
      {text}
    </p>
  ) : null;
}

/** Öffentliche Annahme-Seite `/invite/<token>` (ohne Login erreichbar). */
export function InvitePage({
  token,
  session,
  onAccepted,
  submit = acceptInvite,
  submitAccount = acceptInviteAsAccount,
}: {
  token: string;
  /** Bestehende Session (anderer oder gleicher Workspace), falls vorhanden. */
  session: Session | null | undefined;
  onAccepted: (workspace: string | null) => void;
  submit?: typeof acceptInvite;
  submitAccount?: typeof acceptInviteAsAccount;
}) {
  const id = useId();
  const [email, setEmail] = useState("");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [errors, setErrors] = useState<Errors>({});
  const [failure, setFailure] = useState<Failure | null>(null);
  const [pending, setPending] = useState(false);
  const [newAccount, setNewAccount] = useState(false);

  useEffect(() => {
    // Der Token steht in der URL: nicht per Referer an Dritte weitergeben.
    const meta = document.createElement("meta");
    meta.name = "referrer";
    meta.content = "no-referrer";
    document.head.appendChild(meta);
    const prev = document.title;
    document.title = "Accept invitation · Lineal";
    return () => {
      meta.remove();
      document.title = prev;
    };
  }, []);

  async function run(p: Promise<AcceptResult>) {
    setPending(true);
    setFailure(null);
    const result = await p;
    setPending(false);
    if (result.ok) onAccepted(result.workspace);
    else setFailure(result);
  }

  function onSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (pending) return;
    const found = validateAccept({ email, name, password, confirm });
    setErrors(found);
    if (Object.keys(found).length > 0) return;
    void run(submit({ token, email, name, password }));
  }

  const failureText = failure
    ? failure.reason === "bad_input" && failure.field === "password"
      ? `Use at least ${MIN_PASSWORD_LEN} characters for your password.`
      : failure.reason === "bad_input" && failure.field === "name"
        ? "Enter a name of up to 255 characters."
        : MESSAGES[failure.reason]
    : null;
  const alert = failureText && (
    <p
      role="alert"
      className="rounded-md border border-destructive/40 bg-destructive/10 px-2.5 py-2 text-xs text-destructive"
    >
      {failureText}
      {failure?.reason === "account_exists" && (
        <>
          {" "}
          <Link to="/login" className="underline">
            Go to login
          </Link>
        </>
      )}
    </p>
  );

  if (failure?.reason === "invalid_link") {
    return (
      <Card>
        <h1 className="mb-2 text-lg font-semibold tracking-tight">Invitation not available</h1>
        {alert}
        <p className="mt-4 text-sm text-muted-foreground">
          Already have an account?{" "}
          <Link to="/login" className="underline">
            Log in
          </Link>
        </p>
      </Card>
    );
  }

  if (session && !newAccount) {
    return (
      <Card>
        <h1 className="text-lg font-semibold tracking-tight">Accept invitation</h1>
        <p className="mt-1 mb-4 text-sm text-muted-foreground">
          You are signed in as <strong>{session.user.email}</strong>. If this invitation was sent to
          that address, you can accept it with your existing account.
        </p>
        <div className="flex flex-col gap-3">
          {alert}
          <Button size="lg" disabled={pending} onClick={() => void run(submitAccount(token))}>
            {pending ? "Joining…" : "Accept with this account"}
          </Button>
          <Button variant="ghost" onClick={() => setNewAccount(true)}>
            Create a new account instead
          </Button>
        </div>
      </Card>
    );
  }

  const field = (key: keyof Errors) => ({
    "aria-invalid": errors[key] ? (true as const) : undefined,
    "aria-describedby": errors[key] ? `${id}-${key}-error` : undefined,
  });

  return (
    <Card>
      <h1 className="text-lg font-semibold tracking-tight">Join your team on Lineal</h1>
      <p className="mt-1 mb-4 text-sm text-muted-foreground">
        Create your account to accept the invitation.
      </p>
      <form
        onSubmit={onSubmit}
        noValidate
        aria-label="Accept invitation"
        className="flex flex-col gap-4"
      >
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${id}-email`}>Email</Label>
          <Input
            id={`${id}-email`}
            type="email"
            autoComplete="username"
            autoFocus
            placeholder="The address this invitation was sent to"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            {...field("email")}
          />
          <FieldError id={`${id}-email-error`} text={errors.email} />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${id}-name`}>Name</Label>
          <Input
            id={`${id}-name`}
            autoComplete="name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            {...field("name")}
          />
          <FieldError id={`${id}-name-error`} text={errors.name} />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${id}-password`}>Password</Label>
          <Input
            id={`${id}-password`}
            type="password"
            autoComplete="new-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            {...field("password")}
          />
          <FieldError id={`${id}-password-error`} text={errors.password} />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${id}-confirm`}>Repeat password</Label>
          <Input
            id={`${id}-confirm`}
            type="password"
            autoComplete="new-password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            {...field("confirm")}
          />
          <FieldError id={`${id}-confirm-error`} text={errors.confirm} />
        </div>
        {alert}
        <Button type="submit" size="lg" disabled={pending} aria-busy={pending}>
          {pending ? "Creating account…" : "Create account and join"}
        </Button>
      </form>
    </Card>
  );
}
