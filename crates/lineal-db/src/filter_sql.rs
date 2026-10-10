//! Filter-AST → SQL (WP11, docs/05 §1.10).
//!
//! Der Compiler übersetzt den typisierten Filterbaum aus `lineal_domain::rules::filter` rekursiv in
//! ein Prädikat und schreibt es in einen `sqlx::QueryBuilder`. Regeln:
//!
//! * **Nie String-Interpolation von Werten.** Jeder Wert aus dem Filter (IDs, Texte, Zahlen,
//!   Zeitpunkte) läuft über `push_bind`. Spalten-, Tabellen- und Aliasnamen stammen
//!   ausschließlich aus Konstanten im Code (ein `match` auf das Feld), nie aus Benutzereingaben.
//! * **Berechtigung außerhalb des Benutzerbaums.** Der Aufrufer hängt `team_id = ANY(visible)`
//!   per UND an; innerhalb des Baums gilt für verschachtelte Relationen dieselbe Sichtbarkeit
//!   (Eltern-/Kind-Issues, Teams, Projekte), damit kein Filter Unsichtbares verrät (docs/05
//!   §1.10 Regel 2).
//! * **Laufzeitkontext als CTE.** Sichtbare Teams, Zeitpunkt und „ich“ stehen einmal pro
//!   Abfrage in `lf_vis` und `lf_cx` ([`push_cte_prefix`]); Teilabfragen verweisen darauf,
//!   statt dieselben Werte mehrfach zu binden.
//! * Null-Semantik nach §1.3: `neq` ⇒ `IS DISTINCT FROM`, `nin` ⇒ `IS NULL OR NOT IN`,
//!   leeres `in` ⇒ `FALSE`, leeres `nin` ⇒ `TRUE`, `every` über leere Menge ⇒ `TRUE`; To-One-
//!   Felder werden über skalare Teilabfragen gelesen und sind bei fehlender Relation `NULL`,
//!   wodurch Negationen („is not Bob“) unzugewiesene Issues einschließen.
//!
//! Bekannte Grenzen (dokumentiert, mit `invalid_request` abgelehnt): `team.ancestors.some`
//! unterstützt nur `id`; `UserFilter.email` ist gesperrt (keine E-Mails fremder User).

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use lineal_app::ports::repos::view::FilterEval;
use lineal_app::{AppError, AppResult};
use lineal_domain::rules::cycle_calendar::local_midnight_to_utc;
use lineal_domain::rules::filter::{
    AttachmentFilter, BooleanComparator, CollectionFilter, CommentFilter, ContentComparator,
    CycleFilter, DateComparator, DateValue, EnumComparator, EstimateComparator, IdComparator,
    IdNameFilter, InitiativeFilter, IssueFilter, IssueLabelFilter, MilestoneFilter,
    NumberComparator, ProjectFilter, ProjectStatusFilter, StringComparator, TeamAncestorsFilter,
    TeamFilter, TemplateFilter, TimelessDateComparator, UserFilter, WorkflowStateFilter,
};
use serde_json::{Number, json};
use sqlx::{Postgres, QueryBuilder};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

type Qb<'q> = QueryBuilder<'q, Postgres>;
type R = AppResult<()>;

const T_USER: &str = "\"user\"";
const VIS: &str = "(SELECT id FROM lf_vis)";
const NOW: &str = "(SELECT now FROM lf_cx)";
const ME: &str = "(SELECT me FROM lf_cx)";
const ORG: &str = "(SELECT org FROM lf_cx)";
const OPEN_STATES: &str = "('completed','canceled','duplicate')";

/// Art einer Referenz im Filter (für die Existenz-/Sichtbarkeitsprüfung, docs/05 §1.8 Punkt 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RefKind {
    Team,
    User,
    State,
    Label,
    Cycle,
    Project,
    Milestone,
    Template,
    ProjectStatus,
    ProjectLabel,
    Initiative,
    InitiativeLabel,
}

/// Alle im Filter genannten Referenzen, nach Art gesammelt.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterRefs(pub BTreeMap<RefKind, BTreeSet<Uuid>>);

impl FilterRefs {
    /// Keine Referenzen?
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Auswertungskontext während des Kompilierens.
pub struct Cx<'e> {
    ev: &'e FilterEval,
    today: NaiveDate,
    tz: Tz,
    seq: Cell<u32>,
    refs: RefCell<FilterRefs>,
}

impl<'e> Cx<'e> {
    /// Neuer Kontext (Zeitzone unbekannt → UTC).
    pub fn new(ev: &'e FilterEval) -> Self {
        let tz: Tz = ev.timezone.parse().unwrap_or(chrono_tz::UTC);
        let today = ev.now.with_timezone(&tz).date_naive();
        Self {
            ev,
            today,
            tz,
            seq: Cell::new(0),
            refs: RefCell::new(FilterRefs::default()),
        }
    }

    /// Die gesammelten Referenzen.
    pub fn into_refs(self) -> FilterRefs {
        self.refs.into_inner()
    }

    fn alias(&self, prefix: &str) -> String {
        let n = self.seq.get();
        self.seq.set(n + 1);
        format!("{prefix}{n}")
    }

    fn reg(&self, kind: RefKind, id: Uuid) {
        self.refs.borrow_mut().0.entry(kind).or_default().insert(id);
    }
}

/// `WITH lf_vis AS (...), lf_cx AS (...)` (ohne Leerzeichen am Ende): sichtbare Teams,
/// Zeitpunkt und „ich“ einmal binden. Der Aufrufer hängt `, weitere AS (...)` oder ` SELECT …` an.
pub fn push_cte_prefix<'q>(qb: &mut Qb<'q>, ev: &FilterEval) {
    qb.push("WITH lf_vis(id) AS (SELECT unnest(");
    qb.push_bind(ev.visible_teams.iter().map(|t| t.0).collect::<Vec<Uuid>>());
    qb.push("::uuid[])), lf_cx(now, me, org) AS (SELECT ");
    qb.push_bind(ev.now);
    qb.push("::timestamptz, ");
    qb.push_bind(ev.me.map(|u| u.0));
    qb.push("::uuid, ");
    qb.push_bind(ev.org.0);
    qb.push("::uuid)");
}

fn bad(msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg)
}

fn parse_uuid(s: &str) -> AppResult<Uuid> {
    Uuid::parse_str(s.trim())
        .map_err(|_| bad(format!("`{s}` is not a valid id")).with_details(json!({ "value": s })))
}

fn parse_uuids(v: &[String]) -> AppResult<Vec<Uuid>> {
    v.iter().map(|s| parse_uuid(s)).collect()
}

