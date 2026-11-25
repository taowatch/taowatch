//! Live loop: follow finalized blocks on the (pruned) live node.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use anyhow::Result;
use sqlx::PgPool;
use taowatch_core::chain::{self, Chain};
use tokio::sync::watch;
use tracing::{info, warn};

use crate::schedule::{backoff_secs, due_snapshot};
use crate::store;

/// Run forever, reconnecting with backoff when the node drops us.
pub async fn run(pool: PgPool, url: String) -> Result<()> {
    let mut attempt = 0u32;
    loop {
        match follow(&pool, &url).await {
            Ok(()) => warn!("block stream ended, reconnecting"),
            Err(e) => warn!(error = %e, "live loop failed, reconnecting"),
        }
        attempt += 1;
        tokio::time::sleep(Duration::from_secs(backoff_secs(attempt))).await;
    }
}

async fn follow(pool: &PgPool, url: &str) -> Result<()> {
    info!(%url, "connecting to live node");
    let chain = Chain::connect(url).await?;
    let (tx, mut rx) = watch::channel(0u32);

    // Reader: publish the newest finalized block number. If processing is slow,
    // intermediate blocks are skipped rather than queued.
    let client = chain.client().clone();
    let reader = tokio::spawn(async move {
        let mut blocks = client.stream_blocks().await?;
        while let Some(b) = blocks.next().await {
            let _ = tx.send(b?.number() as u32);
        }
        anyhow::Ok(())
    });

    let mut last_periodic: HashMap<String, Option<u64>> = HashMap::new();
    while rx.changed().await.is_ok() {
        let block = *rx.borrow_and_update();
        if let Err(e) = process_block(pool, &chain, block, &mut last_periodic).await {
            warn!(block, error = %e, "block processing failed");
        }
    }
    reader.await??;
    Ok(())
}

async fn process_block(pool: &PgPool, chain: &Chain, block: u32, last_periodic: &mut HashMap<String, Option<u64>>) -> Result<()> {
    let addresses = store::tracked_addresses(pool).await?;
    let mut keys = HashMap::new();
    for a in &addresses {
        match chain::parse_address(a) {
            Ok(k) => {
                keys.insert(a.clone(), k);
            }
            Err(e) => warn!(address = %a, error = %e, "skipping invalid address"),
        }
    }
    let tracked: HashSet<[u8; 32]> = keys.values().map(|k| k.0).collect();

    let (block_ts, events) = chain.wallet_events(block, &tracked).await?;
    store::set_chain_status(pool, block, block_ts).await?;
    if !events.is_empty() {
        info!(block, n = events.len(), "wallet events");
        store::insert_events(pool, block, block_ts, &events).await?;
        for e in &events {
            store::notify(pool, "event", &format!("{}:{}", e.coldkey, block)).await?;
        }
    }

    for (address, key) in &keys {
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
