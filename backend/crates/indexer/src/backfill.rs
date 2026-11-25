//! History backfill from the archive node. Each worker handles one wallet at a time.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use anyhow::Result;
use rust_decimal::prelude::ToPrimitive;
use sqlx::PgPool;
use taowatch_core::chain::{self, Chain, SnapshotError, WalletSnapshot};
use tokio::sync::OnceCell;
use tracing::{info, warn};

use crate::schedule::{BLOCKS_PER_DAY, CumPoint, Plan, backoff_secs, block_for_day, change_windows, is_timeout, is_transient, plan_days};
use crate::store;

/// History older than dTAO has a different stake layout; ~660 days covers it as of 2026-11.
const MAX_DAYS: u32 = 700;
const MAX_ATTEMPTS: u32 = 40;
/// A point whose calls keep timing out is skipped after this many tries.
const MAX_TIMEOUTS: u32 = 3;
/// Stop walking back after this many points in a row were skipped (very old state is too
/// slow to read on this archive).
const MAX_SKIPPED_IN_A_ROW: u32 = 3;
/// Pause between archive points to stay under the archive's rate limit.
const PAUSE: Duration = Duration::from_millis(800);

pub async fn run(pool: PgPool, archive_url: String, workers: usize) -> Result<()> {
    let requeued = store::requeue_interrupted(&pool).await?;
    if requeued > 0 {
        info!(n = requeued, "resuming interrupted backfill jobs");
    }
    let chain = std::sync::Arc::new(OnceCell::<Chain>::new());
    let mut set = tokio::task::JoinSet::new();
    for w in 0..workers.max(1) {
        let (pool, url, chain) = (pool.clone(), archive_url.clone(), chain.clone());
        set.spawn(async move {
            loop {
                match store::next_job(&pool).await {
                    Ok(Some(job)) => {
                        let archive = match chain.get_or_try_init(|| Chain::connect(&url)).await {
                            Ok(c) => {
                                let _ = store::set_archive_ok(&pool, true).await;
                                c
                            }
                            Err(e) => {
                                warn!(worker = w, error = %e, "archive unreachable");
                                let _ = store::set_archive_ok(&pool, false).await;
                                tokio::time::sleep(Duration::from_secs(30)).await;
                                continue;
                            }
                        };
                        info!(worker = w, address = %job.address, plan = %job.plan, "backfill started");
                        let res = process(&pool, archive, &job).await;
                        let err = res.as_ref().err().map(|e| e.to_string());
                        if let Some(e) = &err {
                            warn!(address = %job.address, error = %e, "backfill failed");
                        }
                        let _ = store::job_finished(&pool, &job.address, err.as_deref()).await;
                    }
                    Ok(None) => tokio::time::sleep(Duration::from_secs(10)).await,
                    Err(e) => {
                        warn!(error = %e, "job queue error");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                    }
                }
            }
        });
    }
    // Workers loop forever; reaching here means one of them panicked.
    if let Some(Err(e)) = set.join_next().await {
        return Err(e.into());
    }
    anyhow::bail!("backfill workers stopped")
}

/// Retry transient archive errors with backoff; `Ok(None)` = pre-dTAO block.
async fn snapshot_with_retry(chain: &Chain, block: u32, key: &chain::AccountId32) -> Result<Option<WalletSnapshot>> {
    let mut timeouts = 0;
    for attempt in 0..MAX_ATTEMPTS {
        match chain.snapshot_at(block, key).await {
            Ok(s) => return Ok(Some(s)),
            Err(SnapshotError::PreDtao) => return Ok(None),
            Err(SnapshotError::Other(e)) if is_transient(&format!("{e:#}")) => {
                if is_timeout(&format!("{e:#}")) {
                    timeouts += 1;
                    if timeouts >= MAX_TIMEOUTS {
                        anyhow::bail!("block {block}: node timed out {timeouts} times: {e:#}");
                    }
                }
                let wait = backoff_secs(attempt);
                warn!(block, attempt, wait, error = %e, "archive busy, backing off");
                tokio::time::sleep(Duration::from_secs(wait)).await;
            }
            Err(SnapshotError::Other(e)) => return Err(e),
        }
    }
    anyhow::bail!("gave up on block {block} after {MAX_ATTEMPTS} attempts")
}

