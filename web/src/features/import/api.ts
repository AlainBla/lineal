/**
 * GraphQL-Operationen und Modelle des Import-Assistenten (docs/13). Gegen `web/schema.graphql`
 * programmiert; `scan`/`stats` sind JSON und hier typisiert.
 */
import { gqlRequest } from "@/lib/graphql";

export type RunStatus =
  "scanning" | "planned" | "running" | "paused" | "failed" | "completed" | "canceled";

export interface ScanProject {
  id: string;
  name: string | null;
  status: string | null;
  issueCount: number;
}

export interface ScanTeam {
  id: string;
  key: string;
  name: string | null;
  private: boolean;
  parentId: string | null;
  issueCount: number;
  conflict: boolean;
  /** Projekte des Teams; ein Projekt über mehrere Teams erscheint bei jedem. */
  projects?: ScanProject[];
}

/** Bestehendes Lineal-Team als Importziel. */
export interface TargetTeam {
  id: string;
  key: string;
  name: string;
}

/** Geplante Zuordnungen (`stats.plan`), Zähler des Dry-Runs. */
export interface ImportPlan {
  teamsMapped?: number;
  statesMapped?: number;
  statesNew?: number;
  labelsMapped?: number;
  labelsNew?: number;
  usersMatched?: number;
  usersPlaceholder?: number;
  relationsOutside?: number;
  aliasesCreated?: number;
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

/** Optionen eines Laufs (`options`), soweit die Oberfläche sie braucht. */
export interface RunOptions {
  teams?: string[];
  keyMapping?: Record<string, string>;
  targetTeams?: Record<string, string>;
  /** Gesetzt bei Läufen des regelmäßigen Abgleichs. */
  syncId?: string;
  delta?: boolean;
  reloadContent?: boolean;
  mergeLinearLabels?: boolean;
}

export interface ImportRunView {
  id: string;
  source: string;
  status: RunStatus;
  phase: number | null;
  step: string | null;
  dryRun: boolean;
  scan: Scan | null;
  options?: RunOptions;
  stats: { entities?: Record<string, EntityStats>; plan?: ImportPlan };
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
  targets: ["import", "targets"] as const,
  sync: ["import", "sync"] as const,
  existingUsers: ["import", "existing-users"] as const,
  joinable: ["import", "joinable-teams"] as const,
};

const RUN_FIELDS = `id source status phase step dryRun scan options stats error startedAt finishedAt createdAt`;

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
  /** Linear-Projekt-IDs; leer = alle. */
  projects: string[];
  /** Linear-Team-Key -> ID eines bestehenden Lineal-Teams. */
  targetTeams: Record<string, string>;
  /** Linear-User-ID -> ID eines bestehenden Lineal-Users (statt E-Mail-Abgleich/Platzhalter). */
  userMapping?: Record<string, string>;
}

/** Bestehender, aktiver Lineal-User als Ziel der Zuordnung. */
export interface ExistingUser {
  id: string;
  name: string;
  displayName: string;
}

export async function fetchExistingUsers(): Promise<ExistingUser[]> {
  const d = await gqlRequest<{ users: ExistingUser[] }>(
    `query ImportExistingUsers { users(first: 200) { id name displayName } }`,
  );
  return d.users;
}