/// Und-/Oder-Verkettung mit Leerwert.
struct Sep(bool, &'static str);

impl Sep {
    fn and() -> Self {
        Sep(true, " AND ")
    }
    fn or() -> Self {
        Sep(true, " OR ")
    }
    fn next<'q>(&mut self, qb: &mut Qb<'q>) {
        if !self.0 {
            qb.push(self.1);
        }
        self.0 = false;
    }
    fn close<'q>(self, qb: &mut Qb<'q>, empty: &str) {
        if self.0 {
            qb.push(empty);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Entitäten: wie komme ich an Spalten einer (ggf. verschachtelten) Relation?
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Vis {
    None,
    /// Zeile der Tabelle `team` muss sichtbar sein.
    Team,
    /// Issue muss in einem sichtbaren Team liegen.
    IssueTeam,
}

/// Eine Zeile im Filterbaum: entweder ein Alias im FROM-Bereich oder eine To-One-Relation,
/// die über skalare Teilabfragen gelesen wird (fehlende Relation ⇒ `NULL`).
#[derive(Clone)]
struct Ent {
    /// SQL-Ausdruck der ID der Zeile.
    id: String,
    alias: Option<String>,
    table: &'static str,
    vis: Vis,
}

impl Ent {
    fn row(alias: &str, table: &'static str) -> Ent {
        Ent {
            id: format!("{alias}.id"),
            alias: Some(alias.to_owned()),
            table,
            vis: Vis::None,
        }
    }

    /// Spaltenausdruck (`alias.col` oder skalare Teilabfrage).
    fn col(&self, cx: &Cx<'_>, c: &str) -> String {
        match &self.alias {
            Some(a) => format!("{a}.{c}"),
            None => {
                let x = cx.alias("x");
                let vis = match self.vis {
                    Vis::None => String::new(),
                    Vis::Team => format!(" AND {x}.id IN {VIS}"),
                    Vis::IssueTeam => format!(" AND {x}.team_id IN {VIS}"),
                };
                format!(
                    "(SELECT {x}.{c} FROM {t} {x} WHERE {x}.organization_id = {ORG} AND {x}.id = {id}{vis})",
                    t = self.table,
                    id = self.id
                )
            }
        }
    }

    /// ID-Ausdruck für Vergleiche; bei sichtbarkeitsbeschränkten Relationen über die
    /// beschränkte Teilabfrage, damit unsichtbare Objekte nicht über ihre ID treffen.
    fn id_col(&self, cx: &Cx<'_>) -> String {
        if self.alias.is_none() && self.vis != Vis::None {
            self.col(cx, "id")
        } else {
            self.id.clone()
        }
    }

    /// To-One-Relation dieser Zeile über die FK-Spalte `fk`.
    fn to(&self, cx: &Cx<'_>, fk: &str, table: &'static str, vis: Vis) -> Ent {
        let mut e = Ent {
            id: self.col(cx, fk),
            alias: None,
            table,
            vis,
        };
        // Sichtbarkeitsbeschränkte Relationen: schon die ID ist `NULL`, wenn das Objekt
        // unsichtbar ist – so erreichen auch Unterrelationen (children, comments, …) nichts.
        if vis != Vis::None {
            e.id = e.col(cx, "id");
        }
        e
    }
}

// ---------------------------------------------------------------------------------------------
// Skalare Comparators
// ---------------------------------------------------------------------------------------------

/// Texte, in denen Akzente entfernt werden (`containsIgnoreCaseAndAccent`); SQL-Seite via
/// `translate`, Wert-Seite via [`fold_accents`] (dieselbe Abbildung, `unaccent` ist nicht
/// installiert).
const ACCENT_FROM: &str = "áàâäãåāăąćčçďéèêëēěęíìîïīłñńňóòôöõøōřśšşťúùûüūůýÿźžż";
const ACCENT_TO: &str = "aaaaaaaaacccdeeeeeeeiiiilnnnooooooorsssstuuuuuuyyzzz";

fn fold_accents(s: &str) -> String {
    let from: Vec<char> = ACCENT_FROM.chars().collect();
    let to: Vec<char> = ACCENT_TO.chars().collect();
    s.to_lowercase()
        .chars()
        .map(|c| from.iter().position(|f| *f == c).map_or(c, |i| to[i]))
        .collect()
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn id_cmp<'q>(
    qb: &mut Qb<'q>,
    cx: &Cx<'_>,
    col: &str,
    c: &IdComparator,
    kind: Option<RefKind>,
    expand_initiatives: bool,
) -> R {
    let reg = |ids: &[Uuid]| {
        if let Some(k) = kind {
            for i in ids {
                cx.reg(k, *i);
            }
        }
    };
    let expand = |ids: Vec<Uuid>| -> Vec<Uuid> {
        if expand_initiatives {
            expand_initiative_ids(&cx.ev.initiative_edges, &ids)
        } else {
            ids
        }
    };
    qb.push("(");
    let mut g = Sep::and();
    if let Some(v) = &c.eq {
        g.next(qb);
        let id = parse_uuid(v)?;
        reg(&[id]);
        if expand_initiatives {
            qb.push(format!("{col} = ANY("));
            qb.push_bind(expand(vec![id]));
            qb.push(")");
        } else {
            qb.push(format!("{col} = "));
            qb.push_bind(id);
        }
    }
    if let Some(v) = &c.neq {
        g.next(qb);
        let id = parse_uuid(v)?;
        reg(&[id]);
        if expand_initiatives {
            qb.push(format!("({col} IS NULL OR NOT ({col} = ANY("));
            qb.push_bind(expand(vec![id]));
            qb.push(")))");
        } else {
            qb.push(format!("{col} IS DISTINCT FROM "));
            qb.push_bind(id);
        }
    }
    if let Some(v) = &c.in_ {
        g.next(qb);
        let ids = parse_uuids(v)?;
        reg(&ids);
        if ids.is_empty() {
            qb.push("FALSE");
        } else {
            qb.push(format!("{col} = ANY("));
            qb.push_bind(expand(ids));
            qb.push(")");
        }
    }
    if let Some(v) = &c.nin {
        g.next(qb);
        let ids = parse_uuids(v)?;
        reg(&ids);
        if ids.is_empty() {
            qb.push("TRUE");
        } else {
            qb.push(format!("({col} IS NULL OR NOT ({col} = ANY("));
            qb.push_bind(expand(ids));
            qb.push(")))");
        }
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

/// `id` plus Nachfahren im Initiative-DAG (docs/05 §1.7, „Initiative X“ ist rekursiv).
fn expand_initiative_ids(edges: &[(Uuid, Uuid)], ids: &[Uuid]) -> Vec<Uuid> {
    let mut seen: BTreeSet<Uuid> = ids.iter().copied().collect();
    let mut stack: Vec<Uuid> = ids.to_vec();
    while let Some(cur) = stack.pop() {
        for (p, c) in edges {
            if *p == cur && seen.insert(*c) {
                stack.push(*c);
            }
        }
    }
    seen.into_iter().collect()
}

fn string_cmp<'q>(qb: &mut Qb<'q>, col: &str, c: &StringComparator) -> R {
    qb.push("(");
    let mut g = Sep::and();
    macro_rules! op {
        ($field:ident, |$v:ident| $body:block) => {
            if let Some($v) = &c.$field {
                g.next(qb);
                $body
            }
        };
    }
    op!(eq, |v| {
        qb.push(format!("{col} = "));
        qb.push_bind(v.clone());
    });
    op!(neq, |v| {
        qb.push(format!("{col} IS DISTINCT FROM "));
        qb.push_bind(v.clone());
    });
    if let Some(v) = &c.in_ {
        g.next(qb);
        if v.is_empty() {
            qb.push("FALSE");
        } else {
            qb.push(format!("{col} = ANY("));
            qb.push_bind(v.clone());
            qb.push(")");
        }
    }
    if let Some(v) = &c.nin {
        g.next(qb);
        if v.is_empty() {
            qb.push("TRUE");
        } else {
            qb.push(format!("({col} IS NULL OR NOT ({col} = ANY("));
            qb.push_bind(v.clone());
            qb.push(")))");
        }
    }
    op!(eq_ignore_case, |v| {
        qb.push(format!("lower({col}) = lower("));
        qb.push_bind(v.clone());
        qb.push(")");
    });
    op!(neq_ignore_case, |v| {
        qb.push(format!("lower({col}) IS DISTINCT FROM lower("));
        qb.push_bind(v.clone());
        qb.push(")");
    });
    op!(contains, |v| {
        qb.push(format!("strpos({col}, "));
        qb.push_bind(v.clone());
        qb.push(") > 0");
    });
    op!(not_contains, |v| {
        qb.push(format!("({col} IS NULL OR strpos({col}, "));
        qb.push_bind(v.clone());
        qb.push(") = 0)");
    });
    op!(contains_ignore_case, |v| {
        qb.push(format!("strpos(lower({col}), lower("));
        qb.push_bind(v.clone());
        qb.push(")) > 0");
    });
    op!(not_contains_ignore_case, |v| {
        qb.push(format!("({col} IS NULL OR strpos(lower({col}), lower("));
        qb.push_bind(v.clone());
        qb.push(")) = 0)");
    });
    op!(contains_ignore_case_and_accent, |v| {
        qb.push(format!(
            "strpos(translate(lower({col}), '{ACCENT_FROM}', '{ACCENT_TO}'), "
        ));
        qb.push_bind(fold_accents(v));
        qb.push(") > 0");
    });
    op!(starts_with, |v| {
        qb.push(format!("starts_with({col}, "));
        qb.push_bind(v.clone());
        qb.push(")");
    });
    op!(starts_with_ignore_case, |v| {
        qb.push(format!("starts_with(lower({col}), lower("));
        qb.push_bind(v.clone());
        qb.push("))");
    });
    op!(not_starts_with, |v| {
        qb.push(format!("({col} IS NULL OR NOT starts_with({col}, "));
        qb.push_bind(v.clone());
        qb.push("))");
    });
    op!(ends_with, |v| {
        qb.push(format!("{col} LIKE "));
        qb.push_bind(format!("%{}", escape_like(v)));
        qb.push(" ESCAPE '\\'");
    });
    op!(not_ends_with, |v| {
        qb.push(format!("({col} IS NULL OR {col} NOT LIKE "));
        qb.push_bind(format!("%{}", escape_like(v)));
        qb.push(" ESCAPE '\\')");
    });
    if let Some(v) = c.null {
        g.next(qb);
        qb.push(format!("{col} IS {}NULL", if v { "" } else { "NOT " }));
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn push_num<'q>(qb: &mut Qb<'q>, n: &Number) {
    match n.as_i64() {
        Some(i) => {
            qb.push_bind(i);
        }
        None => {
            qb.push_bind(n.as_f64().unwrap_or(0.0));
        }
    }
}

fn push_num_list<'q>(qb: &mut Qb<'q>, v: &[Number]) {
    if v.iter().all(|n| n.as_i64().is_some()) {
        qb.push_bind(v.iter().filter_map(Number::as_i64).collect::<Vec<i64>>());
    } else {
        qb.push_bind(v.iter().filter_map(Number::as_f64).collect::<Vec<f64>>());
    }
}

#[allow(clippy::too_many_arguments)]
fn number_ops<'q>(
    qb: &mut Qb<'q>,
    g: &mut Sep,
    col: &str,
    eq: &Option<Number>,
    neq: &Option<Number>,
    in_: &Option<Vec<Number>>,
    nin: &Option<Vec<Number>>,
    lt: &Option<Number>,
    lte: &Option<Number>,
    gt: &Option<Number>,
    gte: &Option<Number>,
    null: Option<bool>,
) {
    for (op, v) in [("=", eq), ("IS DISTINCT FROM", neq)] {
        if let Some(n) = v {
            g.next(qb);
            qb.push(format!("{col} {op} "));
            push_num(qb, n);
        }
    }
    if let Some(v) = in_ {
        g.next(qb);
        if v.is_empty() {
            qb.push("FALSE");
        } else {
            qb.push(format!("{col} = ANY("));
            push_num_list(qb, v);
            qb.push(")");
        }
    }
    if let Some(v) = nin {
        g.next(qb);
        if v.is_empty() {
            qb.push("TRUE");
        } else {
            qb.push(format!("({col} IS NULL OR NOT ({col} = ANY("));
            push_num_list(qb, v);
            qb.push(")))");
        }
    }
    for (op, v) in [("<", lt), ("<=", lte), (">", gt), (">=", gte)] {
        if let Some(n) = v {
            g.next(qb);
            qb.push(format!("{col} {op} "));
            push_num(qb, n);
        }
    }
    if let Some(v) = null {
        g.next(qb);
        qb.push(format!("{col} IS {}NULL", if v { "" } else { "NOT " }));
    }
}

fn number_cmp<'q>(qb: &mut Qb<'q>, col: &str, c: &NumberComparator) -> R {
    qb.push("(");
    let mut g = Sep::and();
    number_ops(
        qb, &mut g, col, &c.eq, &c.neq, &c.in_, &c.nin, &c.lt, &c.lte, &c.gt, &c.gte, c.null,
    );
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn estimate_cmp<'q>(qb: &mut Qb<'q>, col: &str, c: &EstimateComparator) -> R {
    qb.push("(");
    let mut g = Sep::and();
    number_ops(
        qb, &mut g, col, &c.eq, &c.neq, &c.in_, &c.nin, &c.lt, &c.lte, &c.gt, &c.gte, c.null,
    );
    if let Some(list) = &c.and {
        g.next(qb);
        qb.push("(");
        let mut s = Sep::and();
        for sub in list {
            s.next(qb);
            number_cmp(qb, col, sub)?;
        }
        s.close(qb, "TRUE");
        qb.push(")");
    }
    if let Some(list) = &c.or {
        if !list.is_empty() {
            g.next(qb);
            qb.push("(");
            let mut s = Sep::or();
            for sub in list {
                s.next(qb);
                number_cmp(qb, col, sub)?;
            }
            s.close(qb, "FALSE");
            qb.push(")");
        }
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn enum_cmp<'q>(qb: &mut Qb<'q>, col: &str, c: &EnumComparator) -> R {
    qb.push("(");
    let mut g = Sep::and();
    if let Some(v) = &c.eq {
        g.next(qb);
        qb.push(format!("{col} = "));
        qb.push_bind(v.clone());
    }
    if let Some(v) = &c.neq {
        g.next(qb);
        qb.push(format!("{col} IS DISTINCT FROM "));
        qb.push_bind(v.clone());
    }
    if let Some(v) = &c.in_ {
        g.next(qb);
        if v.is_empty() {
            qb.push("FALSE");
        } else {
            qb.push(format!("{col} = ANY("));
            qb.push_bind(v.clone());
            qb.push(")");
        }
    }
    if let Some(v) = &c.nin {
        g.next(qb);
        if v.is_empty() {
            qb.push("TRUE");
        } else {
            qb.push(format!("({col} IS NULL OR NOT ({col} = ANY("));
            qb.push_bind(v.clone());
            qb.push(")))");
        }
    }
    if let Some(v) = c.null {
        g.next(qb);
        qb.push(format!("{col} IS {}NULL", if v { "" } else { "NOT " }));
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

/// `pred` ist ein Boolesches SQL-Prädikat (kann `NULL` liefern, wenn die Relation fehlt).
fn bool_cmp<'q>(qb: &mut Qb<'q>, pred: &str, c: &BooleanComparator) -> R {
    qb.push("(");
    let mut g = Sep::and();
    let p = format!("COALESCE(({pred}), false)");
    for (v, flip) in [(c.eq, false), (c.neq, true)] {
        if let Some(b) = v {
            g.next(qb);
            if b != flip {
                qb.push(p.clone());
            } else {
                qb.push(format!("NOT {p}"));
            }
        }
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

// --- Datumsvergleiche -----------------------------------------------------------------------

/// Aufgelöster Datumswert für Zeitstempel-Spalten.
enum Inst {
    /// Genauer Zeitpunkt.
    At(DateTime<Utc>),
    /// Ganzer Kalendertag in der Referenzzeitzone: `[start, end)`.
    Day(DateTime<Utc>, DateTime<Utc>),
}

fn resolve_dt(cx: &Cx<'_>, v: &DateValue) -> AppResult<Inst> {
    match v {
        DateValue::DateTime(t) => Ok(Inst::At(*t)),
        DateValue::Date(d) => {
            let next = d
                .succ_opt()
                .ok_or_else(|| bad("date out of range").with_details(json!({ "value": d })))?;
            Ok(Inst::Day(
                local_midnight_to_utc(*d, cx.tz),
                local_midnight_to_utc(next, cx.tz),
            ))
        }
        DateValue::Duration(d) => d
            .add_to_datetime(cx.ev.now)
            .map(Inst::At)
            .ok_or_else(|| bad("duration out of range")),
    }
}

fn resolve_date(cx: &Cx<'_>, v: &DateValue) -> AppResult<NaiveDate> {
    match v {
        DateValue::Date(d) => Ok(*d),
        DateValue::DateTime(t) => Ok(t.with_timezone(&cx.tz).date_naive()),
        DateValue::Duration(d) => d
            .add_to_date(cx.today)
            .ok_or_else(|| bad("duration out of range")),
    }
}

fn dt_eq<'q>(qb: &mut Qb<'q>, col: &str, i: &Inst, negate: bool) {
    match (i, negate) {
        (Inst::At(t), false) => {
            qb.push(format!("{col} = "));
            qb.push_bind(*t);
        }
        (Inst::At(t), true) => {
            qb.push(format!("{col} IS DISTINCT FROM "));
            qb.push_bind(*t);
        }
        (Inst::Day(s, e), false) => {
            qb.push(format!("({col} >= "));
            qb.push_bind(*s);
            qb.push(format!(" AND {col} < "));
            qb.push_bind(*e);
            qb.push(")");
        }
        (Inst::Day(s, e), true) => {
            qb.push(format!("({col} IS NULL OR {col} < "));
            qb.push_bind(*s);
            qb.push(format!(" OR {col} >= "));
            qb.push_bind(*e);
            qb.push(")");
        }
    }
}

fn date_cmp<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, col: &str, c: &DateComparator) -> R {
    qb.push("(");
    let mut g = Sep::and();
    if let Some(v) = &c.eq {
        g.next(qb);
        dt_eq(qb, col, &resolve_dt(cx, &v.value())?, false);
    }
    if let Some(v) = &c.neq {
        g.next(qb);
        dt_eq(qb, col, &resolve_dt(cx, &v.value())?, true);
    }
    if let Some(list) = &c.in_ {
        g.next(qb);
        qb.push("(");
        let mut s = Sep::or();
        for v in list {
            s.next(qb);
            dt_eq(qb, col, &resolve_dt(cx, &v.value())?, false);
        }
        s.close(qb, "FALSE");
        qb.push(")");
    }
    if let Some(list) = &c.nin {
        g.next(qb);
        qb.push("(");
        let mut s = Sep::and();
        for v in list {
            s.next(qb);
            dt_eq(qb, col, &resolve_dt(cx, &v.value())?, true);
        }
        s.close(qb, "TRUE");
        qb.push(")");
    }
    // (Operator, ob bei einem ganzen Tag das Ende statt des Anfangs gilt)
    for (v, op_at, op_day, use_end) in [
        (&c.lt, "<", "<", false),
        (&c.lte, "<=", "<", true),
        (&c.gt, ">", ">=", true),
        (&c.gte, ">=", ">=", false),
    ] {
        if let Some(v) = v {
            g.next(qb);
            match resolve_dt(cx, &v.value())? {
                Inst::At(t) => {
                    qb.push(format!("{col} {op_at} "));
                    qb.push_bind(t);
                }
                Inst::Day(s, e) => {
                    qb.push(format!("{col} {op_day} "));
                    qb.push_bind(if use_end { e } else { s });
                }
            }
        }
    }
    if let Some(v) = c.null {
        g.next(qb);
        qb.push(format!("{col} IS {}NULL", if v { "" } else { "NOT " }));
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn timeless_cmp<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, col: &str, c: &TimelessDateComparator) -> R {
    qb.push("(");
    let mut g = Sep::and();
    if let Some(v) = &c.eq {
        g.next(qb);
        qb.push(format!("{col} = "));
        qb.push_bind(resolve_date(cx, &v.value())?);
    }
    if let Some(v) = &c.neq {
        g.next(qb);
        qb.push(format!("{col} IS DISTINCT FROM "));
        qb.push_bind(resolve_date(cx, &v.value())?);
    }
    if let Some(list) = &c.in_ {
        g.next(qb);
        if list.is_empty() {
            qb.push("FALSE");
        } else {
            let ds = list
                .iter()
                .map(|v| resolve_date(cx, &v.value()))
                .collect::<AppResult<Vec<_>>>()?;
            qb.push(format!("{col} = ANY("));
            qb.push_bind(ds);
            qb.push(")");
        }
    }
    if let Some(list) = &c.nin {
        g.next(qb);
        if list.is_empty() {
            qb.push("TRUE");
        } else {
            let ds = list
                .iter()
                .map(|v| resolve_date(cx, &v.value()))
                .collect::<AppResult<Vec<_>>>()?;
            qb.push(format!("({col} IS NULL OR NOT ({col} = ANY("));
            qb.push_bind(ds);
            qb.push(")))");
        }
    }
    for (v, op) in [(&c.lt, "<"), (&c.lte, "<="), (&c.gt, ">"), (&c.gte, ">=")] {
        if let Some(v) = v {
            g.next(qb);
            qb.push(format!("{col} {op} "));
            qb.push_bind(resolve_date(cx, &v.value())?);
        }
    }
    if let Some(v) = c.null {
        g.next(qb);
        qb.push(format!("{col} IS {}NULL", if v { "" } else { "NOT " }));
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn content_cmp<'q>(qb: &mut Qb<'q>, col: &str, c: &ContentComparator) -> R {
    qb.push("(");
    let mut g = Sep::and();
    if let Some(t) = &c.contains {
        g.next(qb);
        match crate::search::tsquery(t) {
            Some(q) => {
                qb.push(format!("{col} @@ to_tsquery('simple', "));
                qb.push_bind(q);
                qb.push(")");
            }
            None => {
                qb.push("FALSE");
            }
        }
    }
    if let Some(t) = &c.not_contains {
        g.next(qb);
        match crate::search::tsquery(t) {
            Some(q) => {
                qb.push(format!("NOT COALESCE({col} @@ to_tsquery('simple', "));
                qb.push_bind(q);
                qb.push("), false)");
            }
            None => {
                qb.push("TRUE");
            }
        }
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Collections (some / every / length / null / and / or)
// ---------------------------------------------------------------------------------------------

/// Beschreibung einer To-Many-Relation: `from` (mit Join), `cond` (Bezug zum Besitzer, bereits
/// inkl. Sichtbarkeit) und die Zeile des Elements.
struct Coll {
    from: String,
    cond: String,
    elem: Ent,
}

impl Coll {
    /// Bedingung samt Mandant des Elements (jede Teilabfrage trägt `organization_id`).
    fn cond_org(&self) -> String {
        match &self.elem.alias {
            Some(a) => format!("{a}.organization_id = {ORG} AND {}", self.cond),
            None => self.cond.clone(),
        }
    }
}

fn collection<'q, T>(
    qb: &mut Qb<'q>,
    cx: &Cx<'_>,
    c: &CollectionFilter<T>,
    spec: &Coll,
    elem: &dyn Fn(&mut Qb<'q>, &Cx<'_>, &Ent, &T) -> R,
) -> R {
    qb.push("(");
    let mut g = Sep::and();
    if let Some(t) = &c.some {
        g.next(qb);
        qb.push(format!(
            "EXISTS (SELECT 1 FROM {} WHERE {} AND ",
            spec.from,
            spec.cond_org()
        ));
        elem(qb, cx, &spec.elem, t)?;
        qb.push(")");
    }
    if let Some(t) = &c.every {
        g.next(qb);
        qb.push(format!(
            "NOT EXISTS (SELECT 1 FROM {} WHERE {} AND NOT ",
            spec.from,
            spec.cond_org()
        ));
        elem(qb, cx, &spec.elem, t)?;
        qb.push(")");
    }
    if let Some(n) = &c.length {
        g.next(qb);
        let sub = format!(
            "(SELECT count(*) FROM {} WHERE {})",
            spec.from,
            spec.cond_org()
        );
        number_cmp(qb, &sub, n)?;
    }
    if let Some(v) = c.null {
        g.next(qb);
        qb.push(format!(
            "{}EXISTS (SELECT 1 FROM {} WHERE {})",
            if v { "NOT " } else { "" },
            spec.from,
            spec.cond_org()
        ));
    }
    if let Some(list) = &c.and {
        g.next(qb);
        qb.push("(");
        let mut s = Sep::and();
        for sub in list {
            s.next(qb);
            collection(qb, cx, sub, spec, elem)?;
        }
        s.close(qb, "TRUE");
        qb.push(")");
    }
    if let Some(list) = &c.or {
        if !list.is_empty() {
            g.next(qb);
            qb.push("(");
            let mut s = Sep::or();
            for sub in list {
                s.next(qb);
                collection(qb, cx, sub, spec, elem)?;
            }
            s.close(qb, "FALSE");
            qb.push(")");
        }
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn and_list<'q, T>(
    qb: &mut Qb<'q>,
    cx: &Cx<'_>,
    list: &[T],
    f: &dyn Fn(&mut Qb<'q>, &Cx<'_>, &T) -> R,
) -> R {
    qb.push("(");
    let mut s = Sep::and();
    for x in list {
        s.next(qb);
        f(qb, cx, x)?;
    }
    s.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn or_list<'q, T>(
    qb: &mut Qb<'q>,
    cx: &Cx<'_>,
    list: &[T],
    f: &dyn Fn(&mut Qb<'q>, &Cx<'_>, &T) -> R,
) -> R {
    // Leere `or`-Gruppen entfallen (docs/05 §1.8 Punkt 4): neutrales Element.
    qb.push("(");
    let mut s = Sep::or();
    for x in list {
        s.next(qb);
        f(qb, cx, x)?;
    }
    s.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Relationen: User, Team, Status, Label, Cycle, Meilenstein, Template
// ---------------------------------------------------------------------------------------------

fn null_check<'q>(qb: &mut Qb<'q>, g: &mut Sep, e: &Ent, null: Option<bool>) {
    if let Some(v) = null {
        g.next(qb);
        qb.push(format!("{} IS {}NULL", e.id, if v { "" } else { "NOT " }));
    }
}

fn user_filter<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, f: &UserFilter) -> R {
    if f.email.is_some() {
        return Err(bad("filtering by user email is not supported")
            .with_details(json!({ "field": "email" })));
    }
    qb.push("(");
    let mut g = Sep::and();
    null_check(qb, &mut g, e, f.null);
    if let Some(c) = &f.id {
        g.next(qb);
        id_cmp(qb, cx, &e.id_col(cx), c, Some(RefKind::User), false)?;
    }
    if let Some(c) = &f.name {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "name"), c)?;
    }
    if let Some(c) = &f.display_name {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "display_name"), c)?;
    }
    if let Some(c) = &f.is_me {
        g.next(qb);
        bool_cmp(qb, &format!("{} = {ME}", e.id), c)?;
    }
    if let Some(c) = &f.active {
        g.next(qb);
        bool_cmp(qb, &e.col(cx, "active"), c)?;
    }
    if let Some(c) = &f.admin {
        g.next(qb);
        bool_cmp(
            qb,
            &format!("{} IN ('owner','admin')", e.col(cx, "role")),
            c,
        )?;
    }
    if let Some(c) = &f.app {
        g.next(qb);
        bool_cmp(qb, &format!("{} = 'app'", e.col(cx, "role")), c)?;
    }
    if let Some(l) = &f.and {
        g.next(qb);
        and_list(qb, cx, l, &|qb, cx, x| user_filter(qb, cx, e, x))?;
    }
    if let Some(l) = &f.or {
        g.next(qb);
        or_list(qb, cx, l, &|qb, cx, x| user_filter(qb, cx, e, x))?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn team_filter<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, f: &TeamFilter) -> R {
    qb.push("(");
    let mut g = Sep::and();
    if let Some(c) = &f.id {
        g.next(qb);
        id_cmp(qb, cx, &e.id_col(cx), c, Some(RefKind::Team), false)?;
    }
    if let Some(c) = &f.key {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "key"), c)?;
    }
    if let Some(c) = &f.name {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "name"), c)?;
    }
    if let Some(c) = &f.visibility {
        g.next(qb);
        enum_cmp(qb, &e.col(cx, "visibility"), c)?;
    }
    if let Some(p) = &f.parent {
        g.next(qb);
        let parent = e.to(cx, "parent_id", "team", Vis::Team);
        team_filter(qb, cx, &parent, p)?;
    }
    if let Some(a) = &f.ancestors {
        g.next(qb);
        ancestors(qb, cx, e, a)?;
    }
    if let Some(l) = &f.and {
        g.next(qb);
        and_list(qb, cx, l, &|qb, cx, x| team_filter(qb, cx, e, x))?;
    }
    if let Some(l) = &f.or {
        g.next(qb);
        or_list(qb, cx, l, &|qb, cx, x| team_filter(qb, cx, e, x))?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

/// `ancestors: { some: { id … } }`: Das Team ist Nachfahre eines der genannten Teams. Wird über
/// die vorberechnete Nachfahren-Menge des Kontexts aufgelöst (nur sichtbare Teams).
fn ancestors<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, a: &TeamAncestorsFilter) -> R {
    let Some(some) = &a.some else {
        qb.push("TRUE");
        return Ok(());
    };
    ancestors_some(qb, cx, e, some)
}

