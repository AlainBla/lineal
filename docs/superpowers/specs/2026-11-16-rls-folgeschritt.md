# RLS als Folgeschritt zu LIN-61 (S-09)

Status: **aufgeschoben** (Design-Notiz, nicht umgesetzt). LIN-61 liefert die De-Privilegierung der
App-Rolle; Row-Level-Security (RLS) ist ein eigener Folge-Task.

## Was LIN-61 bereits leistet

- Server und Worker verbinden sich als Rolle `lineal_app` (nur DML, kein DDL/Superuser/Ownership).
- Die Rolle trägt `NOBYPASSRLS` — RLS-Policies würden also sofort greifen, sobald sie existieren.
- Migrationen und Betreiber-Befehle laufen weiter als Eigentuemer `lineal`.

Damit ist die Angriffsfläche einer SQL-Injection oder eines App-Bugs auf DML begrenzt. Die
**Mandantentrennung** (eine Query ohne `organization_id`-Filter sieht fremde Zeilen) hängt aber
weiterhin allein am Anwendungscode. RLS würde das in der Datenbank erzwingen.

## Warum nicht jetzt (Risiko/Aufwand)

- Nahezu jede Tabelle trägt `organization_id`; Policies auf allen (~60) Tabellen sind invasiv.
- Mehrere Zugriffe laufen bewusst mandantenübergreifend bzw. ohne Org-Kontext: Outbox-Konsumenten
  und Jobs (`begin_system`, `OrgId::nil()`), `sync_seq`, Infrastruktur-Tabellen (`outbox_event`,
  `outbox_delivery`, `job`), Login/Account-Auflösung vor bekanntem Org-Kontext, Import.
- Eine zu strikte Policy bricht diese Pfade still (0 Zeilen statt Fehler) — in Prod schwer zu
  erkennen. Halb umgesetzte RLS ist schlechter als keine. Deshalb bewusst getrennt.

## Vorgeschlagener Entwurf (für den Folge-Task)

1. **Org-Kontext je Transaktion über GUC.** In `PgStore::begin(org)` zusätzlich
   `SET LOCAL app.current_org = '<uuid>'` setzen (transaktionslokal, kein Leak zwischen Pool-
   Nutzern). `begin_system()` setzt eine Kennung, die als „alle Orgs / System“ gilt.
2. **Policies.** Pro mandantengebundener Tabelle:
   `ALTER TABLE t ENABLE ROW LEVEL SECURITY;`
   `CREATE POLICY t_org ON t USING (organization_id = current_setting('app.current_org')::uuid);`
   Für Schreibzugriffe zusätzlich `WITH CHECK (…)`. Separate Policy/Ausnahme für die
   System-/Infrastruktur-Tabellen und den `begin_system`-Pfad.
3. **Eigentuemer umgehen bewusst.** `lineal` (Migrationen) ist Superuser bzw. müsste sonst
   `BYPASSRLS` tragen; die Policies greifen nur für `lineal_app`.
4. **Reihenfolge.** Erst GUC in `begin` setzen (ohne Policies wirkungslos, risikolos ausrollbar),
   dann Policies tabellenweise mit Tests je Pfad — besonders `begin_system`, Outbox, Jobs, Import,
   Login — nachziehen. Rollback je Tabelle: `DISABLE ROW LEVEL SECURITY`.
5. **Tests.** Je Tabelle: mit gesetztem `app.current_org` nur eigene Zeilen sichtbar; fremde Org
   unsichtbar; `begin_system`-Pfad weiterhin vollständig.

## Offene Fragen

- `current_setting('app.current_org', true)` (missing_ok) vs. hartes Fehlen → bewusst Fehler, damit
  ein nicht gesetzter Kontext nicht still alle/keine Zeilen liefert.
- Performance: zusätzliche Policy-Prädikate auf heißen Pfaden messen (Index auf `organization_id`
  ist vorhanden).
