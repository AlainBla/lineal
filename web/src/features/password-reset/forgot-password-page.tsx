import { Link } from "@tanstack/react-router";
import { useId, useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

import { requestPasswordReset, validateForgotEmail, type RequestResult } from "./password-reset";

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

/** Öffentliche Seite „Passwort vergessen" (`/forgot-password`, ohne Login erreichbar). */
export function ForgotPasswordPage({
  submit = requestPasswordReset,
}: {
  submit?: typeof requestPasswordReset;
}) {
  const id = useId();
  const [email, setEmail] = useState("");
  const [workspace, setWorkspace] = useState("");
  const [error, setError] = useState<string | undefined>();
  const [formError, setFormError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [sent, setSent] = useState(false);

  async function onSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (pending) return;
    const found = validateForgotEmail(email);
    setError(found);
    setFormError(null);
    if (found) return;
    setPending(true);
    const result: RequestResult = await submit(email, workspace || undefined);
    setPending(false);
    if (result.kind === "sent") {
      setSent(true);
    } else {
      setFormError(
        result.reason === "network"
          ? "Cannot reach the server. Check your connection and try again."
          : "Something went wrong on our side. Please try again.",
      );
    }
  }

  // Dieselbe, neutrale Bestätigung — unabhängig davon, ob zur Adresse ein Konto gehört.
  if (sent) {
    return (
      <Card>
        <h1 className="mb-2 text-lg font-semibold tracking-tight">Check your email</h1>
        <p className="text-sm text-muted-foreground">
          If an account exists for that email address, we&apos;ve sent a link to reset your
          password. The link is valid for 30 minutes.
        </p>
        <p className="mt-4 text-sm text-muted-foreground">
          <Link to="/login" className="underline">
            Back to login
          </Link>
        </p>
      </Card>
    );
  }

  return (
    <Card>
      <h1 className="text-lg font-semibold tracking-tight">Reset your password</h1>
      <p className="mt-1 mb-4 text-sm text-muted-foreground">
        Enter your email and we&apos;ll send you a link to choose a new password.
      </p>
      <form
        onSubmit={onSubmit}
        noValidate
        aria-label="Forgot password"
        className="flex flex-col gap-4"
      >
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${id}-email`}>Email</Label>
          <Input
            id={`${id}-email`}
            type="email"
            name="email"
            autoComplete="username"
            autoFocus
            placeholder="alice@acme.example"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? `${id}-email-error` : undefined}
          />
          {error && (
            <p id={`${id}-email-error`} className="text-xs text-destructive">
              {error}
            </p>
          )}
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${id}-workspace`}>Workspace (optional)</Label>
          <Input
            id={`${id}-workspace`}
            name="workspace"
            autoComplete="off"
            placeholder="acme"
            value={workspace}
            onChange={(e) => setWorkspace(e.target.value)}
            aria-describedby={`${id}-workspace-hint`}
          />
          <p id={`${id}-workspace-hint`} className="text-xs text-muted-foreground">
            Only needed if your email is used in more than one workspace.
          </p>
        </div>
        {formError && (
          <p
            role="alert"
            className="rounded-md border border-destructive/40 bg-destructive/10 px-2.5 py-2 text-xs text-destructive"
          >
            {formError}
          </p>
        )}
        <Button type="submit" size="lg" disabled={pending} aria-busy={pending}>
          {pending ? "Sending…" : "Send reset link"}
        </Button>
        <p className="text-center text-xs text-muted-foreground">
          <Link to="/login" className="underline">
            Back to login
          </Link>
        </p>
      </form>
    </Card>
  );
}
