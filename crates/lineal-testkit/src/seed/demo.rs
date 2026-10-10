//! Demo-Workspace für die Web-UI (`lineal seed --demo`): erfundene, realistische Daten in
//! `acme` (ENG, WEB, DES) – ≈30 Issues mit Status, Prioritäten, Labels, Sub-Issues und
//! Kommentaren, zwei Projekte mit Meilensteinen, der aktive Cycle von ENG, zwei Templates und
//! zwei Dokumente.
//!
//! Alles läuft über die Use-Cases (`App`), nicht über Roh-SQL: Nummern, Kennungen, History,
//! Events und Sichtbarkeitsregeln entsprechen dem Normalbetrieb. `seed::run` bleibt unverändert;
//! `run` setzt voraus, dass der Basis-Workspace bereits angelegt ist.

use super::{SeedUser, ctx_for};
use chrono::NaiveDate;
use lineal_app::usecases::comment::{CreateCommentInput, ParentSpec};
use lineal_app::usecases::document::{ParentRefs, SaveDocumentInput};
use lineal_app::usecases::issue::CreateIssueInput;
use lineal_app::usecases::project::SaveProjectInput;
use lineal_app::usecases::project::milestone::SaveMilestoneInput;
use lineal_app::usecases::template::SaveTemplateInput;
use lineal_app::{App, AppResult, RequestCtx};
use lineal_domain::enums::TemplateType;
use serde_json::json;

/// Ein Demo-Issue: `(Team, Titel, Status, Priorität, Labels, Assignee, Projekt, Parent-Index, Cycle)`.
struct Spec {
    team: &'static str,
    title: &'static str,
    state: &'static str,
    priority: i64,
    labels: &'static [&'static str],
    assignee: Option<&'static str>,
    project: Option<&'static str>,
    /// Index eines früheren Eintrags (Sub-Issue).
    parent: Option<usize>,
    cycle: bool,
    description: &'static str,
}

const fn s(
    team: &'static str,
    title: &'static str,
    state: &'static str,
    priority: i64,
    labels: &'static [&'static str],
    assignee: Option<&'static str>,
) -> Spec {
    Spec {
        team,
        title,
        state,
        priority,
        labels,
        assignee,
        project: None,
        parent: None,
        cycle: false,
        description: "",
    }
}

