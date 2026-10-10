/**
 * GraphQL-Operationen und Modelle des Import-Assistenten (docs/13). Gegen `web/schema.graphql`
 * programmiert; `scan`/`stats` sind JSON und hier typisiert.
 */
import { gqlRequest } from "@/lib/graphql";

export type RunStatus =
  "scanning" | "planned" | "running" | "paused" | "failed" | "completed" | "canceled";

export interface ScanTeam {
  id: string;
  key: string;
  name: string | null;
  private: boolean;
  parentId: string | null;
  issueCount: number;
  conflict: boolean;
}

export interface ScanUser {
  id: string;
  name: string | null;
  displayName: string | null;
  email: string | null;
  admin: boolean;
  guest: boolean;
  matchedUserId: string | null;
}

export interface Scan {
  workspace?: { name?: string; urlKey?: string };
  teams: ScanTeam[];
  users: ScanUser[];
  counts: Record<string, number>;
  conflicts: Array<{ key: string; reason: string }>;
}

export interface EntityStats {
  created: number;
  updated: number;
  unchanged: number;
  skipped: number;
  conflicts: number;
  errors: number;
}

export interface ImportRunView {
  id: string;
  source: string;
  status: RunStatus;
  phase: number | null;
  step: string | null;
  dryRun: boolean;
  scan: Scan | null;
  stats: { entities?: Record<string, EntityStats> };
  error: string | null;
  startedAt: string | null;
  finishedAt: string | null;
  createdAt: string;
}

export interface ImportIssue {
  severity: "info" | "warning" | "error";
  sourceType: string | null;
  sourceId: string | null;
  message: string;
}

export interface ImportReport {
  entries: ImportIssue[];
  infoCount: number;
  warningCount: number;
  errorCount: number;
}

export const keys = {
  runs: ["import", "runs"] as const,
  run: (id: string) => ["import", "run", id] as const,
  report: (id: string, severity: string | null) => ["import", "report", id, severity] as const,
};

const RUN_FIELDS = `id source status phase step dryRun scan stats error startedAt finishedAt createdAt`;

export const ACTIVE: RunStatus[] = ["scanning", "running"];

export async function fetchRuns(): Promise<ImportRunView[]> {
  const d = await gqlRequest<{ importRuns: ImportRunView[] }>(
    `query ImportRuns { importRuns(first: 20) { ${RUN_FIELDS} } }`,
  );
  return d.importRuns;
}

export async function fetchRun(id: string): Promise<ImportRunView> {
  const d = await gqlRequest<{ importRun: ImportRunView }, Record<string, unknown>>(
    `query ImportRun($id: String!) { importRun(id: $id) { ${RUN_FIELDS} } }`,
    { id },
  );
  return d.importRun;
}

export async function fetchReport(id: string, severity: string | null): Promise<ImportReport> {
  const d = await gqlRequest<{ importReport: ImportReport }, Record<string, unknown>>(
    `query ImportReport($id: String!, $severity: String) {
       importReport(runId: $id, severity: $severity, first: 200) {
         infoCount warningCount errorCount entries { severity sourceType sourceId message }
       }
     }`,
    { id, severity },
  );
  return d.importReport;
}

export async function scan(token: string): Promise<ImportRunView> {
  const d = await gqlRequest<{ importScan: ImportRunView }, Record<string, unknown>>(
    `mutation ImportScan($input: ImportScanInput!) { importScan(input: $input) { ${RUN_FIELDS} } }`,
    { input: { source: "linear", token } },
  );
  return d.importScan;
}

export interface StartInput {
  runId: string;
  teams: string[];
  keyMapping: Record<string, string>;
  dryRun: boolean;
  delta: boolean;
}

export async function start(input: StartInput): Promise<ImportRunView> {
  const d = await gqlRequest<{ importStart: ImportRunView }, Record<string, unknown>>(
    `mutation ImportStart($input: ImportStartInput!) { importStart(input: $input) { ${RUN_FIELDS} } }`,
    { input: { ...input } },
  );
  return d.importStart;
}

export async function control(
  action: "importPause" | "importResume" | "importCancel",
  runId: string,
): Promise<ImportRunView> {
  const d = await gqlRequest<Record<string, ImportRunView>, Record<string, unknown>>(
    `mutation ImportControl($runId: String!) { ${action}(runId: $runId) { ${RUN_FIELDS} } }`,
    { runId },
  );
  const run = d[action];
  if (!run) throw new Error("Unexpected empty response.");
  return run;
}

/** Fortschritt in Prozent aus Phase (1–8) und Schritt (grobe Schätzung, nie über 100). */
export function progressPercent(run: Pick<ImportRunView, "status" | "phase">): number {
  if (run.status === "completed") return 100;
  if (!run.phase) return 0;
  return Math.min(99, Math.round(((run.phase - 1) / 8) * 100));
}

/** Summe der Zähler einer Entität (übernommen = created + updated + unchanged). */
export function handled(s: EntityStats): number {
  return s.created + s.updated + s.unchanged;
}
