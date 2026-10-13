//! Filter-AST für Issues (und Projekte/Initiativen) nach docs/05 §1: serde-Abbild des
//! normativen JSON Schemas (§1.2), Validierung (§1.8) und Normalisierung (§1.8, §1.9).
//!
//! Die Auswertung (SQL-Übersetzung, `isMe`, relative Daten) gehört in die Adapter; hier liegen
//! nur Struktur, Wertebereiche und Grenzen:
//!
//! * Unbekannte Felder (auch die in §1.2 ausgeschlossenen Originalfelder) → Fehler,
//! * jeder Comparator braucht mindestens einen Operator,
//! * `null` nur dort, wo das Schema ihn erlaubt (nicht auf oberster Ebene, nicht bei `title`),
//! * Enum-Werte gegen die Enums aus docs/00 §4, Priorität nur 0–4,
//! * Datumswerte als ISO-Datum/-Zeit oder ISO-8601-Dauer,
//! * Grenzen (Festlegung §1.1): Tiefe ≤ 10, ≤ 200 Bedingungen, ≤ 8 KB serialisiert.
//!
//! Fehler tragen einen JSON-Pointer (`details.path`, z. B. `/and/1/labels/some/id`).

use crate::enums::{Health, InitiativeStatus, ProjectStatusType, StateType, TeamVisibility};
use chrono::{DateTime, Months, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

/// Maximale Verschachtelungstiefe von Filterobjekten (oberste Ebene = 1).
pub const MAX_DEPTH: usize = 10;
/// Maximale Zahl von Feldbedingungen.
pub const MAX_CONDITIONS: usize = 200;
/// Maximale Größe des serialisierten Filters in Bytes.
pub const MAX_BYTES: usize = 8 * 1024;
/// Maximale Länge von `in`/`nin` bei ID-Comparators.
pub const MAX_ID_LIST: usize = 250;

/// Validierungsfehler (`invalid_request` mit `details.path`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid filter at `{path}`: {reason}")]
pub struct FilterError {
    /// JSON-Pointer auf die fehlerhafte Stelle (leer = Wurzel).
    pub path: String,
    /// Menschenlesbarer Grund.
    pub reason: String,
}

// ---------------------------------------------------------------------------------------------
// Datumswerte
// ---------------------------------------------------------------------------------------------

/// ISO-8601-Dauer mit Vorzeichen (`-P2W`, `PT12H`, `P0D`), docs/05 §1.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IsoDuration {
    /// `true` = Vergangenheit.
    pub negative: bool,
    /// Jahre.
    pub years: u32,
    /// Monate.
    pub months: u32,
    /// Wochen.
    pub weeks: u32,
    /// Tage.
    pub days: u32,
    /// Stunden.
    pub hours: u32,
    /// Minuten.
    pub minutes: u32,
    /// Sekunden.
    pub seconds: u32,
}

/// Liest `<Zahl><Bezeichner>`-Folgen in vorgegebener Reihenfolge.
fn parse_units(s: &str, order: &[char]) -> Option<Vec<u32>> {
    let mut vals = vec![0u32; order.len()];
    let mut next_min = 0;
    let mut rest = s;
    while !rest.is_empty() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let n: u32 = rest[..digits].parse().ok()?;
        let unit = rest[digits..].chars().next()?;
        let idx = order.iter().position(|c| *c == unit)?;
        if idx < next_min {
            return None; // falsche Reihenfolge oder doppelt
        }
        vals[idx] = n;
        next_min = idx + 1;
        rest = &rest[digits + unit.len_utf8()..];
    }
    Some(vals)
}

impl IsoDuration {
    /// Parst gemäß Muster `^-?P(?!$)(\d+Y)?(\d+M)?(\d+W)?(\d+D)?(T(?=\d)(\d+H)?(\d+M)?(\d+S)?)?$`.
    pub fn parse(s: &str) -> Option<Self> {
        let (negative, rest) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let rest = rest.strip_prefix('P')?;
        if rest.is_empty() {
            return None;
        }
        let (date, time) = match rest.split_once('T') {
            Some((d, t)) if !t.is_empty() => (d, Some(t)),
            Some(_) => return None,
            None => (rest, None),
        };
        let dv = parse_units(date, &['Y', 'M', 'W', 'D'])?;
        let tv = match time {
            Some(t) => parse_units(t, &['H', 'M', 'S'])?,
            None => vec![0; 3],
        };
        Some(Self {
            negative,
            years: dv[0],
            months: dv[1],
            weeks: dv[2],
            days: dv[3],
            hours: tv[0],
            minutes: tv[1],
            seconds: tv[2],
        })
    }

    fn total_months(&self) -> u32 {
        self.years.saturating_mul(12).saturating_add(self.months)
    }

    fn total_days(&self) -> i64 {
        i64::from(self.weeks) * 7 + i64::from(self.days)
    }

    /// `date + Dauer` in Kalendertagen; Monate/Jahre kalendarisch (`2026-01-31 + P1M = 2026-02-28`).
    pub fn add_to_date(&self, date: NaiveDate) -> Option<NaiveDate> {
        let months = Months::new(self.total_months());
        let d = if self.negative {
            date.checked_sub_months(months)?
                .checked_sub_signed(chrono::Duration::try_days(self.total_days())?)?
        } else {
            date.checked_add_months(months)?
                .checked_add_signed(chrono::Duration::try_days(self.total_days())?)?
        };
        Some(d)
    }

    /// `instant + Dauer` (Monate/Jahre kalendarisch in UTC, Rest als feste Dauer).
    pub fn add_to_datetime(&self, instant: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let months = Months::new(self.total_months());
        let secs =
            i64::from(self.hours) * 3600 + i64::from(self.minutes) * 60 + i64::from(self.seconds);
        let delta = chrono::Duration::try_days(self.total_days())?
            .checked_add(&chrono::Duration::try_seconds(secs)?)?;
        if self.negative {
            instant
                .checked_sub_months(months)?
                .checked_sub_signed(delta)
        } else {
            instant
                .checked_add_months(months)?
                .checked_add_signed(delta)
        }
    }
}

/// Aufgelöster Wert von [`DateTimeOrDuration`] bzw. [`TimelessDateOrDuration`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateValue {
    /// Absoluter Zeitpunkt (`2026-10-01T00:00:00.000Z`).
    DateTime(DateTime<Utc>),
    /// Kalenderdatum (`2026-10-01`).
    Date(NaiveDate),
    /// Relative Dauer.
    Duration(IsoDuration),
}

fn parse_date_value(s: &str, allow_datetime: bool) -> Result<DateValue, String> {
    if allow_datetime {
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Ok(DateValue::DateTime(dt.with_timezone(&Utc)));
        }
    }
    if s.len() == 10 {
        if let Ok(d) = crate::rules::date::parse_iso_date("date", s) {
            return Ok(DateValue::Date(d));
        }
    }
    if let Some(d) = IsoDuration::parse(s) {
        return Ok(DateValue::Duration(d));
    }
    Err(format!(
        "`{s}` is not a valid {} or ISO-8601 duration",
        if allow_datetime {
            "date-time, date"
        } else {
            "date"
        }
    ))
}

macro_rules! date_value_type {
    ($(#[$m:meta])* $name:ident, $allow_dt:literal) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validiert und erzeugt den Wert.
            pub fn parse(s: &str) -> Result<Self, String> {
                parse_date_value(s, $allow_dt).map(|_| Self(s.to_owned()))
            }
            /// Ursprüngliche Schreibweise.
            pub fn as_str(&self) -> &str {
                &self.0
            }
            /// Aufgelöste Form.
            pub fn value(&self) -> DateValue {
                // Beim Erzeugen validiert, daher nie Err.
                parse_date_value(&self.0, $allow_dt).unwrap_or(DateValue::Duration(IsoDuration::default()))
            }
        }
        impl TryFrom<String> for $name {
            type Error = String;
            fn try_from(s: String) -> Result<Self, String> {
                parse_date_value(&s, $allow_dt).map(|_| Self(s))
            }
        }
        impl From<$name> for String {
            fn from(v: $name) -> String {
                v.0
            }
        }
    };
}

date_value_type!(
    /// `DateTimeOrDuration`: ISO-Zeitpunkt, ISO-Datum oder Dauer.
    DateTimeOrDuration,
    true
);
date_value_type!(
    /// `TimelessDateOrDuration`: ISO-Datum oder Dauer.
    TimelessDateOrDuration,
    false
);

// ---------------------------------------------------------------------------------------------
// Struktur-Makro: alle Felder optional, camelCase, unbekannte Felder verboten
// ---------------------------------------------------------------------------------------------

