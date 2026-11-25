//! TaoWatch HTTP API + WebSocket.

mod alerts;
mod auth;
mod data;
mod error;
mod state;
mod ws;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use taowatch_core::{config::Config, db};
use tokio::sync::broadcast;
use tower_http::trace::TraceLayer;
use tower_sessions::cookie::SameSite;
use tower_sessions::{Expiry, SessionManagerLayer};
use tower_sessions_sqlx_store::PostgresStore;
use tracing::info;
use webauthn_rs::WebauthnBuilder;

use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=info".into()))
        .init();
    let cfg = Config::from_env()?;
    let pool = db::connect(&cfg.database_url).await?;

    let origin = url::Url::parse(&cfg.rp_origin).context("RP_ORIGIN must be a URL")?;
    let webauthn = WebauthnBuilder::new(&cfg.rp_id, &origin)?.rp_name("TaoWatch").build()?;

    let store = PostgresStore::new(pool.clone());
    store.migrate().await?;
    let sessions = SessionManagerLayer::new(store)
        .with_name("taowatch")
        .with_secure(origin.scheme() == "https")
        .with_http_only(true)
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(time::Duration::days(30)));

    let (notices, _) = broadcast::channel(256);
    let state = AppState {
        pool,
        webauthn: Arc::new(webauthn),
        notices,
        registration: cfg.registration.clone().into(),
        login_failures: Default::default(),
    };
    tokio::spawn(ws::listen(state.clone()));

    let dev_login = std::env::var("TAOWATCH_DEV_LOGIN").as_deref() == Ok("1") && cfg.rp_id == "localhost" && origin.scheme() != "https";
    let mut routes = router();
    if dev_login {
        tracing::warn!("DEV LOGIN ENABLED: POST /api/dev/login signs in without a passkey. Never use in production.");
        routes = routes.route("/api/dev/login", post(auth::dev_login));
    }
    let app = routes.layer(sessions).layer(TraceLayer::new_for_http()).with_state(state);
    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await?;
    info!(addr = %cfg.bind_addr, rp_id = %cfg.rp_id, "api listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn router() -> Router<AppState> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/chain/status", get(data::chain_status))
        .route("/api/price", get(data::price))
        // auth
        .route("/api/auth/register/start", post(auth::register_start))
        .route("/api/auth/register/finish", post(auth::register_finish))
        .route("/api/auth/login/start", post(auth::login_start))
        .route("/api/auth/login/finish", post(auth::login_finish))
        .route("/api/auth/recover", post(auth::recover))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/config", get(auth::auth_config))
        .route("/api/auth/password/register", post(auth::password_register))
        .route("/api/auth/password/login", post(auth::password_login))
        .route("/api/auth/password", post(auth::set_password))
        .route("/api/auth/methods", get(auth::methods))
        .route("/api/me", get(auth::me))
        .route("/api/passkeys", get(auth::list_passkeys))
        .route("/api/passkeys/register/start", post(auth::add_passkey_start))
        .route("/api/passkeys/register/finish", post(auth::add_passkey_finish))
        .route("/api/passkeys/{id}", patch(auth::rename_passkey).delete(auth::delete_passkey))
        .route("/api/recovery-codes", get(auth::codes_status))
        .route("/api/recovery-codes/regenerate", post(auth::regenerate_codes))
        .route("/api/admin/invites", get(auth::list_invites).post(auth::create_invite))
        .route("/api/admin/invites/{code}", delete(auth::revoke_invite))
        .route("/api/admin/users", get(auth::list_users))
        // data
        .route("/api/wallets", get(data::list_wallets).post(data::add_wallet))
        .route("/api/wallets/{address}", get(data::wallet_detail).patch(data::patch_wallet).delete(data::delete_wallet))
        .route("/api/wallets/{address}/restore", post(data::restore_wallet))
        .route("/api/wallets/{address}/history", get(data::wallet_history))
        .route("/api/portfolio/summary", get(data::portfolio_summary))
        .route("/api/portfolio/history", get(data::portfolio_history))
        .route("/api/claims", get(data::claims))
        .route("/api/claimable", get(data::claimable))
        .route("/api/events", get(data::events))
        .route("/api/alerts", get(alerts::list).post(alerts::create))
        .route("/api/alerts/{id}", patch(alerts::patch).delete(alerts::delete))
        // realtime
        .route("/ws", get(ws::ws_handler))
}

async fn health(axum::extract::State(s): axum::extract::State<AppState>) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    match sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&s.pool).await {
        Ok(_) => (axum::http::StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
        Err(e) => (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "ok": false, "error": e.to_string() }))),
    }
}
