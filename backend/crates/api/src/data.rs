//! Read/write endpoints for wallets, portfolio, claimable and activity.
//! Amounts are `Decimal` serialized as strings; dates are UTC `YYYY-MM-DD`.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use time::{Date, OffsetDateTime};

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

type Ts = OffsetDateTime;

// ── shared helpers ─────────────────────────────────────────────────────────

/// Addresses the user tracks (optionally just one, which must be theirs).
async fn owned(s: &AppState, user: &AuthUser, only: Option<&str>) -> ApiResult<Vec<String>> {
    let all: Vec<String> =
        sqlx::query_scalar("SELECT address FROM user_wallets WHERE user_id = $1 AND deleted_at IS NULL ORDER BY sort, label")
            .bind(user.id)
            .fetch_all(&s.pool)
            .await?;
    match only {
        None | Some("all") => Ok(all),
        Some(a) if all.iter().any(|x| x == a) => Ok(vec![a.to_owned()]),
        Some(_) => Err(ApiError::not_found("wallet not found")),
    }
}

fn range_start(range: Option<&str>) -> ApiResult<Ts> {
    let days = match range.unwrap_or("90d") {
        "7d" => 7,
        "30d" => 30,
        "90d" => 90,
        "1y" => 365,
        "all" => 365 * 20,
        r => return Err(ApiError::bad_request(format!("unknown range {r}; use 7d, 30d, 90d, 1y or all"))),
    };
    Ok(OffsetDateTime::now_utc() - time::Duration::days(days))
}

fn day_str(d: Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), d.month() as u8, d.day())
}

#[derive(Deserialize)]
pub struct RangeQuery {
    range: Option<String>,
    wallet: Option<String>,
    format: Option<String>,
}

/// Quote a free-text CSV field (validator names may contain commas or quotes).
fn csv_field(v: &str) -> String {
    if v.contains([',', '"', '\n', '\r']) { format!("\"{}\"", v.replace('"', "\"\"")) } else { v.to_owned() }
}

fn csv_response(name: &str, header_row: &str, rows: Vec<String>) -> Response {
    let body = std::iter::once(header_row.to_owned()).chain(rows).collect::<Vec<_>>().join("\n") + "\n";
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}.csv\"")),
        ],
        body,
    )
        .into_response()
}

fn opt(d: Option<Decimal>) -> String {
    d.map(|d| d.to_string()).unwrap_or_default()
}

// ── wallets ────────────────────────────────────────────────────────────────

#[derive(Serialize, sqlx::FromRow)]
pub struct WalletRow {
    address: String,
    label: String,
    sort: i32,
    history_status: String,
    history_progress: i32,
    block: Option<i64>,
    #[serde(with = "time::serde::rfc3339::option")]
    ts: Option<Ts>,
    free: Option<Decimal>,
    stake: Option<Decimal>,
    claimable: Option<Decimal>,
    redeemable: Option<Decimal>,
    claimed_total: Option<Decimal>,
    /// Total (free + stake + claimable) change vs a snapshot 24–36 h old; null if there is none.
    change_24h: Option<Decimal>,
    last_claim_block: Option<i64>,
    #[serde(with = "time::serde::rfc3339::option")]
    last_claim_at: Option<Ts>,
    last_claim_amount: Option<Decimal>,
}

/// Shared SELECT for wallet rows ($1 = user id). A macro so `concat!` keeps every query `&'static str` (sqlx 0.9 rejects runtime-built SQL).
macro_rules! wallet_select {
    () => {
        "
    SELECT uw.address, uw.label, uw.sort, w.history_status, w.history_progress,
           l.block, l.ts, l.free, l.stake, l.claimable, l.redeemable, l.claimed_total,
           (l.free + l.stake + coalesce(l.claimable, 0)) - (
               SELECT s.free + s.stake + coalesce(s.claimable, 0) FROM snapshots s
               WHERE s.address = uw.address AND s.ts BETWEEN now() - interval '36 hours' AND now() - interval '24 hours' ORDER BY s.ts DESC LIMIT 1
           ) AS change_24h,
           c.block AS last_claim_block, c.ts AS last_claim_at, c.amount AS last_claim_amount
    FROM user_wallets uw
    JOIN wallets w ON w.address = uw.address
    LEFT JOIN wallet_latest l ON l.address = uw.address
    LEFT JOIN LATERAL (SELECT block, ts, amount FROM claims WHERE address = uw.address ORDER BY ts DESC LIMIT 1) c ON true
    WHERE uw.user_id = $1 AND uw.deleted_at IS NULL"
    };
}

