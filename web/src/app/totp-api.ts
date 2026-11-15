/**
 * LIN-70: REST-Client für die TOTP-Endpunkte (`/auth/login/totp`, `/auth/totp/*`).
 * Verwaltung läuft über die Browser-Session (Cookie + CSRF-Header); Enrollment
 * akzeptiert zusätzlich einen `enroll`-Pending-Token aus dem erzwungenen Login.
 */

const JSON_HEADERS = {
  "content-type": "application/json",
  accept: "application/json",
  "x-lineal-csrf": "1",
};

export type TotpStatus = {
  enabled: boolean;
  confirmed: boolean;
  recoveryRemaining: number;
  ownerEnforced: boolean;
  graceDaysLeft: number | null;
  graceLoginsLeft: number | null;
};

export type EnrollStart = { secretBase32: string; otpauthUri: string };

export type SecondFactor = { code: string } | { recoveryCode: string };

async function expectOk(res: Response, action: string): Promise<void> {
  if (!res.ok) {
    let message = `${action} failed`;
    try {
      const body = (await res.json()) as { message?: string };
      if (body?.message) message = body.message;
    } catch {
      /* kein JSON-Body */
    }
    throw new Error(message);
  }
}

/** Zweiter Login-Schritt. Gibt `ok` bzw. einen Fehlergrund zurück. */
export async function loginTotp(
  pendingToken: string,
  factor: SecondFactor,
  fetchImpl: typeof fetch = fetch,
): Promise<
  { ok: true } | { ok: false; reason: "invalid" | "rate_limited" | "server" | "network" }
> {
  let res: Response;
  try {
    res = await fetchImpl("/auth/login/totp", {
      method: "POST",
      credentials: "same-origin",
      headers: JSON_HEADERS,
      body: JSON.stringify({ pendingToken, ...factor }),
    });
  } catch {
    return { ok: false, reason: "network" };
  }
  if (res.ok) return { ok: true };
  if (res.status === 429) return { ok: false, reason: "rate_limited" };
  if (res.status === 401 || res.status === 400) return { ok: false, reason: "invalid" };
  return { ok: false, reason: "server" };
}

export async function totpStatus(fetchImpl: typeof fetch = fetch): Promise<TotpStatus> {
  const res = await fetchImpl("/auth/totp/status", { credentials: "same-origin" });
  await expectOk(res, "Loading 2FA status");
  return (await res.json()) as TotpStatus;
}

export async function totpEnrollBegin(
  pendingToken?: string,
  fetchImpl: typeof fetch = fetch,
): Promise<EnrollStart> {
  const res = await fetchImpl("/auth/totp/enroll/begin", {
    method: "POST",
    credentials: "same-origin",
    headers: JSON_HEADERS,
    body: JSON.stringify(pendingToken ? { pendingToken } : {}),
  });
  await expectOk(res, "Starting 2FA setup");
  return (await res.json()) as EnrollStart;
}

export async function totpEnrollConfirm(
  code: string,
  pendingToken?: string,
  fetchImpl: typeof fetch = fetch,
): Promise<string[]> {
  const res = await fetchImpl("/auth/totp/enroll/confirm", {
    method: "POST",
    credentials: "same-origin",
    headers: JSON_HEADERS,
    body: JSON.stringify(pendingToken ? { code, pendingToken } : { code }),
  });
  await expectOk(res, "Confirming 2FA");
  const body = (await res.json()) as { recoveryCodes: string[] };
  return body.recoveryCodes;
}

export async function totpDisable(fetchImpl: typeof fetch = fetch): Promise<void> {
  const res = await fetchImpl("/auth/totp/disable", {
    method: "POST",
    credentials: "same-origin",
    headers: JSON_HEADERS,
  });
  await expectOk(res, "Disabling 2FA");
}

export async function totpRegenerate(fetchImpl: typeof fetch = fetch): Promise<string[]> {
  const res = await fetchImpl("/auth/totp/recovery/regenerate", {
    method: "POST",
    credentials: "same-origin",
    headers: JSON_HEADERS,
  });
  await expectOk(res, "Regenerating recovery codes");
  const body = (await res.json()) as { recoveryCodes: string[] };
  return body.recoveryCodes;
}

export async function totpReset(userId: string, fetchImpl: typeof fetch = fetch): Promise<void> {
  const res = await fetchImpl("/auth/totp/reset", {
    method: "POST",
    credentials: "same-origin",
    headers: JSON_HEADERS,
    body: JSON.stringify({ userId }),
  });
  await expectOk(res, "Resetting 2FA");
}
