//! TAO/USDT price from Binance's public WebSocket ticker stream (no API key).
//!
//! - Binance pushes the 24 h ticker about once per second.
//! - Browsers get a `price` notice at most every [`PUSH_EVERY`]; a row is stored every [`STORE_EVERY`].
//! - Binance closes streams after 24 h and pings every few minutes; we reconnect with backoff
//!   and treat [`SILENCE`] without a message as a dead connection.
//!
//! Env: `TAOWATCH_PRICE=off` disables it. `TAOWATCH_PRICE_WS` overrides the stream URL, e.g.
//! `wss://data-stream.binance.vision/ws/taousdt@ticker` (market-data-only host) where
//! `stream.binance.com` is not reachable.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use crate::store;

const DEFAULT_URL: &str = "wss://stream.binance.com:9443/ws/taousdt@ticker";
const PUSH_EVERY: Duration = Duration::from_secs(5);
const STORE_EVERY: Duration = Duration::from_secs(15 * 60);
const SILENCE: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq)]
struct Tick {
    usd: Decimal,
    change_24h: Option<Decimal>,
}

pub async fn run(pool: PgPool) -> Result<()> {
    if std::env::var("TAOWATCH_PRICE").as_deref() == Ok("off") {
        info!("TAO price disabled (TAOWATCH_PRICE=off)");
        std::future::pending::<()>().await;
    }
    let url = std::env::var("TAOWATCH_PRICE_WS").unwrap_or_else(|_| DEFAULT_URL.into());
    let mut attempt = 0u32;
    // Kept across reconnects (and restarts, via the newest row) so samples stay 15 min apart.
    let mut last_store = last_stored(&pool).await?;
    loop {
        match stream(&pool, &url, &mut attempt, &mut last_store).await {
            Ok(()) => info!("price stream closed by Binance, reconnecting"),
            Err(e) => warn!(error = %format!("{e:#}"), "price stream failed, reconnecting"),
        }
        attempt += 1;
        tokio::time::sleep(Duration::from_secs((2u64 << attempt.min(5)).min(60))).await;
    }
}

/// When the newest stored sample was taken, as an `Instant` (None if there is none).
async fn last_stored(pool: &PgPool) -> Result<Option<Instant>> {
    let age: Option<f64> = sqlx::query_scalar("SELECT extract(epoch FROM now() - max(ts))::float8 FROM prices").fetch_one(pool).await?;
    Ok(age.and_then(|a| Instant::now().checked_sub(Duration::from_secs_f64(a.max(0.0)))))
}

async fn stream(pool: &PgPool, url: &str, attempt: &mut u32, last_store: &mut Option<Instant>) -> Result<()> {
    let (ws, _) = tokio_tungstenite::connect_async(url).await.with_context(|| format!("connect {url}"))?;
    info!(%url, "price stream connected");
    let (mut tx, mut rx) = ws.split();
    let mut last_push: Option<(Instant, Tick)> = None;
    loop {
        let msg = match tokio::time::timeout(SILENCE, rx.next()).await {
            Err(_) => anyhow::bail!("no message for {}s", SILENCE.as_secs()),
            Ok(None) => return Ok(()),
            Ok(Some(m)) => m?,
        };
        let text = match msg {
            Message::Text(t) => t,
            Message::Ping(p) => {
                tx.send(Message::Pong(p)).await?;
                continue;
            }
            Message::Close(_) => return Ok(()),
            _ => continue,
        };
        let tick = match parse(&text) {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "unexpected price message");
                continue;
            }
        };
        *attempt = 0;
        let now = Instant::now();
        if last_store.is_none_or(|t| now - t >= STORE_EVERY) {
            sqlx::query("INSERT INTO prices (ts, tao_usd, change_24h, source) VALUES (now(), $1, $2, 'binance')")
                .bind(tick.usd)
                .bind(tick.change_24h)
                .execute(pool)
                .await?;
            *last_store = Some(now);
        }
        let changed = last_push.is_none_or(|(_, prev)| prev != tick);
        if changed && last_push.is_none_or(|(t, _)| now - t >= PUSH_EVERY) {
            let payload = serde_json::json!({ "usd": tick.usd, "change_24h": tick.change_24h }).to_string();
            store::notify(pool, "price", &payload).await?;
            last_push = Some((now, tick));
        }
    }
}

/// Binance 24 h ticker: `{"e":"24hrTicker","s":"TAOUSDT","c":"412.30000000","P":"2.410",…}`
/// (`c` = last price, `P` = 24 h change in percent; numbers are strings).
fn parse(text: &str) -> Result<Tick> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    let field = |k: &str| v.get(k).and_then(|x| x.as_str());
    let usd: Decimal = field("c").context("no last price")?.parse()?;
    let change_24h = field("P").and_then(|p| p.parse::<Decimal>().ok()).map(|d| d.round_dp(4));
    Ok(Tick { usd: usd.round_dp(8).normalize(), change_24h })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_binance_ticker() {
        let t =
            parse(r#"{"e":"24hrTicker","E":1795580000000,"s":"TAOUSDT","p":"9.50","P":"-2.410","c":"412.30000000","v":"123"}"#).unwrap();
        assert_eq!(t.usd, Decimal::new(4123, 1));
        assert_eq!(t.change_24h, Some(Decimal::new(-2410, 3)));
        assert!(parse(r#"{"result":null,"id":1}"#).is_err());
        assert!(parse("not json").is_err());
    }
}
