import { useId, useState, type FormEvent } from "react";

import { login, validateLogin, type LoginErrors, type LoginResult } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const SERVER_MESSAGES: Record<Exclude<LoginResult, { ok: true }>["reason"], string> = {
  invalid_credentials: "Incorrect email or password.",
  rate_limited: "Too many attempts. Please wait a moment and try again.",
  server_error: "Something went wrong on our side. Please try again.",
  network: "Cannot reach the server. Check your connection and try again.",
};

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

  async function onSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (pending) return;
    const found = validateLogin({ email, password });
    setErrors(found);
    setFormError(null);
    if (found.email || found.password) {
      // Fokus auf das erste fehlerhafte Feld (Barrierefreiheit).
      document.getElementById(found.email ? `${id}-email` : `${id}-password`)?.focus();
      return;
    }
    setPending(true);
    const result = await submit(email.trim(), password);
    setPending(false);
    if (result.ok) onSuccess();
    else setFormError(SERVER_MESSAGES[result.reason]);
  }

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
