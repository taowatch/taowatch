use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::broadcast;
use webauthn_rs::Webauthn;

/// A change pushed by the indexer through Postgres NOTIFY, fanned out to WebSockets.
#[derive(Debug, Clone)]
pub enum Notice {
    /// New finalized block processed (payload: block number).
    Block(u64),
    /// A wallet's current state changed.
    Wallet(String),
    /// New on-chain event for a wallet (address, block).
    Event(String, u64),
    /// Backfill progress (address, "42" | "done" | "error").
    Backfill(String, String),
}

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub webauthn: Arc<Webauthn>,
    pub notices: broadcast::Sender<Notice>,
}
