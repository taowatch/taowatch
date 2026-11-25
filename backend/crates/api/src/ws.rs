//! Realtime: Postgres LISTEN/NOTIFY from the indexer → broadcast → per-user WebSockets.
//!
//! Messages to the browser (JSON):
//! `{"type":"hello","block":N}` · `{"type":"block","block":N}` ·
//! `{"type":"wallet","address":…,"latest":{…}}` · `{"type":"event","address":…,"block":N}` ·
//! `{"type":"backfill","address":…,"progress":"42"|"done"|"error"}`

use std::collections::HashSet;
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use serde_json::json;
use sqlx::postgres::PgListener;
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

use crate::auth::AuthUser;
use crate::state::{AppState, Notice};

pub const CHANNELS: [&str; 5] = ["chain_status", "wallet_changed", "event", "backfill", "price"];

/// Parse one NOTIFY into a [`Notice`]. Payload formats are written by `indexer::store`.
pub fn parse_notice(channel: &str, payload: &str) -> Option<Notice> {
    match channel {
        "chain_status" => payload.parse().ok().map(Notice::Block),
        "wallet_changed" => Some(Notice::Wallet(payload.to_owned())),
        "event" => {
            let (addr, block) = payload.rsplit_once(':')?;
            Some(Notice::Event(addr.to_owned(), block.parse().ok()?))
        }
        "price" => Some(Notice::Price(payload.to_owned())),
        "backfill" => {
            let (addr, progress) = payload.rsplit_once(':')?;
            Some(Notice::Backfill(addr.to_owned(), progress.to_owned()))
        }
        _ => None,
    }
}

/// Forward NOTIFYs into the broadcast channel forever, reconnecting on error.
pub async fn listen(state: AppState) {
    loop {
        match PgListener::connect_with(&state.pool).await {
            Ok(mut l) => {
                if let Err(e) = l.listen_all(CHANNELS).await {
                    warn!(error = %e, "LISTEN failed");
                } else {
                    info!("listening for indexer notifications");
                    loop {
                        match l.recv().await {
                            Ok(n) => {
                                if let Some(notice) = parse_notice(n.channel(), n.payload()) {
                                    let _ = state.notices.send(notice); // no receivers is fine
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "notification stream lost");
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => warn!(error = %e, "LISTEN connect failed"),
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

pub async fn ws_handler(State(s): State<AppState>, user: AuthUser, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| session(s, user, socket))
}

async fn owned_set(s: &AppState, user: &AuthUser) -> HashSet<String> {
    sqlx::query_scalar::<_, String>("SELECT address FROM user_wallets WHERE user_id = $1 AND deleted_at IS NULL")
        .bind(user.id)
        .fetch_all(&s.pool)
        .await
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

async fn session(s: AppState, user: AuthUser, socket: WebSocket) {
    let (mut tx, mut rx) = socket.split();
    let mut notices = s.notices.subscribe();
    let mut wallets = owned_set(&s, &user).await;
    let mut refresh = tokio::time::interval(Duration::from_secs(30));
    let mut ping = tokio::time::interval(Duration::from_secs(25));

    let block: Option<i64> = sqlx::query_scalar("SELECT live_block FROM chain_status WHERE id = 1").fetch_one(&s.pool).await.ok().flatten();
    if send(&mut tx, json!({ "type": "hello", "block": block })).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            n = notices.recv() => {
                let msg = match n {
                    Ok(Notice::Block(b)) => Some(json!({ "type": "block", "block": b })),
                    Ok(Notice::Price(p)) => serde_json::from_str::<serde_json::Value>(&p)
                        .ok()
                        .map(|v| json!({ "type": "price", "usd": v["usd"], "change_24h": v["change_24h"] })),
                    Ok(Notice::Wallet(a)) if wallets.contains(&a) => latest_json(&s, &a).await.map(|l| json!({ "type": "wallet", "address": a, "latest": l })),
                    Ok(Notice::Event(a, b)) if wallets.contains(&a) => Some(json!({ "type": "event", "address": a, "block": b })),
                    Ok(Notice::Backfill(a, p)) if wallets.contains(&a) => Some(json!({ "type": "backfill", "address": a, "progress": p })),
                    Ok(_) => None,
                    Err(RecvError::Lagged(skipped)) => Some(json!({ "type": "resync", "skipped": skipped })),
                    Err(RecvError::Closed) => break,
                };
                if let Some(m) = msg
                    && send(&mut tx, m).await.is_err()
                {
                    break;
                }
            }
            incoming = rx.next() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {} // client messages are ignored (no commands over WS)
            },
            _ = refresh.tick() => wallets = owned_set(&s, &user).await,
            _ = ping.tick() => {
                if tx.send(Message::Ping(Default::default())).await.is_err() { break; }
            }
        }
    }
}

async fn send<S>(tx: &mut S, v: serde_json::Value) -> Result<(), axum::Error>
where
    S: SinkExt<Message, Error = axum::Error> + Unpin,
{
    tx.send(Message::Text(Utf8Bytes::from(v.to_string()))).await
}

#[derive(sqlx::FromRow)]
struct LatestText {
    block: i64,
    free: String,
    stake: String,
    claimable: Option<String>,
    redeemable: Option<String>,
    claimed_total: String,
}

async fn latest_json(s: &AppState, address: &str) -> Option<serde_json::Value> {
    let row: Option<LatestText> = sqlx::query_as(
        "SELECT block, free::text AS free, stake::text AS stake, claimable::text AS claimable, redeemable::text AS redeemable, claimed_total::text AS claimed_total FROM wallet_latest WHERE address = $1",
    )
    .bind(address)
    .fetch_optional(&s.pool)
    .await
    .ok()
    .flatten();
    row.map(|r| {
        json!({ "block": r.block, "free": r.free, "stake": r.stake, "claimable": r.claimable, "redeemable": r.redeemable, "claimed_total": r.claimed_total })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_indexer_payloads() {
        assert!(matches!(parse_notice("chain_status", "9581810"), Some(Notice::Block(9581810))));
        assert!(matches!(parse_notice("wallet_changed", "5GSs"), Some(Notice::Wallet(a)) if a == "5GSs"));
        assert!(matches!(parse_notice("event", "5GSs:9533159"), Some(Notice::Event(a, 9533159)) if a == "5GSs"));
        assert!(matches!(parse_notice("backfill", "5GSs:done"), Some(Notice::Backfill(a, p)) if a == "5GSs" && p == "done"));
        assert!(matches!(parse_notice("price", r#"{"usd":"412.3"}"#), Some(Notice::Price(p)) if p.contains("412.3")));
        assert!(parse_notice("event", "garbage").is_none());
        assert!(parse_notice("other", "x").is_none());
    }
}