fn specs() -> Vec<Spec> {
    let co = Some("P-ENG-1");
    vec![
        // --- ENG (0..) ---
        Spec {
            project: co,
            cycle: true,
            description: "Der neue Checkout soll Wallets (Apple Pay, Google Pay) unterstützen.\n\n- [ ] Provider-API anbinden\n- [ ] Fehlerfälle abdecken",
            ..s(
                "ENG",
                "Wallet-Zahlungen im Checkout anbieten",
                "In Progress",
                2,
                &["Feature"],
                Some("alice"),
            )
        },
        Spec {
            parent: Some(0),
            cycle: true,
            project: co,
            ..s(
                "ENG",
                "Provider-SDK einbinden",
                "Done",
                2,
                &["Feature"],
                Some("bob"),
            )
        },
        Spec {
            parent: Some(0),
            cycle: true,
            project: co,
            ..s(
                "ENG",
                "Fehlerbehandlung bei abgelehnter Zahlung",
                "In Progress",
                2,
                &["Improvement"],
                Some("alice"),
            )
        },
        Spec {
            parent: Some(0),
            project: co,
            ..s(
                "ENG",
                "Wallet-Button im Warenkorb",
                "Todo",
                3,
                &["Feature"],
                Some("bob"),
            )
        },
        Spec {
            project: co,
            cycle: true,
            description: "Bei Doppelklick auf „Bezahlen“ entstehen zwei Bestellungen.",
            ..s(
                "ENG",
                "Doppelte Bestellung bei Doppelklick",
                "In Progress",
                1,
                &["Bug"],
                Some("bob"),
            )
        },
        Spec {
            project: co,
            ..s(
                "ENG",
                "Gutscheincodes validieren",
                "Todo",
                3,
                &["Feature"],
                Some("alice"),
            )
        },
        Spec {
            cycle: true,
            ..s(
                "ENG",
                "API-Rate-Limits pro Token einführen",
                "Todo",
                2,
                &["Improvement"],
                Some("alice"),
            )
        },
        Spec {
            description: "p95 der Suche liegt bei 2,4 s.",
            ..s(
                "ENG",
                "Produktsuche ist bei großen Katalogen langsam",
                "Backlog",
                3,
                &["Bug"],
                None,
            )
        },
        s(
            "ENG",
            "Alte Webhook-Endpunkte entfernen",
            "Backlog",
            4,
            &["Improvement"],
            None,
        ),
        s(
            "ENG",
            "Build-Cache in CI aktivieren",
            "Done",
            3,
            &["Improvement"],
            Some("bob"),
        ),
        s(
            "ENG",
            "Staging-Datenbank nachts zurücksetzen",
            "Canceled",
            4,
            &[],
            None,
        ),
        Spec {
            cycle: true,
            ..s(
                "ENG",
                "Login-Fehler nach Passwort-Reset",
                "Todo",
                1,
                &["Bug"],
                Some("bob"),
            )
        },
        // --- WEB ---
        Spec {
            project: Some("Marketing-Site Relaunch"),
            ..s(
                "WEB",
                "Startseite neu aufbauen",
                "In Progress",
                2,
                &["Feature"],
                Some("bob"),
            )
        },
        Spec {
            parent: Some(12),
            project: Some("Marketing-Site Relaunch"),
            ..s(
                "WEB",
                "Hero-Bereich mit neuem Claim",
                "Done",
                3,
                &["Feature"],
                Some("bob"),
            )
        },
        Spec {
            parent: Some(12),
            project: Some("Marketing-Site Relaunch"),
            ..s(
                "WEB",
                "Preistabelle responsiv machen",
                "In Progress",
                2,
                &["Improvement"],
                Some("carol"),
            )
        },
        Spec {
            project: Some("Marketing-Site Relaunch"),
            ..s(
                "WEB",
                "Cookie-Banner an neue Richtlinie anpassen",
                "Todo",
                2,
                &["Improvement"],
                Some("bob"),
            )
        },
        Spec {
            project: Some("Marketing-Site Relaunch"),
            ..s(
                "WEB",
                "Lighthouse-Score über 90 bringen",
                "Backlog",
                3,
                &["Improvement"],
                None,
            )
        },
        Spec {
            description: "Auf iOS 17 verschwindet das Menü nach dem Scrollen.",
            ..s(
                "WEB",
                "Mobile Navigation hängt in Safari",
                "Todo",
                1,
                &["Bug"],
                Some("bob"),
            )
        },
        s(
            "WEB",
            "Dark Mode für das Kundenportal",
            "Backlog",
            3,
            &["Feature"],
            None,
        ),
        s(
            "WEB",
            "Sitemap automatisch generieren",
            "Done",
            4,
            &["Improvement"],
            Some("bob"),
        ),
        s(
            "WEB",
            "Tippfehler in der Fußzeile",
            "Done",
            4,
            &["Bug"],
            Some("carol"),
        ),
        Spec {
            project: Some("Marketing-Site Relaunch"),
            ..s(
                "WEB",
                "Blog-Layout vereinfachen",
                "Backlog",
                4,
                &["Improvement"],
                None,
            )
        },
        // --- DES (privat) ---
        Spec {
            project: Some("Marketing-Site Relaunch"),
            ..s(
                "DES",
                "Neue Bildsprache für den Relaunch",
                "In Progress",
                2,
                &["Feature"],
                Some("bob"),
            )
        },
        Spec {
            parent: Some(22),
            ..s(
                "DES",
                "Moodboard mit drei Richtungen",
                "Done",
                3,
                &[],
                Some("bob"),
            )
        },
        Spec {
            parent: Some(22),
            ..s(
                "DES",
                "Icon-Set auf 24 px vereinheitlichen",
                "Todo",
                3,
                &["Improvement"],
                Some("bob"),
            )
        },
        s(
            "DES",
            "Farbpalette auf Kontrast prüfen",
            "Todo",
            2,
            &["Improvement"],
            Some("bob"),
        ),
        s(
            "DES",
            "Illustrationen für Fehlerseiten",
            "Backlog",
            4,
            &["Feature"],
            None,
        ),
        s(
            "DES",
            "Designsystem-Doku aktualisieren",
            "Backlog",
            3,
            &["Improvement"],
            Some("bob"),
        ),
        s(
            "DES",
            "Altes Logo aus dem Footer entfernen",
            "Canceled",
            4,
            &[],
            None,
        ),
    ]
}