fn ancestors_some<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, t: &TeamFilter) -> R {
    let unsupported = t.key.is_some()
        || t.name.is_some()
        || t.visibility.is_some()
        || t.parent.is_some()
        || t.ancestors.is_some();
    let id_ok = |c: &IdComparator| c.neq.is_none() && c.nin.is_none();
    if unsupported || t.id.as_ref().is_some_and(|c| !id_ok(c)) {
        return Err(bad("`ancestors.some` supports only `id` with `eq` or `in`")
            .with_details(json!({ "field": "ancestors" })));
    }
    qb.push("(");
    let mut g = Sep::and();
    if let Some(c) = &t.id {
        let mut ids: Vec<Uuid> = Vec::new();
        if let Some(v) = &c.eq {
            ids.push(parse_uuid(v)?);
        }
        if let Some(v) = &c.in_ {
            ids.extend(parse_uuids(v)?);
        }
        g.next(qb);
        let mut all: BTreeSet<Uuid> = BTreeSet::new();
        for id in &ids {
            cx.reg(RefKind::Team, *id);
            if let Some(d) = cx.ev.descendants.get(id) {
                all.extend(d.iter().copied());
            }
        }
        if all.is_empty() {
            qb.push("FALSE");
        } else {
            qb.push(format!("{} = ANY(", e.id_col(cx)));
            qb.push_bind(all.into_iter().collect::<Vec<Uuid>>());
            qb.push(")");
        }
    }
    if let Some(l) = &t.and {
        g.next(qb);
        and_list(qb, cx, l, &|qb, cx, x| ancestors_some(qb, cx, e, x))?;
    }
    if let Some(l) = &t.or {
        g.next(qb);
        or_list(qb, cx, l, &|qb, cx, x| ancestors_some(qb, cx, e, x))?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn state_filter<'q>(
    qb: &mut Qb<'q>,
    cx: &Cx<'_>,
    e: &Ent,
    type_col: Option<String>,
    f: &WorkflowStateFilter,
) -> R {
    qb.push("(");
    let mut g = Sep::and();
    if let Some(c) = &f.id {
        g.next(qb);
        id_cmp(qb, cx, &e.id_col(cx), c, Some(RefKind::State), false)?;
    }
    if let Some(c) = &f.type_ {
        g.next(qb);
        enum_cmp(
            qb,
            &type_col.clone().unwrap_or_else(|| e.col(cx, "type")),
            c,
        )?;
    }
    if let Some(c) = &f.name {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "name"), c)?;
    }
    if let Some(c) = &f.position {
        g.next(qb);
        number_cmp(qb, &e.col(cx, "position"), c)?;
    }
    if let Some(t) = &f.team {
        g.next(qb);
        let team = e.to(cx, "team_id", "team", Vis::Team);
        team_filter(qb, cx, &team, t)?;
    }
    if let Some(l) = &f.and {
        g.next(qb);
        and_list(qb, cx, l, &|qb, cx, x| {
            state_filter(qb, cx, e, type_col.clone(), x)
        })?;
    }
    if let Some(l) = &f.or {
        g.next(qb);
        or_list(qb, cx, l, &|qb, cx, x| {
            state_filter(qb, cx, e, type_col.clone(), x)
        })?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn label_filter<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, f: &IssueLabelFilter) -> R {
    qb.push("(");
    let mut g = Sep::and();
    if let Some(c) = &f.id {
        g.next(qb);
        id_cmp(qb, cx, &e.id_col(cx), c, Some(RefKind::Label), false)?;
    }
    if let Some(c) = &f.name {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "name"), c)?;
    }
    if let Some(c) = &f.is_group {
        g.next(qb);
        bool_cmp(qb, &e.col(cx, "is_group"), c)?;
    }
    if let Some(p) = &f.parent {
        g.next(qb);
        let parent = e.to(cx, "parent_id", "issue_label", Vis::None);
        label_filter(qb, cx, &parent, p)?;
    }
    if let Some(t) = &f.team {
        g.next(qb);
        let team = e.to(cx, "team_id", "team", Vis::Team);
        team_filter(qb, cx, &team, t)?;
    }
    if let Some(l) = &f.and {
        g.next(qb);
        and_list(qb, cx, l, &|qb, cx, x| label_filter(qb, cx, e, x))?;
    }
    if let Some(l) = &f.or {
        g.next(qb);
        or_list(qb, cx, l, &|qb, cx, x| label_filter(qb, cx, e, x))?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

/// `EXISTS`-Prädikat über den Zyklus-Eintrag `e` mit zusätzlicher Bedingung.
fn cycle_pred(cx: &Cx<'_>, e: &Ent, cond: &dyn Fn(&str) -> String) -> String {
    let c = cx.alias("cy");
    format!(
        "EXISTS (SELECT 1 FROM cycle {c} WHERE {c}.id = {id} AND {cond})",
        id = e.id,
        cond = cond(&c)
    )
}

fn cycle_filter<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, f: &CycleFilter) -> R {
    qb.push("(");
    let mut g = Sep::and();
    null_check(qb, &mut g, e, f.null);
    if let Some(c) = &f.id {
        g.next(qb);
        id_cmp(qb, cx, &e.id_col(cx), c, Some(RefKind::Cycle), false)?;
    }
    if let Some(c) = &f.number {
        g.next(qb);
        number_cmp(qb, &e.col(cx, "number"), c)?;
    }
    if let Some(c) = &f.name {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "name"), c)?;
    }
    if let Some(c) = &f.starts_at {
        g.next(qb);
        date_cmp(qb, cx, &e.col(cx, "starts_at"), c)?;
    }
    if let Some(c) = &f.ends_at {
        g.next(qb);
        date_cmp(qb, cx, &e.col(cx, "ends_at"), c)?;
    }
    let virtuals: [(&Option<Box<BooleanComparator>>, String); 6] = [
        (
            &f.is_active,
            cycle_pred(cx, e, &|c| {
                format!(
                    "{c}.starts_at <= {NOW} AND {NOW} < {c}.ends_at AND {c}.completed_at IS NULL"
                )
            }),
        ),
        (
            &f.is_next,
            cycle_pred(cx, e, &|c| {
                format!(
                    "{c}.id = (SELECT n.id FROM cycle n WHERE n.team_id = {c}.team_id \
                     AND n.starts_at > {NOW} ORDER BY n.starts_at LIMIT 1)"
                )
            }),
        ),
        (
            &f.is_previous,
            cycle_pred(cx, e, &|c| {
                format!(
                    "{c}.id = (SELECT n.id FROM cycle n WHERE n.team_id = {c}.team_id \
                     AND n.ends_at <= {NOW} ORDER BY n.ends_at DESC LIMIT 1)"
                )
            }),
        ),
        (
            &f.is_past,
            cycle_pred(cx, e, &|c| format!("{c}.ends_at <= {NOW}")),
        ),
        (
            &f.is_future,
            cycle_pred(cx, e, &|c| format!("{c}.starts_at > {NOW}")),
        ),
        (
            &f.is_in_cooldown,
            cycle_pred(cx, e, &|c| {
                format!(
                    "{c}.id = (SELECT n.id FROM cycle n WHERE n.team_id = {c}.team_id \
                     AND n.ends_at <= {NOW} ORDER BY n.ends_at DESC LIMIT 1) \
                     AND NOT EXISTS (SELECT 1 FROM cycle a WHERE a.team_id = {c}.team_id \
                     AND a.starts_at <= {NOW} AND {NOW} < a.ends_at)"
                )
            }),
        ),
    ];
    for (cmp, pred) in virtuals {
        if let Some(c) = cmp {
            g.next(qb);
            bool_cmp(qb, &pred, c)?;
        }
    }
    if let Some(l) = &f.and {
        g.next(qb);
        and_list(qb, cx, l, &|qb, cx, x| cycle_filter(qb, cx, e, x))?;
    }
    if let Some(l) = &f.or {
        g.next(qb);
        or_list(qb, cx, l, &|qb, cx, x| cycle_filter(qb, cx, e, x))?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn milestone_filter<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, f: &MilestoneFilter) -> R {
    qb.push("(");
    let mut g = Sep::and();
    null_check(qb, &mut g, e, f.null);
    if let Some(c) = &f.id {
        g.next(qb);
        id_cmp(qb, cx, &e.id_col(cx), c, Some(RefKind::Milestone), false)?;
    }
    if let Some(c) = &f.name {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "name"), c)?;
    }
    if let Some(c) = &f.target_date {
        g.next(qb);
        timeless_cmp(qb, cx, &e.col(cx, "target_date"), c)?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn template_filter<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, f: &TemplateFilter) -> R {
    qb.push("(");
    let mut g = Sep::and();
    null_check(qb, &mut g, e, f.null);
    if let Some(c) = &f.id {
        g.next(qb);
        id_cmp(qb, cx, &e.id_col(cx), c, Some(RefKind::Template), false)?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn id_name_filter<'q>(
    qb: &mut Qb<'q>,
    cx: &Cx<'_>,
    e: &Ent,
    kind: RefKind,
    expand: bool,
    f: &IdNameFilter,
) -> R {
    qb.push("(");
    let mut g = Sep::and();
    if let Some(c) = &f.id {
        g.next(qb);
        id_cmp(qb, cx, &e.id_col(cx), c, Some(kind), expand)?;
    }
    if let Some(c) = &f.name {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "name"), c)?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Issue
// ---------------------------------------------------------------------------------------------

/// `hasBlockedBy…`-Prädikate: nur Relationen zu **offenen**, sichtbaren Gegenstücken zählen.
fn issue_relation_pred(cx: &Cx<'_>, e: &Ent, kind: &str) -> String {
    let (r, s) = (cx.alias("r"), cx.alias("s"));
    let id = &e.id;
    match kind {
        "blockedBy" => format!(
            "EXISTS (SELECT 1 FROM issue_relation {r} JOIN issue {s} ON {s}.id = {r}.issue_id \
             WHERE {r}.type = 'blocks' AND {r}.related_issue_id = {id} AND {r}.archived_at IS NULL \
             AND {s}.state_type NOT IN {OPEN_STATES} AND NOT {s}.trashed AND {s}.team_id IN {VIS})"
        ),
        "blocking" => format!(
            "EXISTS (SELECT 1 FROM issue_relation {r} JOIN issue {s} ON {s}.id = {r}.related_issue_id \
             WHERE {r}.type = 'blocks' AND {r}.issue_id = {id} AND {r}.archived_at IS NULL \
             AND {s}.state_type NOT IN {OPEN_STATES} AND NOT {s}.trashed AND {s}.team_id IN {VIS})"
        ),
        other => {
            let types = if other == "duplicate" {
                "('duplicate')"
            } else {
                "('related','similar')"
            };
            format!(
                "EXISTS (SELECT 1 FROM issue_relation {r} JOIN issue {s} \
                 ON {s}.id = CASE WHEN {r}.issue_id = {id} THEN {r}.related_issue_id ELSE {r}.issue_id END \
                 WHERE {r}.type IN {types} AND ({r}.issue_id = {id} OR {r}.related_issue_id = {id}) \
                 AND {r}.archived_at IS NULL AND NOT {s}.trashed AND {s}.team_id IN {VIS})"
            )
        }
    }
}

/// SLA-Status als CASE-Ausdruck (docs/03 §11.4).
fn sla_status_expr(cx: &Cx<'_>, e: &Ent) -> String {
    let br = e.col(cx, "sla_breaches_at");
    let hi = e.col(cx, "sla_high_risk_at");
    let me = e.col(cx, "sla_medium_risk_at");
    let done = format!(
        "COALESCE({}, {})",
        e.col(cx, "completed_at"),
        e.col(cx, "canceled_at")
    );
    format!(
        "(CASE WHEN {br} IS NULL THEN NULL \
         WHEN {done} IS NOT NULL THEN (CASE WHEN {done} < {br} THEN 'completed' ELSE 'failed' END) \
         WHEN {NOW} >= {br} THEN 'breached' \
         WHEN {NOW} >= {hi} THEN 'highRisk' \
         WHEN {NOW} >= {me} THEN 'mediumRisk' ELSE 'lowRisk' END)"
    )
}

/// `before`/`during`/`after`: wann wurde das Issue relativ zum Zyklus hinzugefügt.
fn added_period_expr(cx: &Cx<'_>, e: &Ent) -> String {
    let at = e.col(cx, "added_to_cycle_at");
    let cyc = e.to(cx, "cycle_id", "cycle", Vis::None);
    let (s, en) = (cyc.col(cx, "starts_at"), cyc.col(cx, "ends_at"));
    format!(
        "(CASE WHEN {at} IS NULL OR {s} IS NULL THEN NULL WHEN {at} < {s} THEN 'before' \
         WHEN {at} < {en} THEN 'during' ELSE 'after' END)"
    )
}

fn issue_filter<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, f: &IssueFilter) -> R {
    qb.push("(");
    let mut g = Sep::and();
    null_check(qb, &mut g, e, f.null);
    if let Some(c) = &f.id {
        g.next(qb);
        id_cmp(qb, cx, &e.id_col(cx), c, None, false)?;
    }
    if let Some(c) = &f.number {
        g.next(qb);
        number_cmp(qb, &e.col(cx, "number"), c)?;
    }
    if let Some(c) = &f.title {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "title"), c)?;
    }
    if let Some(c) = &f.description {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "description"), c)?;
    }
    if let Some(c) = &f.searchable_content {
        g.next(qb);
        content_cmp(qb, &e.col(cx, "search_vector"), c)?;
    }
    if let Some(c) = &f.priority {
        g.next(qb);
        number_cmp(qb, &e.col(cx, "priority"), c)?;
    }
    if let Some(c) = &f.estimate {
        g.next(qb);
        estimate_cmp(qb, &e.col(cx, "estimate"), c)?;
    }
    let dates: [(&Option<Box<DateComparator>>, &str); 12] = [
        (&f.created_at, "created_at"),
        (&f.updated_at, "updated_at"),
        (&f.archived_at, "archived_at"),
        (&f.started_at, "started_at"),
        (&f.completed_at, "completed_at"),
        (&f.canceled_at, "canceled_at"),
        (&f.triaged_at, "triaged_at"),
        (&f.auto_closed_at, "auto_closed_at"),
        (&f.auto_archived_at, "auto_archived_at"),
        (&f.added_to_cycle_at, "added_to_cycle_at"),
        (&f.snoozed_until_at, "snoozed_until_at"),
        (&f.sla_breaches_at, "sla_breaches_at"),
    ];
    for (c, col) in dates {
        if let Some(c) = c {
            g.next(qb);
            date_cmp(qb, cx, &e.col(cx, col), c)?;
        }
    }
    if let Some(c) = &f.state_entered_at {
        g.next(qb);
        date_cmp(qb, cx, &e.col(cx, "state_entered_at"), c)?;
    }
    if let Some(c) = &f.due_date {
        g.next(qb);
        timeless_cmp(qb, cx, &e.col(cx, "due_date"), c)?;
    }
    if let Some(c) = &f.sla_status {
        g.next(qb);
        enum_cmp(qb, &sla_status_expr(cx, e), c)?;
    }
    if let Some(c) = &f.added_to_cycle_period {
        g.next(qb);
        enum_cmp(qb, &added_period_expr(cx, e), c)?;
    }
    if let Some(s) = &f.state {
        g.next(qb);
        let st = e.to(cx, "state_id", "workflow_state", Vis::None);
        state_filter(qb, cx, &st, Some(e.col(cx, "state_type")), s)?;
    }
    if let Some(t) = &f.team {
        g.next(qb);
        let team = e.to(cx, "team_id", "team", Vis::None);
        team_filter(qb, cx, &team, t)?;
    }
    for (u, fk) in [
        (&f.assignee, "assignee_id"),
        (&f.creator, "creator_id"),
        (&f.delegate, "delegate_id"),
        (&f.snoozed_by, "snoozed_by_id"),
    ] {
        if let Some(u) = u {
            g.next(qb);
            let ue = e.to(cx, fk, T_USER, Vis::None);
            user_filter(qb, cx, &ue, u)?;
        }
    }
    if let Some(c) = &f.subscribers {
        g.next(qb);
        let s = cx.alias("sb");
        let spec = Coll {
            from: format!("issue_subscription {s}"),
            cond: format!(
                "{s}.issue_id = {} AND {s}.active AND {s}.thread_comment_id IS NULL AND {s}.archived_at IS NULL",
                e.id
            ),
            elem: Ent::row(&s, "issue_subscription"),
        };
        collection(qb, cx, c, &spec, &|qb, cx, row, uf: &UserFilter| {
            let ue = row.to(cx, "user_id", T_USER, Vis::None);
            user_filter(qb, cx, &ue, uf)
        })?;
    }
    if let Some(c) = &f.labels {
        g.next(qb);
        let (j, l) = (cx.alias("j"), cx.alias("l"));
        let spec = Coll {
            from: format!("issue_to_label {j} JOIN issue_label {l} ON {l}.id = {j}.label_id"),
            cond: format!("{j}.issue_id = {}", e.id),
            elem: Ent::row(&l, "issue_label"),
        };
        collection(qb, cx, c, &spec, &|qb, cx, row, lf: &IssueLabelFilter| {
            label_filter(qb, cx, row, lf)
        })?;
    }
    if let Some(cf) = &f.cycle {
        g.next(qb);
        let ce = e.to(cx, "cycle_id", "cycle", Vis::None);
        cycle_filter(qb, cx, &ce, cf)?;
    }
    if let Some(pf) = &f.project {
        g.next(qb);
        let pe = e.to(cx, "project_id", "project", Vis::None);
        project_filter(qb, cx, &pe, pf)?;
    }
    if let Some(m) = &f.project_milestone {
        g.next(qb);
        let me = e.to(cx, "project_milestone_id", "project_milestone", Vis::None);
        milestone_filter(qb, cx, &me, m)?;
    }
    if let Some(p) = &f.parent {
        g.next(qb);
        let pe = e.to(cx, "parent_id", "issue", Vis::IssueTeam);
        // `null` auf dem Parent fragt nur die eigene FK-Spalte ab (verrät nichts Fremdes).
        issue_filter(qb, cx, &pe, p)?;
    }
    if let Some(c) = &f.children {
        g.next(qb);
        let ch = cx.alias("ch");
        let spec = Coll {
            from: format!("issue {ch}"),
            cond: format!(
                "{ch}.parent_id = {} AND NOT {ch}.trashed AND {ch}.team_id IN {VIS}",
                e.id
            ),
            elem: Ent::row(&ch, "issue"),
        };
        collection(qb, cx, c, &spec, &|qb, cx, row, inner: &IssueFilter| {
            issue_filter(qb, cx, row, inner)
        })?;
    }
    if let Some(c) = &f.comments {
        g.next(qb);
        let cm = cx.alias("cm");
        let spec = Coll {
            from: format!("comment {cm}"),
            cond: format!("{cm}.issue_id = {} AND {cm}.archived_at IS NULL", e.id),
            elem: Ent::row(&cm, "comment"),
        };
        collection(qb, cx, c, &spec, &|qb, cx, row, cf: &CommentFilter| {
            comment_filter(qb, cx, row, cf)
        })?;
    }
    if let Some(c) = &f.attachments {
        g.next(qb);
        let at = cx.alias("at");
        let spec = Coll {
            from: format!("attachment {at}"),
            cond: format!("{at}.issue_id = {} AND {at}.archived_at IS NULL", e.id),
            elem: Ent::row(&at, "attachment"),
        };
        collection(qb, cx, c, &spec, &|qb, cx, row, af: &AttachmentFilter| {
            attachment_filter(qb, cx, row, af)
        })?;
    }
    for (c, kind) in [
        (&f.has_blocked_by_relations, "blockedBy"),
        (&f.has_blocking_relations, "blocking"),
        (&f.has_duplicate_relations, "duplicate"),
        (&f.has_related_relations, "related"),
    ] {
        if let Some(c) = c {
            g.next(qb);
            bool_cmp(qb, &issue_relation_pred(cx, e, kind), c)?;
        }
    }
    if let Some(t) = &f.last_applied_template {
        g.next(qb);
        let te = e.to(cx, "last_applied_template_id", "template", Vis::None);
        template_filter(qb, cx, &te, t)?;
    }
    if let Some(l) = &f.and {
        g.next(qb);
        and_list(qb, cx, l, &|qb, cx, x| issue_filter(qb, cx, e, x))?;
    }
    if let Some(l) = &f.or {
        g.next(qb);
        or_list(qb, cx, l, &|qb, cx, x| issue_filter(qb, cx, e, x))?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn comment_filter<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, f: &CommentFilter) -> R {
    qb.push("(");
    let mut g = Sep::and();
    if let Some(c) = &f.body {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "body"), c)?;
    }
    if let Some(u) = &f.user {
        g.next(qb);
        let ue = e.to(cx, "user_id", T_USER, Vis::None);
        user_filter(qb, cx, &ue, u)?;
    }
    if let Some(c) = &f.created_at {
        g.next(qb);
        date_cmp(qb, cx, &e.col(cx, "created_at"), c)?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn attachment_filter<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, f: &AttachmentFilter) -> R {
    qb.push("(");
    let mut g = Sep::and();
    for (c, col) in [
        (&f.url, "url"),
        (&f.title, "title"),
        (&f.source_type, "source_type"),
    ] {
        if let Some(c) = c {
            g.next(qb);
            string_cmp(qb, &e.col(cx, col), c)?;
        }
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Projekt und Initiative
// ---------------------------------------------------------------------------------------------

fn project_status_filter<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, f: &ProjectStatusFilter) -> R {
    qb.push("(");
    let mut g = Sep::and();
    if let Some(c) = &f.id {
        g.next(qb);
        id_cmp(
            qb,
            cx,
            &e.id_col(cx),
            c,
            Some(RefKind::ProjectStatus),
            false,
        )?;
    }
    if let Some(c) = &f.name {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "name"), c)?;
    }
    if let Some(c) = &f.type_ {
        g.next(qb);
        enum_cmp(qb, &e.col(cx, "type"), c)?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

/// Initiative ist sichtbar: nicht für Gäste; Lead-Team (falls gesetzt) sichtbar.
fn initiative_visible_cond(cx: &Cx<'_>, alias: &str) -> String {
    if cx.ev.is_guest {
        "FALSE".to_owned()
    } else {
        format!("({alias}.lead_team_id IS NULL OR {alias}.lead_team_id IN {VIS})")
    }
}

/// Projekt ist sichtbar, wenn mindestens ein beteiligtes Team sichtbar ist.
fn project_visible_cond(alias_id: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM project_team vpt WHERE vpt.project_id = {alias_id} AND vpt.team_id IN {VIS})"
    )
}

