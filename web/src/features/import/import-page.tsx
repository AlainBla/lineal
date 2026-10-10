import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, CheckCircle2, Info, Loader2, ShieldAlert, XCircle } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { GraphQLRequestError } from "@/lib/graphql";
import { cn } from "@/lib/utils";

import {
  ACTIVE,
  control,
  fetchReport,
  fetchRun,
  fetchRuns,
  handled,
  keys,
  progressPercent,
  scan,
  start,
  type ImportRunView,
  type ScanTeam,
} from "./api";

function errorText(err: unknown): string {
  if (err instanceof GraphQLRequestError) {
    if (err.code === "forbidden") return "Only workspace admins can import data.";
    return err.message;
  }
  return err instanceof Error ? err.message : "Something went wrong.";
}

function Card({
  title,
  children,
  aside,
}: {
  title: string;
  children: ReactNode;
  aside?: ReactNode;
}) {
  return (
    <section className="rounded-lg border border-border bg-surface">
      <header className="flex h-10 items-center justify-between border-b border-border px-4">
        <h2 className="text-sm font-medium">{title}</h2>
        {aside}
      </header>
      <div className="p-4">{children}</div>
    </section>
  );
}

const STATUS_LABEL: Record<string, string> = {
  scanning: "Scanning",
  planned: "Plan ready",
  running: "Running",
  paused: "Paused",
  failed: "Failed",
  completed: "Completed",
  canceled: "Canceled",
};

function StatusBadge({ status }: { status: string }) {
  const tone =
    status === "completed"
      ? "text-success"
      : status === "failed" || status === "canceled"
        ? "text-destructive"
        : "text-muted-foreground";
  return (
    <span className={cn("text-xs font-medium", tone)} data-testid="run-status">
      {STATUS_LABEL[status] ?? status}
    </span>
  );
}

/** Schritt 1: Token eingeben und Verbindung prüfen. */
function ConnectStep({ onStarted }: { onStarted: (run: ImportRunView) => void }) {
  const [token, setToken] = useState("");
  const m = useMutation({
    mutationFn: () => scan(token.trim()),
    onSuccess: (run) => {
      setToken("");
      onStarted(run);
    },
  });
  return (
    <Card title="Connect to Linear">
      <p className="mb-3 max-w-xl text-sm text-muted-foreground">
        Create a personal API key in Linear (Settings, API) with read access and paste it here. The
        key is stored encrypted, only used for read-only requests, and deleted when the import
        finishes or after 7 days.
      </p>
      <form
        className="flex max-w-xl items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (token.trim()) m.mutate();
        }}
      >
        <Input
          type="password"
          autoComplete="off"
          spellCheck={false}
          aria-label="Linear API key"
          placeholder="lin_api_…"
          value={token}
          onChange={(e) => setToken(e.target.value)}
        />
        <Button type="submit" disabled={!token.trim() || m.isPending}>
          {m.isPending ? <Loader2 className="animate-spin" /> : null}
          Check connection
        </Button>
      </form>
      {m.isError ? (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {errorText(m.error)}
        </p>
      ) : null}
    </Card>
  );
}