macro_rules! node {
    ($(#[$m:meta])* $name:ident { $( $(#[$fm:meta])* $f:ident : $t:ty ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $name {
            $( $(#[$fm])* #[serde(default, skip_serializing_if = "Option::is_none")] pub $f: Option<$t>, )*
        }
        impl $name {
            /// Kein Feld gesetzt?
            pub fn is_empty(&self) -> bool {
                true $(&& self.$f.is_none())*
            }
        }
    };
}

node!(
    /// `IDComparator`.
    IdComparator {
        eq: String,
        neq: String,
        #[serde(rename = "in")]
        in_: Vec<String>,
        nin: Vec<String>,
    }
);

node!(
    /// `BooleanComparator`.
    BooleanComparator { eq: bool, neq: bool }
);

node!(
    /// `NumberComparator` / `NullableNumberComparator` (`null` nur, wo nullable).
    NumberComparator {
        eq: Number,
        neq: Number,
        #[serde(rename = "in")]
        in_: Vec<Number>,
        nin: Vec<Number>,
        lt: Number,
        lte: Number,
        gt: Number,
        gte: Number,
        null: bool,
    }
);

node!(
    /// `EstimateComparator`: wie Number plus `and`/`or` über Teil-Comparators.
    EstimateComparator {
        eq: Number,
        neq: Number,
        #[serde(rename = "in")]
        in_: Vec<Number>,
        nin: Vec<Number>,
        lt: Number,
        lte: Number,
        gt: Number,
        gte: Number,
        null: bool,
        and: Vec<NumberComparator>,
        or: Vec<NumberComparator>,
    }
);

node!(
    /// `StringComparator` / `NullableStringComparator`.
    StringComparator {
        eq: String,
        neq: String,
        #[serde(rename = "in")]
        in_: Vec<String>,
        nin: Vec<String>,
        eq_ignore_case: String,
        neq_ignore_case: String,
        contains: String,
        not_contains: String,
        contains_ignore_case: String,
        not_contains_ignore_case: String,
        contains_ignore_case_and_accent: String,
        starts_with: String,
        starts_with_ignore_case: String,
        not_starts_with: String,
        ends_with: String,
        not_ends_with: String,
        null: bool,
    }
);

node!(
    /// `ContentComparator` (Volltext über Titel und Beschreibung).
    ContentComparator { contains: String, not_contains: String }
);

node!(
    /// `DateComparator` / `NullableDateComparator`.
    DateComparator {
        eq: DateTimeOrDuration,
        neq: DateTimeOrDuration,
        #[serde(rename = "in")]
        in_: Vec<DateTimeOrDuration>,
        nin: Vec<DateTimeOrDuration>,
        lt: DateTimeOrDuration,
        lte: DateTimeOrDuration,
        gt: DateTimeOrDuration,
        gte: DateTimeOrDuration,
        null: bool,
    }
);

node!(
    /// `NullableTimelessDateComparator` (`dueDate`, `targetDate`).
    TimelessDateComparator {
        eq: TimelessDateOrDuration,
        neq: TimelessDateOrDuration,
        #[serde(rename = "in")]
        in_: Vec<TimelessDateOrDuration>,
        nin: Vec<TimelessDateOrDuration>,
        lt: TimelessDateOrDuration,
        lte: TimelessDateOrDuration,
        gt: TimelessDateOrDuration,
        gte: TimelessDateOrDuration,
        null: bool,
    }
);

node!(
    /// `EnumComparator` (Werte werden feldabhängig gegen das Enum geprüft).
    EnumComparator {
        eq: String,
        neq: String,
        #[serde(rename = "in")]
        in_: Vec<String>,
        nin: Vec<String>,
        null: bool,
    }
);

/// To-Many-Filter (`CollectionOf`): `some`, `every`, `length`, `null`, `and`, `or`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectionFilter<T> {
    /// Mindestens ein Element erfüllt den Filter.
    #[serde(default = "none", skip_serializing_if = "Option::is_none")]
    pub some: Option<Box<T>>,
    /// Alle Elemente erfüllen den Filter (leere Menge: wahr).
    #[serde(default = "none", skip_serializing_if = "Option::is_none")]
    pub every: Option<Box<T>>,
    /// Vergleich der Elementanzahl.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<Box<NumberComparator>>,
    /// `true` = Menge leer, `false` = nicht leer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub null: Option<bool>,
    /// UND über Teil-Collection-Filter.
    #[serde(default = "none", skip_serializing_if = "Option::is_none")]
    pub and: Option<Vec<CollectionFilter<T>>>,
    /// ODER über Teil-Collection-Filter.
    #[serde(default = "none", skip_serializing_if = "Option::is_none")]
    pub or: Option<Vec<CollectionFilter<T>>>,
}

fn none<T>() -> Option<T> {
    None
}

impl<T> Default for CollectionFilter<T> {
    fn default() -> Self {
        Self {
            some: None,
            every: None,
            length: None,
            null: None,
            and: None,
            or: None,
        }
    }
}

node!(
    /// `UserFilter` / `NullableUserFilter`.
    UserFilter {
        id: Box<IdComparator>,
        name: Box<StringComparator>,
        display_name: Box<StringComparator>,
        email: Box<StringComparator>,
        is_me: Box<BooleanComparator>,
        active: Box<BooleanComparator>,
        admin: Box<BooleanComparator>,
        app: Box<BooleanComparator>,
        null: bool,
        and: Vec<UserFilter>,
        or: Vec<UserFilter>,
    }
);

node!(
    /// Wrapper für `ancestors: { some: TeamFilter }`.
    TeamAncestorsFilter { some: Box<TeamFilter> }
);

node!(
    /// `TeamFilter`.
    TeamFilter {
        id: Box<IdComparator>,
        key: Box<StringComparator>,
        name: Box<StringComparator>,
        visibility: Box<EnumComparator>,
        parent: Box<TeamFilter>,
        ancestors: Box<TeamAncestorsFilter>,
        and: Vec<TeamFilter>,
        or: Vec<TeamFilter>,
    }
);

node!(
    /// `WorkflowStateFilter`.
    WorkflowStateFilter {
        id: Box<IdComparator>,
        name: Box<StringComparator>,
        #[serde(rename = "type")]
        type_: Box<EnumComparator>,
        position: Box<NumberComparator>,
        team: Box<TeamFilter>,
        and: Vec<WorkflowStateFilter>,
        or: Vec<WorkflowStateFilter>,
    }
);

node!(
    /// `IssueLabelFilter`.
    IssueLabelFilter {
        id: Box<IdComparator>,
        name: Box<StringComparator>,
        is_group: Box<BooleanComparator>,
        parent: Box<IssueLabelFilter>,
        team: Box<TeamFilter>,
        and: Vec<IssueLabelFilter>,
        or: Vec<IssueLabelFilter>,
    }
);

node!(
    /// `NullableCycleFilter`.
    CycleFilter {
        id: Box<IdComparator>,
        number: Box<NumberComparator>,
        name: Box<StringComparator>,
        starts_at: Box<DateComparator>,
        ends_at: Box<DateComparator>,
        is_active: Box<BooleanComparator>,
        is_next: Box<BooleanComparator>,
        is_previous: Box<BooleanComparator>,
        is_past: Box<BooleanComparator>,
        is_future: Box<BooleanComparator>,
        is_in_cooldown: Box<BooleanComparator>,
        null: bool,
        and: Vec<CycleFilter>,
        or: Vec<CycleFilter>,
    }
);

node!(
    /// `NullableProjectMilestoneFilter`.
    MilestoneFilter {
        id: Box<IdComparator>,
        name: Box<StringComparator>,
        target_date: Box<TimelessDateComparator>,
        null: bool,
    }
);

node!(
    /// `lastAppliedTemplate`.
    TemplateFilter { id: Box<IdComparator>, null: bool }
);

node!(
    /// Element von `comments`.
    CommentFilter {
        body: Box<StringComparator>,
        user: Box<UserFilter>,
        created_at: Box<DateComparator>,
    }
);

node!(
    /// Element von `attachments`.
    AttachmentFilter {
        url: Box<StringComparator>,
        title: Box<StringComparator>,
        source_type: Box<StringComparator>,
    }
);

node!(
    /// Einfacher Element-Filter (`id`, `name`) für Labels/Initiativen/Projekte in Collections.
    IdNameFilter { id: Box<IdComparator>, name: Box<StringComparator> }
);

node!(
    /// `ProjectStatusFilter`.
    ProjectStatusFilter {
        id: Box<IdComparator>,
        name: Box<StringComparator>,
        #[serde(rename = "type")]
        type_: Box<EnumComparator>,
    }
);

node!(
    /// `ProjectFilter` (docs/05 §1.11) bzw. `NullableProjectFilter`.
    ProjectFilter {
        id: Box<IdComparator>,
        name: Box<StringComparator>,
        summary: Box<StringComparator>,
        status: Box<ProjectStatusFilter>,
        priority: Box<NumberComparator>,
        health: Box<EnumComparator>,
        lead: Box<UserFilter>,
        creator: Box<UserFilter>,
        members: Box<CollectionFilter<UserFilter>>,
        teams: Box<CollectionFilter<TeamFilter>>,
        lead_team: Box<TeamFilter>,
        labels: Box<CollectionFilter<IdNameFilter>>,
        initiatives: Box<CollectionFilter<IdNameFilter>>,
        project_milestones: Box<CollectionFilter<MilestoneFilter>>,
        start_date: Box<TimelessDateComparator>,
        target_date: Box<TimelessDateComparator>,
        created_at: Box<DateComparator>,
        updated_at: Box<DateComparator>,
        started_at: Box<DateComparator>,
        completed_at: Box<DateComparator>,
        canceled_at: Box<DateComparator>,
        health_updated_at: Box<DateComparator>,
        has_blocked_by_relations: Box<BooleanComparator>,
        has_blocking_relations: Box<BooleanComparator>,
        last_applied_template: Box<TemplateFilter>,
        null: bool,
        and: Vec<ProjectFilter>,
        or: Vec<ProjectFilter>,
    }
);

node!(
    /// `InitiativeFilter` (docs/05 §1.11).
    InitiativeFilter {
        id: Box<IdComparator>,
        name: Box<StringComparator>,
        status: Box<EnumComparator>,
        priority: Box<NumberComparator>,
        health: Box<EnumComparator>,
        owner: Box<UserFilter>,
        creator: Box<UserFilter>,
        lead_team: Box<TeamFilter>,
        labels: Box<CollectionFilter<IdNameFilter>>,
        target_date: Box<TimelessDateComparator>,
        created_at: Box<DateComparator>,
        updated_at: Box<DateComparator>,
        parent_initiatives: Box<CollectionFilter<IdNameFilter>>,
        sub_initiatives: Box<CollectionFilter<IdNameFilter>>,
        projects: Box<CollectionFilter<IdNameFilter>>,
        and: Vec<InitiativeFilter>,
        or: Vec<InitiativeFilter>,
    }
);

node!(
    /// `IssueFilter` (docs/05 §1.2), die Wurzel des Filterbaums.
    IssueFilter {
        id: Box<IdComparator>,
        number: Box<NumberComparator>,
        title: Box<StringComparator>,
        description: Box<StringComparator>,
        searchable_content: Box<ContentComparator>,
        priority: Box<NumberComparator>,
        estimate: Box<EstimateComparator>,
        created_at: Box<DateComparator>,
        updated_at: Box<DateComparator>,
        archived_at: Box<DateComparator>,
        started_at: Box<DateComparator>,
        completed_at: Box<DateComparator>,
        canceled_at: Box<DateComparator>,
        triaged_at: Box<DateComparator>,
        auto_closed_at: Box<DateComparator>,
        auto_archived_at: Box<DateComparator>,
        added_to_cycle_at: Box<DateComparator>,
        snoozed_until_at: Box<DateComparator>,
        sla_breaches_at: Box<DateComparator>,
        state_entered_at: Box<DateComparator>,
        due_date: Box<TimelessDateComparator>,
        sla_status: Box<EnumComparator>,
        added_to_cycle_period: Box<EnumComparator>,
        state: Box<WorkflowStateFilter>,
        team: Box<TeamFilter>,
        assignee: Box<UserFilter>,
        creator: Box<UserFilter>,
        delegate: Box<UserFilter>,
        snoozed_by: Box<UserFilter>,
        subscribers: Box<CollectionFilter<UserFilter>>,
        labels: Box<CollectionFilter<IssueLabelFilter>>,
        cycle: Box<CycleFilter>,
        project: Box<ProjectFilter>,
        project_milestone: Box<MilestoneFilter>,
        parent: Box<IssueFilter>,
        children: Box<CollectionFilter<IssueFilter>>,
        comments: Box<CollectionFilter<CommentFilter>>,
        attachments: Box<CollectionFilter<AttachmentFilter>>,
        has_blocked_by_relations: Box<BooleanComparator>,
        has_blocking_relations: Box<BooleanComparator>,
        has_duplicate_relations: Box<BooleanComparator>,
        has_related_relations: Box<BooleanComparator>,
        last_applied_template: Box<TemplateFilter>,
        null: bool,
        and: Vec<IssueFilter>,
        or: Vec<IssueFilter>,
    }
);

// ---------------------------------------------------------------------------------------------
// Validierung
// ---------------------------------------------------------------------------------------------

type R = Result<(), FilterError>;

struct V {
    path: Vec<String>,
    depth: usize,
    conditions: usize,
}

impl V {
    fn pointer(&self) -> String {
        self.path
            .iter()
            .map(|s| format!("/{}", s.replace('~', "~0").replace('/', "~1")))
            .collect()
    }

    fn fail(&self, reason: impl Into<String>) -> R {
        Err(FilterError {
            path: self.pointer(),
            reason: reason.into(),
        })
    }

    /// Die Wurzel zählt nicht als Verschachtelungsebene: 10 verschachtelte `and`/Relationen
    /// unter der Wurzel sind zulässig, 11 nicht (docs/05 §1.1 Regel 8; B8.30).
    fn enter(&mut self) -> R {
        self.depth += 1;
        if self.depth > MAX_DEPTH + 1 {
            return self.fail(format!("nesting deeper than {MAX_DEPTH} levels"));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn cond(&mut self) -> R {
        self.conditions += 1;
        if self.conditions > MAX_CONDITIONS {
            return self.fail(format!("more than {MAX_CONDITIONS} conditions"));
        }
        Ok(())
    }

    /// Prüft ein optionales Feld (Pfad, Bedingungszähler, `nullable`-Kontext).
    fn field<T: Check>(&mut self, name: &str, val: &Option<T>, nullable: bool) -> R {
        if let Some(x) = val {
            self.path.push(name.to_owned());
            self.cond()?;
            x.check(self, nullable)?;
            self.path.pop();
        }
        Ok(())
    }

    /// Wie [`V::field`] für `and`/`or`-Listen.
    fn list<T: Check>(&mut self, name: &str, val: &Option<Vec<T>>, nullable: bool) -> R {
        if let Some(items) = val {
            // `and`/`or` sind keine Bedingungen; gezählt werden nur die Felder darunter (B8.30).
            self.path.push(name.to_owned());
            for (i, x) in items.iter().enumerate() {
                self.path.push(i.to_string());
                x.check(self, nullable)?;
                self.path.pop();
            }
            self.path.pop();
        }
        Ok(())
    }

    fn enum_field(&mut self, name: &str, val: &Option<Box<EnumComparator>>, allowed: &[&str]) -> R {
        if let Some(c) = val {
            self.path.push(name.to_owned());
            self.cond()?;
            c.check(self, true)?;
            let all =
                c.eq.iter()
                    .chain(c.neq.iter())
                    .chain(c.in_.iter().flatten())
                    .chain(c.nin.iter().flatten());
            for value in all {
                if !allowed.contains(&value.as_str()) {
                    return self.fail(format!(
                        "unknown value `{value}`, allowed: {}",
                        allowed.join(", ")
                    ));
                }
            }
            self.path.pop();
        }
        Ok(())
    }

    fn priority_field(&mut self, name: &str, val: &Option<Box<NumberComparator>>) -> R {
        if let Some(c) = val {
            self.path.push(name.to_owned());
            self.cond()?;
            c.check(self, true)?;
            let all =
                c.eq.iter()
                    .chain(c.neq.iter())
                    .chain(c.in_.iter().flatten())
                    .chain(c.nin.iter().flatten());
            for n in all {
                let x = n.as_f64().unwrap_or(f64::NAN);
                if x.fract() != 0.0 || !(0.0..=4.0).contains(&x) {
                    return self.fail(format!("priority value {n} must be an integer 0..4"));
                }
            }
            self.path.pop();
        }
        Ok(())
    }
}

trait Check {
    fn check(&self, v: &mut V, nullable: bool) -> R;
}

impl<T: Check> Check for Box<T> {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        (**self).check(v, nullable)
    }
}

fn names<T: Copy>(all: &[T], f: impl Fn(&T) -> &'static str) -> Vec<&'static str> {
    all.iter().map(f).collect()
}

fn no_null(v: &V, null: &Option<bool>, nullable: bool) -> R {
    if null.is_some() && !nullable {
        return v.fail("`null` is not allowed here");
    }
    Ok(())
}

fn non_empty(v: &V, empty: bool) -> R {
    if empty {
        return v.fail("comparator needs at least one operator");
    }
    Ok(())
}

impl Check for IdComparator {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        non_empty(v, self.is_empty())?;
        for (name, list) in [("in", &self.in_), ("nin", &self.nin)] {
            if list.as_ref().is_some_and(|l| l.len() > MAX_ID_LIST) {
                v.path.push(name.into());
                return v.fail(format!("at most {MAX_ID_LIST} values"));
            }
        }
        Ok(())
    }
}

impl Check for BooleanComparator {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        non_empty(v, self.is_empty())
    }
}

impl Check for NumberComparator {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        non_empty(v, self.is_empty())?;
        no_null(v, &self.null, nullable)
    }
}