fn project_filter<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, f: &ProjectFilter) -> R {
    qb.push("(");
    let mut g = Sep::and();
    null_check(qb, &mut g, e, f.null);
    if let Some(c) = &f.id {
        g.next(qb);
        id_cmp(qb, cx, &e.id_col(cx), c, Some(RefKind::Project), false)?;
    }
    if let Some(c) = &f.name {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "name"), c)?;
    }
    if let Some(c) = &f.summary {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "description"), c)?;
    }
    if let Some(s) = &f.status {
        g.next(qb);
        let se = e.to(cx, "status_id", "project_status", Vis::None);
        project_status_filter(qb, cx, &se, s)?;
    }
    if let Some(c) = &f.priority {
        g.next(qb);
        number_cmp(qb, &e.col(cx, "priority"), c)?;
    }
    if let Some(c) = &f.health {
        g.next(qb);
        enum_cmp(qb, &e.col(cx, "health"), c)?;
    }
    for (u, fk) in [(&f.lead, "lead_id"), (&f.creator, "creator_id")] {
        if let Some(u) = u {
            g.next(qb);
            let ue = e.to(cx, fk, T_USER, Vis::None);
            user_filter(qb, cx, &ue, u)?;
        }
    }
    if let Some(c) = &f.members {
        g.next(qb);
        let pm = cx.alias("pm");
        let spec = Coll {
            from: format!("project_member {pm}"),
            cond: format!("{pm}.project_id = {}", e.id),
            elem: Ent::row(&pm, "project_member"),
        };
        collection(qb, cx, c, &spec, &|qb, cx, row, uf: &UserFilter| {
            let ue = row.to(cx, "user_id", T_USER, Vis::None);
            user_filter(qb, cx, &ue, uf)
        })?;
    }
    if let Some(c) = &f.teams {
        g.next(qb);
        let pt = cx.alias("pt");
        let spec = Coll {
            from: format!("project_team {pt}"),
            cond: format!("{pt}.project_id = {} AND {pt}.team_id IN {VIS}", e.id),
            elem: Ent::row(&pt, "project_team"),
        };
        collection(qb, cx, c, &spec, &|qb, cx, row, tf: &TeamFilter| {
            let te = row.to(cx, "team_id", "team", Vis::Team);
            team_filter(qb, cx, &te, tf)
        })?;
    }
    if let Some(t) = &f.lead_team {
        g.next(qb);
        let te = e.to(cx, "lead_team_id", "team", Vis::Team);
        team_filter(qb, cx, &te, t)?;
    }
    if let Some(c) = &f.labels {
        g.next(qb);
        let (j, l) = (cx.alias("pj"), cx.alias("pl"));
        let spec = Coll {
            from: format!("project_to_label {j} JOIN project_label {l} ON {l}.id = {j}.label_id"),
            cond: format!("{j}.project_id = {}", e.id),
            elem: Ent::row(&l, "project_label"),
        };
        collection(qb, cx, c, &spec, &|qb, cx, row, nf: &IdNameFilter| {
            id_name_filter(qb, cx, row, RefKind::ProjectLabel, false, nf)
        })?;
    }
    if let Some(c) = &f.initiatives {
        g.next(qb);
        let (j, i) = (cx.alias("ip"), cx.alias("in"));
        let spec = Coll {
            from: format!(
                "initiative_to_project {j} JOIN initiative {i} ON {i}.id = {j}.initiative_id"
            ),
            cond: format!(
                "{j}.project_id = {} AND NOT {i}.trashed AND {}",
                e.id,
                initiative_visible_cond(cx, &i)
            ),
            elem: Ent::row(&i, "initiative"),
        };
        collection(qb, cx, c, &spec, &|qb, cx, row, nf: &IdNameFilter| {
            id_name_filter(qb, cx, row, RefKind::Initiative, true, nf)
        })?;
    }
    if let Some(c) = &f.project_milestones {
        g.next(qb);
        let m = cx.alias("pms");
        let spec = Coll {
            from: format!("project_milestone {m}"),
            cond: format!("{m}.project_id = {} AND {m}.archived_at IS NULL", e.id),
            elem: Ent::row(&m, "project_milestone"),
        };
        collection(qb, cx, c, &spec, &|qb, cx, row, mf: &MilestoneFilter| {
            milestone_filter(qb, cx, row, mf)
        })?;
    }
    for (c, col) in [
        (&f.start_date, "start_date"),
        (&f.target_date, "target_date"),
    ] {
        if let Some(c) = c {
            g.next(qb);
            timeless_cmp(qb, cx, &e.col(cx, col), c)?;
        }
    }
    let dates: [(&Option<Box<DateComparator>>, &str); 6] = [
        (&f.created_at, "created_at"),
        (&f.updated_at, "updated_at"),
        (&f.started_at, "started_at"),
        (&f.completed_at, "completed_at"),
        (&f.canceled_at, "canceled_at"),
        (&f.health_updated_at, "health_updated_at"),
    ];
    for (c, col) in dates {
        if let Some(c) = c {
            g.next(qb);
            date_cmp(qb, cx, &e.col(cx, col), c)?;
        }
    }
    for (c, blocked_by) in [
        (&f.has_blocked_by_relations, true),
        (&f.has_blocking_relations, false),
    ] {
        if let Some(c) = c {
            g.next(qb);
            let (r, s, st) = (cx.alias("pr"), cx.alias("ps"), cx.alias("pst"));
            let (own, other) = if blocked_by {
                ("related_project_id", "project_id")
            } else {
                ("project_id", "related_project_id")
            };
            let pred = format!(
                "EXISTS (SELECT 1 FROM project_relation {r} JOIN project {s} ON {s}.id = {r}.{other} \
                 JOIN project_status {st} ON {st}.id = {s}.status_id \
                 WHERE {r}.type = 'blocks' AND {r}.{own} = {} AND {r}.archived_at IS NULL \
                 AND NOT {s}.trashed AND {st}.type NOT IN ('completed','canceled') AND {})",
                e.id,
                project_visible_cond(&format!("{s}.id"))
            );
            bool_cmp(qb, &pred, c)?;
        }
    }
    if let Some(t) = &f.last_applied_template {
        g.next(qb);
        let te = e.to(cx, "last_applied_template_id", "template", Vis::None);
        template_filter(qb, cx, &te, t)?;
    }
    if let Some(l) = &f.and {
        g.next(qb);
        and_list(qb, cx, l, &|qb, cx, x| project_filter(qb, cx, e, x))?;
    }
    if let Some(l) = &f.or {
        g.next(qb);
        or_list(qb, cx, l, &|qb, cx, x| project_filter(qb, cx, e, x))?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

fn initiative_filter<'q>(qb: &mut Qb<'q>, cx: &Cx<'_>, e: &Ent, f: &InitiativeFilter) -> R {
    qb.push("(");
    let mut g = Sep::and();
    if let Some(c) = &f.id {
        g.next(qb);
        id_cmp(qb, cx, &e.id_col(cx), c, Some(RefKind::Initiative), false)?;
    }
    if let Some(c) = &f.name {
        g.next(qb);
        string_cmp(qb, &e.col(cx, "name"), c)?;
    }
    if let Some(c) = &f.status {
        g.next(qb);
        enum_cmp(qb, &e.col(cx, "status"), c)?;
    }
    if let Some(c) = &f.priority {
        g.next(qb);
        number_cmp(qb, &e.col(cx, "priority"), c)?;
    }
    if let Some(c) = &f.health {
        g.next(qb);
        enum_cmp(qb, &e.col(cx, "health"), c)?;
    }
    for (u, fk) in [(&f.owner, "owner_id"), (&f.creator, "creator_id")] {
        if let Some(u) = u {
            g.next(qb);
            let ue = e.to(cx, fk, T_USER, Vis::None);
            user_filter(qb, cx, &ue, u)?;
        }
    }
    if let Some(t) = &f.lead_team {
        g.next(qb);
        let te = e.to(cx, "lead_team_id", "team", Vis::Team);
        team_filter(qb, cx, &te, t)?;
    }
    if let Some(c) = &f.labels {
        g.next(qb);
        let (j, l) = (cx.alias("ij"), cx.alias("il"));
        let spec = Coll {
            from: format!(
                "initiative_to_label {j} JOIN initiative_label {l} ON {l}.id = {j}.label_id"
            ),
            cond: format!("{j}.initiative_id = {}", e.id),
            elem: Ent::row(&l, "initiative_label"),
        };
        collection(qb, cx, c, &spec, &|qb, cx, row, nf: &IdNameFilter| {
            id_name_filter(qb, cx, row, RefKind::InitiativeLabel, false, nf)
        })?;
    }
    if let Some(c) = &f.target_date {
        g.next(qb);
        timeless_cmp(qb, cx, &e.col(cx, "target_date"), c)?;
    }
    for (c, col) in [(&f.created_at, "created_at"), (&f.updated_at, "updated_at")] {
        if let Some(c) = c {
            g.next(qb);
            date_cmp(qb, cx, &e.col(cx, col), c)?;
        }
    }
    for (c, from_col, elem_col) in [
        (&f.parent_initiatives, "child_id", "parent_id"),
        (&f.sub_initiatives, "parent_id", "child_id"),
    ] {
        if let Some(c) = c {
            g.next(qb);
            let (j, i) = (cx.alias("ie"), cx.alias("in"));
            let spec = Coll {
                from: format!(
                    "initiative_parent {j} JOIN initiative {i} ON {i}.id = {j}.{elem_col}"
                ),
                cond: format!(
                    "{j}.{from_col} = {} AND NOT {i}.trashed AND {}",
                    e.id,
                    initiative_visible_cond(cx, &i)
                ),
                elem: Ent::row(&i, "initiative"),
            };
            collection(qb, cx, c, &spec, &|qb, cx, row, nf: &IdNameFilter| {
                id_name_filter(qb, cx, row, RefKind::Initiative, false, nf)
            })?;
        }
    }
    if let Some(c) = &f.projects {
        g.next(qb);
        let (j, p) = (cx.alias("ip"), cx.alias("pr"));
        let spec = Coll {
            from: format!("initiative_to_project {j} JOIN project {p} ON {p}.id = {j}.project_id"),
            cond: format!(
                "{j}.initiative_id = {} AND NOT {p}.trashed AND {}",
                e.id,
                project_visible_cond(&format!("{p}.id"))
            ),
            elem: Ent::row(&p, "project"),
        };
        collection(qb, cx, c, &spec, &|qb, cx, row, nf: &IdNameFilter| {
            id_name_filter(qb, cx, row, RefKind::Project, false, nf)
        })?;
    }
    if let Some(l) = &f.and {
        g.next(qb);
        and_list(qb, cx, l, &|qb, cx, x| initiative_filter(qb, cx, e, x))?;
    }
    if let Some(l) = &f.or {
        g.next(qb);
        or_list(qb, cx, l, &|qb, cx, x| initiative_filter(qb, cx, e, x))?;
    }
    g.close(qb, "TRUE");
    qb.push(")");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Öffentliche Einstiege
// ---------------------------------------------------------------------------------------------

/// Schreibt das Prädikat eines `IssueFilter` über der Issue-Zeile `alias` (Tabelle `issue`).
/// Muss in einer Abfrage stehen, die mit [`push_cte_prefix`] beginnt.
pub fn push_issue_filter<'q>(
    qb: &mut Qb<'q>,
    cx: &Cx<'_>,
    alias: &str,
    f: &IssueFilter,
) -> AppResult<()> {
    let e = Ent {
        id: format!("{alias}.id"),
        alias: Some(alias.to_owned()),
        table: "issue",
        vis: Vis::None,
    };
    issue_filter(qb, cx, &e, f)
}

