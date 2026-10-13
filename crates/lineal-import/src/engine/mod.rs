//! Import-Engine: Phasen 1–8 (docs/13 §4.1) mit Checkpoints, Mapping-Upserts, Dry-Run und Delta.
//!
//! * Jede Seite (50–100 Objekte) wird in **einer** vom Import gehaltenen Transaktion geschrieben;
//!   dieselbe Transaktion speichert den Checkpoint in `import_run.checkpoint`. Netzwerkzugriffe
//!   auf die Quelle passieren nie innerhalb einer offenen Transaktion (Ausnahme: Dry-Run, der
//!   eine einzige Transaktion hält und sie am Ende zurückrollt).
//! * Bereits gemappte Objekte werden über `import_mapping` erkannt (Upsert statt Doppel-Anlage);
//!   unveränderte (gleiche `checksum`) werden übersprungen.
//! * Alle Events tragen `EventOrigin::Import`.

mod content;
mod finish;
mod issues;
pub mod node;
mod people;
mod planning;
mod views;
mod workflow;

use crate::blob::BlobSink;
use crate::markdown::RewriteCtx;
use crate::port::{ImportSource, PageQuery, SourceEntity, SourceError, SourcePage};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use lineal_app::App;
use lineal_app::error::{AppError, AppResult, ErrorKind};
use lineal_app::ports::Tx;
use lineal_app::ports::repos::NewEvent;
use lineal_app::ports::repos::import::{
    ImportRun, ImportRunUpdate, ImportTable, MappingRow, NewMapping, ReportEntry,
};
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::OrgId;
use node::{N, checksum_of};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;
use uuid::Uuid;

/// Texte aus der Quelle: Obergrenzen (Zeichen).
pub(crate) const MAX_DESCRIPTION: usize = 100_000;
pub(crate) const MAX_COMMENT: usize = 100_000;
pub(crate) const MAX_DOCUMENT: usize = 500_000;

/// Einstellungen der Engine.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Seitengröße gegenüber der Quelle (50–100).
    pub page_size: u32,
    /// Testhaken: nach dieser Phase (1–8) anhalten (`RunOutcome::Stopped`), Checkpoint bleibt erhalten.
    pub stop_after_phase: Option<u8>,
    /// So lange gilt `team.import_locked_until` ab jetzt (wird je Phase erneuert).
    pub team_lock: ChronoDuration,
    /// Lease-Dauer des Runners.
    pub lease: Duration,
    /// Takt, in dem Lease und Team-Sperren auch außerhalb von Einheiten verlängert werden
    /// (lange Rate-Limit-Wartezeiten der Quelle).
    pub heartbeat: Duration,
    /// Takt, in dem Pause/Abbruch während eines langen Quellzugriffs geprüft werden.
    pub control_poll: Duration,
    /// Obergrenze der gespeicherten Berichtseinträge je Lauf.
    pub max_report_entries: usize,
    /// Gleichzeitige Downloads (docs/13 §4.1 Phase 6: 4).
    pub download_concurrency: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            page_size: 50,
            stop_after_phase: None,
            team_lock: ChronoDuration::hours(1),
            lease: Duration::from_secs(120),
            heartbeat: Duration::from_secs(30),
            control_poll: Duration::from_secs(2),
            max_report_entries: 10_000,
            download_concurrency: 4,
        }
    }
}

/// Ausgang eines `run`-Aufrufs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Completed,
    /// Dry-Run beendet (nichts geschrieben); Lauf steht wieder auf `planned`.
    DryRunFinished,
    Paused,
    Canceled,
    /// Nur mit `stop_after_phase`: Lauf bleibt `running` mit Checkpoint.
    Stopped {
        after_phase: u8,
    },
    /// Lauf ist fehlgeschlagen (fortsetzbar); Text ohne Zugangsdaten.
    Failed(String),
}

/// Der Importer.
pub struct ImportEngine<'a> {
    pub app: &'a App,
    pub source: &'a dyn ImportSource,
    pub sink: &'a dyn BlobSink,
    pub config: EngineConfig,
}

/// Aufbereitete Optionen eines Laufs.
#[derive(Debug, Clone, Default)]
pub(crate) struct Options {
    pub teams: Vec<String>,
    pub user_mapping: HashMap<String, Option<Uuid>>,
    pub key_mapping: HashMap<String, String>,
    pub dry_run: bool,
    pub updated_after: Option<DateTime<Utc>>,
    /// Nur Issues dieser Linear-Projekte (leer = alle).
    pub projects: HashSet<String>,
    /// Linear-Team-Key -> bestehendes Lineal-Team.
    pub target_teams: HashMap<String, Uuid>,
}

