import { Link } from "@tanstack/react-router";
import { useId, useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

import {
  MIN_PASSWORD_LEN,
  resetPassword,
  validateNewPassword,
  type ResetErrors,
  type ResetResult,
} from "./password-reset";

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

type Failure = Extract<ResetResult, { kind: "error" }>["reason"];

const MESSAGES: Record<Failure, string> = {
  invalid_link:
    "This reset link is no longer valid. It may have expired or already been used. Request a new one.",
  weak_password: `Use at least ${MIN_PASSWORD_LEN} characters for your password.`,
  rate_limited: "Too many attempts. Please wait a moment and try again.",
  server_error: "Something went wrong on our side. Please try again.",
  network: "Cannot reach the server. Check your connection and try again.",
};

/** Öffentliche Seite „Neues Passwort setzen" (`/reset-password?token=…`, ohne Login erreichbar). */
export function ResetPasswordPage({
  token,
  onDone,
  submit = resetPassword,
}: {
  token: string;
  /** Nach erfolgreichem Reset (z. B. Navigation zur Login-Seite). */
  onDone: () => void;
  submit?: typeof resetPassword;
}) {
  const id = useId();
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [errors, setErrors] = useState<ResetErrors>({});
  const [failure, setFailure] = useState<Failure | null>(null);
  const [pending, setPending] = useState(false);
  const [done, setDone] = useState(false);

  // Fehlender Token (Link ohne `?token=`): behandeln wie ein ungültiger Link.
  const missingToken = token.trim() === "";

  async function onSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (pending) return;
    const found = validateNewPassword(password, confirm);
    setErrors(found);
    setFailure(null);
    if (found.password || found.confirm) return;
    setPending(true);
    const result = await submit(token, password);
    setPending(false);
    if (result.kind === "ok") {
      setDone(true);
      onDone();
    } else {
      setFailure(result.reason);
    }
  }

  if (done) {
    return (
      <Card>
        <h1 className="mb-2 text-lg font-semibold tracking-tight">Password updated</h1>
        <p className="text-sm text-muted-foreground">
          Your password has been changed and all other sessions were signed out. Log in with your
          new password.
        </p>
        <p className="mt-4 text-sm">
          <Link to="/login" className="underline">
            Go to login
          </Link>
        </p>
      </Card>
    );
  }

  if (missingToken || failure === "invalid_link") {
    return (
      <Card>
        <h1 className="mb-2 text-lg font-semibold tracking-tight">Reset link not valid</h1>
        <p role="alert" className="text-sm text-muted-foreground">
          {MESSAGES.invalid_link}
        </p>
        <p className="mt-4 text-sm">
          <Link to="/forgot-password" className="underline">
            Request a new link
          </Link>
        </p>
      </Card>
    );
  }

  const field = (key: keyof ResetErrors) => ({
    "aria-invalid": errors[key] ? (true as const) : undefined,
    "aria-describedby": errors[key] ? `${id}-${key}-error` : undefined,
  });

  return (
    <Card>
      <h1 className="text-lg font-semibold tracking-tight">Choose a new password</h1>
      <p className="mt-1 mb-4 text-sm text-muted-foreground">
        Use at least {MIN_PASSWORD_LEN} characters.
      </p>
      <form
        onSubmit={onSubmit}
        noValidate
        aria-label="Reset password"
        className="flex flex-col gap-4"
      >
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${id}-password`}>New password</Label>
          <Input
            id={`${id}-password`}
            type="password"
            autoComplete="new-password"
            autoFocus
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            {...field("password")}
          />
          {errors.password && (
            <p id={`${id}-password-error`} className="text-xs text-destructive">
              {errors.password}
            </p>
          )}
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
          {errors.confirm && (
            <p id={`${id}-confirm-error`} className="text-xs text-destructive">
              {errors.confirm}
            </p>
          )}
        </div>
        {failure && (
          <p
            role="alert"
            className="rounded-md border border-destructive/40 bg-destructive/10 px-2.5 py-2 text-xs text-destructive"
          >
            {MESSAGES[failure]}
          </p>
        )}
        <Button type="submit" size="lg" disabled={pending} aria-busy={pending}>
          {pending ? "Updating…" : "Set new password"}
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