/// Prädikat eines `ProjectFilter` über der Projektzeile `alias`.
pub fn push_project_filter<'q>(
    qb: &mut Qb<'q>,
    cx: &Cx<'_>,
    alias: &str,
    f: &ProjectFilter,
) -> AppResult<()> {
    project_filter(qb, cx, &Ent::row(alias, "project"), f)
}

/// Prädikat eines `InitiativeFilter` über der Initiativzeile `alias`.
pub fn push_initiative_filter<'q>(
    qb: &mut Qb<'q>,
    cx: &Cx<'_>,
    alias: &str,
    f: &InitiativeFilter,
) -> AppResult<()> {
    initiative_filter(qb, cx, &Ent::row(alias, "initiative"), f)
}

/// Sammelt die Referenzen eines Filters, ohne eine echte Abfrage zu bauen.
pub fn collect_issue_refs(ev: &FilterEval, f: &IssueFilter) -> AppResult<FilterRefs> {
    let cx = Cx::new(ev);
    let mut qb: Qb<'_> = QueryBuilder::new("");
    push_issue_filter(&mut qb, &cx, "i", f)?;
    Ok(cx.into_refs())
}

/// Wie [`collect_issue_refs`] für Projektfilter.
pub fn collect_project_refs(ev: &FilterEval, f: &ProjectFilter) -> AppResult<FilterRefs> {
    let cx = Cx::new(ev);
    let mut qb: Qb<'_> = QueryBuilder::new("");
    push_project_filter(&mut qb, &cx, "p", f)?;
    Ok(cx.into_refs())
}