impl Options {
    /// Teilimport (Projektfilter oder bestehendes Zielteam): Personen und Labels werden erst bei
    /// Bedarf angelegt, statt den ganzen Quell-Workspace zu übernehmen.
    pub fn scoped(&self) -> bool {
        !self.projects.is_empty() || !self.target_teams.is_empty()
    }

    fn parse(v: &Value) -> Options {
        let strings = |k: &str| -> Vec<String> {
            v[k].as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let user_mapping = v["userMapping"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, val)| (k.clone(), val.as_str().and_then(|s| s.parse::<Uuid>().ok())))
                    .collect()
            })
            .unwrap_or_default();
        let key_mapping = v["keyMapping"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, val)| Some((k.clone(), val.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        Options {
            teams: strings("teams"),
            user_mapping,
            key_mapping,
            dry_run: v["dryRun"].as_bool().unwrap_or(false),
            updated_after: v["updatedAfter"].as_str().and_then(|s| s.parse().ok()),
            projects: strings("projects").into_iter().collect(),
            target_teams: v["targetTeams"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, val)| Some((k.clone(), val.as_str()?.parse().ok()?)))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    pub fn target_key(&self, source_key: &str) -> String {
        self.key_mapping
            .get(source_key)
            .cloned()
            .unwrap_or_else(|| source_key.to_owned())
    }
}

/// Reihenfolge nach Abhängigkeiten (docs/13 §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Users,
    Teams,
    Memberships,
    States,
    Labels,
    ProjectStatuses,
    ProjectLabels,
    Templates,
    Cycles,
    Initiatives,
    Projects,
    Milestones,
    Issues,
    IssueParents,
    IssueRelations,
    Comments,
    IssueHistory,
    IssueSubscriptions,
    Attachments,
    EmbeddedFiles,
    Documents,
    ProjectUpdates,
    InitiativeUpdates,
    UpdateComments,
    Views,
    Finalize,
}

impl Step {
    pub const ALL: [Step; 26] = [
        Self::Users,
        Self::Teams,
        Self::Memberships,
        Self::States,
        Self::Labels,
        Self::ProjectStatuses,
        Self::ProjectLabels,
        Self::Templates,
        Self::Cycles,
        Self::Initiatives,
        Self::Projects,
        Self::Milestones,
        Self::Issues,
        Self::IssueParents,
        Self::IssueRelations,
        Self::Comments,
        Self::IssueHistory,
        Self::IssueSubscriptions,
        Self::Attachments,
        Self::EmbeddedFiles,
        Self::Documents,
        Self::ProjectUpdates,
        Self::InitiativeUpdates,
        Self::UpdateComments,
        Self::Views,
        Self::Finalize,
    ];

    pub const fn phase(&self) -> u8 {
        match self {
            Self::Users | Self::Teams | Self::Memberships => 1,
            Self::States
            | Self::Labels
            | Self::ProjectStatuses
            | Self::ProjectLabels
            | Self::Templates => 2,
            Self::Cycles | Self::Initiatives | Self::Projects | Self::Milestones => 3,
            Self::Issues | Self::IssueParents | Self::IssueRelations => 4,
            Self::Comments | Self::IssueHistory | Self::IssueSubscriptions => 5,
            Self::Attachments | Self::EmbeddedFiles => 6,
            Self::Documents
            | Self::ProjectUpdates
            | Self::InitiativeUpdates
            | Self::UpdateComments
            | Self::Views => 7,
            Self::Finalize => 8,
        }
    }

    pub const fn name(&self) -> &'static str {
        match self {
            Self::Users => "Users",
            Self::Teams => "Teams",
            Self::Memberships => "Memberships",
            Self::States => "States",
            Self::Labels => "Labels",
            Self::ProjectStatuses => "ProjectStatuses",
            Self::ProjectLabels => "ProjectLabels",
            Self::Templates => "Templates",
            Self::Cycles => "Cycles",
            Self::Initiatives => "Initiatives",
            Self::Projects => "Projects",
            Self::Milestones => "Milestones",
            Self::Issues => "Issues",
            Self::IssueParents => "IssueParents",
            Self::IssueRelations => "IssueRelations",
            Self::Comments => "Comments",
            Self::IssueHistory => "IssueHistory",
            Self::IssueSubscriptions => "IssueSubscriptions",
            Self::Attachments => "Attachments",
            Self::EmbeddedFiles => "EmbeddedFiles",
            Self::Documents => "Documents",
            Self::ProjectUpdates => "ProjectUpdates",
            Self::InitiativeUpdates => "InitiativeUpdates",
            Self::UpdateComments => "UpdateComments",
            Self::Views => "Views",
            Self::Finalize => "Finalize",
        }
    }

    pub(crate) fn from_name(n: &str) -> Option<usize> {
        Self::ALL.iter().position(|s| s.name() == n)
    }
}

