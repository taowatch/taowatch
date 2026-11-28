//! All database writes of the indexer.

use anyhow::Result;
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use taowatch_core::chain::{EventKind, WalletEvent, WalletSnapshot};
use taowatch_core::rao_to_tao;
use time::OffsetDateTime;

fn tao(rao: u64) -> Decimal {
    rao_to_tao(rao as u128)
}

/// Signed RAO (basket bookkeeping can dip below zero) as TAO.
fn signed_tao(rao: i64) -> Decimal {
    Decimal::new(rao, 9)
}

pub fn ts(ms: u64) -> Result<OffsetDateTime> {
    Ok(OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)?)
}

pub async fn tracked_addresses(pool: &PgPool) -> Result<Vec<String>> {
    // Wallets someone still tracks, plus CLI-tracked ones that no user owns.
    Ok(sqlx::query_scalar(
        "SELECT w.address FROM wallets w
         WHERE EXISTS (SELECT 1 FROM user_wallets uw WHERE uw.address = w.address AND uw.deleted_at IS NULL)
            OR NOT EXISTS (SELECT 1 FROM user_wallets uw WHERE uw.address = w.address)
         ORDER BY w.address",
    )
    .fetch_all(pool)
    .await?)
}

/// Last block the live loop finished, to resume after a restart.
pub async fn live_block(pool: &PgPool) -> Result<Option<u32>> {
    let b: Option<i64> = sqlx::query_scalar("SELECT live_block FROM chain_status WHERE id = 1").fetch_one(pool).await?;
    Ok(b.and_then(|b| u32::try_from(b).ok()))
}

pub async fn set_chain_status(pool: &PgPool, block: u32, block_ts_ms: u64) -> Result<()> {
    let lag = ((OffsetDateTime::now_utc() - ts(block_ts_ms)?).whole_seconds() / 12).max(0) as i32;
    sqlx::query("UPDATE chain_status SET live_block = $1, live_ts = $2, seen_at = now(), lag_blocks = $3 WHERE id = 1")
        .bind(block as i64)
        .bind(ts(block_ts_ms)?)
        .bind(lag)
        .execute(pool)
        .await?;
    notify(pool, "chain_status", &block.to_string()).await
}

pub async fn set_archive_ok(pool: &PgPool, ok: bool) -> Result<()> {
    sqlx::query("UPDATE chain_status SET archive_ok = $1 WHERE id = 1").bind(ok).execute(pool).await?;
    Ok(())
}

pub async fn notify(pool: &PgPool, channel: &str, payload: &str) -> Result<()> {
    sqlx::query("SELECT pg_notify($1, $2)").bind(channel).bind(payload).execute(pool).await?;
    Ok(())
}

