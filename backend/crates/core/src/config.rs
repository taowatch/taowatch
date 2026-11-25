use anyhow::{Context, Result};

/// Runtime configuration, read from environment variables (and `.env` in development).
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    /// Live RPC (pruned node is fine): every finalized block.
    pub subtensor_ws: String,
    /// Archive RPC for history backfill. Rate-limited: keep concurrency low.
    pub subtensor_archive_ws: String,
    pub bind_addr: String,
    /// WebAuthn relying party id, e.g. `taowatch.example.com` (or `localhost` in dev).
    pub rp_id: String,
    /// WebAuthn origin, e.g. `https://taowatch.example.com`.
    pub rp_origin: String,
    /// Max concurrent archive workers (the public archive rate-limits above ~2).
    pub backfill_workers: usize,
    /// Who may create accounts: `open` (anyone), `invite` (invite code), `closed`.
    /// The very first account can always be created and becomes admin. Unset means `invite`,
    /// so upgrading an older deployment never opens sign-up by accident.
    pub registration: String,
}

fn var(name: &str, default: Option<&str>) -> Result<String> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => default.map(str::to_owned).with_context(|| format!("missing env var {name}")),
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();
        Ok(Self {
            database_url: var("DATABASE_URL", None)?,
            subtensor_ws: var("SUBTENSOR_WS", Some("wss://entrypoint-finney.opentensor.ai:443"))?,
            subtensor_archive_ws: var("SUBTENSOR_ARCHIVE_WS", Some("wss://archive.chain.opentensor.ai:443"))?,
            bind_addr: var("BIND_ADDR", Some("0.0.0.0:8080"))?,
            rp_id: var("RP_ID", Some("localhost"))?,
            rp_origin: var("RP_ORIGIN", Some("http://localhost:8080"))?,
            backfill_workers: var("BACKFILL_WORKERS", Some("1"))?.parse().context("BACKFILL_WORKERS")?,
            registration: match var("TAOWATCH_REGISTRATION", Some("invite"))?.as_str() {
                r @ ("open" | "invite" | "closed") => r.to_owned(),
                r => anyhow::bail!("TAOWATCH_REGISTRATION must be open, invite or closed (got {r})"),
            },
        })
    }
}
