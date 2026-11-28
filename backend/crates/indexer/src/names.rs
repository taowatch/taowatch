//! Validator names: fill `validators.label` with the on-chain identity name of every hotkey
//! a tracked wallet stakes to, has claimed from, or has an event with.
//!
//! - New hotkeys get a name within [`CHECK_EVERY`].
//! - Every name is re-read once per [`REFRESH_EVERY`], so renames show up.
//! - A hotkey without an identity is stored with a NULL label and retried at the next refresh.

use std::time::Duration;

use anyhow::Result;
use sqlx::PgPool;
use taowatch_core::chain::{self, Chain};
use tracing::{info, warn};

const CHECK_EVERY: Duration = Duration::from_secs(10 * 60);
const REFRESH_EVERY: Duration = Duration::from_secs(24 * 3600);

pub async fn run(pool: PgPool, url: String) -> Result<()> {
    let mut last_refresh: Option<tokio::time::Instant> = None;
    let mut tick = tokio::time::interval(CHECK_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let all = last_refresh.is_none_or(|t| t.elapsed() >= REFRESH_EVERY);
        match update(&pool, &url, all).await {
            Ok(n) => {
                if n > 0 {
                    info!(hotkeys = n, refresh = all, "validator names updated");
                }
                if all {
                    last_refresh = Some(tokio::time::Instant::now());
                }
            }
            Err(e) => warn!(error = %format!("{e:#}"), "validator names failed, retrying later"),
        }
    }
}

/// Read names for hotkeys missing from `validators` (or all of them when `all`). Returns how many were read.
async fn update(pool: &PgPool, url: &str, all: bool) -> Result<usize> {
    let hotkeys: Vec<String> = sqlx::query_scalar(
        "SELECT h FROM (SELECT hotkey AS h FROM positions_latest UNION SELECT hotkey FROM claims
                       UNION SELECT hotkey FROM events WHERE hotkey IS NOT NULL) t
         WHERE $1 OR NOT EXISTS (SELECT 1 FROM validators v WHERE v.hotkey = t.h)",
    )
    .bind(all)
    .fetch_all(pool)
    .await?;
    if hotkeys.is_empty() {
        return Ok(0);
    }
    let chain = Chain::connect(url).await?;
    for hk in &hotkeys {
        let name = chain.validator_name(&chain::parse_address(hk)?).await?;
        sqlx::query("INSERT INTO validators (hotkey, label) VALUES ($1, $2) ON CONFLICT (hotkey) DO UPDATE SET label = EXCLUDED.label")
            .bind(hk)
            .bind(name)
            .execute(pool)
            .await?;
    }
    Ok(hotkeys.len())
}
