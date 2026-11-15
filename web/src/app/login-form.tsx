import { useId, useState, type FormEvent } from "react";

import { login, validateLogin, type LoginErrors, type LoginResult } from "@/app/auth";
import { TotpEnroll } from "@/app/totp-enroll";
import { loginTotp } from "@/app/totp-api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const SERVER_MESSAGES: Record<Extract<LoginResult, { kind: "error" }>["reason"], string> = {
  invalid_credentials: "Incorrect email or password.",
  rate_limited: "Too many attempts. Please wait a moment and try again.",
  server_error: "Something went wrong on our side. Please try again.",
  network: "Cannot reach the server. Check your connection and try again.",
};

/** Zweiter Login-Schritt: TOTP-Code oder Recovery-Code. */
function TotpStep({ pendingToken, onSuccess }: { pendingToken: string; onSuccess: () => void }) {
  const [useRecovery, setUseRecovery] = useState(false);
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    if (pending || value.trim() === "") return;
    setPending(true);
    setError(null);
    const factor = useRecovery
      ? { recoveryCode: value.trim() }
      : { code: value.replace(/\D/g, "") };
    const result = await loginTotp(pendingToken, factor);
    setPending(false);
    if (result.ok) {
      onSuccess();
      return;
    }
    setError(
      result.reason === "rate_limited"
        ? "Too many attempts. Please wait a moment."
        : result.reason === "network"
          ? "Cannot reach the server."
          : useRecovery
            ? "Invalid recovery code."
            : "Invalid code.",
    );
  }

  return (
    <form onSubmit={onSubmit} className="flex flex-col gap-4" aria-label="Two-factor">
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="totp-code">
          {useRecovery ? "Recovery code" : "Code from your authenticator app"}
        </Label>
        <Input
          id="totp-code"
          autoFocus
          inputMode={useRecovery ? "text" : "numeric"}
          autoComplete="one-time-code"
          maxLength={useRecovery ? 9 : 6}
          placeholder={useRecovery ? "XXXX-XXXX" : "123456"}
          value={value}
          onChange={(e) =>
            setValue(useRecovery ? e.target.value : e.target.value.replace(/\D/g, ""))
          }
          aria-invalid={error ? true : undefined}
        />
        {error && (
          <p role="alert" className="text-xs text-destructive">
            {error}
          </p>
        )}
      </div>
      <Button type="submit" size="lg" disabled={pending} aria-busy={pending}>
        {pending ? "Verifying…" : "Log in"}
      </Button>
      <button
        type="button"
        className="text-xs text-primary hover:underline"
        onClick={() => {
          setUseRecovery((v) => !v);
          setValue("");
          setError(null);
        }}
      >
        {useRecovery ? "Use your authenticator app instead" : "Use a recovery code instead"}
      </button>
    </form>
  );
}

export function LoginForm({
  onSuccess,
  submit = login,
}: {
  onSuccess: () => void;
  /** Für Tests austauschbar. */
  submit?: typeof login;
}) {
  const id = useId();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [errors, setErrors] = useState<LoginErrors>({});
  const [formError, setFormError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [step, setStep] = useState<
    | { name: "password" }
    | { name: "totp"; pendingToken: string }
    | { name: "enroll"; pendingToken: string }
  >({ name: "password" });

  async function onSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (pending) return;
    const found = validateLogin({ email, password });
    setErrors(found);
    setFormError(null);
    if (found.email || found.password) {
      document.getElementById(found.email ? `${id}-email` : `${id}-password`)?.focus();
      return;
    }
    setPending(true);
    const result = await submit(email.trim(), password);
    setPending(false);
    if (result.kind === "ok") onSuccess();
    else if (result.kind === "totp") setStep({ name: "totp", pendingToken: result.pendingToken });
    else if (result.kind === "enroll")
      setStep({ name: "enroll", pendingToken: result.pendingToken });
    else setFormError(SERVER_MESSAGES[result.reason]);
  }

  if (step.name === "totp")
    return <TotpStep pendingToken={step.pendingToken} onSuccess={onSuccess} />;

  if (step.name === "enroll")
    return (
      <div className="flex flex-col gap-4">
        <div>
          <h2 className="text-sm font-semibold">Two-factor is required</h2>
          <p className="mt-1 text-xs text-muted-foreground">
            As an owner you must set up two-factor authentication to continue.
          </p>
        </div>
        <TotpEnroll pendingToken={step.pendingToken} onComplete={onSuccess} />
      </div>
    );

  return (
    <form onSubmit={onSubmit} noValidate className="flex flex-col gap-4" aria-label="Log in">
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
          aria-invalid={errors.email ? true : undefined}
          aria-describedby={errors.email ? `${id}-email-error` : undefined}
        />
        {errors.email && (
          <p id={`${id}-email-error`} className="text-xs text-destructive">
            {errors.email}
          </p>
        )}
      </div>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${id}-password`}>Password</Label>
        <Input
          id={`${id}-password`}
          type="password"
          name="password"
          autoComplete="current-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          aria-invalid={errors.password ? true : undefined}
          aria-describedby={errors.password ? `${id}-password-error` : undefined}
        />
        {errors.password && (
          <p id={`${id}-password-error`} className="text-xs text-destructive">
            {errors.password}
          </p>
        )}
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
        {pending ? "Signing in…" : "Log in"}
      </Button>
    </form>
  );
}