async fn process(pool: &PgPool, chain: &Chain, job: &store::Job) -> Result<()> {
    let plan = Plan::parse(&job.plan, MAX_DAYS).unwrap_or(Plan::Short);
    let key = chain::parse_address(&job.address)?;
    let head = chain.head().await?;
    let days = plan_days(plan);
    let total = days.len() as i32;
    let stored = store::stored_blocks(pool, &job.address).await?;
    let half_day = (BLOCKS_PER_DAY / 2) as i64;

    let mut done = 0;
    let mut skipped_in_a_row = 0;
    for d in days {
        done += 1;
        let Some(block) = block_for_day(head, d) else { continue };
        // Resume support: skip points already covered by an earlier run.
        if stored.iter().any(|b| (b - block as i64).abs() < half_day) {
            continue;
        }
        match snapshot_with_retry(chain, block, &key).await {
            Ok(Some(s)) => {
                store::insert_snapshot(pool, &job.address, "backfill", &s).await?;
                skipped_in_a_row = 0;
            }
            Ok(None) => {
                // Points are newest first: everything older is before dTAO as well.
                info!(address = %job.address, days_back = d, "reached dTAO launch, history complete");
                store::job_progress(pool, &job.address, total, total).await?;
                break;
            }
            Err(e) => {
                warn!(address = %job.address, days_back = d, error = %format!("{e:#}"), "point skipped");
                skipped_in_a_row += 1;
                if skipped_in_a_row >= MAX_SKIPPED_IN_A_ROW {
                    warn!(address = %job.address, days_back = d, "older history is unreadable on this archive, stopping here");
                    store::job_progress(pool, &job.address, total, total).await?;
                    break;
                }
            }
        }
        store::job_progress(pool, &job.address, done, total).await?;
        tokio::time::sleep(PAUSE).await;
    }

    derive_claims(pool, chain, &job.address, &key).await?;
    info!(address = %job.address, "backfill done");
    Ok(())
}

/// Find every claim in the backfilled range.
///
/// `BasketClaimed` storage only says *where* something changed (claims and stake
/// changes both move it). For each window between snapshots where it changed we
/// binary-search the exact blocks, then read those blocks' events: a claim is
/// recorded only if a `BasketClaimed` event exists, with the TAO amount it reports.
async fn derive_claims(pool: &PgPool, chain: &Chain, address: &str, key: &chain::AccountId32) -> Result<()> {
    let rows = store::claimed_history(pool, address).await?;
    let mut by_hotkey: BTreeMap<String, Vec<CumPoint>> = BTreeMap::new();
    for (hk, block, t, cum) in rows {
        let rao = (cum * rust_decimal::Decimal::from(1_000_000_000u64)).to_u64().unwrap_or(0);
        let ms = (t.unix_timestamp_nanos() / 1_000_000) as u64;
        by_hotkey.entry(hk).or_default().push(CumPoint { block: block as u32, ts_ms: ms, claimed_cum: rao });
    }

    let wallet = HashSet::from([key.0]);
    let mut checked = HashSet::new();
    let mut found = 0;
    for (hk, pts) in &by_hotkey {
        let hotkey = chain::parse_address(hk)?;
        for w in change_windows(pts) {
            let blocks = retry(|| chain.basket_change_blocks(&hotkey, key, w.after_block, w.at_block)).await?;
            for block in blocks {
                if !checked.insert(block) {
                    continue; // already read this block's events for another validator
                }
                let (ts_ms, events) = retry(|| chain.wallet_events(block, &wallet)).await?;
                let mut per_hotkey: BTreeMap<String, u64> = BTreeMap::new();
                for e in events {
                    if let chain::EventKind::Claim { hotkey, tao_rao } = e.kind {
                        *per_hotkey.entry(hotkey.to_string()).or_default() += tao_rao;
                    }
                }
                if per_hotkey.is_empty() {
                    continue; // bookkeeping moved because of a stake change, not a claim
                }
                let mut tx = pool.begin().await?;
                for (claim_hk, rao) in per_hotkey {
                    store::record_exact_claim(&mut tx, address, &claim_hk, block, store::ts(ts_ms)?, rao).await?;
                    found += 1;
                }
                tx.commit().await?;
            }
        }
    }
    info!(%address, claims = found, blocks_checked = checked.len(), "claims confirmed from events");
    Ok(())
}

/// Retry an archive call on transient errors (rate limit, timeouts) with backoff.
async fn retry<T, F, Fut>(mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt < MAX_ATTEMPTS && is_transient(&format!("{e:#}")) => {
                tokio::time::sleep(Duration::from_secs(backoff_secs(attempt))).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}
