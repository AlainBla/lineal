# 16 – Bots & API-Keys

Diese Anleitung beschreibt, wie ein Workspace-Admin einen **Bot** anlegt und mit einem **API-Key** ausstattet, damit automatisch erstellte Issues und Kommentare als der Bot und **nicht als die anlegende Person** erscheinen. Feature: LIN-22.

Voraussetzung: [00-konventionen.md](00-konventionen.md). Auth-Grundlagen (Key-Typen, Header, Scopes, Rate-Limits) stehen in [09 §2](09-api-events.md), der MCP-Zugang in [07 §1.3](07-mcp-server.md), Rollen und Teamzugriff in [02](02-teams-rechte.md).

---

## 1. Persönlicher Key vs. Bot-Key

**Autor einer Aktion ist immer die Identität, der der verwendete Key gehört.** Ein persönlicher Key handelt als du, ein Bot-Key als der Bot.

| | Besitzer | Erscheint im Ticket als | Verwaltung unter |
|---|---|---|---|
| Persönlicher Key | du | dein Name | `…/settings/api` |
| Bot-Key | der Bot (Rolle `app`) | der Bot | `…/settings/members` → Bereich **Bots** |

Die Herkunftsanzeige **„via MCP" / „via API"** (`source`, [03 §8.2](03-issues-workflow.md)) ist davon **unabhängig**: Sie zeigt nur den *Transportweg*, nicht den Autor. Ein Bot-Key, der über den MCP-Server schreibt, erzeugt also „*Bot* created the issue (via MCP)".

Wer bisher seinen persönlichen Key in einer Automation nutzt, tauscht ihn dort einfach gegen den neuen Bot-Key – sonst ändert sich nichts.

## 2. Voraussetzungen

- Du bist **Owner oder Admin** des Workspace.
- Bots und ihre Keys lassen sich **nur im Browser** (eingeloggte Session) verwalten – nicht per API, MCP oder mit einem anderen API-Key.
- Ein Bot ist ein User mit Rolle `app`: **kein Login**, **keine E-Mail-Benachrichtigungen**, zählt nicht als lizenzpflichtiges Mitglied. Er handelt ausschließlich über seine API-Keys.

## 3. Bot anlegen

1. **Settings → Members** öffnen (`/<workspace>/settings/members`). Der Bereich **„Bots"** erscheint nur für Admins, unterhalb der Mitgliederliste.
2. **New bot** klicken und ausfüllen:
   - **Name** – Anzeigename, z. B. `Lineal Bot` (1–255 Zeichen).
   - **Email** – z. B. `lineal-bot@example.com`. Muss im Workspace eindeutig sein und einer erlaubten Einladungs-Domain entsprechen. Der **lokale Teil wird der @-Handle** (hier `@lineal-bot`) für Erwähnungen.
3. **Create bot**. Der Bot erscheint in der Liste und kann ab sofort Keys erhalten.

## 4. API-Key für den Bot erstellen

In der Zeile des Bots auf **New key** klicken:

| Feld | Bedeutung |
|---|---|
| **Label** | Pflicht, ≤ 100 Zeichen. Beschreibt den Einsatz, z. B. `MCP-Agent prod`. |
| **Access** | **Read** ist immer gesetzt. **Write** zusätzlich ankreuzen, wenn der Bot Issues/Kommentare anlegen oder ändern soll. Bot-Keys erhalten nie den Scope `admin`. |
| **Teams** | Nichts ausgewählt = alle Teams, die der Bot sehen kann. Einzelne Teams auswählen schränkt den Key darauf ein. |
| **Expires** | Optionales Ablaufdatum. |

Nach **Create key** wird der **Klartext-Token genau einmal** angezeigt (Präfix `lin_api_…`). Jetzt kopieren und sicher ablegen – gespeichert wird nur sein SHA-256-Hash, der Token lässt sich später nicht mehr abrufen.

## 5. Token verwenden

**Immer** als Bearer-Header senden – das funktioniert für beide Zugänge:

```
Authorization: Bearer lin_api_…
```

**Direkte API** (`POST /graphql`):

```bash
curl https://<lineal-host>/graphql \
  -H "Authorization: Bearer lin_api_…" \
  -H "Content-Type: application/json" \
  -d '{"query":"mutation { issueCreate(input:{ teamId:\"…\", title:\"Hallo vom Bot\" }) { issue { identifier } } }"}'
```

**MCP-Server** (`POST /mcp`, Streamable HTTP) – hier ist das `Bearer`-Schema **zwingend**:

```json
{
  "mcpServers": {
    "lineal": {
      "url": "https://<lineal-host>/mcp",
      "headers": { "Authorization": "Bearer lin_api_…" }
    }
  }
}
```

Scope-Bedarf der MCP-Tools: `list_*`, `get_*`, `extract_images`, `search_documentation` brauchen **read**; alles Schreibende (`save_issue`, `save_comment`, …) braucht **write** (write schließt read ein). Für einen schreibenden Agenten also einen **Write**-Key ausstellen.

## 6. Teamzugriff

Ein Bot sieht **öffentliche Teams** automatisch. Soll er in **privaten Teams** arbeiten, ihn wie jedes Mitglied über die Mitgliederliste zum Team hinzufügen. Die am Key gewählte Team-Einschränkung wirkt zusätzlich: Die effektiven Rechte sind die **Schnittmenge** aus Team-/Rollenzugriff des Bots und Scopes/Teams des Keys – ein Key kann nie mehr als der Bot selbst.

## 7. Sperren & Widerrufen

- **Einzelnen Key widerrufen:** in der Bot-Zeile beim Key auf **Revoke**. Wirkt sofort und ist endgültig; für einen Rollentausch neuen Key anlegen, dann alten widerrufen.
- **Bot stilllegen:** über die Mitgliederliste suspendieren (`active = false`). Dann wirken **alle** seine Keys nicht mehr. Reaktivieren stellt sie wieder her.

## 8. Wie es im Ticket erscheint

| Weg | Autor | Herkunft | Anzeige |
|---|---|---|---|
| Bot-Key über MCP-Server | Bot | `mcp` | *Bot* … **(via MCP)** |
| Bot-Key direkt über API | Bot | `api` | *Bot* … **(via API)** |
| Dein persönlicher Key | du | `api`/`mcp` | *dein Name* … (via API/MCP) |

Erwähnungen des Bots laufen über den aus der E-Mail abgeleiteten Handle (`@lineal-bot`). Für Rate-Limits zählt der Bot-Key wie jeder API-Key als eigener Bucket auf Basis der Bot-User-ID ([09 §3](09-api-events.md)).