fn user_for(team: &str) -> SeedUser {
    if team == "DES" {
        SeedUser::Bob
    } else {
        SeedUser::Alice
    }
}

fn date(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(y, m, d)
}

/// Legt die Demo-Daten an. Voraussetzung: `seed::run` ist gelaufen.
pub async fn run(app: &App) -> AppResult<()> {
    let alice: RequestCtx = ctx_for(SeedUser::Alice);
    let bob: RequestCtx = ctx_for(SeedUser::Bob);
    let carol: RequestCtx = ctx_for(SeedUser::Carol);

    // Projekte: das Seed-Projekt `P-ENG-1` bekommt WEB als zweites Team und einen weiteren Meilenstein;
    // „Marketing-Site Relaunch“ (Lead WEB, mit DES) ist neu.
    app.save_project(
        &alice,
        SaveProjectInput {
            id: Some("P-ENG-1".into()),
            add_teams: vec!["WEB".into()],
            target_date: Some(date(2026, 12, 15)),
            ..Default::default()
        },
    )
    .await?;
    app.save_milestone(
        &alice,
        SaveMilestoneInput {
            project: "P-ENG-1".into(),
            name: Some("Beta".into()),
            description: Some("Wallets für 10 % der Kunden freischalten.".into()),
            target_date: Some(date(2026, 11, 20)),
            ..Default::default()
        },
    )
    .await?;
    app.save_project(
        &bob,
        SaveProjectInput {
            name: Some("Marketing-Site Relaunch".into()),
            summary: Some("Neue Marketing-Site mit überarbeitetem Design".into()),
            description: Some("## Ziel\nRelaunch bis Ende Q4 mit besserer Performance.".into()),
            add_teams: vec!["WEB".into(), "DES".into()],
            lead_team: Some("WEB".into()),
            lead: Some(Some("bob".into())),
            state: Some("started".into()),
            priority: Some(2),
            color: Some("#26B5CE".into()),
            target_date: Some(date(2026, 12, 18)),
            ..Default::default()
        },
    )
    .await?;
    for (name, target) in [
        ("Design-Freigabe", date(2026, 11, 6)),
        ("Go-Live", date(2026, 12, 18)),
    ] {
        app.save_milestone(
            &bob,
            SaveMilestoneInput {
                project: "Marketing-Site Relaunch".into(),
                name: Some(name.into()),
                target_date: Some(target),
                ..Default::default()
            },
        )
        .await?;
    }

    // Issues (in Reihenfolge; Sub-Issues verweisen auf ihren Parent).
    let mut ids: Vec<String> = Vec::new();
    for sp in specs() {
        let ctx = if user_for(sp.team) == SeedUser::Bob {
            &bob
        } else {
            &alice
        };
        let view = app
            .create_issue(
                ctx,
                CreateIssueInput {
                    team: sp.team.into(),
                    title: Some(sp.title.into()),
                    description: (!sp.description.is_empty()).then(|| sp.description.into()),
                    state: Some(sp.state.into()),
                    priority: Some(sp.priority),
                    assignee: sp.assignee.map(str::to_owned),
                    labels: (!sp.labels.is_empty())
                        .then(|| sp.labels.iter().map(|l| (*l).to_owned()).collect()),
                    project: sp.project.map(str::to_owned),
                    parent: sp.parent.map(|i| ids[i].clone()),
                    cycle: sp.cycle.then(|| "2".to_owned()),
                    ..Default::default()
                },
            )
            .await?;
        ids.push(view.issue.identifier.clone());
    }

    // Kommentare (nur von Usern mit Zugriff auf das Team).
    let comments: [(usize, &RequestCtx, &str); 8] = [
        (
            0,
            &bob,
            "Der Provider will eine Domain-Verifizierung – ich kümmere mich darum.",
        ),
        (
            0,
            &alice,
            "Danke! Bitte bis Freitag, dann können wir in den Cycle-Review gehen.",
        ),
        (
            4,
            &bob,
            "Reproduzierbar mit langsamem Netz. Ich sperre den Button nach dem ersten Klick.",
        ),
        (
            4,
            &alice,
            "Wir sollten zusätzlich serverseitig idempotent sein (Idempotency-Key).",
        ),
        (
            12,
            &carol,
            "Die Preistabelle bricht unter 360 px um – Screenshot folgt.",
        ),
        (
            17,
            &bob,
            "Betrifft nur Safari 17.x. Workaround: `position: sticky` entfernen.",
        ),
        (
            23,
            &bob,
            "Drei Richtungen sind im Figma-Board, Feedback bitte bis Mittwoch.",
        ),
        (
            25,
            &bob,
            "Kontrast der Sekundärfarbe liegt bei 3,8:1 – das reicht nicht für Fließtext.",
        ),
    ];
    for (i, who, body) in comments {
        app.create_comment(
            who,
            CreateCommentInput {
                parent: Some(ParentSpec::Issue(ids[i].clone())),
                reply_to: None,
                body: body.into(),
                quoted_text: None,
            },
        )
        .await?;
    }

    // Templates: ein Workspace-Template (Dokument) und ein Team-Template (Issue mit Sub-Issues).
    app.save_template(
        &alice,
        SaveTemplateInput {
            template_type: Some(TemplateType::Document),
            name: Some("Technische Spezifikation".into()),
            description: Some("Gliederung für Specs und RFCs".into()),
            template_data: Some(json!({
                "title": "Spezifikation: ",
                "content": "## Kontext\n{{placeholder: Warum brauchen wir das?}}\n\n## Lösung\n{{placeholder: Wie soll es funktionieren?}}\n\n## Offene Fragen\n",
                "icon": "FileText"
            })),
            ..Default::default()
        },
    )
    .await?;
    app.save_template(
        &alice,
        SaveTemplateInput {
            template_type: Some(TemplateType::Issue),
            name: Some("Neues Feature".into()),
            team: Some("ENG".into()),
            description: Some("Feature mit Design, Umsetzung und Review".into()),
            template_data: Some(json!({
                "title": "Feature: ",
                "description": "## Ziel\n{{placeholder: Was soll der Nutzer danach können?}}\n\n## Akzeptanzkriterien\n- [ ] {{placeholder: Kriterium}}",
                "stateType": "backlog",
                "priority": 3,
                "subIssues": [
                    { "title": "Design abstimmen" },
                    { "title": "Umsetzung" },
                    { "title": "Tests und Review", "dueIn": "P10D" }
                ]
            })),
            ..Default::default()
        },
    )
    .await?;

    // Dokumente: Spezifikation am Projekt, Onboarding am Team.
    app.save_document(
        &alice,
        SaveDocumentInput {
            title: Some("Checkout-Spezifikation".into()),
            content: Some("## Kontext\nDer bestehende Checkout konvertiert schlecht auf Mobilgeräten.\n\n## Lösung\nEin einseitiger Checkout mit Wallets und gespeicherten Adressen.\n\n## Offene Fragen\n- Welche Provider sind im Q4 realistisch?\n".into()),
            icon: Some("FileText".into()),
            parent: ParentRefs {
                project: Some("P-ENG-1".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await?;
    app.save_document(
        &bob,
        SaveDocumentInput {
            title: Some("Onboarding Web-Team".into()),
            content: Some("## Willkommen\nRepo klonen, `npm ci`, `npm run dev`.\n\n## Arbeitsweise\n- Kleine Pull Requests\n- Review innerhalb eines Arbeitstags\n".into()),
            parent: ParentRefs {
                team: Some("WEB".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}