/** Sichtbare Lineal-Teams (mögliche Importziele). */
export async function fetchTargetTeams(): Promise<TargetTeam[]> {
  const d = await gqlRequest<{ teams: TargetTeam[] }>(
    `query ImportTargetTeams { teams(first: 200) { id key name } }`,
  );
  return d.teams;
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

// ---------- Regelmäßiger Abgleich (docs/13 §12) ----------

export type SyncInterval = "off" | "hourly" | "every6h" | "daily";

export interface ImportSyncView {
  id: string;
  interval: SyncInterval;
  /** Minuten seit 00:00 UTC (nur `daily`). */
  dailyAtMinute: number;
  paused: boolean;
  teams: string[];
  projects: string[];
  /** Ein Schlüssel ist gespeichert (er selbst wird nie ausgegeben). */
  hasKey: boolean;
  lastRunId: string | null;
  lastRunAt: string | null;
  lastSuccessAt: string | null;
  lastStatus: "running" | "completed" | "failed" | null;
  lastError: string | null;
  nextRunAt: string | null;
}

const SYNC_FIELDS = `id interval dailyAtMinute paused teams projects hasKey lastRunId lastRunAt
  lastSuccessAt lastStatus lastError nextRunAt`;

export async function fetchSync(): Promise<ImportSyncView | null> {
  const d = await gqlRequest<{ importSync: ImportSyncView | null }>(
    `query ImportSync { importSync { ${SYNC_FIELDS} } }`,
  );
  return d.importSync;
}

export interface SyncSaveInput {
  interval: SyncInterval;
  dailyAtMinute?: number;
  teams: string[];
  /** Nur beim Einrichten Pflicht; sonst nur zum Ersetzen des Schlüssels. */
  token?: string;
}

export async function saveSync(input: SyncSaveInput): Promise<ImportSyncView> {
  const d = await gqlRequest<{ importSyncSave: ImportSyncView }, Record<string, unknown>>(
    `mutation ImportSyncSave($input: ImportSyncInput!) { importSyncSave(input: $input) { ${SYNC_FIELDS} } }`,
    { input: { ...input } },
  );
  return d.importSyncSave;
}

export async function pauseSync(paused: boolean): Promise<ImportSyncView> {
  const d = await gqlRequest<{ importSyncPause: ImportSyncView }, Record<string, unknown>>(
    `mutation ImportSyncPause($paused: Boolean!) { importSyncPause(paused: $paused) { ${SYNC_FIELDS} } }`,
    { paused },
  );
  return d.importSyncPause;
}

export async function deleteSync(): Promise<void> {
  await gqlRequest(`mutation ImportSyncDelete { importSyncDelete }`);
}

export interface SyncRunInput {
  /** Texte mit Tag-artigem Inhalt neu aus Linear holen (nur lokal unveränderte Objekte). */
  reloadContent: boolean;
  /** Frühere „Name (Linear)“-Duplikat-Labels zusammenführen. */
  mergeLinearLabels: boolean;
}

export async function runSync(v: SyncRunInput): Promise<ImportRunView> {
  const d = await gqlRequest<{ importSyncRun: ImportRunView }, Record<string, unknown>>(
    `mutation ImportSyncRun($reloadContent: Boolean, $mergeLinearLabels: Boolean) {
       importSyncRun(reloadContent: $reloadContent, mergeLinearLabels: $mergeLinearLabels) { ${RUN_FIELDS} }
     }`,
    { ...v },
  );
  return d.importSyncRun;
}

// ---------- Teams nach dem Import beitreten ----------

export interface JoinableTeam {
  id: string;
  key: string;
  name: string;
  viewer: { isMember: boolean; canJoin: boolean } | null;
}

export async function fetchTeamsWithViewer(): Promise<JoinableTeam[]> {
  const d = await gqlRequest<{ teams: JoinableTeam[] }>(
    `query ImportJoinableTeams { teams(first: 250) { id key name viewer { isMember canJoin } } }`,
  );
  return d.teams;
}

/** Teams des Laufs in Lineal: Ziel-Team, abweichender Key oder gleicher Key. */
export function importedTeamIds(run: ImportRunView, teams: JoinableTeam[]): string[] {
  const o = run.options ?? {};
  const keys = o.teams ?? run.scan?.teams.map((t) => t.key) ?? [];
  const out = new Set<string>();
  for (const k of keys) {
    const direct = o.targetTeams?.[k];
    if (direct) {
      out.add(direct);
      continue;
    }
    const key = (o.keyMapping?.[k] ?? k).toUpperCase();
    const hit = teams.find((t) => t.key.toUpperCase() === key);
    if (hit) out.add(hit.id);
  }
  return [...out];
}

export async function joinTeams(teamIds: string[], userId: string): Promise<void> {
  for (const teamId of teamIds) {
    await gqlRequest(
      `mutation ImportJoinTeam($teamId: String!, $userId: String!) {
         teamMembershipCreate(teamId: $teamId, userId: $userId) { success }
       }`,
      { teamId, userId },
    );
  }
}

// ---------- Platzhalter einem User zuordnen ----------

export interface PlaceholderMergeResult {
  success: boolean;
  moved: number;
  dropped: number;
  mentions: number;
}

export async function mergePlaceholder(
  placeholderId: string,
  targetUserId: string,
): Promise<PlaceholderMergeResult> {
  const d = await gqlRequest<
    { importPlaceholderMerge: PlaceholderMergeResult },
    Record<string, unknown>
  >(
    `mutation ImportPlaceholderMerge($placeholderId: String!, $targetUserId: String!) {
       importPlaceholderMerge(placeholderId: $placeholderId, targetUserId: $targetUserId) {
         success moved dropped mentions
       }
     }`,
    { placeholderId, targetUserId },
  );
  return d.importPlaceholderMerge;
}
