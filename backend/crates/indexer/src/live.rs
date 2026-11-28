//! Live loop: follow finalized blocks on the (pruned) live node.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use anyhow::Result;
use sqlx::PgPool;
use taowatch_core::chain::{self, AccountId32, Chain};
use tracing::{info, warn};

use crate::schedule::{backoff_secs, due_snapshot};
use crate::store;

/// Run forever, reconnecting with backoff when the node drops us.
pub async fn run(pool: PgPool, url: String) -> Result<()> {
    let mut attempt = 0u32;
    // Last block whose events are stored; kept across reconnects and restarts so no block is skipped.
    let mut last_block = store::live_block(&pool).await?.unwrap_or(0);
    loop {
        match follow(&pool, &url, &mut last_block).await {
            Ok(()) => warn!("block stream ended, reconnecting"),
            Err(e) => warn!(error = %e, "live loop failed, reconnecting"),
        }
        attempt += 1;
        tokio::time::sleep(Duration::from_secs(backoff_secs(attempt))).await;
    }
}

async fn follow(pool: &PgPool, url: &str, last_block: &mut u32) -> Result<()> {
    info!(url = %crate::redact(url), "connecting to live node");
    let chain = Chain::connect(url).await?;

    // Poll the finalized head instead of relying on a block subscription: on the public
    // node the subscription sometimes stops delivering without an error. Polling cannot
    // stall silently. GRANDPA often finalizes several blocks at once, so every block since
    // the last one gets its events read; wallet snapshots are taken only at the head.
    let mut tick = tokio::time::interval(POLL_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_progress = Instant::now();
    let mut last_periodic: HashMap<String, Option<u64>> = HashMap::new();
    loop {
        tick.tick().await;
        let head = match tokio::time::timeout(RPC_TIMEOUT, chain.head()).await {
            Ok(Ok(h)) => Some(h),
            Ok(Err(e)) => {
                warn!(error = %e, "reading finalized head failed");
                None
            }
            Err(_) => {
                warn!("reading finalized head timed out");
                None
            }
        };
        match head {
            Some(head) if head > *last_block => {
                last_progress = Instant::now();
                let keys = tracked_keys(pool).await?;
                let from = catch_up_from(*last_block, head);
                if from > *last_block + 1 && *last_block > 0 {
                    warn!(missed = from - *last_block - 1, "too far behind the head; older blocks are left to the backfill");
                }
                for block in from..=head {
                    let at_head = block == head;
                    match tokio::time::timeout(BLOCK_TIMEOUT, process_block(pool, &chain, block, &keys, at_head, &mut last_periodic)).await
                    {
                        Ok(Ok(())) => *last_block = block,
                        Ok(Err(e)) => {
                            warn!(block, error = %e, "block processing failed, retrying next poll");
                            break;
                        }
                        Err(_) => {
                            warn!(block, "block processing timed out, retrying next poll");
                            break;
                        }
                    }
                }
            }
            _ if last_progress.elapsed() > NO_BLOCK_TIMEOUT => {
                anyhow::bail!("no new finalized block for {}s, reconnecting", NO_BLOCK_TIMEOUT.as_secs())
            }
            _ => {}
        }
    }
}

/// First block to process: the one after `last`, unless that is more than [`MAX_CATCH_UP`]
/// blocks behind (or nothing was processed yet), then only the head.
fn catch_up_from(last: u32, head: u32) -> u32 {
    if last == 0 || head - last > MAX_CATCH_UP { head } else { last + 1 }
}

/// Most blocks read in one go after a stall or restart (the live node is pruned to ~256 blocks).
const MAX_CATCH_UP: u32 = 100;
/// How often to ask for the finalized head (blocks come every ~12 s).
const POLL_EVERY: Duration = Duration::from_secs(6);
/// Timeout for one head request.
const RPC_TIMEOUT: Duration = Duration::from_secs(15);
/// Reconnect when the finalized head has not advanced for this long.
const NO_BLOCK_TIMEOUT: Duration = Duration::from_secs(60);
/// Upper bound for handling one block (events + all wallet snapshots).
const BLOCK_TIMEOUT: Duration = Duration::from_secs(90);

type Keys = HashMap<String, AccountId32>;

async fn tracked_keys(pool: &PgPool) -> Result<Keys> {
    let mut keys = HashMap::new();
    for a in store::tracked_addresses(pool).await? {
        match chain::parse_address(&a) {
            Ok(k) => {
                keys.insert(a, k);
            }
            Err(e) => warn!(address = %a, error = %e, "skipping invalid address"),
        }
    }
    Ok(keys)
}

/// Store the block's wallet events; at the head also snapshot every tracked wallet.
async fn process_block(
    pool: &PgPool,
    chain: &Chain,
    block: u32,
    keys: &Keys,
    at_head: bool,
    last_periodic: &mut HashMap<String, Option<u64>>,
) -> Result<()> {
    let tracked: HashSet<[u8; 32]> = keys.values().map(|k| k.0).collect();
    let (block_ts, events) = chain.wallet_events(block, &tracked).await?;
    if !events.is_empty() {
        info!(block, n = events.len(), "wallet events");
        store::insert_events(pool, block, block_ts, &events).await?;
        // A stake into a validator we have not seen: name it now, before the browsers refresh.
        match tokio::time::timeout(Duration::from_secs(20), crate::names::name_new(pool, chain)).await {
            Ok(Ok(n)) if n > 0 => info!(block, hotkeys = n, "named new validators"),
            Ok(Ok(_)) => {}
            Ok(Err(e)) => warn!(block, error = %format!("{e:#}"), "naming new validators failed (retried by the names job)"),
            Err(_) => warn!(block, "naming new validators timed out (retried by the names job)"),
        }
        for e in &events {
            store::notify(pool, "event", &format!("{}:{}", e.coldkey, block)).await?;
        }
    }
    // After the events: a block counts as done (and is resumed after) only once they are stored.
    store::set_chain_status(pool, block, block_ts).await?;
    if !at_head {
        return Ok(());
    }

    for (address, key) in keys {
        let snap = match tokio::time::timeout(Duration::from_secs(30), chain.snapshot_at(block, key)).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                warn!(%address, block, error = %e, "snapshot failed");
                continue;
            }
            Err(_) => {
                warn!(%address, block, "snapshot timed out");
                continue;
            }
        };
        store::store_latest(pool, address, &snap).await?;

        let last = match last_periodic.get(address) {
            Some(v) => *v,
            None => store::last_periodic_snapshot_ms(pool, address).await?,
        };
        if let Some(kind) = due_snapshot(last, snap.timestamp_ms) {
            store::insert_snapshot(pool, address, kind.as_str(), &snap).await?;
            last_periodic.insert(address.clone(), Some(snap.timestamp_ms));
            info!(%address, block, kind = kind.as_str(), "snapshot stored");
        } else {
            last_periodic.insert(address.clone(), last);
        }
        store::notify(pool, "wallet_changed", address).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_up_on_every_skipped_block() {
        assert_eq!(catch_up_from(1000, 1003), 1001);
        assert_eq!(catch_up_from(0, 1003), 1003);
        assert_eq!(catch_up_from(1000, 1000 + MAX_CATCH_UP), 1001);
        assert_eq!(catch_up_from(1000, 1001 + MAX_CATCH_UP), 1001 + MAX_CATCH_UP);
    }
}
