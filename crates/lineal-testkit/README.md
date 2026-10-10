# lineal-testkit

Seed-Daten, Fakes und Test-Helfer. Nur als `dev-dependency` einbinden (Ausnahme: das
Feature `seed` von `lineal-server` für `lineal seed`).

| Baustein | Zweck |
|---|---|
| `seed::run(tx)` | legt den Workspace `acme` (ENG, WEB, DES; Alice, Bob, Carol) an; ruft `seed::{identity, issues, planning, content}` |
| `SeedData::acme()` | feste IDs der Beispieldaten |
| `ctx_for(SeedUser::Alice)` | `RequestCtx` für einen Seed-User |
| `FakeStore` / `fake_app()` | `App` ohne Datenbank; `FakeStore::with_principal(hash, principal)` konfiguriert `find_principal_by_token_hash` |
| `test_app(store)` | `App` mit beliebigem `Store` und Fake-Diensten (feste Uhr, Seq-IDs) |

## Testmuster

**In `lineal-db`** (Migrationen liegen im Crate):

```rust
#[sqlx::test(migrations = "./migrations")]
async fn my_test(pool: sqlx::PgPool) { /* … */ }
```

**In allen anderen Crates** (`lineal-app`-Use-Cases mit echter DB, `lineal-api`, `lineal-mcp`, `lineal-jobs`):

```rust
use lineal_testkit::{ctx_for, seed, test_app, SeedUser};
use lineal_db::PgStore;

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn create_issue_assigns_number(pool: sqlx::PgPool) {
    let store = PgStore::new(pool);
    let mut tx = store.begin_system().await.unwrap();
    let data = seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();

    let app = test_app(std::sync::Arc::new(store));
    let ctx = ctx_for(SeedUser::Alice);
    // app.create_issue(&ctx, …).await …
}
```

`#[sqlx::test]` legt pro Testfunktion eine frische Datenbank an (benötigt `DATABASE_URL`,
Default `postgres://lineal:lineal@localhost:5440/lineal`) und löscht sie danach.
Das Crate des Tests braucht `lineal-testkit`, `lineal-db` und `sqlx` als `dev-dependencies`.

**Ohne Datenbank** (Auth, Adapter-Verhalten):

```rust
let store = FakeStore::new().with_principal("abc…", principal);
let app = test_app(Arc::new(store));
```
