/**
 * LIN-70: Enrollment-Fluss (geteilt zwischen erzwungenem Login und Account-Panel).
 * Schritt 1: QR/Secret koppeln und ersten Code bestätigen. Schritt 2: Recovery-Codes
 * einmalig anzeigen. `pendingToken` kennzeichnet den erzwungenen Pfad (ohne Session).
 */
import { useEffect, useState, type FormEvent } from "react";
import { QRCodeSVG } from "qrcode.react";

import { totpEnrollBegin, totpEnrollConfirm, type EnrollStart } from "@/app/totp-api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

function copy(text: string) {
  void navigator.clipboard?.writeText(text);
}

function RecoveryCodes({ codes, onDone }: { codes: string[]; onDone: () => void }) {
  const [saved, setSaved] = useState(false);
  const asText = codes.join("\n");
  return (
    <div className="flex flex-col gap-4">
      <div>
        <h3 className="text-sm font-semibold">Save your recovery codes</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          Each code works once if you lose your authenticator. We show them only now.
        </p>
      </div>
      <ul className="grid grid-cols-2 gap-x-6 gap-y-1.5 rounded-lg border border-border bg-muted/40 p-3 font-mono text-sm">
        {codes.map((c) => (
          <li key={c}>{c}</li>
        ))}
      </ul>
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" size="sm" onClick={() => copy(asText)}>
          Copy
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => {
            const blob = new Blob([asText], { type: "text/plain" });
            const url = URL.createObjectURL(blob);
            const a = document.createElement("a");
            a.href = url;
            a.download = "lineal-recovery-codes.txt";
            a.click();
            URL.revokeObjectURL(url);
          }}
        >
          Download
        </Button>
      </div>
      <label className="flex items-center gap-2 text-sm">
        <input type="checkbox" checked={saved} onChange={(e) => setSaved(e.target.checked)} />I have
        saved these codes
      </label>
      <Button type="button" disabled={!saved} onClick={onDone}>
        Done — two-factor is on
      </Button>
    </div>
  );
}

export function TotpEnroll({
  pendingToken,
  onComplete,
  onCancel,
}: {
  pendingToken?: string;
  onComplete: () => void;
  onCancel?: () => void;
}) {
  const [start, setStart] = useState<EnrollStart | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [confirmError, setConfirmError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [recovery, setRecovery] = useState<string[] | null>(null);

  useEffect(() => {
    let active = true;
    totpEnrollBegin(pendingToken)
      .then((s) => active && setStart(s))
      .catch(
        (e: unknown) => active && setStartError(e instanceof Error ? e.message : "Failed to start"),
      );
    return () => {
      active = false;
    };
  }, [pendingToken]);

  async function onConfirm(e: FormEvent) {
    e.preventDefault();
    if (pending || code.trim().length < 6) return;
    setPending(true);
    setConfirmError(null);
    try {
      const codes = await totpEnrollConfirm(code.trim(), pendingToken);
      setRecovery(codes);
    } catch (err) {
      setConfirmError(err instanceof Error ? err.message : "Invalid code");
    } finally {
      setPending(false);
    }
  }

  if (recovery) return <RecoveryCodes codes={recovery} onDone={onComplete} />;

  if (startError)
    return (
      <p role="alert" className="text-sm text-destructive">
        {startError}
      </p>
    );

  if (!start) return <p className="text-sm text-muted-foreground">Preparing…</p>;

  return (
    <form onSubmit={onConfirm} className="flex flex-col gap-4">
      <p className="text-xs text-muted-foreground">
        Scan the code with your authenticator app, then enter the 6-digit code.
      </p>
      <div className="flex flex-wrap items-start gap-4">
        <div className="rounded-lg border border-border-strong bg-white p-2">
          <QRCodeSVG value={start.otpauthUri} size={132} />
        </div>
        <div className="min-w-[180px] flex-1">
          <Label className="text-xs">Or enter this key manually</Label>
          <div className="mt-1 flex items-center gap-2">
            <code className="rounded bg-muted px-2 py-1 font-mono text-xs tracking-wider break-all">
              {start.secretBase32}
            </code>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => copy(start.secretBase32)}
            >
              Copy
            </Button>
          </div>
        </div>
      </div>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="totp-enroll-code">Code from the app</Label>
        <Input
          id="totp-enroll-code"
          inputMode="numeric"
          autoComplete="one-time-code"
          maxLength={6}
          placeholder="123456"
          value={code}
          onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))}
          aria-invalid={confirmError ? true : undefined}
        />
        {confirmError && (
          <p role="alert" className="text-xs text-destructive">
            {confirmError}
          </p>
        )}
      </div>
      <div className="flex gap-2">
        <Button type="submit" disabled={pending || code.length < 6} aria-busy={pending}>
          {pending ? "Confirming…" : "Confirm & activate"}
        </Button>
        {onCancel && (
          <Button type="button" variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}
