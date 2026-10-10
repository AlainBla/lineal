//! `CommentRepo` für `PgTx` (Stub; Methoden ergänzt das zuständige Arbeitspaket).

use crate::store::PgTx;
use async_trait::async_trait;
use lineal_app::ports::repos::CommentRepo;

#[async_trait]
impl CommentRepo for PgTx {}
