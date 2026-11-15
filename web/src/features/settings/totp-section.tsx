/**
 * LIN-70: TOTP-Abschnitt im Account-Panel — Status, Einrichtung (QR/Recovery),
 * Recovery neu erzeugen, Abschalten. Für erzwungene Owner ist „Disable" gesperrt.
 */
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { TotpEnroll } from "@/app/totp-enroll";
import { totpDisable, totpRegenerate, totpStatus } from "@/app/totp-api";
import { Button } from "@/components/ui/button";
import { showToast } from "@/features/issues/toast-store";

import { Section } from "./ui";

function copy(text: string) {
  void navigator.clipboard?.writeText(text);
}

function RecoveryList({ codes, onDone }: { codes: string[]; onDone: () => void }) {
  return (
    <div className="flex flex-col gap-3">
      <p className="text-xs text-muted-foreground">
        Save these recovery codes — they replace your authenticator if you lose it. Shown once.
      </p>
      <ul className="grid grid-cols-2 gap-x-6 gap-y-1.5 rounded-lg border border-border bg-muted/40 p-3 font-mono text-sm">
        {codes.map((c) => (
          <li key={c}>{c}</li>
        ))}
      </ul>
      <div className="flex gap-2">
        <Button type="button" variant="outline" size="sm" onClick={() => copy(codes.join("\n"))}>
          Copy
        </Button>
        <Button type="button" size="sm" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  );
}

export function TotpSection() {
  const status = useQuery({ queryKey: ["totp-status"], queryFn: () => totpStatus() });
  const [enrolling, setEnrolling] = useState(false);
  const [newCodes, setNewCodes] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  const s = status.data;

  async function run(action: () => Promise<void>) {
    if (busy) return;
    setBusy(true);
    try {
      await action();
    } catch (e) {
      showToast({
        message: e instanceof Error ? e.message : "Something went wrong",
        tone: "error",
      });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Section
      title="Two-factor authentication"
      description="Protect your account with a time-based code from an authenticator app."
    >
      {status.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}

      {newCodes && (
        <RecoveryList
          codes={newCodes}
          onDone={() => {
            setNewCodes(null);
            void status.refetch();
          }}
        />
      )}

      {!newCodes && s && !s.confirmed && !enrolling && (
        <div className="flex flex-col gap-3">
          {s.ownerEnforced && (
            <p className="rounded-md border border-warning/40 bg-warning/10 px-2.5 py-2 text-xs text-warning">
              Two-factor is required for owners.
              {s.graceDaysLeft != null &&
                ` Grace period: ${s.graceDaysLeft} day(s) / ${s.graceLoginsLeft} login(s) left.`}
            </p>
          )}
          <div>
            <Button onClick={() => setEnrolling(true)}>Set up two-factor</Button>
          </div>
        </div>
      )}

      {!newCodes && s && !s.confirmed && enrolling && (
        <TotpEnroll
          onComplete={() => {
            setEnrolling(false);
            void status.refetch();
            showToast({ message: "Two-factor enabled" });
          }}
          onCancel={() => setEnrolling(false)}
        />
      )}

      {!newCodes && s && s.confirmed && (
        <div className="flex flex-col gap-3">
          <p className="text-sm">
            <span className="font-medium">Enabled</span> · {s.recoveryRemaining} recovery code(s)
            left
          </p>
          <div className="flex flex-wrap gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  const codes = await totpRegenerate();
                  setNewCodes(codes);
                })
              }
            >
              Regenerate recovery codes
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={busy || s.ownerEnforced}
              title={s.ownerEnforced ? "Owners must keep two-factor enabled" : undefined}
              onClick={() =>
                void run(async () => {
                  await totpDisable();
                  showToast({ message: "Two-factor disabled" });
                  await status.refetch();
                })
              }
            >
              Disable
            </Button>
          </div>
        </div>
      )}
    </Section>
  );
}