/// Wie [`collect_issue_refs`] für Initiativfilter.
pub fn collect_initiative_refs(ev: &FilterEval, f: &InitiativeFilter) -> AppResult<FilterRefs> {
    let cx = Cx::new(ev);
    let mut qb: Qb<'_> = QueryBuilder::new("");
    push_initiative_filter(&mut qb, &cx, "n", f)?;
    Ok(cx.into_refs())
}

/// `mentionsTriage` (docs/05 §1.10 Regel 3): Der Filter nennt irgendwo `state.type` mit dem
/// Wert `triage` (`eq`/`in`) – dann entfällt die implizite Triage-Ausblendung.
pub fn mentions_triage_type(f: &IssueFilter) -> bool {
    let hit = |c: &EnumComparator| {
        c.eq.as_deref() == Some("triage")
            || c.in_
                .as_ref()
                .is_some_and(|v| v.iter().any(|s| s == "triage"))
    };
    f.state
        .as_ref()
        .and_then(|s| s.type_.as_ref())
        .is_some_and(|c| hit(c))
        || f.and.iter().flatten().any(mentions_triage_type)
        || f.or.iter().flatten().any(mentions_triage_type)
}

/// Status-IDs, die der Filter positiv nennt (`state.id` `eq`/`in`); ein Issue in Triage mit
/// einer dieser IDs bleibt sichtbar (UI-Filter „Status is Triage“).
pub fn mentioned_state_ids(f: &IssueFilter) -> Vec<Uuid> {
    let mut out = Vec::new();
    if let Some(c) = f.state.as_ref().and_then(|s| s.id.as_ref()) {
        if let Some(v) = &c.eq {
            out.extend(Uuid::parse_str(v).ok());
        }
        if let Some(v) = &c.in_ {
            out.extend(v.iter().filter_map(|s| Uuid::parse_str(s).ok()));
        }
    }
    for sub in f.and.iter().flatten().chain(f.or.iter().flatten()) {
        out.extend(mentioned_state_ids(sub));
    }
    out
}