/** Schritt 2: Plan mit Teams, Key-Konflikten und User-Zuordnung. */
function PlanStep({
  run,
  onChanged,
}: {
  run: ImportRunView;
  onChanged: (r: ImportRunView) => void;
}) {
  const scanData = run.scan;
  const teams = scanData?.teams ?? [];
  const [selected, setSelected] = useState<Set<string>>(() => new Set(teams.map((t) => t.key)));
  const [keyMap, setKeyMap] = useState<Record<string, string>>({});
  const [delta, setDelta] = useState(false);
  const m = useMutation({
    mutationFn: (dryRun: boolean) =>
      start({
        runId: run.id,
        teams: teams.filter((t) => selected.has(t.key)).map((t) => t.key),
        keyMapping: Object.fromEntries(Object.entries(keyMap).filter(([, v]) => v.trim())),
        dryRun,
        delta,
      }),
    onSuccess: onChanged,
  });
  const unresolved = teams.filter(
    (t) => selected.has(t.key) && t.conflict && !(keyMap[t.key] ?? "").trim(),
  );
  const users = scanData?.users ?? [];
  const matched = users.filter((u) => u.matchedUserId).length;
  const counts = scanData?.counts ?? {};
  return (
    <div className="flex flex-col gap-4">
      <Card title={`Teams in ${scanData?.workspace?.name ?? "Linear"}`}>
        <ul className="flex flex-col divide-y divide-border">
          {teams.map((t: ScanTeam) => (
            <li key={t.id} className="flex items-center gap-3 py-2 text-sm">
              <input
                type="checkbox"
                aria-label={`Import team ${t.key}`}
                checked={selected.has(t.key)}
                onChange={(e) => {
                  const next = new Set(selected);
                  if (e.target.checked) next.add(t.key);
                  else next.delete(t.key);
                  setSelected(next);
                }}
              />
              <span className="w-14 font-mono text-xs">{t.key}</span>
              <span className="flex-1">{t.name}</span>
              {t.private ? (
                <span className="text-xs text-muted-foreground">private, stays private</span>
              ) : null}
              <span className="text-xs text-muted-foreground">{t.issueCount} issues</span>
              {t.conflict ? (
                <label className="flex items-center gap-1.5 text-xs text-destructive">
                  <ShieldAlert className="size-3.5" />
                  Key exists, new key:
                  <Input
                    className="h-7 w-20"
                    aria-label={`Target key for ${t.key}`}
                    value={keyMap[t.key] ?? ""}
                    onChange={(e) =>
                      setKeyMap({ ...keyMap, [t.key]: e.target.value.toUpperCase().slice(0, 7) })
                    }
                  />
                </label>
              ) : null}
            </li>
          ))}
        </ul>
        {unresolved.length > 0 ? (
          <p role="alert" className="mt-3 text-sm text-destructive">
            Team key conflict: choose a different key for {unresolved.map((t) => t.key).join(", ")}{" "}
            or deselect the team.
          </p>
        ) : null}
      </Card>
      <Card title="People">
        <p className="text-sm text-muted-foreground">
          {users.length} people found, {matched} matched to existing users by e-mail address. The
          others are created as placeholder users without login and are linked automatically when
          they are invited with the same e-mail address.
        </p>
        <p className="mt-2 text-xs text-muted-foreground" data-testid="plan-counts">
          {Object.entries(counts)
            .filter(([, n]) => n > 0)
            .map(([k, n]) => `${n} ${k}`)
            .join(" · ")}
        </p>
      </Card>
      <div className="flex items-center gap-2">
        <label className="flex items-center gap-1.5 text-sm text-muted-foreground">
          <input type="checkbox" checked={delta} onChange={(e) => setDelta(e.target.checked)} />
          Delta import (only changes since the last import)
        </label>
        <span className="flex-1" />
        <Button
          variant="outline"
          disabled={m.isPending || selected.size === 0 || unresolved.length > 0}
          onClick={() => m.mutate(true)}
        >
          Dry run
        </Button>
        <Button
          disabled={m.isPending || selected.size === 0 || unresolved.length > 0}
          onClick={() => m.mutate(false)}
        >
          Start import
        </Button>
      </div>
      {m.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {errorText(m.error)}
        </p>
      ) : null}
      {run.dryRun && run.stats.entities ? (
        <Card title="Result of the dry run">
          <Counters run={run} />
        </Card>
      ) : null}
    </div>
  );
}

