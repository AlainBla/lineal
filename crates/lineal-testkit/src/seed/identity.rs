//! Seed-Bereich `identity` (WP02). Im Scaffold leer.

use super::SeedData;
use lineal_app::AppResult;
use lineal_app::ports::Tx;

/// Legt die Beispieldaten dieses Bereichs an.
pub async fn seed(_tx: &mut dyn Tx, _data: &SeedData) -> AppResult<()> {
    Ok(())
}