impl Check for EstimateComparator {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        non_empty(v, self.is_empty())?;
        no_null(v, &self.null, nullable)?;
        v.list("and", &self.and, true)?;
        v.list("or", &self.or, true)
    }
}

impl Check for StringComparator {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        non_empty(v, self.is_empty())?;
        no_null(v, &self.null, nullable)
    }
}

impl Check for ContentComparator {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        non_empty(v, self.is_empty())?;
        if self.contains.as_deref() == Some("") || self.not_contains.as_deref() == Some("") {
            return v.fail("search text must not be empty");
        }
        Ok(())
    }
}

impl Check for DateComparator {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        non_empty(v, self.is_empty())?;
        no_null(v, &self.null, nullable)
    }
}

impl Check for TimelessDateComparator {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        non_empty(v, self.is_empty())?;
        no_null(v, &self.null, nullable)
    }
}

impl Check for EnumComparator {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        non_empty(v, self.is_empty())?;
        no_null(v, &self.null, nullable)
    }
}

impl<T: Check> Check for CollectionFilter<T> {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        v.enter()?;
        v.field("some", &self.some, false)?;
        v.field("every", &self.every, false)?;
        v.field("length", &self.length, false)?;
        v.list("and", &self.and, false)?;
        v.list("or", &self.or, false)?;
        v.leave();
        Ok(())
    }
}