/// Unterbrechung eines Laufs.
#[derive(Debug)]
pub(crate) enum Halt {
    Pause,
    Cancel,
    Stop(u8),
    Err(AppError),
}

impl From<AppError> for Halt {
    fn from(e: AppError) -> Self {
        Halt::Err(e)
    }
}

impl From<SourceError> for Halt {
    fn from(e: SourceError) -> Self {
        Halt::Err(src_err(e))
    }
}

pub(crate) type HResult<T> = Result<T, Halt>;

/// Quellfehler als `AppError` (Text enthält nie Zugangsdaten).
pub fn src_err(e: SourceError) -> AppError {
    match e {
        SourceError::Auth => AppError::unauthenticated("The source rejected the credentials."),
        other => AppError::internal(other.to_string()),
    }
}

/// Zähler je Entitätstyp.
#[derive(Debug, Clone, Default)]
pub(crate) struct Counters {
    pub created: u64,
    pub updated: u64,
    pub unchanged: u64,
    pub skipped: u64,
    pub conflicts: u64,
    pub errors: u64,
}

impl Counters {
    fn to_json(&self) -> Value {
        json!({
            "created": self.created, "updated": self.updated, "unchanged": self.unchanged,
            "skipped": self.skipped, "conflicts": self.conflicts, "errors": self.errors,
        })
    }