pub async fn list_wallets(State(s): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<WalletRow>>> {
    Ok(Json(
        sqlx::query_as::<_, WalletRow>(concat!(wallet_select!(), " ORDER BY uw.sort, uw.label")).bind(user.id).fetch_all(&s.pool).await?,
    ))
}

#[derive(Deserialize)]
pub struct AddWallet {
    address: String,
    label: String,
    /// rec | short | none
    history: Option<String>,
}

pub async fn add_wallet(State(s): State<AppState>, user: AuthUser, Json(req): Json<AddWallet>) -> ApiResult<(StatusCode, Json<WalletRow>)> {
    let key = taowatch_core::chain::parse_address(&req.address)
        .map_err(|_| ApiError::bad_request("not a valid SS58 address (checksum failed)"))?;
    let address = key.to_string();
    if !address.starts_with('5') {
        return Err(ApiError::bad_request("Bittensor addresses use SS58 prefix 42 and start with 5"));
    }
    let label = req.label.trim();
    if label.is_empty() || label.chars().count() > 24 {
        return Err(ApiError::bad_request("label: 1–24 characters"));
    }
    let plan = req.history.as_deref().unwrap_or("rec");
    if !["rec", "short", "none"].contains(&plan) {
        return Err(ApiError::bad_request("history must be rec, short or none"));
    }
    let mut tx = s.pool.begin().await?;
    let new_wallet = sqlx::query("INSERT INTO wallets (address, history_status) VALUES ($1, $2) ON CONFLICT (address) DO NOTHING")
        .bind(&address)
        .bind(if plan == "none" { "skipped" } else { "pending" })
        .execute(&mut *tx)
        .await?
        .rows_affected()
        == 1;
    // Another user may already track it: history is shared, so only queue for new wallets.
    if new_wallet && plan != "none" {
        sqlx::query("INSERT INTO backfill_jobs (address, plan) VALUES ($1, $2) ON CONFLICT (address) DO NOTHING")
            .bind(&address)
            .bind(plan)
            .execute(&mut *tx)
            .await?;
    }
    let sort: i32 = sqlx::query_scalar("SELECT coalesce(max(sort) + 1, 0) FROM user_wallets WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    let inserted = sqlx::query(
        "INSERT INTO user_wallets (user_id, address, label, sort) VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, address) DO UPDATE SET label = EXCLUDED.label, deleted_at = NULL
         WHERE user_wallets.deleted_at IS NOT NULL",
    )
    .bind(user.id)
    .bind(&address)
    .bind(label)
    .bind(sort)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(ApiError::conflict("you already track this wallet"));
    }
    tx.commit().await?;
    let row = sqlx::query_as::<_, WalletRow>(concat!(wallet_select!(), " AND uw.address = $2"))
        .bind(user.id)
        .bind(&address)
        .fetch_one(&s.pool)
        .await?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[derive(Deserialize)]
pub struct PatchWallet {
    label: Option<String>,
    sort: Option<i32>,
}

pub async fn patch_wallet(
    State(s): State<AppState>,
    user: AuthUser,
    Path(address): Path<String>,
    Json(req): Json<PatchWallet>,
) -> ApiResult<StatusCode> {
    if let Some(l) = &req.label
        && (l.trim().is_empty() || l.trim().chars().count() > 24)
    {
        return Err(ApiError::bad_request("label: 1–24 characters"));
    }
    let n = sqlx::query(
        "UPDATE user_wallets SET label = coalesce($3, label), sort = coalesce($4, sort) WHERE user_id = $1 AND address = $2 AND deleted_at IS NULL",
    )
    .bind(user.id)
    .bind(&address)
    .bind(req.label.as_deref().map(str::trim))
    .bind(req.sort)
    .execute(&s.pool)
    .await?
    .rows_affected();
    if n == 0 { Err(ApiError::not_found("wallet not found")) } else { Ok(StatusCode::NO_CONTENT) }
}

/// Soft delete: the UI offers "undo" (POST …/restore).
pub async fn delete_wallet(State(s): State<AppState>, user: AuthUser, Path(address): Path<String>) -> ApiResult<StatusCode> {
    let n = sqlx::query("UPDATE user_wallets SET deleted_at = now() WHERE user_id = $1 AND address = $2 AND deleted_at IS NULL")
        .bind(user.id)
        .bind(&address)
        .execute(&s.pool)
        .await?
        .rows_affected();
    if n == 0 { Err(ApiError::not_found("wallet not found")) } else { Ok(StatusCode::NO_CONTENT) }
}

pub async fn restore_wallet(State(s): State<AppState>, user: AuthUser, Path(address): Path<String>) -> ApiResult<StatusCode> {
    let n = sqlx::query("UPDATE user_wallets SET deleted_at = NULL WHERE user_id = $1 AND address = $2 AND deleted_at IS NOT NULL")
        .bind(user.id)
        .bind(&address)
        .execute(&s.pool)
        .await?
        .rows_affected();
    if n == 0 { Err(ApiError::not_found("nothing to restore")) } else { Ok(StatusCode::NO_CONTENT) }
}

// ── wallet detail ──────────────────────────────────────────────────────────

#[derive(Serialize, sqlx::FromRow)]
pub struct PositionRow {
    hotkey: String,
    label: Option<String>,
    netuid: i32,
    amount: Decimal,
    value_tao: Decimal,
    /// TAO per alpha (pool price); null on root.
    price: Option<Decimal>,
    accrued: Option<Decimal>,
    redeemable: Option<Decimal>,
    claimed_total: Option<Decimal>,
    last_claim_block: Option<i64>,
    #[serde(with = "time::serde::rfc3339::option")]
    last_claim_at: Option<Ts>,
}

#[derive(Serialize)]
pub struct WalletDetail {
    wallet: WalletRow,
    positions: Vec<PositionRow>,
    backfill: Option<BackfillStatus>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct BackfillStatus {
    status: String,
    plan: String,
    done: i32,
    total: i32,
    last_error: Option<String>,
}

pub async fn wallet_detail(State(s): State<AppState>, user: AuthUser, Path(address): Path<String>) -> ApiResult<Json<WalletDetail>> {
    owned(&s, &user, Some(&address)).await?;
    let wallet = sqlx::query_as::<_, WalletRow>(concat!(wallet_select!(), " AND uw.address = $2"))
        .bind(user.id)
        .bind(&address)
        .fetch_one(&s.pool)
        .await?;
    let positions = sqlx::query_as::<_, PositionRow>(
        "SELECT p.hotkey, v.label, p.netuid, p.amount, p.value_tao,
                CASE WHEN p.pool_alpha > 0 THEN p.pool_tao / p.pool_alpha END AS price,
                p.accrued, p.redeemable, p.claimed_total, p.last_claim_block, p.last_claim_at
         FROM positions_latest p LEFT JOIN validators v ON v.hotkey = p.hotkey
         WHERE p.address = $1 ORDER BY p.value_tao DESC",
    )
    .bind(&address)
    .fetch_all(&s.pool)
    .await?;
    let backfill =
        sqlx::query_as::<_, BackfillStatus>("SELECT status, plan, done, total, last_error FROM backfill_jobs WHERE address = $1")
            .bind(&address)
            .fetch_optional(&s.pool)
            .await?;
    Ok(Json(WalletDetail { wallet, positions, backfill }))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ClaimRow {
    address: String,
    hotkey: String,
    /// Validator's on-chain name (`validators.label`), when known.
    label: Option<String>,
    block: Option<i64>,
    #[serde(with = "time::serde::rfc3339")]
    ts: Ts,
    amount: Decimal,
    exact: bool,
}

pub async fn claims(State(s): State<AppState>, user: AuthUser, Query(q): Query<RangeQuery>) -> ApiResult<Json<Vec<ClaimRow>>> {
    let addrs = owned(&s, &user, q.wallet.as_deref()).await?;
    Ok(Json(
        sqlx::query_as::<_, ClaimRow>(
            "SELECT c.address, c.hotkey, v.label, c.block, c.ts, c.amount, c.exact FROM claims c LEFT JOIN validators v ON v.hotkey = c.hotkey
             WHERE c.address = ANY($1) AND c.ts >= $2 ORDER BY c.ts DESC LIMIT 500",
        )
        .bind(&addrs)
        .bind(range_start(q.range.as_deref().or(Some("all")))?)
        .fetch_all(&s.pool)
        .await?,
    ))
}

// ── daily series (portfolio, wallet history, claimable) ────────────────────

/// One wallet's point for one UTC day (its latest snapshot of the day, or "now").
#[derive(sqlx::FromRow)]
struct WalletDay {
    address: String,
    day: Date,
    ts: Ts,
    block: i64,
    free: Decimal,
    stake: Decimal,
    claimable: Option<Decimal>,
    redeemable: Option<Decimal>,
}

#[derive(Serialize, Clone)]
pub struct DayPoint {
    day: String,
    block: i64,
    free: Decimal,
    stake: Decimal,
    claimable: Option<Decimal>,
    redeemable: Option<Decimal>,
    total: Decimal,
    /// Sum of confirmed claims since the previous point.
    claimed: Decimal,
    /// claimable − previous claimable + claimed since the previous point. Null when unknown.
    real_delta: Option<Decimal>,
    /// Change of `total` vs the previous point.
    change: Option<Decimal>,
    /// Not every requested wallet has data for this day.
    partial: bool,
}

/// One point per UTC day (last snapshot of the day), summed over wallets, plus "now".
///
/// Wallets are snapshotted at different times of day, so `claimed` and `real_delta` are
/// computed per wallet (claims between that wallet's previous point and this one) and then summed.
async fn daily_series(s: &AppState, addrs: &[String], from: Ts) -> ApiResult<Vec<DayPoint>> {
    let rows = sqlx::query_as::<_, WalletDay>(
        "WITH pts AS (
             SELECT DISTINCT ON (address, (ts AT TIME ZONE 'UTC')::date)
                    address, (ts AT TIME ZONE 'UTC')::date AS day, ts, block, free, stake, claimable, redeemable
             FROM snapshots WHERE address = ANY($1) AND ts >= $2
             ORDER BY address, (ts AT TIME ZONE 'UTC')::date, ts DESC
         ), now_pts AS (
             SELECT address, (ts AT TIME ZONE 'UTC')::date AS day, ts, block, free, stake, claimable, redeemable
             FROM wallet_latest WHERE address = ANY($1)
         )
         SELECT * FROM pts WHERE NOT EXISTS (SELECT 1 FROM now_pts n WHERE n.address = pts.address AND n.day = pts.day)
         UNION ALL SELECT * FROM now_pts
         ORDER BY day, address",
    )
    .bind(addrs)
    .bind(from)
    .fetch_all(&s.pool)
    .await?;

    let claim_rows: Vec<(String, Ts, Decimal)> =
        sqlx::query_as("SELECT address, ts, amount FROM claims WHERE address = ANY($1) AND ts >= $2 ORDER BY ts")
            .bind(addrs)
            .bind(from - time::Duration::days(8))
            .fetch_all(&s.pool)
            .await?;
    let mut claims: BTreeMap<String, Vec<(Ts, Decimal)>> = BTreeMap::new();
    for (a, t, amt) in claim_rows {
        claims.entry(a).or_default().push((t, amt));
    }
    Ok(aggregate_days(&rows, &claims, addrs.len()))
}

/// Sum per-wallet day points (sorted by day) into portfolio points.
fn aggregate_days(rows: &[WalletDay], claims: &BTreeMap<String, Vec<(Ts, Decimal)>>, n_wallets: usize) -> Vec<DayPoint> {
    let none: Vec<(Ts, Decimal)> = Vec::new();
    // Each wallet's previous point: (ts, claimable).
    let mut prev: BTreeMap<&str, (Ts, Option<Decimal>)> = BTreeMap::new();
    let mut out: Vec<DayPoint> = Vec::new();
    let mut prev_total: Option<Decimal> = None;
    for day in rows.chunk_by(|a, b| a.day == b.day) {
        let mut p = DayPoint {
            day: day_str(day[0].day),
            block: 0,
            free: Decimal::ZERO,
            stake: Decimal::ZERO,
            claimable: None,
            redeemable: None,
            total: Decimal::ZERO,
            claimed: Decimal::ZERO,
            real_delta: None,
            change: None,
            partial: day.len() < n_wallets,
        };
        for r in day {
            let wc = claims.get(&r.address).unwrap_or(&none);
            // Claims between this wallet's previous point and this one, by timestamp: a point
            // is taken at some time of day, so a claim later that day belongs to the next point.
            let before = prev.get(r.address.as_str()).copied();
            let after = before.map(|(t, _)| t).unwrap_or(r.ts - time::Duration::days(1));
            let claimed = claims_between(wc, after, r.ts);
            if let (Some((_, Some(a))), Some(b)) = (before, r.claimable) {
                p.real_delta = Some(p.real_delta.unwrap_or_default() + b - a + claimed);
            }
            p.block = p.block.max(r.block);
            p.free += r.free;
            p.stake += r.stake;
            p.claimable = add_opt(p.claimable, r.claimable);
            p.redeemable = add_opt(p.redeemable, r.redeemable);
            p.claimed += claimed;
            prev.insert(r.address.as_str(), (r.ts, r.claimable));
        }
        p.total = p.free + p.stake + p.claimable.unwrap_or_default();
        p.change = prev_total.map(|t| p.total - t);
        prev_total = Some(p.total);
        out.push(p);
    }
    out
}

fn add_opt(a: Option<Decimal>, b: Option<Decimal>) -> Option<Decimal> {
    match (a, b) {
        (None, None) => None,
        _ => Some(a.unwrap_or_default() + b.unwrap_or_default()),
    }
}

/// Sum of claims with `after < ts <= upto`.
fn claims_between(claims: &[(Ts, Decimal)], after: Ts, upto: Ts) -> Decimal {
    claims.iter().filter(|(t, _)| *t > after && *t <= upto).map(|(_, a)| *a).sum()
}

fn series_csv(name: &str, pts: &[DayPoint]) -> Response {
    csv_response(
        name,
        "day,block,free,stake,claimable,redeemable,total,claimed,real_delta,change,partial",
        pts.iter()
            .map(|p| {
                format!(
                    "{},{},{},{},{},{},{},{},{},{},{}",
                    p.day,
                    p.block,
                    p.free,
                    p.stake,
                    opt(p.claimable),
                    opt(p.redeemable),
                    p.total,
                    p.claimed,
                    opt(p.real_delta),
                    opt(p.change),
                    p.partial
                )
            })
            .collect(),
    )
}

pub async fn portfolio_history(State(s): State<AppState>, user: AuthUser, Query(q): Query<RangeQuery>) -> ApiResult<Response> {
    let addrs = owned(&s, &user, q.wallet.as_deref()).await?;
    let pts = daily_series(&s, &addrs, range_start(q.range.as_deref())?).await?;
    Ok(if q.format.as_deref() == Some("csv") { series_csv("portfolio", &pts) } else { Json(pts).into_response() })
}

pub async fn wallet_history(
    State(s): State<AppState>,
    user: AuthUser,
    Path(address): Path<String>,
    Query(q): Query<RangeQuery>,
) -> ApiResult<Response> {
    let addrs = owned(&s, &user, Some(&address)).await?;
    let pts = daily_series(&s, &addrs, range_start(q.range.as_deref())?).await?;
    Ok(if q.format.as_deref() == Some("csv") { series_csv(&format!("wallet-{address}"), &pts) } else { Json(pts).into_response() })
}

#[derive(Serialize)]
pub struct Summary {
    wallets: usize,
    block: Option<i64>,
    free: Decimal,
    stake: Decimal,
    claimable: Option<Decimal>,
    redeemable: Option<Decimal>,
    total: Decimal,
    claimed_total: Decimal,
    change_24h: Option<Decimal>,
    claimable_change_24h: Option<Decimal>,
    last_claim: Option<LastClaim>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct LastClaim {
    address: String,
    hotkey: String,
    label: Option<String>,
    block: Option<i64>,
    #[serde(with = "time::serde::rfc3339")]
    ts: Ts,
    amount: Decimal,
}

#[derive(sqlx::FromRow)]
struct LatestTotals {
    block: Option<i64>,
    free: Option<Decimal>,
    stake: Option<Decimal>,
    claimable: Option<Decimal>,
    redeemable: Option<Decimal>,
    claimed_total: Option<Decimal>,
}

pub async fn portfolio_summary(State(s): State<AppState>, user: AuthUser) -> ApiResult<Json<Summary>> {
    let addrs = owned(&s, &user, None).await?;
    let LatestTotals { block, free, stake, claimable, redeemable, claimed_total } =
        sqlx::query_as::<_, LatestTotals>("SELECT max(block) AS block, sum(free) AS free, sum(stake) AS stake, sum(claimable) AS claimable, sum(redeemable) AS redeemable, sum(claimed_total) AS claimed_total FROM wallet_latest WHERE address = ANY($1)")
            .bind(&addrs)
            .fetch_one(&s.pool)
            .await?;
    let (free, stake) = (free.unwrap_or_default(), stake.unwrap_or_default());
    let total = free + stake + claimable.unwrap_or_default();
    // 24 h ago: newest snapshot per wallet that is at least 24 h old.
    let (old_total, old_claimable, n_old): (Option<Decimal>, Option<Decimal>, i64) = sqlx::query_as(
        "SELECT sum(free + stake + coalesce(claimable, 0)), sum(claimable), count(*) FROM (
             SELECT DISTINCT ON (address) free, stake, claimable FROM snapshots
             WHERE address = ANY($1) AND ts BETWEEN now() - interval '36 hours' AND now() - interval '24 hours' ORDER BY address, ts DESC) t",
    )
    .bind(&addrs)
    .fetch_one(&s.pool)
    .await?;
    let complete = n_old as usize == addrs.len() && !addrs.is_empty();
    let claimed_24h: Decimal =
        sqlx::query_scalar("SELECT coalesce(sum(amount), 0) FROM claims WHERE address = ANY($1) AND ts > now() - interval '24 hours'")
            .bind(&addrs)
            .fetch_one(&s.pool)
            .await?;
    let last_claim = sqlx::query_as::<_, LastClaim>(
        "SELECT c.address, c.hotkey, v.label, c.block, c.ts, c.amount FROM claims c LEFT JOIN validators v ON v.hotkey = c.hotkey
         WHERE c.address = ANY($1) ORDER BY c.ts DESC LIMIT 1",
    )
    .bind(&addrs)
    .fetch_optional(&s.pool)
    .await?;
    Ok(Json(Summary {
        wallets: addrs.len(),
        block,
        free,
        stake,
        claimable,
        redeemable,
        total,
        claimed_total: claimed_total.unwrap_or_default(),
        change_24h: old_total.filter(|_| complete).map(|o| total - o),
        claimable_change_24h: match (claimable, old_claimable.filter(|_| complete)) {
            (Some(now), Some(old)) => Some(now - old + claimed_24h),
            _ => None,
        },
        last_claim,
    }))
}

// ── claimable page ─────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ClaimableResponse {
    days: Vec<DayPoint>,
    /// Per validator: daily accrued claimable (for the stacked chart).
    validators: Vec<ValidatorSeries>,
}

#[derive(Serialize)]
pub struct ValidatorSeries {
    hotkey: String,
    label: Option<String>,
    points: Vec<(String, Decimal)>,
}

pub async fn claimable(State(s): State<AppState>, user: AuthUser, Query(q): Query<RangeQuery>) -> ApiResult<Response> {
    let addrs = owned(&s, &user, q.wallet.as_deref()).await?;
    let from = range_start(q.range.as_deref().or(Some("all")))?;
    let days: Vec<DayPoint> = daily_series(&s, &addrs, from).await?.into_iter().filter(|p| p.claimable.is_some()).collect();
    if q.format.as_deref() == Some("csv") {
        return Ok(series_csv("claimable", &days));
    }
    let rows: Vec<(String, Option<String>, Date, Decimal)> = sqlx::query_as(
        "SELECT t.hotkey, v.label, t.day, sum(t.accrued) FROM (
             SELECT DISTINCT ON (address, hotkey, (ts AT TIME ZONE 'UTC')::date)
                    address, hotkey, (ts AT TIME ZONE 'UTC')::date AS day, accrued
             FROM position_snapshots WHERE address = ANY($1) AND ts >= $2 AND accrued IS NOT NULL
             ORDER BY address, hotkey, (ts AT TIME ZONE 'UTC')::date, ts DESC) t
         LEFT JOIN validators v ON v.hotkey = t.hotkey
         GROUP BY t.hotkey, v.label, t.day ORDER BY t.hotkey, t.day",
    )
    .bind(&addrs)
    .bind(from)
    .fetch_all(&s.pool)
    .await?;
    let mut map: BTreeMap<String, ValidatorSeries> = BTreeMap::new();
    for (hk, label, day, acc) in rows {
        map.entry(hk.clone()).or_insert_with(|| ValidatorSeries { hotkey: hk, label, points: vec![] }).points.push((day_str(day), acc));
    }
    Ok(Json(ClaimableResponse { days, validators: map.into_values().collect() }).into_response())
}