impl Check for UserFilter {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        v.enter()?;
        no_null(v, &self.null, nullable)?;
        v.field("id", &self.id, false)?;
        v.field("name", &self.name, false)?;
        v.field("displayName", &self.display_name, false)?;
        v.field("email", &self.email, false)?;
        v.field("isMe", &self.is_me, false)?;
        v.field("active", &self.active, false)?;
        v.field("admin", &self.admin, false)?;
        v.field("app", &self.app, false)?;
        v.list("and", &self.and, true)?;
        v.list("or", &self.or, true)?;
        v.leave();
        Ok(())
    }
}

impl Check for TeamAncestorsFilter {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        v.enter()?;
        v.field("some", &self.some, false)?;
        v.leave();
        Ok(())
    }
}

impl Check for TeamFilter {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        v.enter()?;
        v.field("id", &self.id, false)?;
        v.field("key", &self.key, false)?;
        v.field("name", &self.name, false)?;
        v.enum_field(
            "visibility",
            &self.visibility,
            &names(TeamVisibility::ALL, TeamVisibility::as_str),
        )?;
        v.field("parent", &self.parent, false)?;
        v.field("ancestors", &self.ancestors, false)?;
        v.list("and", &self.and, false)?;
        v.list("or", &self.or, false)?;
        v.leave();
        Ok(())
    }
}

impl Check for WorkflowStateFilter {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        v.enter()?;
        v.field("id", &self.id, false)?;
        v.field("name", &self.name, false)?;
        v.enum_field(
            "type",
            &self.type_,
            &names(StateType::ALL, StateType::as_str),
        )?;
        v.field("position", &self.position, false)?;
        v.field("team", &self.team, false)?;
        v.list("and", &self.and, false)?;
        v.list("or", &self.or, false)?;
        v.leave();
        Ok(())
    }
}

impl Check for IssueLabelFilter {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        v.enter()?;
        v.field("id", &self.id, false)?;
        v.field("name", &self.name, false)?;
        v.field("isGroup", &self.is_group, false)?;
        v.field("parent", &self.parent, false)?;
        v.field("team", &self.team, false)?;
        v.list("and", &self.and, false)?;
        v.list("or", &self.or, false)?;
        v.leave();
        Ok(())
    }
}

impl Check for CycleFilter {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        v.enter()?;
        no_null(v, &self.null, nullable)?;
        v.field("id", &self.id, false)?;
        v.field("number", &self.number, false)?;
        v.field("name", &self.name, false)?;
        v.field("startsAt", &self.starts_at, false)?;
        v.field("endsAt", &self.ends_at, false)?;
        v.field("isActive", &self.is_active, false)?;
        v.field("isNext", &self.is_next, false)?;
        v.field("isPrevious", &self.is_previous, false)?;
        v.field("isPast", &self.is_past, false)?;
        v.field("isFuture", &self.is_future, false)?;
        v.field("isInCooldown", &self.is_in_cooldown, false)?;
        v.list("and", &self.and, true)?;
        v.list("or", &self.or, true)?;
        v.leave();
        Ok(())
    }
}

impl Check for MilestoneFilter {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        v.enter()?;
        no_null(v, &self.null, nullable)?;
        v.field("id", &self.id, false)?;
        v.field("name", &self.name, false)?;
        v.field("targetDate", &self.target_date, true)?;
        v.leave();
        Ok(())
    }
}

impl Check for TemplateFilter {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        v.enter()?;
        no_null(v, &self.null, nullable)?;
        v.field("id", &self.id, false)?;
        v.leave();
        Ok(())
    }
}

impl Check for CommentFilter {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        v.enter()?;
        v.field("body", &self.body, false)?;
        v.field("user", &self.user, false)?;
        v.field("createdAt", &self.created_at, false)?;
        v.leave();
        Ok(())
    }
}

impl Check for AttachmentFilter {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        v.enter()?;
        v.field("url", &self.url, false)?;
        v.field("title", &self.title, false)?;
        v.field("sourceType", &self.source_type, false)?;
        v.leave();
        Ok(())
    }
}

impl Check for IdNameFilter {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        v.enter()?;
        v.field("id", &self.id, false)?;
        v.field("name", &self.name, false)?;
        v.leave();
        Ok(())
    }
}

impl Check for ProjectStatusFilter {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        v.enter()?;
        v.field("id", &self.id, false)?;
        v.field("name", &self.name, false)?;
        v.enum_field(
            "type",
            &self.type_,
            &names(ProjectStatusType::ALL, ProjectStatusType::as_str),
        )?;
        v.leave();
        Ok(())
    }
}

fn health_values() -> Vec<&'static str> {
    names(Health::ALL, Health::as_str)
}

impl Check for ProjectFilter {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        v.enter()?;
        no_null(v, &self.null, nullable)?;
        v.field("id", &self.id, false)?;
        v.field("name", &self.name, false)?;
        v.field("summary", &self.summary, false)?;
        v.field("status", &self.status, false)?;
        v.priority_field("priority", &self.priority)?;
        v.enum_field("health", &self.health, &health_values())?;
        v.field("lead", &self.lead, true)?;
        v.field("creator", &self.creator, true)?;
        v.field("members", &self.members, false)?;
        v.field("teams", &self.teams, false)?;
        v.field("leadTeam", &self.lead_team, false)?;
        v.field("labels", &self.labels, false)?;
        v.field("initiatives", &self.initiatives, false)?;
        v.field("projectMilestones", &self.project_milestones, false)?;
        v.field("startDate", &self.start_date, true)?;
        v.field("targetDate", &self.target_date, true)?;
        v.field("createdAt", &self.created_at, false)?;
        v.field("updatedAt", &self.updated_at, false)?;
        v.field("startedAt", &self.started_at, true)?;
        v.field("completedAt", &self.completed_at, true)?;
        v.field("canceledAt", &self.canceled_at, true)?;
        v.field("healthUpdatedAt", &self.health_updated_at, true)?;
        v.field(
            "hasBlockedByRelations",
            &self.has_blocked_by_relations,
            false,
        )?;
        v.field("hasBlockingRelations", &self.has_blocking_relations, false)?;
        v.field("lastAppliedTemplate", &self.last_applied_template, true)?;
        v.list("and", &self.and, false)?;
        v.list("or", &self.or, false)?;
        v.leave();
        Ok(())
    }
}

