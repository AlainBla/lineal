/**
 * Abschnitt „Sync“ der Import-Seite (docs/13 §12): regelmäßiger Abgleich Linear -> Lineal.
 * Zeigt Status, nächsten Lauf und letztes Ergebnis; Einrichten/Ändern, „Jetzt abgleichen“,
 * Pausieren, Löschen. Der Linear-API-Schlüssel wird nur gesendet, nie angezeigt oder behalten.
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { GraphQLRequestError } from "@/lib/graphql";

import {
  deleteSync,
  fetchSync,
  keys,
  pauseSync,
  runSync,
  saveSync,
  type ImportRunView,
  type ImportSyncView,
  type SyncInterval,
} from "./api";

const INTERVAL_LABEL: Record<SyncInterval, string> = {
  off: "Off (manual only)",
  hourly: "Every hour",
  every6h: "Every 6 hours",
  daily: "Daily",
};

function errorText(err: unknown): string {
  if (err instanceof GraphQLRequestError) return err.message;
  return err instanceof Error ? err.message : "Something went wrong.";
}

function when(iso: string | null): string {
  if (!iso) return "–";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

/** Minuten seit 00:00 UTC <-> `HH:MM` (UTC). */
export function minuteToTime(m: number): string {
  const h = Math.floor(m / 60);
  return `${String(h).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}

export function timeToMinute(t: string): number {
  const [h, m] = t.split(":").map((x) => Number.parseInt(x, 10));
  return Math.min(1439, Math.max(0, (h || 0) * 60 + (m || 0)));
}

function SyncForm({
  sync,
  teamKeys,
  onDone,
  onCancel,
}: {
  sync: ImportSyncView | null;
  teamKeys: string[];
  onDone: (s: ImportSyncView) => void;
  onCancel?: () => void;
}) {
  const [interval, setIntervalValue] = useState<SyncInterval>(sync?.interval ?? "daily");
  const [time, setTime] = useState(minuteToTime(sync?.dailyAtMinute ?? 180));
  const [chosen, setChosen] = useState<Set<string>>(() => new Set(sync?.teams ?? teamKeys));
  const [token, setToken] = useState("");
  const save = useMutation({
    mutationFn: () =>
      saveSync({
        interval,
        dailyAtMinute: interval === "daily" ? timeToMinute(time) : undefined,
        teams: teamKeys.filter((k) => chosen.has(k)),
        token: token.trim() || undefined,
      }),
    onSuccess: (s) => {
      setToken("");
      onDone(s);
    },
  });
  const needsKey = !sync;
  const invalid = (needsKey && !token.trim()) || (teamKeys.length > 0 && chosen.size === 0);
  return (
    <form
      aria-label="Sync settings"
      className="flex max-w-xl flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!invalid) save.mutate();
      }}
    >
      <label className="flex items-center gap-2 text-sm">
        <span className="w-28 text-muted-foreground">Interval</span>
        <select
          aria-label="Sync interval"
          className="h-8 rounded-md border border-border bg-surface px-2"
          value={interval}
          onChange={(e) => setIntervalValue(e.target.value as SyncInterval)}
        >
          {(Object.keys(INTERVAL_LABEL) as SyncInterval[]).map((i) => (
            <option key={i} value={i}>
              {INTERVAL_LABEL[i]}
            </option>
          ))}
        </select>
        {interval === "daily" ? (
          <Input
            type="time"
            aria-label="Daily at (UTC)"
            className="h-8 w-28"
            value={time}
            onChange={(e) => setTime(e.target.value)}
          />
        ) : null}
        {interval === "daily" ? <span className="text-xs text-muted-foreground">UTC</span> : null}
      </label>
      {teamKeys.length > 0 ? (
        <fieldset className="flex flex-wrap items-center gap-3 text-sm">
          <legend className="sr-only">Teams to keep in sync</legend>
          <span className="w-28 text-muted-foreground">Teams</span>
          {teamKeys.map((k) => (
            <label key={k} className="flex items-center gap-1.5">
              <input
                type="checkbox"
                aria-label={`Sync team ${k}`}
                checked={chosen.has(k)}
                onChange={(e) => {
                  const next = new Set(chosen);
                  if (e.target.checked) next.add(k);
                  else next.delete(k);
                  setChosen(next);
                }}
              />
              <span className="font-mono text-xs">{k}</span>
            </label>
          ))}
        </fieldset>
      ) : null}
      <label className="flex items-center gap-2 text-sm">
        <span className="w-28 text-muted-foreground">
          {needsKey ? "Linear API key" : "Replace key"}
        </span>
        <Input
          type="password"
          autoComplete="off"
          spellCheck={false}
          aria-label="Linear API key for the sync"
          placeholder={needsKey ? "lin_api_…" : "Leave empty to keep the stored key"}
          value={token}
          onChange={(e) => setToken(e.target.value)}
        />
      </label>
      <p className="text-xs text-muted-foreground">
        The key is stored encrypted, used for read-only requests only and never shown again. Linear
        stays the source: a field you changed in Lineal since the last sync is not overwritten.
      </p>
      <div className="flex gap-2">
        <Button type="submit" disabled={invalid || save.isPending}>
          {save.isPending ? <Loader2 className="animate-spin" /> : null}
          {sync ? "Save sync settings" : "Set up sync"}
        </Button>
        {onCancel ? (
          <Button type="button" variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
        ) : null}
      </div>
      {save.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {errorText(save.error)}
        </p>
      ) : null}
    </form>
  );
}

/** Abschnitt „Sync“. `completedRun`: letzter abgeschlossener Linear-Import (Basis des Abgleichs). */
export function SyncSection({
  completedRun,
  onOpenRun,
}: {
  completedRun: ImportRunView | null;
  onOpenRun: (id: string) => void;
}) {
  const qc = useQueryClient();
  const sync = useQuery({
    queryKey: keys.sync,
    queryFn: fetchSync,
    refetchInterval: (q) => (q.state.data?.lastStatus === "running" ? 3000 : false),
  });
  const [editing, setEditing] = useState(false);
  const [reload, setReload] = useState(false);
  const [mergeLabels, setMergeLabels] = useState(false);
  const refresh = (s?: ImportSyncView | null) => {
    if (s !== undefined) qc.setQueryData(keys.sync, s);
    void qc.invalidateQueries({ queryKey: keys.sync });
    void qc.invalidateQueries({ queryKey: keys.runs });
  };
  const pause = useMutation({ mutationFn: (p: boolean) => pauseSync(p), onSuccess: refresh });
  const del = useMutation({
    mutationFn: deleteSync,
    onSuccess: () => {
      refresh(null);
      setEditing(false);
    },
  });
  const now = useMutation({
    mutationFn: () => runSync({ reloadContent: reload, mergeLinearLabels: mergeLabels }),
    onSuccess: () => {
      setReload(false);
      setMergeLabels(false);
      refresh();
    },
  });
  const forbidden = sync.error instanceof GraphQLRequestError && sync.error.code === "forbidden";
  if (forbidden) return null;
  const s = sync.data;
  const teamKeys =
    completedRun?.options?.teams ?? completedRun?.scan?.teams.map((t) => t.key) ?? [];
  const canSetUp = completedRun !== null || !!s;
  const err = pause.error ?? del.error ?? now.error;
  return (
    <section
      aria-label="Sync"
      className="rounded-lg border border-border bg-surface"
      data-testid="sync-section"
    >
      <header className="flex h-10 items-center justify-between border-b border-border px-4">
        <h2 className="text-sm font-medium">Sync</h2>
        {s ? (
          <span className="text-xs font-medium text-muted-foreground" data-testid="sync-state">
            {s.paused ? "Paused" : s.interval === "off" ? "Manual only" : "Active"}
          </span>
        ) : null}
      </header>
      <div className="flex flex-col gap-3 p-4">
        {sync.isLoading ? <p className="text-sm text-muted-foreground">Loading…</p> : null}
        {!sync.isLoading && !s ? (
          <>
            <p className="max-w-xl text-sm text-muted-foreground">
              Keep Lineal up to date with Linear: a recurring job imports what changed in Linear
              since the last run. New objects are created, deleted or archived ones are archived,
              and anything you edited in Lineal in the meantime is kept and listed in the report.
            </p>
            {canSetUp ? (
              <SyncForm sync={null} teamKeys={teamKeys} onDone={refresh} />
            ) : (
              <p className="text-sm text-muted-foreground">
                Complete an import first; the sync continues from it.
              </p>
            )}
          </>
        ) : null}
        {s && !editing ? (
          <>
            <dl className="grid grid-cols-[8rem_1fr] gap-y-1 text-sm">
              <dt className="text-muted-foreground">Interval</dt>
              <dd data-testid="sync-interval">
                {INTERVAL_LABEL[s.interval]}
                {s.interval === "daily" ? ` at ${minuteToTime(s.dailyAtMinute)} UTC` : ""}
              </dd>
              <dt className="text-muted-foreground">Teams</dt>
              <dd className="font-mono text-xs">{s.teams.join(", ") || "–"}</dd>
              <dt className="text-muted-foreground">Next run</dt>
              <dd data-testid="sync-next">
                {s.paused || s.interval === "off" ? "–" : when(s.nextRunAt)}
              </dd>
              <dt className="text-muted-foreground">Last result</dt>
              <dd data-testid="sync-last">
                {s.lastStatus === null ? (
                  "No run yet"
                ) : (
                  <>
                    <span
                      className={
                        s.lastStatus === "failed"
                          ? "font-medium text-destructive"
                          : s.lastStatus === "completed"
                            ? "font-medium text-success"
                            : "font-medium"
                      }
                    >
                      {s.lastStatus === "running"
                        ? "Running"
                        : s.lastStatus === "completed"
                          ? "Completed"
                          : "Failed"}
                    </span>{" "}
                    · {when(s.lastRunAt)}
                    {s.lastRunId ? (
                      <>
                        {" · "}
                        <button
                          type="button"
                          className="underline"
                          onClick={() => onOpenRun(s.lastRunId as string)}
                        >
                          Show report
                        </button>
                      </>
                    ) : null}
                  </>
                )}
              </dd>
              {s.lastStatus === "completed" ? (
                <>
                  <dt className="text-muted-foreground">Last success</dt>
                  <dd>{when(s.lastSuccessAt)}</dd>
                </>
              ) : null}
            </dl>
            {s.lastError ? (
              <p role="alert" className="text-sm text-destructive">
                {s.lastError}
              </p>
            ) : null}
            <fieldset className="flex flex-col gap-1 text-sm">
              <legend className="sr-only">Options for this run</legend>
              <label className="flex items-center gap-1.5 text-muted-foreground">
                <input
                  type="checkbox"
                  checked={reload}
                  onChange={(e) => setReload(e.target.checked)}
                />
                Reload texts that contain placeholders such as &lt;token&gt; (descriptions, comments
                and documents you did not change in Lineal; reads everything from Linear)
              </label>
              <label className="flex items-center gap-1.5 text-muted-foreground">
                <input
                  type="checkbox"
                  checked={mergeLabels}
                  onChange={(e) => setMergeLabels(e.target.checked)}
                />
                Merge earlier duplicate labels named “… (Linear)” into the label with the same name
              </label>
            </fieldset>
            <div className="flex flex-wrap gap-2">
              <Button
                onClick={() => now.mutate()}
                disabled={now.isPending || s.lastStatus === "running"}
              >
                {now.isPending ? <Loader2 className="animate-spin" /> : null}
                Sync now
              </Button>
              <Button variant="outline" onClick={() => setEditing(true)}>
                Edit
              </Button>
              <Button
                variant="outline"
                disabled={pause.isPending}
                onClick={() => pause.mutate(!s.paused)}
              >
                {s.paused ? "Resume" : "Pause"}
              </Button>
              <Button
                variant="ghost"
                disabled={del.isPending}
                onClick={() => {
                  if (window.confirm("Delete the sync and the stored Linear API key?"))
                    del.mutate();
                }}
              >
                Delete
              </Button>
            </div>
          </>
        ) : null}
        {s && editing ? (
          <SyncForm
            sync={s}
            teamKeys={teamKeys.length > 0 ? teamKeys : s.teams}
            onDone={(next) => {
              refresh(next);
              setEditing(false);
            }}
            onCancel={() => setEditing(false)}
          />
        ) : null}
        {err ? (
          <p role="alert" className="text-sm text-destructive">
            {errorText(err)}
          </p>
        ) : null}
      </div>
    </section>
  );
}