    fn from_json(v: &Value) -> Self {
        let n = |k: &str| v[k].as_u64().unwrap_or(0);
        Self {
            created: n("created"),
            updated: n("updated"),
            unchanged: n("unchanged"),
            skipped: n("skipped"),
            conflicts: n("conflicts"),
            errors: n("errors"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Created,
    Updated,
    Unchanged,
    Conflict,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Upserted {
    pub id: Uuid,
    pub outcome: Outcome,
}

impl Upserted {
    /// Wurde die Zeile neu angelegt oder geändert (Nebentabellen neu schreiben)?
    pub fn wrote(&self) -> bool {
        matches!(self.outcome, Outcome::Created | Outcome::Updated)
    }
}

/// Metadaten eines Labels (für single-select-Gruppen).
#[derive(Debug, Clone)]
pub(crate) struct LabelMeta {
    pub parent: Option<String>,
    pub is_group: bool,
}

/// Die offene Transaktion. Der `Mutex` macht die Sitzung `Sync` (Futures bleiben `Send`); es gibt
/// nie parallelen Zugriff, `get_mut` sperrt nicht.
#[derive(Default)]
pub(crate) struct TxSlot(std::sync::Mutex<Option<Box<dyn Tx>>>);

impl TxSlot {
    pub(crate) fn slot(&mut self) -> &mut Option<Box<dyn Tx>> {
        self.0
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Zustand eines laufenden Importlaufs.
pub(crate) struct Session<'e, 'a> {
    pub eng: &'e ImportEngine<'a>,
    pub org: OrgId,
    pub run: ImportRun,
    pub opts: Options,
    pub dry: bool,
    pub now: DateTime<Utc>,
    pub tx: TxSlot,
    pub report: Vec<ReportEntry>,
    pub report_total: usize,
    pub stats: BTreeMap<String, Counters>,
    pub cache: HashMap<(&'static str, String), Uuid>,
    pub rewrite: RewriteCtx,
    /// Quell-Team-IDs der ausgewählten Teams.
    pub selected: HashSet<String>,
    /// Quell-Team-ID -> Quell-Key.
    pub team_keys: HashMap<String, String>,
    pub labels: HashMap<String, LabelMeta>,
    /// Linear-Benutzername (klein) -> Quell-User-ID (für das Umschreiben von Erwähnungen).
    pub users_by_username: HashMap<String, String>,
    pub reported_tags: HashSet<String>,
    pub source_kind: &'static str,
    /// Quell-Team-ID -> bestehendes Lineal-Team (Import in ein vorhandenes Team).
    pub target_by_source: HashMap<String, Uuid>,
    /// Quell-User-ID -> Objekt (für das Anlegen bei Bedarf im Teilimport).
    pub user_nodes: HashMap<String, Value>,
    /// Quell-Label-ID -> Objekt (dito).
    pub label_nodes: HashMap<String, Value>,
    /// Zähler für den Plan/Bericht (`stats.plan`), z. B. `statesMapped`.
    pub plan: BTreeMap<String, u64>,
    /// Dry-Run: bereits gezählte Download-URLs (je Datei nur einmal).
    pub dl_seen: HashSet<String>,
}

impl<'a> ImportEngine<'a> {
    /// Neue Engine.
    pub fn new(app: &'a App, source: &'a dyn ImportSource, sink: &'a dyn BlobSink) -> Self {
        Self {
            app,
            source,
            sink,
            config: EngineConfig::default(),
        }
    }

    #[must_use]
    pub fn with_config(mut self, config: EngineConfig) -> Self {
        self.config = config;
        self
    }

    /// Führt (oder setzt fort) einen Lauf im Status `running` aus.
    pub async fn run(&self, org: OrgId, run_id: Uuid) -> AppResult<RunOutcome> {
        let mut tx = self.app.store.begin(org).await?;
        let run = tx
            .imports()
            .get_import_run(run_id)
            .await?
            .ok_or_else(|| AppError::not_found("Import run not found."))?;
        drop(tx);
        if run.status != "running" {
            return Err(AppError::conflict(format!(
                "the run is `{}`, not running",
                run.status
            )));
        }
        let opts = Options::parse(&run.options);
        let dry = opts.dry_run;
        let stats = run.stats["entities"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), Counters::from_json(v)))
                    .collect()
            })
            .unwrap_or_default();
        let mut s = Session {
            eng: self,
            org,
            source_kind: if run.source == "linearCsv" {
                "linearCsv"
            } else {
                "linear"
            },
            run,
            opts,
            dry,
            now: self.app.clock.now(),
            tx: TxSlot::default(),
            report: Vec::new(),
            report_total: 0,
            stats,
            cache: HashMap::new(),
            rewrite: RewriteCtx::default(),
            selected: HashSet::new(),
            team_keys: HashMap::new(),
            labels: HashMap::new(),
            users_by_username: HashMap::new(),
            reported_tags: HashSet::new(),
            target_by_source: HashMap::new(),
            user_nodes: HashMap::new(),
            label_nodes: HashMap::new(),
            plan: BTreeMap::new(),
            dl_seen: HashSet::new(),
        };
        let result = {
            let beat = heartbeat(
                self.app,
                org,
                s.run.id,
                self.config.lease,
                self.config.team_lock,
                self.config.heartbeat,
                s.source_kind,
                dry,
            );
            tokio::select! {
                r = s.drive() => r,
                never = beat => match never {},
            }
        };
        s.finish(result).await
    }
}

/// Hält Lease und Team-Sperren eines Laufs aktuell, solange er läuft (auch während langer
/// Wartezeiten auf die Quelle). Jeder Schritt ist eine eigene kurze Transaktion mit genau einer
/// Tabelle (Import-Lauf bzw. Teams), damit sie nie mit der Einheit des Laufs verklemmt.
#[allow(clippy::too_many_arguments)]
async fn heartbeat(
    app: &App,
    org: OrgId,
    run_id: Uuid,
    lease: Duration,
    team_lock: ChronoDuration,
    every: Duration,
    source: &str,
    dry: bool,
) -> std::convert::Infallible {
    let lease = ChronoDuration::from_std(lease).unwrap_or(ChronoDuration::minutes(2));
    loop {
        tokio::time::sleep(every).await;
        if dry {
            continue;
        }
        let now = app.clock.now();
        let beat = async {
            let mut tx = app.store.begin(org).await?;
            tx.imports().set_import_lease(run_id, Some(now + lease)).await?;
            tx.commit().await?;
            let mut tx = app.store.begin(org).await?;
            let mut ids = Vec::new();
            let mut after: Option<String> = None;
            loop {
                let page = tx
                    .imports()
                    .list_mappings(source, "Team", after.as_deref(), 500)
                    .await?;
                if page.is_empty() {
                    break;
                }
                after = page.last().map(|m| m.source_id.clone());
                ids.extend(page.iter().map(|m| m.target_id));
            }
            if !ids.is_empty() {
                tx.imports()
                    .set_team_import_lock(&ids, Some(now + team_lock))
                    .await?;
            }
            tx.commit().await
        };
        if let Err(e) = beat.await {
            tracing::warn!(run = %run_id, error = %e.message, "import heartbeat failed");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Session: Grundfunktionen
// ---------------------------------------------------------------------------------------------

impl Session<'_, '_> {
    pub(crate) fn tx(&mut self) -> AppResult<&mut dyn Tx> {
        match self.tx.slot().as_mut() {
            Some(t) => Ok(&mut **t),
            None => Err(AppError::internal("no open import transaction")),
        }
    }

    pub(crate) fn source(&self) -> &str {
        self.source_kind
    }

    pub(crate) fn counters(&mut self, entity: &str) -> &mut Counters {
        self.stats.entry(entity.to_owned()).or_default()
    }

    /// Bericht: Eintrag vormerken (Obergrenze je Lauf).
    pub(crate) fn note(
        &mut self,
        severity: &str,
        ty: &str,
        id: Option<&str>,
        message: impl Into<String>,
    ) {
        self.report_total += 1;
        if self.report_total > self.eng.config.max_report_entries {
            return;
        }
        let mut message: String = message.into();
        if message.chars().count() > 500 {
            message = message.chars().take(500).collect();
        }
        self.report.push(ReportEntry {
            severity: severity.to_owned(),
            source_type: Some(ty.to_owned()),
            source_id: id.map(|s| s.chars().take(200).collect()),
            message,
            created_at: None,
        });
    }

    /// Pause/Abbruch zwischen zwei Einheiten prüfen (frische Transaktion, sieht auch Fremdänderungen).
    async fn check_control(&mut self) -> HResult<()> {
        self.poll_control().await
    }

    /// Wie `check_control`, aber ohne `&mut` (für Prüfungen während eines Quellzugriffs).
    async fn poll_control(&self) -> HResult<()> {
        let mut tx = self.eng.app.store.begin(self.org).await?;
        let status = tx
            .imports()
            .get_import_run(self.run.id)
            .await?
            .map(|r| r.status)
            .unwrap_or_default();
        drop(tx);
        match status.as_str() {
            "paused" => Err(Halt::Pause),
            "canceled" | "failed" | "completed" | "planned" if !self.dry => Err(Halt::Cancel),
            "canceled" => Err(Halt::Cancel),
            _ => Ok(()),
        }
    }

    /// Öffnet (falls nötig) die Transaktion der aktuellen Einheit.
    pub(crate) async fn begin_unit(&mut self) -> HResult<()> {
        self.check_control().await?;
        if self.tx.slot().is_none() {
            let tx = self.eng.app.store.begin(self.org).await?;
            *self.tx.slot() = Some(tx);
        }
        Ok(())
    }

    pub(crate) fn stats_json(&self) -> Value {
        let entities: Map<String, Value> = self
            .stats
            .iter()
            .map(|(k, v)| (k.clone(), v.to_json()))
            .collect();
        let mut base = self.run.stats.as_object().cloned().unwrap_or_default();
        base.insert("entities".into(), Value::Object(entities));
        base.insert("plan".into(), json!(self.plan));
        Value::Object(base)
    }

    /// Schließt die Einheit: Bericht und Checkpoint in derselben Transaktion, dann Commit.
    /// Im Dry-Run bleibt die Transaktion offen (wird am Ende zurückgerollt).
    pub(crate) async fn end_unit(&mut self, checkpoint: Option<Value>) -> HResult<()> {
        if self.dry {
            return Ok(());
        }
        let now = self.eng.app.clock.now();
        let report = std::mem::take(&mut self.report);
        let stats = self.stats_json();
        let lease = now
            + ChronoDuration::from_std(self.eng.config.lease).unwrap_or(ChronoDuration::minutes(2));
        let run_id = self.run.id;
        let tx = self.tx()?;
        if !report.is_empty() {
            tx.imports().add_report_entries(run_id, &report).await?;
        }
        tx.imports()
            .update_import_run(
                run_id,
                &ImportRunUpdate {
                    checkpoint,
                    stats: Some(stats),
                    now: Some(now),
                    ..Default::default()
                },
            )
            .await?;
        tx.imports().set_import_lease(run_id, Some(lease)).await?;
        if let Some(tx) = self.tx.slot().take() {
            tx.commit().await?;
        }
        Ok(())
    }

    /// Ein Event mit `origin = import` in die Outbox schreiben.
    pub(crate) async fn event(&mut self, ev: DomainEvent) -> AppResult<()> {
        self.tx()?
            .outbox()
            .push(NewEvent::new(ev, EventOrigin::Import))
            .await?;
        Ok(())
    }

    /// Mapping inkl. Checksum.
    pub(crate) async fn mapping(
        &mut self,
        ty: &'static str,
        src: &str,
    ) -> AppResult<Option<MappingRow>> {
        let source = self.source().to_owned();
        let rows = self
            .tx()?
            .imports()
            .get_mappings(&source, ty, &[src.to_owned()])
            .await?;
        Ok(rows.into_iter().next())
    }

    /// Planzähler um `n` erhöhen (`stats.plan`).
    pub(crate) fn plan_add_n(&mut self, key: &str, n: u64) {
        *self.plan.entry(key.to_owned()).or_default() += n;
    }

    /// Planzähler erhöhen (`stats.plan`).
    pub(crate) fn plan_add(&mut self, key: &str) {
        *self.plan.entry(key.to_owned()).or_default() += 1;
    }

    /// Ziel-ID zu einer Quell-ID (Cache, sonst Datenbank). Im Teilimport werden Personen und
    /// Issue-Labels beim ersten Bezug angelegt bzw. zugeordnet.
    pub(crate) async fn target(&mut self, ty: &'static str, src: &str) -> AppResult<Option<Uuid>> {
        if let Some(id) = self.cache.get(&(ty, src.to_owned())) {
            return Ok(Some(*id));
        }
        let m = self.mapping(ty, src).await?;
        if let Some(m) = &m {
            self.cache.insert((ty, src.to_owned()), m.target_id);
            return Ok(Some(m.target_id));
        }
        if self.opts.scoped() {
            match ty {
                "User" => {
                    if let Some(node) = self.user_nodes.get(src).cloned() {
                        let r = self.user(N(&node)).await;
                        match r {
                            Ok(()) => {}
                            Err(e) if matches!(e.kind, ErrorKind::InvalidRequest) => {
                                self.counters("User").errors += 1;
                                self.note("error", "User", Some(src), e.message);
                            }
                            Err(e) => return Err(e),
                        }
                        return Ok(self.cache.get(&("User", src.to_owned())).copied());
                    }
                }
                "IssueLabel" if !self.opts.target_teams.is_empty() => {
                    if let Some(node) = self.label_nodes.get(src).cloned() {
                        Box::pin(self.label_on_demand(&node)).await?;
                        return Ok(self.cache.get(&("IssueLabel", src.to_owned())).copied());
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    pub(crate) async fn target_opt(
        &mut self,
        ty: &'static str,
        src: Option<&str>,
    ) -> AppResult<Option<Uuid>> {
        match src {
            Some(s) => self.target(ty, s).await,
            None => Ok(None),
        }
    }

    pub(crate) fn new_id(&self) -> Uuid {
        self.eng.app.ids.new_uuid()
    }

    pub(crate) async fn put_mapping(
        &mut self,
        ty: &'static str,
        src: &str,
        target: Uuid,
        node: N<'_>,
        checksum: &str,
    ) -> AppResult<()> {
        let m = NewMapping {
            import_run_id: self.run.id,
            source: self.source().to_owned(),
            source_type: ty.to_owned(),
            source_id: src.to_owned(),
            target_id: target,
            source_updated_at: node.ts("updatedAt"),
            checksum: Some(checksum.to_owned()),
            imported_at: self.now,
        };
        self.tx()?.imports().upsert_mapping(&m).await?;
        self.cache.insert((ty, src.to_owned()), target);
        Ok(())
    }

    /// Legt ein Objekt an, aktualisiert es (Delta) oder lässt es unverändert.
    ///
    /// * kein Mapping: `INSERT` (+ Mapping),
    /// * gleiche `checksum`: nichts tun,
    /// * andere `checksum`: nur aktualisieren, wenn die Lineal-Zeile seit dem Import nicht
    ///   geändert wurde (`updated_at <= imported_at`), sonst Konflikt (Lineal-Stand bleibt, Bericht),
    /// * `insert_only` (append-only Tabellen): ein vorhandenes Mapping bedeutet immer „unverändert“.
    pub(crate) async fn upsert(
        &mut self,
        ty: &'static str,
        node: N<'_>,
        table: ImportTable,
        mut row: Map<String, Value>,
        insert_only: bool,
    ) -> AppResult<Upserted> {
        let src = node
            .id()
            .ok_or_else(|| AppError::invalid_request("object without id"))?
            .to_owned();
        let checksum = checksum_of(node.0);
        match self.mapping(ty, &src).await? {
            None => {
                let id = self.new_id();
                row.insert("id".into(), json!(id));
                self.tx()?
                    .imports()
                    .insert_row(table, &Value::Object(row))
                    .await?;
                self.put_mapping(ty, &src, id, node, &checksum).await?;
                self.counters(ty).created += 1;
                Ok(Upserted {
                    id,
                    outcome: Outcome::Created,
                })
            }
            Some(m) => {
                self.cache.insert((ty, src.clone()), m.target_id);
                if insert_only || m.checksum.as_deref() == Some(checksum.as_str()) {
                    self.counters(ty).unchanged += 1;
                    return Ok(Upserted {
                        id: m.target_id,
                        outcome: Outcome::Unchanged,
                    });
                }
                let current = self
                    .tx()?
                    .imports()
                    .row_updated_at(table, m.target_id)
                    .await?;
                if current.is_some_and(|u| u > m.imported_at) {
                    self.counters(ty).conflicts += 1;
                    self.note(
                        "warning",
                        ty,
                        Some(&src),
                        "changed in Lineal since the import; the Lineal state was kept",
                    );
                    return Ok(Upserted {
                        id: m.target_id,
                        outcome: Outcome::Conflict,
                    });
                }
                // `created_at` bleibt wie importiert.
                row.remove("created_at");
                let existed = self
                    .tx()?
                    .imports()
                    .update_row(table, m.target_id, &Value::Object(row))
                    .await?;
                if !existed {
                    self.counters(ty).errors += 1;
                    self.note(
                        "error",
                        ty,
                        Some(&src),
                        "the mapped object no longer exists in Lineal",
                    );
                    return Ok(Upserted {
                        id: m.target_id,
                        outcome: Outcome::Conflict,
                    });
                }
                self.put_mapping(ty, &src, m.target_id, node, &checksum)
                    .await?;
                self.counters(ty).updated += 1;
                Ok(Upserted {
                    id: m.target_id,
                    outcome: Outcome::Updated,
                })
            }
        }
    }

    /// Führt einen Quellzugriff aus und prüft dabei regelmäßig Pause/Abbruch, damit sie auch
    /// während langer Rate-Limit-Wartezeiten zeitnah wirken. Der Zugriff selbst wird dabei nie
    /// mitten in einer Datenbankoperation abgebrochen (die Prüfung läuft zwischen den Polls).
    pub(crate) async fn guarded<T>(
        &self,
        fut: impl std::future::Future<Output = Result<T, SourceError>>,
    ) -> HResult<T> {
        tokio::pin!(fut);
        loop {
            tokio::select! {
                r = &mut fut => return r.map_err(Halt::from),
                () = tokio::time::sleep(self.eng.config.control_poll) => {}
            }
            self.poll_control().await?;
        }
    }

    /// Eine Seite lesen (außerhalb jeder Transaktion; Dry-Run hält seine eine Transaktion offen).
    pub(crate) async fn fetch(
        &self,
        entity: SourceEntity,
        cursor: Option<String>,
    ) -> HResult<SourcePage> {
        self.guarded(self.eng.source.page(&PageQuery {
            entity,
            cursor,
            updated_after: self.opts.updated_after,
            limit: self.eng.config.page_size.clamp(1, 100),
            scope: self.scope(),
        }))
        .await
    }

    /// Einschränkung der Abfragen auf die gewählten Teams/Projekte (leer = alles).
    fn scope(&self) -> crate::port::Scope {
        let mut teams: Vec<String> = if self.opts.teams.is_empty() {
            Vec::new()
        } else {
            self.selected.iter().cloned().collect()
        };
        teams.sort();
        let mut projects: Vec<String> = self.opts.projects.iter().cloned().collect();
        projects.sort();
        crate::port::Scope { teams, projects }
    }

    /// Alle Objekte einer (kleinen) Entität.
    pub(crate) async fn fetch_all(&self, entity: SourceEntity) -> HResult<Vec<Value>> {
        self.guarded(
            self.eng
                .source
                .collect(entity, self.opts.updated_after, 1000),
        )
        .await
    }

    /// Alle Objekte einer Entität ohne Delta-Filter (für Referenzen, z. B. Teams/Labels).
    pub(crate) async fn fetch_all_unfiltered(&self, entity: SourceEntity) -> HResult<Vec<Value>> {
        self.guarded(self.eng.source.collect(entity, None, 1000))
            .await
    }

    /// Fehler eines einzelnen Objekts melden, ohne den Lauf abzubrechen. Datenbank-/Systemfehler
    /// (alles außer `conflict`/`invalid_request`) brechen den Lauf ab.
    pub(crate) fn item_result(
        &mut self,
        ty: &'static str,
        id: Option<&str>,
        r: AppResult<()>,
    ) -> HResult<()> {
        match r {
            Ok(()) => Ok(()),
            Err(e) if matches!(e.kind, ErrorKind::Conflict | ErrorKind::InvalidRequest) => {
                self.counters(ty).errors += 1;
                self.note("error", ty, id, e.message);
                Ok(())
            }
            Err(e) => Err(Halt::Err(e)),
        }
    }

    /// Rewrite für Markdown inkl. Bericht über unbekannte Elemente.
    pub(crate) fn md(
        &mut self,
        ty: &str,
        id: &str,
        text: Option<&str>,
        max: usize,
    ) -> Option<String> {
        let text = text?;
        // Externe Bild-URLs werden nie abgerufen (kein Server-Request aus importiertem Inhalt).
        let external = text
            .match_indices("](http")
            .filter(|(i, _)| text[..*i].rfind("![").is_some())
            .filter(|(i, _)| !text[*i..].starts_with("](https://uploads.linear.app/"))
            .count();
        if external > 0 {
            self.note(
                "info",
                ty,
                Some(id),
                format!("{external} external image/link URL(s) are kept as links and are never fetched by the import"),
            );
        }
        let out = self.rewrite.rewrite(text, max);
        for t in &out.unknown_tags {
            if self.reported_tags.insert(t.clone()) {
                self.note(
                    "info",
                    ty,
                    Some(id),
                    format!("unknown markup element <{t}> was removed"),
                );
            }
        }
        if out.truncated {
            self.note(
                "warning",
                ty,
                Some(id),
                "text exceeded the length limit and was truncated",
            );
        }
        Some(out.text)
    }

    pub(crate) async fn lock_teams(&mut self) -> AppResult<()> {
        if self.dry {
            return Ok(());
        }
        let source = self.source().to_owned();
        let until = self.eng.app.clock.now() + self.eng.config.team_lock;
        let mut ids = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let page = self
                .tx()?
                .imports()
                .list_mappings(&source, "Team", after.as_deref(), 500)
                .await?;
            if page.is_empty() {
                break;
            }
            after = page.last().map(|m| m.source_id.clone());
            ids.extend(page.iter().map(|m| m.target_id));
        }
        self.tx()?
            .imports()
            .set_team_import_lock(&ids, Some(until))
            .await?;
        Ok(())
    }

    // -------------------------------------------------------------------------------------
    // Ablauf
    // -------------------------------------------------------------------------------------

    /// Linear-Benutzername -> Lineal-`display_name` für bereits importierte User.
    pub(crate) async fn refresh_user_names(&mut self) -> AppResult<()> {
        let source = self.source().to_owned();
        let mut tx = self.eng.app.store.begin(self.org).await?;
        for (uname, src) in &self.users_by_username {
            let rows = tx
                .imports()
                .get_mappings(&source, "User", std::slice::from_ref(src))
                .await?;
            if let Some(m) = rows.first()
                && let Some(u) = tx
                    .users()
                    .get(lineal_domain::ids::UserId(m.target_id))
                    .await?
            {
                self.rewrite.users.insert(uname.clone(), u.display_name);
            }
        }
        Ok(())
    }

    /// Vorbereitung: Workspace, Benutzer-/Label-Metadaten, Umschreibe-Kontext, Team-Auswahl.
    pub(crate) async fn init(&mut self) -> HResult<()> {
        // Ziel-Teams und Rechte des Starters bei jeder Ausführung neu prüfen (vor jedem Schreiben).
        self.eng
            .app
            .import_verify_targets(self.org, &self.run)
            .await?;
        let ws = self.eng.source.workspace().await?;
        if self.run.checkpoint["step"].is_null() {
            for n in self.eng.source.notices() {
                self.note("info", "Source", None, n);
            }
        }
        let mut users = HashMap::new();
        for u in self.fetch_all_unfiltered(SourceEntity::Users).await? {
            let n = N(&u);
            if let (Some(name), Some(id)) = (n.text("displayName"), n.id()) {
                users.insert(name.to_lowercase(), id.to_owned());
            }
            if let Some(id) = n.id() {
                self.user_nodes.insert(id.to_owned(), u.clone());
            }
        }
        // Linear-Benutzername -> Lineal-display_name wird beim Schreiben der User ergänzt
        // (siehe `people::users`); nach einem Neustart aus dem Mapping rekonstruiert.
        let mut rewrite = RewriteCtx {
            source_ws: ws.url_key.clone(),
            target_base: self
                .eng
                .app
                .config
                .base_url
                .trim_end_matches('/')
                .to_owned(),
            asset_prefix: self.eng.app.asset_prefix(),
            ..Default::default()
        };
        let mut tx = self.eng.app.store.begin(self.org).await?;
        rewrite.target_ws = tx
            .orgs()
            .get()
            .await?
            .map(|o| o.url_key)
            .unwrap_or_default();
        drop(tx);
        self.rewrite = rewrite;
        self.users_by_username = users;
        self.refresh_user_names().await?;
        for t in self.fetch_all_unfiltered(SourceEntity::Teams).await? {
            let n = N(&t);
            let (Some(id), Some(key)) = (n.id(), n.text("key")) else {
                continue;
            };
            self.team_keys.insert(id.to_owned(), key.to_owned());
            if self.opts.teams.is_empty() || self.opts.teams.iter().any(|k| k == key) {
                self.selected.insert(id.to_owned());
            }
            let mut target = self.opts.target_key(key);
            if let Some(team) = self.opts.target_teams.get(key).copied() {
                if self.selected.contains(id) {
                    self.target_by_source.insert(id.to_owned(), team);
                }
                let mut tx = self.eng.app.store.begin(self.org).await?;
                let found = tx.teams().get(lineal_domain::ids::TeamId(team)).await?;
                drop(tx);
                target = found
                    .ok_or_else(|| AppError::not_found("Could not find referenced Team."))?
                    .key;
            }
            if target != key {
                self.rewrite.key_map.insert(key.to_owned(), target);
            }
        }
        for l in self.fetch_all_unfiltered(SourceEntity::IssueLabels).await? {
            let n = N(&l);
            if let Some(id) = n.id() {
                self.label_nodes.insert(id.to_owned(), l.clone());
                self.labels.insert(
                    id.to_owned(),
                    LabelMeta {
                        parent: n.ref_id("parent").map(str::to_owned),
                        is_group: n.bool("isGroup").unwrap_or(false),
                    },
                );
            }
        }
        Ok(())
    }
}