// ── activity ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct EventsQuery {
    wallet: Option<String>,
    /// claim | stake | transfer | all
    r#type: Option<String>,
    /// Cursor: return events strictly older than this block.
    before: Option<i64>,
    limit: Option<i64>,
    format: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct EventRow {
    block: i64,
    idx: i32,
    #[serde(with = "time::serde::rfc3339")]
    ts: Ts,
    address: String,
    kind: String,
    hotkey: Option<String>,
    label: Option<String>,
    netuid: Option<i32>,
    /// Source of a `stake_moved` (hotkey / netuid are its destination).
    from_hotkey: Option<String>,
    from_label: Option<String>,
    from_netuid: Option<i32>,
    amount: Option<Decimal>,
    source: String,
}

pub async fn events(State(s): State<AppState>, user: AuthUser, Query(q): Query<EventsQuery>) -> ApiResult<Response> {
    let addrs = owned(&s, &user, q.wallet.as_deref()).await?;
    let kinds: Option<Vec<&str>> = match q.r#type.as_deref().unwrap_or("all") {
        "all" => None,
        "claim" => Some(vec!["claim"]),
        "stake" => Some(vec!["stake_added", "stake_removed", "stake_moved", "stake_transfer_in", "stake_transfer_out"]),
        "transfer" => Some(vec!["transfer_in", "transfer_out"]),
        t => return Err(ApiError::bad_request(format!("unknown type {t}"))),
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let rows = sqlx::query_as::<_, EventRow>(
        "SELECT e.block, e.idx, e.ts, e.address, e.kind, e.hotkey, v.label, e.netuid,
                e.from_hotkey, f.label AS from_label, e.from_netuid, e.amount, e.source
         FROM events e LEFT JOIN validators v ON v.hotkey = e.hotkey LEFT JOIN validators f ON f.hotkey = e.from_hotkey
         WHERE e.address = ANY($1) AND ($2::text[] IS NULL OR e.kind = ANY($2)) AND e.kind <> 'root_claim_total'
           AND ($3::bigint IS NULL OR e.block < $3)
         ORDER BY e.block DESC, e.idx DESC LIMIT $4",
    )
    .bind(&addrs)
    .bind(kinds.map(|k| k.into_iter().map(String::from).collect::<Vec<_>>()))
    .bind(q.before)
    .bind(limit)
    .fetch_all(&s.pool)
    .await?;
    if q.format.as_deref() == Some("csv") {
        return Ok(csv_response(
            "activity",
            "block,idx,ts,address,kind,hotkey,validator,netuid,amount,source",
            rows.iter()
                .map(|e| {
                    format!(
                        "{},{},{},{},{},{},{},{},{},{}",
                        e.block,
                        e.idx,
                        e.ts.format(&time::format_description::well_known::Rfc3339).unwrap_or_default(),
                        e.address,
                        e.kind,
                        e.hotkey.clone().unwrap_or_default(),
                        csv_field(e.label.as_deref().unwrap_or_default()),
                        e.netuid.map(|n| n.to_string()).unwrap_or_default(),
                        opt(e.amount),
                        e.source
                    )
                })
                .collect(),
        ));
    }
    Ok(Json(rows).into_response())
}