impl Check for InitiativeFilter {
    fn check(&self, v: &mut V, _nullable: bool) -> R {
        v.enter()?;
        v.field("id", &self.id, false)?;
        v.field("name", &self.name, false)?;
        v.enum_field(
            "status",
            &self.status,
            &names(InitiativeStatus::ALL, InitiativeStatus::as_str),
        )?;
        v.priority_field("priority", &self.priority)?;
        v.enum_field("health", &self.health, &health_values())?;
        v.field("owner", &self.owner, true)?;
        v.field("creator", &self.creator, true)?;
        v.field("leadTeam", &self.lead_team, false)?;
        v.field("labels", &self.labels, false)?;
        v.field("targetDate", &self.target_date, true)?;
        v.field("createdAt", &self.created_at, false)?;
        v.field("updatedAt", &self.updated_at, false)?;
        v.field("parentInitiatives", &self.parent_initiatives, false)?;
        v.field("subInitiatives", &self.sub_initiatives, false)?;
        v.field("projects", &self.projects, false)?;
        v.list("and", &self.and, false)?;
        v.list("or", &self.or, false)?;
        v.leave();
        Ok(())
    }
}

const SLA_STATUSES: [&str; 6] = [
    "breached",
    "highRisk",
    "mediumRisk",
    "lowRisk",
    "completed",
    "failed",
];
const CYCLE_PERIODS: [&str; 3] = ["before", "during", "after"];

impl Check for IssueFilter {
    fn check(&self, v: &mut V, nullable: bool) -> R {
        v.enter()?;
        no_null(v, &self.null, nullable)?;
        v.field("id", &self.id, false)?;
        v.field("number", &self.number, false)?;
        v.field("title", &self.title, false)?;
        v.field("description", &self.description, true)?;
        v.field("searchableContent", &self.searchable_content, false)?;
        v.priority_field("priority", &self.priority)?;
        v.field("estimate", &self.estimate, true)?;
        v.field("createdAt", &self.created_at, false)?;
        v.field("updatedAt", &self.updated_at, false)?;
        v.field("archivedAt", &self.archived_at, true)?;
        v.field("startedAt", &self.started_at, true)?;
        v.field("completedAt", &self.completed_at, true)?;
        v.field("canceledAt", &self.canceled_at, true)?;
        v.field("triagedAt", &self.triaged_at, true)?;
        v.field("autoClosedAt", &self.auto_closed_at, true)?;
        v.field("autoArchivedAt", &self.auto_archived_at, true)?;
        v.field("addedToCycleAt", &self.added_to_cycle_at, true)?;
        v.field("snoozedUntilAt", &self.snoozed_until_at, true)?;
        v.field("slaBreachesAt", &self.sla_breaches_at, true)?;
        v.field("stateEnteredAt", &self.state_entered_at, false)?;
        v.field("dueDate", &self.due_date, true)?;
        v.enum_field("slaStatus", &self.sla_status, &SLA_STATUSES)?;
        v.enum_field(
            "addedToCyclePeriod",
            &self.added_to_cycle_period,
            &CYCLE_PERIODS,
        )?;
        v.field("state", &self.state, false)?;
        v.field("team", &self.team, false)?;
        v.field("assignee", &self.assignee, true)?;
        v.field("creator", &self.creator, true)?;
        v.field("delegate", &self.delegate, true)?;
        v.field("snoozedBy", &self.snoozed_by, true)?;
        v.field("subscribers", &self.subscribers, false)?;
        v.field("labels", &self.labels, false)?;
        v.field("cycle", &self.cycle, true)?;
        v.field("project", &self.project, true)?;
        v.field("projectMilestone", &self.project_milestone, true)?;
        v.field("parent", &self.parent, true)?;
        v.field("children", &self.children, false)?;
        v.field("comments", &self.comments, false)?;
        v.field("attachments", &self.attachments, false)?;
        v.field(
            "hasBlockedByRelations",
            &self.has_blocked_by_relations,
            false,
        )?;
        v.field("hasBlockingRelations", &self.has_blocking_relations, false)?;
        v.field(
            "hasDuplicateRelations",
            &self.has_duplicate_relations,
            false,
        )?;
        v.field("hasRelatedRelations", &self.has_related_relations, false)?;
        v.field("lastAppliedTemplate", &self.last_applied_template, true)?;
        v.list("and", &self.and, false)?;
        v.list("or", &self.or, false)?;
        v.leave();
        Ok(())
    }
}

fn run<T: Check + Serialize>(filter: &T) -> R {
    let mut v = V {
        path: Vec::new(),
        depth: 0,
        conditions: 0,
    };
    filter.check(&mut v, false)?;
    let size = serde_json::to_vec(filter).map_or(0, |b| b.len());
    if size > MAX_BYTES {
        return Err(FilterError {
            path: String::new(),
            reason: format!("serialized filter is {size} bytes, limit is {MAX_BYTES}"),
        });
    }
    Ok(())
}

/// Validiert einen `IssueFilter` (docs/05 §1.8 Punkt 1 und 3 sowie die Grenzen aus §1.1;
/// Referenz-Existenz → `not_found` prüft der Core).
pub fn validate(filter: &IssueFilter) -> Result<(), FilterError> {
    run(filter)
}

/// Validiert einen `ProjectFilter` (docs/05 §1.11).
pub fn validate_project(filter: &ProjectFilter) -> Result<(), FilterError> {
    run(filter)
}

/// Validiert einen `InitiativeFilter` (docs/05 §1.11).
pub fn validate_initiative(filter: &InitiativeFilter) -> Result<(), FilterError> {
    run(filter)
}

// ---------------------------------------------------------------------------------------------
// Parsen mit JSON-Pointer im Fehler
// ---------------------------------------------------------------------------------------------

fn pointer_of(path: &serde_path_to_error::Path) -> String {
    use serde_path_to_error::Segment;
    path.iter()
        .filter_map(|seg| match seg {
            Segment::Seq { index } => Some(index.to_string()),
            Segment::Map { key } => Some(key.replace('~', "~0").replace('/', "~1")),
            Segment::Enum { .. } | Segment::Unknown => None,
        })
        .map(|s| format!("/{s}"))
        .collect()
}

/// Deserialisiert einen Filter aus JSON-Text; Schemaverstöße enthalten den JSON-Pointer.
/// Validiert **nicht** (Grenzen/Enum-Werte): dafür [`parse_issue_filter`].
pub fn from_json_str<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, FilterError> {
    let value: Value = serde_json::from_str(json).map_err(|e| FilterError {
        path: String::new(),
        reason: e.to_string(),
    })?;
    from_json_value(&value)
}

/// Strukturprüfung vor dem Deserialisieren (Fehler = `(Pointer, Grund)`):
///
/// * Wurzel und alle Filter-/Comparator-Knoten müssen JSON-Objekte sein; Arrays gibt es nur
///   unter `and`, `or`, `in`, `nin` (serde würde Structs sonst auch aus Arrays lesen),
/// * Objektschachtelung ≤ `MAX_DEPTH + 3` als Schutz vor Stack-Überlauf; die genaue Grenze
///   prüft [`validate`].
fn shape_error(
    value: &Value,
    depth: usize,
    key: Option<&str>,
    path: &mut Vec<String>,
) -> Option<(String, String)> {
    const LIMIT: usize = MAX_DEPTH + 4;
    let here = |path: &Vec<String>| path.iter().map(|s| format!("/{s}")).collect::<String>();
    match value {
        Value::Object(map) => {
            if depth + 1 > LIMIT {
                return Some((
                    here(path),
                    format!("nesting deeper than {MAX_DEPTH} levels"),
                ));
            }
            for (k, v) in map {
                path.push(k.replace('~', "~0").replace('/', "~1"));
                if let Some(e) = shape_error(v, depth + 1, Some(k), path) {
                    return Some(e);
                }
                path.pop();
            }
            None
        }
        Value::Array(items) => {
            if !matches!(key, Some("and" | "or" | "in" | "nin")) {
                return Some((here(path), "expected an object".to_owned()));
            }
            for (i, v) in items.iter().enumerate() {
                path.push(i.to_string());
                if v.is_array() {
                    return Some((here(path), "nested arrays are not allowed".to_owned()));
                }
                if let Some(e) = shape_error(v, depth, None, path) {
                    return Some(e);
                }
                path.pop();
            }
            None
        }
        _ => None,
    }
}