function Counters({ run }: { run: ImportRunView }) {
  const entities = Object.entries(run.stats.entities ?? {});
  if (entities.length === 0) return <p className="text-sm text-muted-foreground">No data yet.</p>;
  return (
    <table className="w-full text-sm">
      <thead>
        <tr className="text-left text-xs text-muted-foreground">
          <th className="py-1 font-medium">Object</th>
          <th className="font-medium">Imported</th>
          <th className="font-medium">Skipped</th>
          <th className="font-medium">Conflicts</th>
          <th className="font-medium">Errors</th>
        </tr>
      </thead>
      <tbody>
        {entities.map(([name, s]) => (
          <tr key={name} className="border-t border-border">
            <td className="py-1">{name}</td>
            <td>{handled(s)}</td>
            <td>{s.skipped}</td>
            <td>{s.conflicts}</td>
            <td className={s.errors > 0 ? "text-destructive" : undefined}>{s.errors}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** Schritt 3: Fortschritt mit Pause/Fortsetzen/Abbrechen. */
function ProgressStep({
  run,
  onChanged,
}: {
  run: ImportRunView;
  onChanged: (r: ImportRunView) => void;
}) {
  const act = useMutation({
    mutationFn: (a: "importPause" | "importResume" | "importCancel") => control(a, run.id),
    onSuccess: onChanged,
  });
  const pct = progressPercent(run);
  return (
    <Card
      title={run.dryRun ? "Dry run in progress" : "Import in progress"}
      aside={<StatusBadge status={run.status} />}
    >
      <div
        role="progressbar"
        aria-label="Import progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct}
        className="h-2 overflow-hidden rounded-full bg-surface-active"
      >
        <div className="h-full bg-primary transition-[width]" style={{ width: `${pct}%` }} />
      </div>
      <p className="mt-2 text-sm text-muted-foreground">
        {run.phase ? `Phase ${run.phase} of 8` : "Starting"}
        {run.step ? ` · ${run.step}` : ""}
      </p>
      <div className="mt-4">
        <Counters run={run} />
      </div>
      <div className="mt-4 flex gap-2">
        {run.status === "running" ? (
          <Button variant="outline" onClick={() => act.mutate("importPause")}>
            Pause
          </Button>
        ) : null}
        {run.status === "paused" || run.status === "failed" ? (
          <Button onClick={() => act.mutate("importResume")}>Resume</Button>
        ) : null}
        <Button variant="ghost" onClick={() => act.mutate("importCancel")}>
          Cancel
        </Button>
      </div>
      {act.isError ? (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {errorText(act.error)}
        </p>
      ) : null}
    </Card>
  );
}

/** Schritt 4: Bericht. */
function ReportStep({ run, onRestart }: { run: ImportRunView; onRestart: () => void }) {
  const [severity, setSeverity] = useState<string | null>(null);
  const report = useQuery({
    queryKey: keys.report(run.id, severity),
    queryFn: () => fetchReport(run.id, severity),
  });
  const Icon =
    run.status === "completed" ? CheckCircle2 : run.status === "failed" ? XCircle : AlertTriangle;
  return (
    <div className="flex flex-col gap-4">
      <Card
        title="Report"
        aside={
          <span className="flex items-center gap-1.5">
            <Icon className="size-4" />
            <StatusBadge status={run.status} />
          </span>
        }
      >
        {run.error ? (
          <p role="alert" className="mb-3 text-sm text-destructive">
            {run.error}
          </p>
        ) : null}
        <Counters run={run} />
      </Card>
      <Card
        title="Notes and warnings"
        aside={
          <div className="flex gap-1">
            {[null, "error", "warning", "info"].map((s) => (
              <Button
                key={s ?? "all"}
                size="sm"
                variant={severity === s ? "secondary" : "ghost"}
                onClick={() => setSeverity(s)}
              >
                {s ?? "all"}
              </Button>
            ))}
          </div>
        }
      >
        {report.data ? (
          <p className="mb-2 text-xs text-muted-foreground">
            {report.data.errorCount} errors · {report.data.warningCount} warnings ·{" "}
            {report.data.infoCount} notes
          </p>
        ) : null}
        <ul className="flex max-h-96 flex-col divide-y divide-border overflow-auto text-sm">
          {(report.data?.entries ?? []).map((e, i) => (
            <li key={i} className="flex items-start gap-2 py-1.5">
              {e.severity === "error" ? (
                <XCircle className="mt-0.5 size-3.5 text-destructive" />
              ) : e.severity === "warning" ? (
                <AlertTriangle className="mt-0.5 size-3.5 text-warning" />
              ) : (
                <Info className="mt-0.5 size-3.5 text-muted-foreground" />
              )}
              <span className="flex-1">{e.message}</span>
              {e.sourceType ? (
                <span className="text-xs text-muted-foreground">{e.sourceType}</span>
              ) : null}
            </li>
          ))}
          {report.data && report.data.entries.length === 0 ? (
            <li className="py-2 text-muted-foreground">Nothing to report.</li>
          ) : null}
        </ul>
      </Card>
      <div>
        <Button variant="outline" onClick={onRestart}>
          New scan / delta import
        </Button>
      </div>
    </div>
  );
}

export function ImportPage() {
  const qc = useQueryClient();
  const [runId, setRunId] = useState<string | null>(null);
  const runs = useQuery({ queryKey: keys.runs, queryFn: fetchRuns });
  const latest = useMemo(() => runs.data?.[0] ?? null, [runs.data]);
  const currentId = runId ?? latest?.id ?? null;
  const run = useQuery({
    queryKey: keys.run(currentId ?? ""),
    queryFn: () => fetchRun(currentId ?? ""),
    enabled: currentId !== null,
    // Solange der Job arbeitet, den Fortschritt abfragen.
    refetchInterval: (q) => (q.state.data && ACTIVE.includes(q.state.data.status) ? 2000 : false),
  });
  const setRun = (r: ImportRunView) => {
    qc.setQueryData(keys.run(r.id), r);
    void qc.invalidateQueries({ queryKey: keys.runs });
    setRunId(r.id);
  };
  const r = run.data;
  const forbidden =
    (runs.error instanceof GraphQLRequestError && runs.error.code === "forbidden") ||
    (run.error instanceof GraphQLRequestError && run.error.code === "forbidden");

  let body: ReactNode;
  if (forbidden) {
    body = (
      <p role="alert" className="text-sm text-destructive">
        Only workspace admins can import data.
      </p>
    );
  } else if (runs.isLoading || (currentId && run.isLoading)) {
    body = <p className="text-sm text-muted-foreground">Loading…</p>;
  } else if (!r || runId === "new") {
    body = <ConnectStep onStarted={setRun} />;
  } else if (r.status === "scanning") {
    body = (
      <Card title="Checking the connection" aside={<StatusBadge status={r.status} />}>
        <p className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" /> Reading teams, people and counts from Linear…
        </p>
      </Card>
    );
  } else if (r.status === "planned") {
    body = <PlanStep run={r} onChanged={setRun} />;
  } else if (["running", "paused"].includes(r.status)) {
    body = <ProgressStep run={r} onChanged={setRun} />;
  } else {
    body = <ReportStep run={r} onRestart={() => setRunId("new")} />;
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-4 p-6">
      <header>
        <h1 className="text-lg font-semibold">Import from Linear</h1>
        <p className="text-sm text-muted-foreground">
          Move an existing Linear workspace into Lineal. Identifiers such as ENG-123 stay the same,
          and nothing notifies anyone while the import runs.
        </p>
      </header>
      {body}
    </div>
  );
}
