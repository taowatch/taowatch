//! TaoWatch indexer.
//!
//! ```text
//! indexer run                                   live loop + backfill workers (needs DATABASE_URL)
//! indexer track <ss58> [rec|short|none]         start tracking a wallet and queue its history
//! indexer snapshot <ss58> [--archive] [--at N]  read one wallet and print it (no database)
//! indexer events <block> <ss58> [--archive]     print the wallet's events in one block (no database)
//! ```

mod backfill;
mod live;
mod price;
mod schedule;
mod store;

use std::collections::HashSet;

use anyhow::{Context, Result};
use taowatch_core::chain::{self, Chain, WalletSnapshot};
use taowatch_core::{config::Config, db, rao_to_tao};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    // One TLS crypto provider for the whole process (sqlx and reqwest both use rustls + ring).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let rpc_url = || {
        let _ = dotenvy_load();
        let var = if flag("--archive") { "SUBTENSOR_ARCHIVE_WS" } else { "SUBTENSOR_WS" };
        let default = if flag("--archive") { "wss://archive.chain.opentensor.ai:443" } else { "wss://entrypoint-finney.opentensor.ai:443" };
        std::env::var(var).unwrap_or_else(|_| default.into())
    };

    match args.first().map(String::as_str) {
        Some("run") => {
            let cfg = Config::from_env()?;
            let pool = db::connect(&cfg.database_url).await?;
            info!(live = %redact(&cfg.subtensor_ws), archive = %redact(&cfg.subtensor_archive_ws), workers = cfg.backfill_workers, "indexer starting");
            tokio::try_join!(
                live::run(pool.clone(), cfg.subtensor_ws.clone()),
                backfill::run(pool.clone(), cfg.subtensor_archive_ws.clone(), cfg.backfill_workers),
                price::run(pool),
            )?;
            Ok(())
        }
        Some("track") => {
            let address = args.get(1).context("usage: indexer track <ss58> [rec|short|none]")?;
            let plan = args.get(2).map(String::as_str).unwrap_or("rec");
            schedule::Plan::parse(plan, 1).context("plan must be rec, short or none")?;
            let key = chain::parse_address(address)?;
            let address = key.to_string(); // canonical SS58
            let cfg = Config::from_env()?;
            let pool = db::connect(&cfg.database_url).await?;
            let mut tx = pool.begin().await?;
            sqlx::query("INSERT INTO wallets (address, history_status) VALUES ($1, $2) ON CONFLICT (address) DO NOTHING")
                .bind(&address)
                .bind(if plan == "none" { "skipped" } else { "pending" })
                .execute(&mut *tx)
                .await?;
            if plan != "none" {
                sqlx::query("INSERT INTO backfill_jobs (address, plan, status) VALUES ($1, $2, 'queued') ON CONFLICT (address) DO UPDATE SET plan = EXCLUDED.plan, status = 'queued', updated_at = now()")
                    .bind(&address)
                    .bind(plan)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
            println!("tracking {address} (history: {plan})");
            Ok(())
        }
        Some("snapshot") => {
            let address = args.get(1).context("usage: indexer snapshot <ss58> [--archive] [--at <block>]")?;
            let chain = Chain::connect(&rpc_url()).await?;
            let key = chain::parse_address(address)?;
            let snap = match opt("--at") {
                Some(b) => chain.snapshot_at(b.parse()?, &key).await,
                None => chain.snapshot(&key).await,
            }
            .map_err(anyhow::Error::from)?;
            print_snapshot(&snap);
            Ok(())
        }
        Some("events") => {
            let block: u32 = args.get(1).context("usage: indexer events <block> <ss58> [--archive]")?.parse()?;
            let key = chain::parse_address(args.get(2).context("missing <ss58>")?)?;
            let chain = Chain::connect(&rpc_url()).await?;
            let (ts, events) = chain.wallet_events(block, &HashSet::from([key.0])).await?;
            println!("block #{block} at {} ms — {} wallet event(s)", ts, events.len());
            for e in events {
                println!("  #{:<4} {:?}", e.index, e.kind);
            }
            Ok(())
        }
        _ => {
            eprintln!(
                "usage: indexer <run | track <ss58> [plan] | snapshot <ss58> [--archive] [--at N] | events <block> <ss58> [--archive]>"
            );
            std::process::exit(2);
        }
    }
}

fn dotenvy_load() -> Option<()> {
    Config::from_env().ok().map(|_| ())
}

fn print_snapshot(s: &WalletSnapshot) {
    println!("block      #{}", s.block);
    println!("free       {} τ", rao_to_tao(s.free_rao));
    println!("stake      {} τ ({} positions)", rao_to_tao(s.stake_rao()), s.positions.len());
    for p in &s.positions {
        println!("  SN{:<4} {}  {} τ", p.netuid, p.hotkey, rao_to_tao(p.value_rao()));
    }
    match s.claimable_rao {
        Some(c) => println!(
            "claimable  {} τ (receive {})",
            rao_to_tao(c as u128),
            s.redeemable_rao().map(|r| format!("{} τ", rao_to_tao(r as u128))).unwrap_or("n/a".into())
        ),
        None => println!("claimable  n/a (no basket runtime at this block)"),
    }
    let claimed: u128 = s.claimed_rao.iter().map(|(_, v)| *v as u128).sum();
    println!("claimed    {} τ all-time", rao_to_tao(claimed));
}

/// Hide credentials in an RPC URL (`?apikey=…`, `user:pass@`) before logging it.
fn redact(url: &str) -> String {
    let base = url.split_once('?').map_or(url, |(b, _)| b);
    let base = match base.split_once("://") {
        Some((scheme, rest)) => match rest.split_once('@') {
            Some((_, host)) => format!("{scheme}://***@{host}"),
            None => base.to_owned(),
        },
        None => base.to_owned(),
    };
    if url.contains('?') { format!("{base}?***") } else { base }
}

#[cfg(test)]
mod tests {
    #[test]
    fn redacts_keys() {
        assert_eq!(super::redact("wss://x.io/ws?apikey=secret"), "wss://x.io/ws?***");
        assert_eq!(super::redact("wss://u:p@x.io:443"), "wss://***@x.io:443");
        assert_eq!(super::redact("wss://x.io:443"), "wss://x.io:443");
    }
}