/// Wie [`from_json_str`] für einen bereits geparsten JSON-Wert.
pub fn from_json_value<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, FilterError> {
    if !value.is_object() {
        return Err(FilterError {
            path: String::new(),
            reason: "expected an object".to_owned(),
        });
    }
    if let Some((path, reason)) = shape_error(value, 0, None, &mut Vec::new()) {
        return Err(FilterError { path, reason });
    }
    serde_path_to_error::deserialize(value).map_err(|e| FilterError {
        path: pointer_of(e.path()),
        reason: e.inner().to_string(),
    })
}

/// JSON-Text → validierter `IssueFilter` (Schema, Enum-Werte, Grenzen).
pub fn parse_issue_filter(json: &str) -> Result<IssueFilter, FilterError> {
    let f: IssueFilter = from_json_str(json)?;
    validate(&f)?;
    Ok(f)
}

// ---------------------------------------------------------------------------------------------
// Normalisierung (docs/05 §1.8 Punkt 4)
// ---------------------------------------------------------------------------------------------

/// Felder, deren Comparator `eq`/`neq` kanonisch als `in`/`nin` mit einem Element geschrieben wird.
const SET_FORM_KEYS: [&str; 9] = [
    "id",
    "type",
    "visibility",
    "slaStatus",
    "addedToCyclePeriod",
    "priority",
    "status",
    "health",
    "position",
];

fn normalize_value(value: Value, key: Option<&str>) -> Value {
    match value {
        Value::Object(map) => {
            let set_form = key.is_some_and(|k| SET_FORM_KEYS.contains(&k));
            let mut entries: Vec<(String, Value)> = Vec::with_capacity(map.len());
            for (k, v) in map {
                // Leere or-Gruppen entfallen.
                if k == "or" && v.as_array().is_some_and(Vec::is_empty) {
                    continue;
                }
                entries.push((k.clone(), normalize_value(v, Some(&k))));
            }
            if set_form {
                for (from, to) in [("eq", "in"), ("neq", "nin")] {
                    let has_target = entries.iter().any(|(k, _)| k == to);
                    if !has_target {
                        if let Some(pos) = entries
                            .iter()
                            .position(|(k, v)| k == from && !v.is_array() && !v.is_object())
                        {
                            let (_, v) = entries.remove(pos);
                            entries.push((to.to_owned(), Value::Array(vec![v])));
                        }
                    }
                }
            }
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = Map::new();
            for (k, v) in entries {
                out.insert(k, v);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|v| normalize_value(v, None))
                .collect(),
        ),
        other => other,
    }
}

/// Normalisierte JSON-Form (docs/05 §1.8): leere `or`-Gruppen entfernt, `eq` → `in` und
/// `neq` → `nin` mit einem Element (ID-, Enum- und Prioritätsfelder), Schlüssel alphabetisch.
pub fn normalize_json(value: Value) -> Value {
    normalize_value(value, None)
}

/// Kanonische JSON-Zeichenkette eines Filters (byte-gleich für gleichwertige Filter,
/// Grundlage von `CustomView.filterData` und der URL-Serialisierung, docs/05 §1.9).
pub fn canonical_json<T: Serialize>(filter: &T) -> String {
    match serde_json::to_value(filter) {
        Ok(v) => normalize_json(v).to_string(),
        Err(_) => String::new(),
    }
}

