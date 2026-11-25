//! Alert rules (CRUD). Evaluation and delivery (Web Push, Telegram) are not implemented yet.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub const METRICS: [&str; 6] =
    ["claimable_real_delta", "claimable_level", "days_since_claim", "total_change_24h", "stake_removed", "source_offline"];
const OPS: [&str; 2] = ["below", "above"];
const UNITS: [&str; 4] = ["pct", "tao", "days", "blocks"];
const CHANNELS: [&str; 2] = ["web_push", "telegram"];

#[derive(Serialize, sqlx::FromRow)]
pub struct Rule {
    id: Uuid,
    wallet_address: Option<String>,
    metric: String,
    op: String,
    threshold: Decimal,
    unit: String,
    channels: Vec<String>,
    enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Deserialize)]
pub struct NewRule {
    wallet_address: Option<String>,
    metric: String,
    op: String,
    threshold: Decimal,
    unit: String,
    channels: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct PatchRule {
    enabled: Option<bool>,
    op: Option<String>,
    threshold: Option<Decimal>,
}

fn check(field: &str, v: &str, allowed: &[&str]) -> ApiResult<()> {
    if allowed.contains(&v) { Ok(()) } else { Err(ApiError::bad_request(format!("{field} must be one of {}", allowed.join(", ")))) }
}

pub async fn list(State(s): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<Rule>>> {
    Ok(Json(
        sqlx::query_as::<_, Rule>(
            "SELECT id, wallet_address, metric, op, threshold, unit, channels, enabled, created_at FROM alert_rules WHERE user_id = $1 ORDER BY created_at",
        )
        .bind(user.id)
        .fetch_all(&s.pool)
        .await?,
    ))
}

pub async fn create(State(s): State<AppState>, user: AuthUser, Json(r): Json<NewRule>) -> ApiResult<(StatusCode, Json<Rule>)> {
    check("metric", &r.metric, &METRICS)?;
    check("op", &r.op, &OPS)?;
    check("unit", &r.unit, &UNITS)?;
    let channels = r.channels.unwrap_or_else(|| vec!["web_push".into()]);
    for c in &channels {
        check("channel", c, &CHANNELS)?;
    }
    if let Some(a) = &r.wallet_address {
        let owns: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_wallets WHERE user_id = $1 AND address = $2 AND deleted_at IS NULL)")
                .bind(user.id)
                .bind(a)
                .fetch_one(&s.pool)
                .await?;
        if !owns {
            return Err(ApiError::not_found("wallet not found"));
        }
    }
    let rule = sqlx::query_as::<_, Rule>(
        "INSERT INTO alert_rules (id, user_id, wallet_address, metric, op, threshold, unit, channels)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id, wallet_address, metric, op, threshold, unit, channels, enabled, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(user.id)
    .bind(&r.wallet_address)
    .bind(&r.metric)
    .bind(&r.op)
    .bind(r.threshold)
    .bind(&r.unit)
    .bind(&channels)
    .fetch_one(&s.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(rule)))
}

pub async fn patch(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(p): Json<PatchRule>) -> ApiResult<Json<Rule>> {
    if let Some(op) = &p.op {
        check("op", op, &OPS)?;
    }
    sqlx::query_as::<_, Rule>(
        "UPDATE alert_rules SET enabled = coalesce($3, enabled), op = coalesce($4, op), threshold = coalesce($5, threshold)
         WHERE id = $1 AND user_id = $2
         RETURNING id, wallet_address, metric, op, threshold, unit, channels, enabled, created_at",
    )
    .bind(id)
    .bind(user.id)
    .bind(p.enabled)
    .bind(&p.op)
    .bind(p.threshold)
    .fetch_optional(&s.pool)
    .await?
    .map(Json)
    .ok_or_else(|| ApiError::not_found("rule not found"))
}

pub async fn delete(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let n = sqlx::query("DELETE FROM alert_rules WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&s.pool)
        .await?
        .rows_affected();
    if n == 0 { Err(ApiError::not_found("rule not found")) } else { Ok(StatusCode::NO_CONTENT) }
}