// ── chain status (public) ──────────────────────────────────────────────────

#[derive(Serialize, sqlx::FromRow)]
pub struct ChainStatus {
    live_block: Option<i64>,
    #[serde(with = "time::serde::rfc3339::option")]
    live_ts: Option<Ts>,
    #[serde(with = "time::serde::rfc3339::option")]
    seen_at: Option<Ts>,
    lag_blocks: Option<i32>,
    archive_ok: Option<bool>,
}

pub async fn chain_status(State(s): State<AppState>) -> ApiResult<Json<ChainStatus>> {
    Ok(Json(
        sqlx::query_as::<_, ChainStatus>("SELECT live_block, live_ts, seen_at, lag_blocks, archive_ok FROM chain_status WHERE id = 1")
            .fetch_one(&s.pool)
            .await?,
    ))
}

// ── TAO price (public) ─────────────────────────────────────────────────────

#[derive(Serialize, sqlx::FromRow)]
pub struct Price {
    usd: Decimal,
    change_24h: Option<Decimal>,
    #[serde(with = "time::serde::rfc3339")]
    ts: Ts,
    /// The newest stored sample is older than two store intervals: the price feed is down.
    stale: bool,
}

pub async fn price(State(s): State<AppState>) -> ApiResult<Json<Option<Price>>> {
    Ok(Json(
        sqlx::query_as::<_, Price>(
            "SELECT tao_usd AS usd, change_24h, ts, ts < now() - interval '35 minutes' AS stale
             FROM prices ORDER BY ts DESC LIMIT 1",
        )
        .fetch_optional(&s.pool)
        .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn claims_land_between_snapshots_not_calendar_days() {
        // Real case: daily points taken ~06:00 UTC, claims on 2026-11-18 at 11:34 and 11:35.
        let claims = vec![
            (datetime!(2026-11-18 11:34:12 UTC), Decimal::new(14874478774, 9)),
            (datetime!(2026-11-18 11:35:00 UTC), Decimal::new(9951774119, 9)),
        ];
        let p18 = datetime!(2026-11-18 06:00:00 UTC);
        let p19 = datetime!(2026-11-19 06:00:00 UTC);
        assert_eq!(claims_between(&claims, p18 - time::Duration::days(1), p18), Decimal::ZERO);
        assert_eq!(claims_between(&claims, p18, p19), Decimal::new(24826252893, 9));
        // Boundaries: exclusive start, inclusive end.
        assert_eq!(
            claims_between(&claims, datetime!(2026-11-18 11:34:12 UTC), datetime!(2026-11-18 11:35:00 UTC)),
            Decimal::new(9951774119, 9)
        );
    }

    fn wd(address: &str, ts: Ts, claimable: Decimal) -> WalletDay {
        WalletDay {
            address: address.into(),
            day: ts.date(),
            ts,
            block: 0,
            free: Decimal::ZERO,
            stake: Decimal::ZERO,
            claimable: Some(claimable),
            redeemable: None,
        }
    }

    #[test]
    fn real_delta_uses_each_wallets_own_snapshot_time() {
        // A is snapshotted at 06:00, B at 23:00. A claims 10 τ on day 2 at 12:00, after its
        // day-2 point: that claim belongs to A's day-3 point, not to day 2.
        let rows = vec![
            wd("A", datetime!(2026-11-01 06:00 UTC), Decimal::new(10, 0)),
            wd("B", datetime!(2026-11-01 23:00 UTC), Decimal::new(5, 0)),
            wd("A", datetime!(2026-11-02 06:00 UTC), Decimal::new(11, 0)),
            wd("B", datetime!(2026-11-02 23:00 UTC), Decimal::new(6, 0)),
            wd("A", datetime!(2026-11-03 06:00 UTC), Decimal::new(2, 0)),
            wd("B", datetime!(2026-11-03 23:00 UTC), Decimal::new(7, 0)),
        ];
        let mut claims = BTreeMap::new();
        claims.insert("A".to_string(), vec![(datetime!(2026-11-02 12:00 UTC), Decimal::new(10, 0))]);
        let pts = aggregate_days(&rows, &claims, 2);
        assert_eq!(pts.len(), 3);
        assert_eq!(pts[0].real_delta, None);
        // Day 2: A +1, B +1, no claim yet for A.
        assert_eq!((pts[1].claimed, pts[1].real_delta), (Decimal::ZERO, Some(Decimal::new(2, 0))));
        // Day 3: A 11 → 2 with a 10 τ claim = +1; B +1.
        assert_eq!((pts[2].claimed, pts[2].real_delta), (Decimal::new(10, 0), Some(Decimal::new(2, 0))));
        assert!(!pts[2].partial);
    }
}