/// Replace the current state of one wallet.
pub async fn store_latest(pool: &PgPool, address: &str, s: &WalletSnapshot) -> Result<()> {
    let mut tx = pool.begin().await?;
    // claimed_total = TAO actually received, i.e. the sum of confirmed claim events.
    sqlx::query(
        "INSERT INTO wallet_latest (address, block, ts, free, stake, claimable, redeemable, claimed_total, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, (SELECT coalesce(sum(amount), 0) FROM claims WHERE address = $1), now())
         ON CONFLICT (address) DO UPDATE SET block = EXCLUDED.block, ts = EXCLUDED.ts, free = EXCLUDED.free,
           stake = EXCLUDED.stake, claimable = EXCLUDED.claimable, redeemable = EXCLUDED.redeemable,
           claimed_total = EXCLUDED.claimed_total, updated_at = now()
         WHERE wallet_latest.block <= EXCLUDED.block",
    )
    .bind(address)
    .bind(s.block as i64)
    .bind(ts(s.timestamp_ms)?)
    .bind(rao_to_tao(s.free_rao))
    .bind(rao_to_tao(s.stake_rao()))
    .bind(s.claimable_rao.map(tao))
    .bind(s.redeemable_rao().map(tao))
    .execute(&mut *tx)
    .await?;

    // Keep last-claim fields (filled by events / backfill) across rewrites.
    sqlx::query(
        "DELETE FROM positions_latest p WHERE p.address = $1
           AND NOT (p.hotkey, p.netuid) IN (SELECT * FROM UNNEST($2::text[], $3::int[]))",
    )
    .bind(address)
    .bind(s.positions.iter().map(|p| p.hotkey.to_string()).collect::<Vec<_>>())
    .bind(s.positions.iter().map(|p| p.netuid as i32).collect::<Vec<_>>())
    .execute(&mut *tx)
    .await?;
    for p in &s.positions {
        let preview = s.previews.iter().find(|v| v.hotkey == p.hotkey && p.netuid == 0);
        sqlx::query(
            "INSERT INTO positions_latest (address, hotkey, netuid, amount, pool_tao, pool_alpha, value_tao, accrued, redeemable, claimed_total)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                     CASE WHEN $3 = 0 THEN (SELECT sum(amount) FROM claims WHERE address = $1 AND hotkey = $2) END)
             ON CONFLICT (address, hotkey, netuid) DO UPDATE SET amount = EXCLUDED.amount, pool_tao = EXCLUDED.pool_tao,
               pool_alpha = EXCLUDED.pool_alpha, value_tao = EXCLUDED.value_tao, accrued = EXCLUDED.accrued,
               redeemable = EXCLUDED.redeemable, claimed_total = EXCLUDED.claimed_total",
        )
        .bind(address)
        .bind(p.hotkey.to_string())
        .bind(p.netuid as i32)
        .bind(tao(p.amount_rao))
        .bind(p.pool.map(|(t, _)| tao(t)))
        .bind(p.pool.map(|(_, a)| tao(a)))
        .bind(rao_to_tao(p.value_rao()))
        .bind(preview.map(|v| tao(v.accrued_rao)))
        .bind(preview.map(|v| tao(v.redeemable_rao)))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Time of the newest hourly/daily snapshot for a wallet (ms), if any.
pub async fn last_periodic_snapshot_ms(pool: &PgPool, address: &str) -> Result<Option<u64>> {
    let t: Option<OffsetDateTime> = sqlx::query_scalar("SELECT max(ts) FROM snapshots WHERE address = $1 AND kind IN ('hourly', 'daily')")
        .bind(address)
        .fetch_one(pool)
        .await?;
    Ok(t.map(|t| (t.unix_timestamp_nanos() / 1_000_000) as u64))
}

/// Store one historical/periodic snapshot (wallet + per-position rows). Idempotent per (address, block).
pub async fn insert_snapshot(pool: &PgPool, address: &str, kind: &str, s: &WalletSnapshot) -> Result<()> {
    let mut tx = pool.begin().await?;
    let t = ts(s.timestamp_ms)?;
    let inserted = sqlx::query(
        "INSERT INTO snapshots (ts, address, block, kind, free, stake, claimable, redeemable)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT DO NOTHING",
    )
    .bind(t)
    .bind(address)
    .bind(s.block as i64)
    .bind(kind)
    .bind(rao_to_tao(s.free_rao))
    .bind(rao_to_tao(s.stake_rao()))
    .bind(s.claimable_rao.map(tao))
    .bind(s.redeemable_rao().map(tao))
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 1 {
        insert_positions(&mut tx, address, t, s).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn insert_positions(tx: &mut Transaction<'_, Postgres>, address: &str, t: OffsetDateTime, s: &WalletSnapshot) -> Result<()> {
    for p in &s.positions {
        let preview = s.previews.iter().find(|v| v.hotkey == p.hotkey && p.netuid == 0);
        let claimed = s.claimed_rao.iter().find(|(h, _)| *h == p.hotkey && p.netuid == 0).map(|(_, v)| *v);
        sqlx::query(
            "INSERT INTO position_snapshots (ts, address, block, hotkey, netuid, amount, value_tao, accrued, redeemable, basket_cum)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(t)
        .bind(address)
        .bind(s.block as i64)
        .bind(p.hotkey.to_string())
        .bind(p.netuid as i32)
        .bind(tao(p.amount_rao))
        .bind(rao_to_tao(p.value_rao()))
        .bind(preview.map(|v| tao(v.accrued_rao)))
        .bind(preview.map(|v| tao(v.redeemable_rao)))
        .bind(claimed.map(signed_tao))
        .execute(&mut **tx)
        .await?;
    }
    // A validator can hold claimed history without a current stake (fully unstaked): keep its cumulative too.
    for (hk, cum) in s.claimed_rao.iter().filter(|(h, _)| !s.positions.iter().any(|p| p.hotkey == *h && p.netuid == 0)) {
        sqlx::query(
            "INSERT INTO position_snapshots (ts, address, block, hotkey, netuid, amount, value_tao, basket_cum)
             VALUES ($1, $2, $3, $4, 0, 0, 0, $5)",
        )
        .bind(t)
        .bind(address)
        .bind(s.block as i64)
        .bind(hk.to_string())
        .bind(signed_tao(*cum))
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Persist the events of one block for tracked wallets; claims also go to `claims`.
pub async fn insert_events(pool: &PgPool, block: u32, block_ts_ms: u64, events: &[WalletEvent]) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let t = ts(block_ts_ms)?;
    let mut tx = pool.begin().await?;
    // One validator can emit several BasketClaimed events in a block (seen on finney): sum them.
    let mut claims: std::collections::BTreeMap<(String, String), u64> = Default::default();
    for e in events {
        let address = e.coldkey.to_string();
        let from = match &e.kind {
            EventKind::StakeMoved { from_hotkey, from_netuid, .. } => Some((from_hotkey.to_string(), *from_netuid as i32)),
            _ => None,
        };
        let (kind, hotkey, netuid, amount): (&str, Option<String>, Option<i32>, Option<u64>) = match &e.kind {
            EventKind::Claim { hotkey, tao_rao } => ("claim", Some(hotkey.to_string()), Some(0), Some(*tao_rao)),
            EventKind::RootClaimTotal { tao_rao } => ("root_claim_total", None, Some(0), Some(*tao_rao)),
            EventKind::StakeAdded { hotkey, netuid, tao_rao } => {
                ("stake_added", Some(hotkey.to_string()), Some(*netuid as i32), Some(*tao_rao))
            }
            EventKind::StakeRemoved { hotkey, netuid, tao_rao } => {
                ("stake_removed", Some(hotkey.to_string()), Some(*netuid as i32), Some(*tao_rao))
            }
            EventKind::StakeMoved { to_hotkey, to_netuid, tao_rao, .. } => {
                ("stake_moved", Some(to_hotkey.to_string()), Some(*to_netuid as i32), Some(*tao_rao))
            }
            EventKind::StakeTransferredOut { hotkey, netuid, tao_rao, .. } => {
                ("stake_transfer_out", Some(hotkey.to_string()), Some(*netuid as i32), Some(*tao_rao))
            }
            EventKind::StakeTransferredIn { hotkey, netuid, tao_rao, .. } => {
                ("stake_transfer_in", Some(hotkey.to_string()), Some(*netuid as i32), Some(*tao_rao))
            }
            EventKind::TransferIn { tao_rao, .. } => ("transfer_in", None, None, Some(*tao_rao)),
            EventKind::TransferOut { tao_rao, .. } => ("transfer_out", None, None, Some(*tao_rao)),
        };
        sqlx::query(
            "INSERT INTO events (block, idx, ts, address, kind, hotkey, netuid, amount, from_hotkey, from_netuid, source)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'event') ON CONFLICT DO NOTHING",
        )
        .bind(block as i64)
        .bind(e.index as i32)
        .bind(t)
        .bind(&address)
        .bind(kind)
        .bind(&hotkey)
        .bind(netuid)
        .bind(amount.map(tao))
        .bind(from.as_ref().map(|f| &f.0))
        .bind(from.as_ref().map(|f| f.1))
        .execute(&mut *tx)
        .await?;
        if let EventKind::Claim { hotkey, tao_rao } = &e.kind {
            *claims.entry((address, hotkey.to_string())).or_default() += tao_rao;
        }
    }
    for ((address, hotkey), rao) in claims {
        record_exact_claim(&mut tx, &address, &hotkey, block, t, rao).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Store an exactly-known claim and update the position's last-claim fields.
pub async fn record_exact_claim(
    tx: &mut Transaction<'_, Postgres>,
    address: &str,
    hotkey: &str,
    block: u32,
    t: OffsetDateTime,
    amount_rao: u64,
) -> Result<()> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM claims WHERE address = $1 AND hotkey = $2 AND block = $3)")
        .bind(address)
        .bind(hotkey)
        .bind(block as i64)
        .fetch_one(&mut **tx)
        .await?;
    if !exists {
        sqlx::query("INSERT INTO claims (block, ts, address, hotkey, amount, exact) VALUES ($1, $2, $3, $4, $5, true)")
            .bind(block as i64)
            .bind(t)
            .bind(address)
            .bind(hotkey)
            .bind(tao(amount_rao))
            .execute(&mut **tx)
            .await?;
    }
    sqlx::query(
        "UPDATE positions_latest SET last_claim_block = $3, last_claim_at = $4
         WHERE address = $1 AND hotkey = $2 AND netuid = 0 AND (last_claim_block IS NULL OR last_claim_block < $3)",
    )
    .bind(address)
    .bind(hotkey)
    .bind(block as i64)
    .bind(t)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ── backfill bookkeeping ───────────────────────────────────────────────────

pub struct Job {
    pub address: String,
    pub plan: String,
}

/// Jobs left `running` by a previous indexer process were interrupted: queue them again.
/// Call once at startup, before any worker takes a job (only one indexer runs at a time).
pub async fn requeue_interrupted(pool: &PgPool) -> Result<u64> {
    Ok(sqlx::query("UPDATE backfill_jobs SET status = 'queued' WHERE status = 'running'").execute(pool).await?.rows_affected())
}

/// Put a job taken by a worker back in the queue (e.g. no archive reachable).
pub async fn requeue(pool: &PgPool, address: &str) -> Result<()> {
    sqlx::query("UPDATE backfill_jobs SET status = 'queued' WHERE address = $1 AND status = 'running'").bind(address).execute(pool).await?;
    Ok(())
}

/// Claim the next queued job. A `running` job belongs to a worker and is never handed out twice.
pub async fn next_job(pool: &PgPool) -> Result<Option<Job>> {
    let row: Option<(String, String)> = sqlx::query_as(
        "UPDATE backfill_jobs SET status = 'running', updated_at = now()
         WHERE address = (SELECT address FROM backfill_jobs WHERE status = 'queued'
                          ORDER BY updated_at LIMIT 1 FOR UPDATE SKIP LOCKED)
         RETURNING address, plan",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(address, plan)| Job { address, plan }))
}

pub async fn job_progress(pool: &PgPool, address: &str, done: i32, total: i32) -> Result<()> {
    let pct = if total == 0 { 100 } else { done * 100 / total };
    sqlx::query("UPDATE backfill_jobs SET done = $2, total = $3, updated_at = now() WHERE address = $1")
        .bind(address)
        .bind(done)
        .bind(total)
        .execute(pool)
        .await?;
    sqlx::query("UPDATE wallets SET history_status = 'running', history_progress = $2 WHERE address = $1")
        .bind(address)
        .bind(pct)
        .execute(pool)
        .await?;
    notify(pool, "backfill", &format!("{address}:{pct}")).await
}

pub async fn job_finished(pool: &PgPool, address: &str, error: Option<&str>) -> Result<()> {
    let status = if error.is_some() { "error" } else { "done" };
    sqlx::query("UPDATE backfill_jobs SET status = $2, last_error = $3, updated_at = now() WHERE address = $1")
        .bind(address)
        .bind(status)
        .bind(error)
        .execute(pool)
        .await?;
    sqlx::query("UPDATE wallets SET history_status = $2, history_progress = CASE WHEN $2 = 'done' THEN 100 ELSE history_progress END WHERE address = $1")
        .bind(address)
        .bind(status)
        .execute(pool)
        .await?;
    notify(pool, "backfill", &format!("{address}:{status}")).await
}

/// Blocks already stored for this wallet (so an interrupted job resumes).
pub async fn stored_blocks(pool: &PgPool, address: &str) -> Result<std::collections::HashSet<i64>> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT block FROM snapshots WHERE address = $1")
        .bind(address)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect())
}

/// Blocks whose events are already stored for this wallet.
pub async fn event_blocks(pool: &PgPool, address: &str) -> Result<std::collections::HashSet<i64>> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT DISTINCT block FROM events WHERE address = $1 AND source = 'event'")
        .bind(address)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect())
}

/// Per-hotkey cumulative-claimed history for a wallet, sorted by block.
pub async fn claimed_history(pool: &PgPool, address: &str) -> Result<Vec<(String, i64, OffsetDateTime, Decimal)>> {
    Ok(sqlx::query_as(
        "SELECT hotkey, block, ts, basket_cum FROM position_snapshots
         WHERE address = $1 AND netuid = 0 AND basket_cum IS NOT NULL ORDER BY hotkey, block",
    )
    .bind(address)
    .fetch_all(pool)
    .await?)
}