/// Hängt die implizite Triage-Ausblendung an (` AND (state_type <> 'triage' OR …)`).
pub fn push_triage_guard<'q>(qb: &mut Qb<'q>, alias: &str, f: &IssueFilter) {
    if mentions_triage_type(f) {
        return;
    }
    let ids = mentioned_state_ids(f);
    qb.push(format!(" AND ({alias}.state_type <> 'triage'"));
    if !ids.is_empty() {
        qb.push(format!(" OR {alias}.state_id = ANY("));
        qb.push_bind(ids);
        qb.push(")");
    }
    qb.push(")");
}

#[cfg(test)]
mod tests {
    use super::*;
    use lineal_domain::rules::filter::parse_issue_filter;

    fn ev() -> FilterEval {
        FilterEval {
            org: lineal_domain::ids::OrgId::nil(),
            is_guest: false,
            now: "2026-10-10T07:15:00Z".parse().unwrap_or_default(),
            timezone: "Europe/Berlin".into(),
            me: None,
            visible_teams: Vec::new(),
            descendants: BTreeMap::new(),
            initiative_edges: Vec::new(),
            strict_refs: false,
        }
    }

    fn sql(json: &str) -> String {
        let f = parse_issue_filter(json).unwrap();
        let e = ev();
        let cx = Cx::new(&e);
        let mut qb: Qb<'_> = QueryBuilder::new("");
        push_issue_filter(&mut qb, &cx, "i", &f).unwrap();
        qb.sql().to_string()
    }