/// Normalisierter Filter (siehe [`normalize_json`]).
pub fn normalize(filter: &IssueFilter) -> Result<IssueFilter, FilterError> {
    let value = serde_json::to_value(filter).map_err(|e| FilterError {
        path: String::new(),
        reason: e.to_string(),
    })?;
    from_json_value(&normalize_json(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: Value) -> Result<IssueFilter, FilterError> {
        let f: IssueFilter = from_json_value(&v)?;
        validate(&f)?;
        Ok(f)
    }
    fn ok(v: Value) -> IssueFilter {
        parse(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
    }
    fn bad(v: Value) -> FilterError {
        match parse(v.clone()) {
            Ok(_) => panic!("expected error for {v}"),
            Err(e) => e,
        }
    }

    /// Beispiele aus docs/05 §1.5–1.9: serde-Roundtrip ohne Verlust.
    #[test]
    fn spec_examples_round_trip() {
        let examples = [
            json!({}),
            json!({"priority": {"in": [1, 2]}}),
            json!({"estimate": {"gte": 2, "lte": 5}}),
            json!({"assignee": {"id": {"in": ["u-bob"]}}}),
            json!({"assignee": {"id": {"nin": ["u-bob", "u-carol"]}}}),
            json!({"assignee": {"null": true}}),
            json!({"assignee": {"or": [{"null": true}, {"id": {"in": ["u-bob"]}}]}}),
            json!({"assignee": {"isMe": {"eq": true}}}),
            json!({"labels": {"some": {"id": {"in": ["l-bug"]}}}}),
            json!({"labels": {"every": {"id": {"nin": ["l-bug"]}}}}),
            json!({"labels": {"length": {"eq": 0}}}),
            json!({"or": [
                {"labels": {"every": {"id": {"neq": "l-bug"}}}},
                {"labels": {"every": {"id": {"neq": "l-fe"}}}}
            ]}),
            json!({"labels": {"some": {"or": [
                {"id": {"in": ["l-platform"]}},
                {"parent": {"id": {"in": ["l-platform"]}}}
            ]}}}),
            json!({"labels": {"some": {"name": {"eqIgnoreCase": "Bug"}}}}),
            json!({"dueDate": {"lt": "P0D"}}),
            json!({"or": [{"dueDate": {"gte": "P0D"}}, {"dueDate": {"null": true}}]}),
            json!({"dueDate": {"gte": "P0D", "lte": "P1D"}}),
            json!({"createdAt": {"gt": "-P1W"}}),
            json!({"createdAt": {"lt": "2026-09-01"}}),
            json!({"completedAt": {"gte": "2026-07-01", "lte": "2026-09-30"}}),
            json!({"completedAt": {"gte": "2026-07-01T00:00:00.000Z"}}),
            json!({"stateEnteredAt": {"lt": "-P2W"}}),
            json!({"dueDate": {"lt": "P0D"}, "state": {"type": {"nin": ["completed", "canceled", "duplicate"]}}}),
            json!({"cycle": {"isActive": {"eq": true}}}),
            json!({"cycle": {"null": true}}),
            json!({"project": {"null": true}}),
            json!({"project": {"status": {"type": {"in": ["started", "planned"]}}}}),
            json!({"project": {"initiatives": {"some": {"id": {"in": ["i-x"]}}}}}),
            json!({"project": {"labels": {"some": {"id": {"in": ["pl-1"]}}}}}),
            json!({"projectMilestone": {"null": true}}),
            json!({"children": {"length": {"gt": 0}}}),
            json!({"parent": {"null": false}}),
            json!({"hasBlockedByRelations": {"eq": true}}),
            json!({"team": {"or": [
                {"id": {"eq": "t-eng"}},
                {"ancestors": {"some": {"id": {"eq": "t-eng"}}}}
            ]}}),
            json!({"slaStatus": {"in": ["breached", "highRisk"]}}),
            json!({"addedToCyclePeriod": {"eq": "during"}}),
            json!({"autoClosedAt": {"null": false}}),
            json!({"title": {"containsIgnoreCaseAndAccent": "geschäft"}}),
            json!({"searchableContent": {"contains": "login"}}),
            json!({"attachments": {"some": {"url": {"containsIgnoreCase": "github.com"}}}}),
            json!({"attachments": {"length": {"eq": 0}}}),
            json!({"comments": {"some": {"body": {"contains": "x"}, "user": {"isMe": {"eq": true}}}}}),
            json!({"estimate": {"or": [{"gte": 2, "lte": 5}, {"null": true}]}}),
            json!({"lastAppliedTemplate": {"id": {"in": ["t-1"]}}}),
            json!({"creator": {"app": {"eq": true}}}),
            json!({"state": {"team": {"visibility": {"eq": "public"}}}}),
            json!({"and": [
                {"priority": {"in": [1, 2]}},
                {"labels": {"some": {"id": {"in": ["l-bug"]}}}}
            ]}),
            json!({"and": [
                {"team": {"id": {"in": ["5d0a-eng"]}}},
                {"state": {"type": {"in": ["backlog", "unstarted", "started"]}}},
                {"labels": {"some": {"or": [
                    {"id": {"in": ["l-bug"]}}, {"parent": {"id": {"in": ["l-bug"]}}}
                ]}}}
            ]}),
        ];
        for ex in examples {
            let f = ok(ex.clone());
            assert_eq!(serde_json::to_value(&f).unwrap(), ex, "{ex}");
        }
    }

    #[test]
    fn rejects_unknown_and_out_of_scope_fields_with_pointer() {
        let e = bad(json!({"and": [{}, {"customerCount": {"gt": 1}}]}));
        assert_eq!(e.path, "/and/1/customerCount");
        assert!(e.reason.contains("customerCount"), "{e}");
        for f in [
            "needs",
            "releases",
            "sourceMetadata",
            "recurringIssueTemplate",
            "sharedWith",
        ] {
            assert!(parse(json!({ f: {"null": true} })).is_err(), "{f}");
        }
        // Falsche Operatoren / Typen.
        assert!(parse(json!({"priority": {"contains": "x"}})).is_err());
        assert!(parse(json!({"title": {"eq": 5}})).is_err());
        assert!(parse(json!({"labels": {"some": {"bogus": {}}}})).is_err());
        let e = bad(json!({"and": [{"labels": {"some": {"id": {"eq": 5}}}}]}));
        assert_eq!(e.path, "/and/0/labels/some/id/eq");
    }

    #[test]
    fn arrays_where_objects_are_expected_are_rejected() {
        // serde würde Structs sonst positionell aus Arrays lesen.
        for (v, path) in [
            (json!([]), ""),
            (json!({"priority": [1]}), "/priority"),
            (json!({"state": [[]]}), "/state"),
            (json!({"and": [[]]}), "/and/0"),
            (json!({"labels": {"some": []}}), "/labels/some"),
        ] {
            let e = from_json_value::<IssueFilter>(&v).unwrap_err();
            assert_eq!(e.path, path, "{v}");
        }
        assert!(parse_issue_filter("[1]").is_err());
        assert!(parse_issue_filter(r#"{"title": "x"}"#).is_err());
    }

    #[test]
    fn extreme_nesting_is_rejected_before_deserializing() {
        let mut v = json!({"priority": {"eq": 1}});
        for _ in 0..120 {
            v = json!({"parent": v});
        }
        let e = from_json_value::<IssueFilter>(&v).unwrap_err();
        assert!(e.reason.contains("nesting"), "{e}");
        assert!(e.path.starts_with("/parent/parent"), "{}", e.path);
        // Text mit mehr als 128 Ebenen: serde_json lehnt selbst ab, kein Absturz.
        let text = format!(
            "{}{}",
            "{\"parent\":".repeat(500),
            "{}".to_owned() + &"}".repeat(500)
        );
        assert!(parse_issue_filter(&text).is_err());
    }

    #[test]
    fn comparators_need_an_operator() {
        for v in [
            json!({"priority": {}}),
            json!({"title": {}}),
            json!({"assignee": {"id": {}}}),
            json!({"slaStatus": {}}),
            json!({"hasBlockedByRelations": {}}),
            json!({"searchableContent": {}}),
            json!({"dueDate": {}}),
            json!({"createdAt": {}}),
            json!({"estimate": {}}),
            json!({"estimate": {"or": [{}]}}),
        ] {
            let e = bad(v.clone());
            assert!(e.reason.contains("at least one operator"), "{v}: {e}");
        }
        // Leere Filterobjekte und leere and/or sind erlaubt.
        ok(json!({"assignee": {}}));
        ok(json!({"and": [], "or": []}));
        ok(json!({"labels": {}}));
    }

    #[test]
    fn null_only_where_schema_allows() {
        let e = bad(json!({"null": true}));
        assert_eq!(e.path, "");
        let e = bad(json!({"and": [{"null": false}]}));
        assert_eq!(e.path, "/and/0");
        ok(json!({"parent": {"null": true}}));
        assert!(parse(json!({"title": {"null": true}})).is_err());
        assert!(parse(json!({"number": {"null": true}})).is_err());
        assert!(parse(json!({"createdAt": {"null": true}})).is_err());
        assert!(parse(json!({"stateEnteredAt": {"null": true}})).is_err());
        assert!(parse(json!({"subscribers": {"some": {"null": true}}})).is_err());
        assert!(parse(json!({"state": {"null": true}})).is_err()); // nicht im Schema
        ok(json!({"description": {"null": true}}));
        ok(json!({"priority": {"null": true}}));
        ok(json!({"estimate": {"null": true}}));
        ok(json!({"archivedAt": {"null": true}}));
        ok(json!({"dueDate": {"null": true}}));
        ok(json!({"assignee": {"null": true}}));
        ok(json!({"cycle": {"null": true}}));
        ok(json!({"slaStatus": {"null": true}}));
        ok(json!({"labels": {"null": true}}));
    }

    #[test]
    fn enum_values_are_checked_per_field() {
        let e = bad(json!({"state": {"type": {"in": ["started", "doing"]}}}));
        assert_eq!(e.path, "/state/type");
        assert!(e.reason.contains("doing"));
        assert!(parse(json!({"slaStatus": {"eq": "Breached"}})).is_err()); // camelCase ist normativ
        assert!(parse(json!({"slaStatus": {"eq": "atRisk"}})).is_err());
        assert!(parse(json!({"addedToCyclePeriod": {"in": ["before", "later"]}})).is_err());
        assert!(parse(json!({"team": {"visibility": {"eq": "secret"}}})).is_err());
        assert!(parse(json!({"project": {"status": {"type": {"eq": "done"}}}})).is_err());
        assert!(parse(json!({"project": {"health": {"eq": "green"}}})).is_err());
        for t in StateType::ALL {
            ok(json!({"state": {"type": {"eq": t.as_str()}}}));
        }
    }

    #[test]
    fn priority_values_are_0_to_4_integers() {
        for good in [
            json!({"in": [0, 1, 2, 3, 4]}),
            json!({"lte": 2}),
            json!({"eq": 0}),
        ] {
            ok(json!({"priority": good}));
        }
        for badv in [json!({"in": [5]}), json!({"eq": -1}), json!({"nin": [1.5]})] {
            let e = bad(json!({"priority": badv.clone()}));
            assert_eq!(e.path, "/priority", "{badv}");
        }
    }

    #[test]
    fn date_values_are_validated_on_parse() {
        for good in [
            "-P2W",
            "P0D",
            "PT12H",
            "-PT5M",
            "P1Y2M3W4DT5H6M7S",
            "P1M",
            "2026-10-01",
            "2026-10-01T07:15:00.000Z",
            "2026-10-01T09:15:00+02:00",
        ] {
            ok(json!({"createdAt": {"gt": good}}));
        }
        for bad_value in [
            "",
            "P",
            "PT",
            "-",
            "P1D1W",
            "P1X",
            "1W",
            "P1.5D",
            "P-1D",
            "2026-13-01",
            "2026-1-5",
            "yesterday",
            "P1WT",
            "PT1D",
            "2026-10-01T07:15:00",
        ] {
            assert!(
                parse(json!({"createdAt": {"gt": bad_value}})).is_err(),
                "{bad_value:?}"
            );
        }
        // dueDate: nur Datum oder Dauer, kein Zeitpunkt.
        ok(json!({"dueDate": {"eq": "2026-11-30"}}));
        assert!(parse(json!({"dueDate": {"eq": "2026-11-30T00:00:00Z"}})).is_err());
        assert!(parse(json!({"dueDate": {"in": ["2026-11-30", "nope"]}})).is_err());
    }

    #[test]
    fn duration_parsing_and_arithmetic() {
        let d = IsoDuration::parse("-P1W2D").unwrap();
        assert!(d.negative && d.weeks == 1 && d.days == 2);
        let d = IsoDuration::parse("P1DT2H3M4S").unwrap();
        assert_eq!((d.days, d.hours, d.minutes, d.seconds), (1, 2, 3, 4));
        assert_eq!(IsoDuration::parse("P1D1W"), None);

        let jan31 = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let p1m = IsoDuration::parse("P1M").unwrap();
        assert_eq!(p1m.add_to_date(jan31), NaiveDate::from_ymd_opt(2026, 2, 28));
        let m1w = IsoDuration::parse("-P1W").unwrap();
        assert_eq!(m1w.add_to_date(jan31), NaiveDate::from_ymd_opt(2026, 1, 24));
        assert_eq!(
            IsoDuration::parse("P0D").unwrap().add_to_date(jan31),
            Some(jan31)
        );

        let now = DateTime::parse_from_rfc3339("2026-10-10T07:15:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let back = IsoDuration::parse("-PT12H")
            .unwrap()
            .add_to_datetime(now)
            .unwrap();
        assert_eq!(back.to_rfc3339(), "2026-10-09T19:15:00+00:00");
        let year = IsoDuration::parse("P1Y")
            .unwrap()
            .add_to_datetime(now)
            .unwrap();
        assert_eq!(year.to_rfc3339(), "2027-10-10T07:15:00+00:00");

        match DateTimeOrDuration::parse("-P2W").unwrap().value() {
            DateValue::Duration(d) => assert!(d.negative && d.weeks == 2),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            DateTimeOrDuration::parse("2026-10-01").unwrap().value(),
            DateValue::Date(_)
        ));
        assert!(matches!(
            DateTimeOrDuration::parse("2026-10-01T00:00:00Z")
                .unwrap()
                .value(),
            DateValue::DateTime(_)
        ));
    }

    #[test]
    fn id_lists_are_capped_at_250() {
        let ids = |n: usize| (0..n).map(|i| format!("id-{i}")).collect::<Vec<_>>();
        ok(json!({"id": {"in": ids(250)}}));
        let e = bad(json!({"id": {"in": ids(251)}}));
        assert!(e.path.starts_with("/id/in"), "{}", e.path);
        assert!(parse(json!({"team": {"id": {"nin": ids(251)}}})).is_err());
    }

    #[test]
    fn search_text_must_not_be_empty() {
        assert!(parse(json!({"searchableContent": {"contains": ""}})).is_err());
        assert!(parse(json!({"searchableContent": {"notContains": ""}})).is_err());
        ok(json!({"searchableContent": {"contains": "a"}}));
        // StringComparator erlaubt Leerstring (kein minLength im Schema).
        ok(json!({"title": {"eq": ""}}));
    }

    fn nested_and(levels: usize) -> Value {
        let mut v = json!({"priority": {"eq": 1}});
        for _ in 1..levels {
            v = json!({"and": [v]});
        }
        v
    }

    #[test]
    fn depth_limit_is_ten() {
        // 10 Ebenen unterhalb der Wurzel sind zulässig, 11 nicht.
        ok(nested_and(11));
        let e = bad(nested_and(12));
        assert!(e.reason.contains("nesting"), "{e}");
        // Tiefe über Relationen zählt ebenso: labels -> some -> parent -> ...
        let mut deep = json!({"id": {"eq": "x"}});
        for _ in 0..9 {
            deep = json!({"parent": deep});
        }
        assert!(parse(json!({"labels": {"some": deep}})).is_err());
    }

    #[test]
    fn depth_limit_applies_to_relation_chains() {
        // IssueFilter.parent.parent... : 10 Ebenen ok, 11 zu tief.
        let chain = |n: usize| {
            let mut v = json!({"priority": {"eq": 1}});
            for _ in 1..n {
                v = json!({"parent": v});
            }
            v
        };
        ok(chain(11));
        assert!(parse(chain(12)).is_err());
    }

    #[test]
    fn condition_limit_is_200() {
        let conds = |n: usize| json!({"and": (0..n - 1).map(|_| json!({"priority": {"eq": 1}})).collect::<Vec<_>>()});
        // `and` selbst zählt nicht, nur die n-1 Elemente mit je 1 Feld.
        ok(conds(201));
        let e = bad(conds(202));
        assert!(e.reason.contains("200"), "{e}");
    }

    #[test]
    fn size_limit_is_8_kb() {
        // Wenige Bedingungen, aber große Strings.
        let big = "x".repeat(MAX_BYTES);
        let e = bad(json!({"title": {"contains": big}}));
        assert!(e.reason.contains("bytes"), "{e}");
        ok(json!({"title": {"contains": "x".repeat(4000)}}));
    }

    #[test]
    fn parse_issue_filter_from_text_reports_pointer() {
        let f = parse_issue_filter(r#"{"and":[{"priority":{"in":[1,2]}}]}"#).unwrap();
        assert!(f.and.is_some());
        let e = parse_issue_filter(r#"{"and":[{"priority":{"in":"x"}}]}"#).unwrap_err();
        assert_eq!(e.path, "/and/0/priority/in");
        let e = parse_issue_filter(r#"{"state":{"type":{"eq":"nope"}}}"#).unwrap_err();
        assert_eq!(e.path, "/state/type");
        assert!(parse_issue_filter("not json").is_err());
        assert!(parse_issue_filter("[]").is_err());
    }

    #[test]
    fn normalization_makes_equivalent_filters_byte_equal() {
        let a = json!({"state": {"type": {"eq": "started"}}, "assignee": {"id": {"neq": "u-bob"}}, "or": [], "priority": {"eq": 1}});
        let b = json!({"priority": {"in": [1]}, "assignee": {"id": {"nin": ["u-bob"]}}, "state": {"type": {"in": ["started"]}}});
        let (fa, fb) = (ok(a), ok(b));
        assert_eq!(canonical_json(&fa), canonical_json(&fb));
        // Schlüssel alphabetisch.
        assert_eq!(
            canonical_json(&fb),
            r#"{"assignee":{"id":{"nin":["u-bob"]}},"priority":{"in":[1]},"state":{"type":{"in":["started"]}}}"#
        );
        // Typisierte Normalisierung entspricht der JSON-Normalisierung.
        let n = normalize(&fa).unwrap();
        assert_eq!(n.or, None);
        assert_eq!(n.priority.unwrap().in_, Some(vec![Number::from(1)]));
    }

    #[test]
    fn normalization_is_idempotent_and_keeps_other_comparators() {
        let f = ok(json!({
            "and": [
                {"team": {"id": {"eq": "t-eng"}}},
                {"title": {"eq": "x"}},          // String-eq bleibt eq
                {"estimate": {"eq": 3}},         // estimate bleibt (nicht in der Mengenform-Liste)
                {"labels": {"some": {"id": {"eq": "l-bug"}}}}
            ],
            "or": [{"priority": {"gte": 2}}]
        }));
        let once = canonical_json(&f);
        let again = canonical_json(&parse_issue_filter(&once).unwrap());
        assert_eq!(once, again);
        let v: Value = serde_json::from_str(&once).unwrap();
        assert_eq!(v["and"][0]["team"]["id"], json!({"in": ["t-eng"]}));
        assert_eq!(v["and"][1]["title"], json!({"eq": "x"}));
        assert_eq!(v["and"][2]["estimate"], json!({"eq": 3}));
        assert_eq!(
            v["and"][3]["labels"]["some"]["id"],
            json!({"in": ["l-bug"]})
        );
        // Wenn in bereits existiert, bleibt eq unangetastet (UND-Semantik).
        let both = ok(json!({"id": {"eq": "a", "in": ["a", "b"]}}));
        assert_eq!(canonical_json(&both), r#"{"id":{"eq":"a","in":["a","b"]}}"#);
    }

    #[test]
    fn project_and_initiative_filters() {
        let p: ProjectFilter = from_json_value(&json!({
            "status": {"type": {"in": ["started"]}},
            "health": {"in": ["atRisk", "offTrack"]},
            "lead": {"isMe": {"eq": true}},
            "teams": {"some": {"key": {"eq": "ENG"}}},
            "targetDate": {"lt": "P0D"},
            "initiatives": {"some": {"name": {"contains": "Self"}}}
        }))
        .unwrap();
        assert!(validate_project(&p).is_ok());
        // `null` ist nur in verschachtelter Nutzung (issue.project) zulässig, nicht an der Wurzel.
        let p: ProjectFilter = from_json_value(&json!({"null": true})).unwrap();
        assert!(validate_project(&p).is_err());
    }

    #[test]
    fn project_filter_top_level_null_and_enums() {
        let p: ProjectFilter = from_json_value(&json!({"health": {"eq": "green"}})).unwrap();
        assert!(validate_project(&p).is_err());
        let p: ProjectFilter = from_json_value(
            &json!({"priority": {"in": [1, 2]}, "status": {"type": {"eq": "paused"}}}),
        )
        .unwrap();
        assert!(validate_project(&p).is_ok());
        assert!(from_json_value::<ProjectFilter>(&json!({"bogus": 1})).is_err());
        let i: InitiativeFilter = from_json_value(&json!({"status": {"in": ["Active", "Planned"]}, "projects": {"some": {"id": {"in": ["p1"]}}}})).unwrap();
        assert!(validate_initiative(&i).is_ok());
        let i: InitiativeFilter = from_json_value(&json!({"status": {"eq": "active"}})).unwrap();
        assert!(validate_initiative(&i).is_err());
    }

    #[test]
    fn project_filter_nested_in_issue_filter() {
        ok(json!({"project": {"health": {"eq": "onTrack"}, "lead": {"null": true}}}));
        ok(json!({"project": {"projectMilestones": {"some": {"targetDate": {"lt": "P0D"}}}}}));
        assert!(parse(json!({"project": {"health": {"eq": "green"}}})).is_err());
    }
}
