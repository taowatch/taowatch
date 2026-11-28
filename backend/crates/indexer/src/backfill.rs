//! History backfill from archive nodes (with fallback). Each worker handles one wallet at a time.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
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
    let archives = std::sync::Arc::new(Archives::new(&archive_url));
    info!(endpoints = archives.urls.len(), "archive endpoints (fallback in this order)");
    let mut set = tokio::task::JoinSet::new();
    for w in 0..workers.max(1) {
        let (pool, archives) = (pool.clone(), archives.clone());
        set.spawn(async move {
            loop {
                match store::next_job(&pool).await {
                    Ok(Some(job)) => {
                        if let Err(e) = archives.get().await {
                            warn!(worker = w, error = %e, "archive unreachable");
                            let _ = store::set_archive_ok(&pool, false).await;
                            let _ = store::requeue(&pool, &job.address).await;
                            tokio::time::sleep(Duration::from_secs(30)).await;
                            continue;
                        }
                        let _ = store::set_archive_ok(&pool, true).await;
                        info!(worker = w, address = %job.address, plan = %job.plan, "backfill started");
                        let res = process(&pool, &archives, &job).await;
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

/// Retry an archive call on transient errors (rate limit, timeouts), failing over between
/// endpoints: `retry!(archives, |chain| chain.head())`. A macro, not a function taking an async
/// closure, so the worker future stays `Send`.
macro_rules! retry {
    ($archives:expr, |$chain:ident| $call:expr) => {{
        let archives: &Archives = $archives;
        let mut attempt = 0u32;
        loop {
            let res = match archives.get().await {
                Ok((i, $chain)) => $call.await.map_err(|e| (Some(i), anyhow::Error::from(e))),
                Err(e) => Err((None, e)),
            };
            match res {
                Ok(v) => break Ok::<_, anyhow::Error>(v),
                Err((i, e)) if attempt < MAX_ATTEMPTS && (i.is_none() || is_transient(&format!("{e:#}"))) => {
                    if let Some(i) = i {
                        archives.fail_over(i);
                    }
                    tokio::time::sleep(archives.wait(attempt)).await;
                    attempt += 1;
                }
                Err((_, e)) => break Err(e),
            }
        }
    }};
}

/// Archive endpoints in order of preference (`SUBTENSOR_ARCHIVE_WS`, comma-separated).
/// When the current one is rate-limited or times out, work moves on to the next one.
pub struct Archives {
    urls: Vec<String>,
    chains: Vec<OnceCell<Chain>>,
    current: AtomicUsize,
}

impl Archives {
    pub fn new(list: &str) -> Self {
        let urls: Vec<String> = list.split(',').map(str::trim).filter(|u| !u.is_empty()).map(str::to_owned).collect();
        let chains = urls.iter().map(|_| OnceCell::new()).collect();
        Self { urls, chains, current: AtomicUsize::new(0) }
    }

    /// The endpoint in use (connecting on first use). On a connection error, the next one takes over.
    async fn get(&self) -> Result<(usize, &Chain)> {
        anyhow::ensure!(!self.urls.is_empty(), "SUBTENSOR_ARCHIVE_WS is empty");
        let i = self.current.load(Ordering::Relaxed) % self.urls.len();
        match self.chains[i].get_or_try_init(|| Chain::connect(&self.urls[i])).await {
            Ok(c) => Ok((i, c)),
            Err(e) => {
                self.fail_over(i);
                Err(e.context(format!("connect {}", crate::redact(&self.urls[i]))))
            }
        }
    }

    /// Move on from endpoint `from` (no-op if another worker already did).
    fn fail_over(&self, from: usize) {
        let n = self.urls.len();
        if n > 1 && self.current.compare_exchange(from, (from + 1) % n, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            info!(from = %crate::redact(&self.urls[from]), to = %crate::redact(&self.urls[(from + 1) % n]), "switching archive endpoint");
        }
    }

    /// Wait before the next try: short while other endpoints are left to try, then exponential.
    fn wait(&self, attempt: u32) -> Duration {
        let n = self.urls.len().max(1) as u32;
        if !(attempt + 1).is_multiple_of(n) { Duration::from_secs(1) } else { Duration::from_secs(backoff_secs(attempt / n)) }
    }
}

/// Retry transient archive errors, failing over between endpoints; `Ok(None)` = pre-dTAO block.
async fn snapshot_with_retry(archives: &Archives, block: u32, key: &chain::AccountId32) -> Result<Option<WalletSnapshot>> {
    let mut timeouts = 0;
    let max_timeouts = MAX_TIMEOUTS * archives.urls.len().max(1) as u32;
    for attempt in 0..MAX_ATTEMPTS {
        let (i, chain) = match archives.get().await {
            Ok(c) => c,
            Err(e) => {
                warn!(block, attempt, error = %format!("{e:#}"), "archive unreachable");
                tokio::time::sleep(archives.wait(attempt)).await;
                continue;
            }
        };
        match chain.snapshot_at(block, key).await {
            Ok(s) => return Ok(Some(s)),
            Err(SnapshotError::PreDtao) => return Ok(None),
            Err(SnapshotError::Other(e)) if is_transient(&format!("{e:#}")) => {
                if is_timeout(&format!("{e:#}")) {
                    timeouts += 1;
                    if timeouts >= max_timeouts {
                        anyhow::bail!("block {block}: archive timed out {timeouts} times: {e:#}");
                    }
                }
                archives.fail_over(i);
                let wait = archives.wait(attempt);
                warn!(block, attempt, wait = wait.as_secs(), error = %format!("{e:#}"), "archive busy, retrying");
                tokio::time::sleep(wait).await;
            }
            Err(SnapshotError::Other(e)) => return Err(e),
        }
    }
    anyhow::bail!("gave up on block {block} after {MAX_ATTEMPTS} attempts")
}

async fn process(pool: &PgPool, archives: &Archives, job: &store::Job) -> Result<()> {
    let plan = Plan::parse(&job.plan, MAX_DAYS).unwrap_or(Plan::Short);
    let key = chain::parse_address(&job.address)?;
    let head = retry!(archives, |c| c.head())?;
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
        match snapshot_with_retry(archives, block, &key).await {
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

    derive_claims(pool, archives, &job.address, &key).await?;
    derive_events(pool, archives, &job.address, &key, head).await?;
    info!(address = %job.address, "backfill done");
    Ok(())
}

/// Find every claim in the backfilled range.
///
/// `BasketClaimed` storage only says *where* something changed (claims and stake
/// changes both move it). For each window between snapshots where it changed we
/// binary-search the exact blocks, then read those blocks' events: a claim is
/// recorded only if a `BasketClaimed` event exists, with the TAO amount it reports.
async fn derive_claims(pool: &PgPool, archives: &Archives, address: &str, key: &chain::AccountId32) -> Result<()> {
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
            let blocks = retry!(archives, |c| c.basket_change_blocks(&hotkey, key, w.after_block, w.at_block))?;
            for block in blocks {
                if !checked.insert(block) {
                    continue; // already read this block's events for another validator
                }
                let (ts_ms, events) = retry!(archives, |c| c.wallet_events(block, &wallet))?;
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

/// Find every stake / unstake / move / transfer the wallet made in the backfilled range.
///
/// They all need the coldkey's signature, so its nonce goes up in exactly those blocks: per
/// window between stored snapshots (and up to the head) where the nonce changed, bisect the
/// blocks, then store their events. Blocks whose events are already stored (the live loop saw
/// them) are not read again; event storage is idempotent, so a rerun only costs nonce reads.
async fn derive_events(pool: &PgPool, archives: &Archives, address: &str, key: &chain::AccountId32, head: u32) -> Result<()> {
    let mut points: Vec<u32> = store::stored_blocks(pool, address).await?.into_iter().map(|b| b as u32).filter(|b| *b < head).collect();
    points.push(head);
    points.sort_unstable();
    points.dedup();
    let known = store::event_blocks(pool, address).await?;
    let wallet = HashSet::from([key.0]);
    let (mut signed, mut stored, mut failed) = (0usize, 0usize, 0usize);
    let mut prev: Option<(u32, u32)> = None;
    for at in points {
        let n = retry!(archives, |c| c.nonce_at(at, key))?;
        if let Some((from, n_from)) = prev
            && n != n_from
        {
            let blocks = retry!(archives, |c| c.signed_blocks(key, from, n_from, at, n))?;
            signed += blocks.len();
            for block in blocks.into_iter().filter(|b| !known.contains(&(*b as i64))) {
                match retry!(archives, |c| c.wallet_events(block, &wallet)) {
                    Ok((ts_ms, events)) => {
                        store::insert_events(pool, block, ts_ms, &events).await?;
                        stored += events.len();
                    }
                    // An old runtime with a different event layout: skip that block, keep going.
                    Err(e) => {
                        failed += 1;
                        warn!(%address, block, error = %format!("{e:#}"), "events unreadable, skipped");
                    }
                }
            }
        }
        prev = Some((at, n));
        tokio::time::sleep(PAUSE / 4).await;
    }
    info!(%address, signed_blocks = signed, events = stored, unreadable = failed, "wallet events backfilled");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_fail_over_in_order_and_wrap() {
        let a = Archives::new(" wss://a , wss://b,,wss://c ");
        assert_eq!(a.urls, ["wss://a", "wss://b", "wss://c"]);
        a.fail_over(0);
        assert_eq!(a.current.load(Ordering::Relaxed), 1);
        a.fail_over(0); // stale: another worker already moved on
        assert_eq!(a.current.load(Ordering::Relaxed), 1);
        a.fail_over(1);
        a.fail_over(2);
        assert_eq!(a.current.load(Ordering::Relaxed), 0);
        // Try the other endpoints quickly, back off once all were tried.
        assert_eq!(a.wait(0), Duration::from_secs(1));
        assert_eq!(a.wait(1), Duration::from_secs(1));
        assert_eq!(a.wait(2), Duration::from_secs(backoff_secs(0)));
        assert_eq!(Archives::new("wss://a").wait(0), Duration::from_secs(backoff_secs(0)));
    }
}