    #[test]
    fn values_are_never_interpolated() {
        let evil = "x'); DROP TABLE issue; --";
        let s = sql(&format!(
            r#"{{"title":{{"containsIgnoreCase":"{evil}"}},"assignee":{{"name":{{"eq":"{evil}"}}}}}}"#
        ));
        assert!(!s.contains("DROP"), "{s}");
        assert!(s.contains("$1"), "{s}");
    }

    #[test]
    fn empty_in_is_false_and_empty_nin_is_true() {
        assert!(sql(r#"{"priority":{"in":[]}}"#).contains("FALSE"));
        assert!(sql(r#"{"priority":{"nin":[]}}"#).contains("TRUE"));
    }

    #[test]
    fn accents_fold_the_same_on_both_sides() {
        assert_eq!(fold_accents("Müller-Ärger"), "muller-arger");
    }

    #[test]
    fn ancestors_with_other_fields_is_rejected() {
        let f =
            parse_issue_filter(r#"{"team":{"ancestors":{"some":{"key":{"eq":"ENG"}}}}}"#).unwrap();
        let e = ev();
        let cx = Cx::new(&e);
        let mut qb: Qb<'_> = QueryBuilder::new("");
        assert!(push_issue_filter(&mut qb, &cx, "i", &f).is_err());
    }

    #[test]
    fn email_filter_is_rejected() {
        let f = parse_issue_filter(r#"{"assignee":{"email":{"eq":"a@b.c"}}}"#).unwrap();
        let e = ev();
        let cx = Cx::new(&e);
        let mut qb: Qb<'_> = QueryBuilder::new("");
        assert!(push_issue_filter(&mut qb, &cx, "i", &f).is_err());
    }
}
