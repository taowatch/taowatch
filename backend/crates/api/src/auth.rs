//! Sign-in (passkeys via webauthn-rs, or username + Argon2id password), sessions, invites, recovery codes.

use axum::Json;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tower_sessions::Session;
use uuid::Uuid;
use webauthn_rs::prelude::*;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const SESSION_UID: &str = "uid";
const RECOVERY_CODES: usize = 6;
/// Set after a recovery-code sign-in: the next password change needs no current password.
const SESSION_RECOVERED: &str = "recovered";

// ── current user ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AuthUser {
    pub id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub role: String,
}

impl AuthUser {
    pub fn require_admin(&self) -> ApiResult<()> {
        if self.role == "admin" { Ok(()) } else { Err(ApiError::forbidden("admin only")) }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let session =
            Session::from_request_parts(parts, state).await.map_err(|(_, msg)| ApiError(StatusCode::INTERNAL_SERVER_ERROR, msg.into()))?;
        let uid: Uuid = session.get(SESSION_UID).await?.ok_or_else(ApiError::unauthorized)?;
        sqlx::query_as::<_, AuthUser>("SELECT id, username, display_name, role FROM users WHERE id = $1")
            .bind(uid)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(ApiError::unauthorized)
    }
}

pub async fn me(user: AuthUser) -> Json<AuthUser> {
    Json(user)
}

// ── helpers ────────────────────────────────────────────────────────────────

/// Unambiguous alphabet (no 0/O, 1/I/L) for codes people type.
const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

fn random_code(groups: usize) -> String {
    let mut bytes = Vec::new();
    while bytes.len() < groups * 4 {
        bytes.extend_from_slice(Uuid::new_v4().as_bytes()); // CSPRNG via getrandom
    }
    bytes
        .chunks(4)
        .take(groups)
        .map(|g| g.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

fn hash_code(code: &str) -> String {
    let normalized: String = code.trim().to_ascii_uppercase().chars().filter(|c| *c != '-' && *c != ' ').collect();
    hex::encode(Sha256::digest(normalized.as_bytes()))
}

fn valid_username(u: &str) -> bool {
    (2..=32).contains(&u.len()) && u.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c))
}

async fn load_passkeys(pool: &sqlx::PgPool, user_id: Uuid) -> ApiResult<Vec<(Vec<u8>, Passkey)>> {
    let rows: Vec<(Vec<u8>, serde_json::Value)> =
        sqlx::query_as("SELECT id, credential FROM passkeys WHERE user_id = $1").bind(user_id).fetch_all(pool).await?;
    rows.into_iter()
        .map(|(id, v)| Ok((id, serde_json::from_value(v).map_err(|e| ApiError::from(anyhow::anyhow!("stored passkey: {e}")))?)))
        .collect()
}

/// Persist counter/backup-state changes after a successful assertion.
async fn touch_passkey(pool: &sqlx::PgPool, mut keys: Vec<(Vec<u8>, Passkey)>, res: &AuthenticationResult) -> ApiResult<()> {
    if let Some((id, pk)) = keys.iter_mut().find(|(id, _)| id.as_slice() == res.cred_id().as_slice()) {
        pk.update_credential(res);
        sqlx::query("UPDATE passkeys SET credential = $2, last_used_at = now() WHERE id = $1")
            .bind(&*id)
            .bind(serde_json::to_value(&*pk).map_err(anyhow::Error::from)?)
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn start_session(session: &Session, uid: Uuid) -> ApiResult<()> {
    session.cycle_id().await?; // prevent session fixation
    // A recovery grant belongs to the sign-in that created it, never to a later one.
    session.remove_value(SESSION_RECOVERED).await?;
    session.insert(SESSION_UID, uid).await?;
    Ok(())
}

async fn new_recovery_codes(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, user_id: Uuid) -> ApiResult<Vec<String>> {
    sqlx::query("DELETE FROM recovery_codes WHERE user_id = $1").bind(user_id).execute(&mut **tx).await?;
    let codes: Vec<String> = (0..RECOVERY_CODES).map(|_| random_code(3)).collect();
    for c in &codes {
        sqlx::query("INSERT INTO recovery_codes (user_id, code_hash) VALUES ($1, $2)")
            .bind(user_id)
            .bind(hash_code(c))
            .execute(&mut **tx)
            .await?;
    }
    Ok(codes)
}

// ── registration ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RegisterStart {
    username: String,
    display_name: Option<String>,
    invite: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PendingRegistration {
    user_id: Uuid,
    username: String,
    display_name: Option<String>,
    invite: Option<String>,
    state: PasskeyRegistration,
}

pub async fn register_start(
    State(s): State<AppState>,
    session: Session,
    Json(req): Json<RegisterStart>,
) -> ApiResult<Json<CreationChallengeResponse>> {
    let username = req.username.trim().to_ascii_lowercase();
    if !valid_username(&username) {
        return Err(ApiError::bad_request("username: 2–32 characters, a–z 0–9 _ . -"));
    }
    let invite = normalize_invite(req.invite);
    check_can_register(&s, invite.as_deref()).await?;
    let taken: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE username = $1)").bind(&username).fetch_one(&s.pool).await?;
    if taken {
        return Err(ApiError::conflict("username is taken"));
    }
    let user_id = Uuid::new_v4();
    let display = req.display_name.clone().unwrap_or_else(|| username.clone());
    let (ccr, state) = s.webauthn.start_passkey_registration(user_id, &username, &display, None)?;
    session.insert("reg", PendingRegistration { user_id, username, display_name: req.display_name, invite, state }).await?;
    Ok(Json(ccr))
}

#[derive(Deserialize)]
pub struct RegisterFinish {
    credential: RegisterPublicKeyCredential,
    /// Device name shown in Settings, e.g. "MacBook · Touch ID".
    name: Option<String>,
}

#[derive(Serialize)]
pub struct RegisterDone {
    user: AuthUser,
    recovery_codes: Vec<String>,
}

pub async fn register_finish(
    State(s): State<AppState>,
    session: Session,
    Json(req): Json<RegisterFinish>,
) -> ApiResult<Json<RegisterDone>> {
    let pending: PendingRegistration = session.remove("reg").await?.ok_or_else(|| ApiError::bad_request("no registration in progress"))?;
    let passkey = s.webauthn.finish_passkey_registration(&req.credential, &pending.state)?;

    let mut tx = s.pool.begin().await?;
    let user =
        create_user(&s, &mut tx, pending.user_id, &pending.username, pending.display_name.as_deref(), pending.invite.as_deref(), None)
            .await?;
    sqlx::query("INSERT INTO passkeys (id, user_id, credential, name) VALUES ($1, $2, $3, $4)")
        .bind(passkey.cred_id().as_slice())
        .bind(user.id)
        .bind(serde_json::to_value(&passkey).map_err(anyhow::Error::from)?)
        .bind(req.name.unwrap_or_else(|| "passkey".into()))
        .execute(&mut *tx)
        .await?;
    let recovery_codes = new_recovery_codes(&mut tx, user.id).await?;
    tx.commit().await?;

    start_session(&session, user.id).await?;
    Ok(Json(RegisterDone { user, recovery_codes }))
}

// ── who may register ───────────────────────────────────────────────────────

fn normalize_invite(i: Option<String>) -> Option<String> {
    i.map(|i| i.trim().to_ascii_uppercase()).filter(|i| !i.is_empty())
}

/// Early check before a ceremony starts (re-checked inside `create_user`).
async fn check_can_register(s: &AppState, invite: Option<&str>) -> ApiResult<()> {
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(&s.pool).await?;
    if users == 0 {
        return Ok(()); // the first account is always allowed and becomes admin
    }
    match &*s.registration {
        "open" => Ok(()),
        "closed" => Err(ApiError::forbidden("sign-up is closed on this server")),
        _ => {
            let code = invite.ok_or_else(|| ApiError::forbidden("an invite code is required"))?;
            let ok: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM invites WHERE code = $1 AND used_by IS NULL AND revoked_at IS NULL AND expires_at > now())",
            )
            .bind(code)
            .fetch_one(&s.pool)
            .await?;
            if ok { Ok(()) } else { Err(ApiError::forbidden("invite code is invalid, used or expired")) }
        }
    }
}

/// Insert the user inside `tx`, enforcing the registration mode. The first user becomes admin.
async fn create_user(
    s: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    username: &str,
    display_name: Option<&str>,
    invite: Option<&str>,
    password_hash: Option<&str>,
) -> ApiResult<AuthUser> {
    // Serialize sign-ups so two concurrent first sign-ups cannot both become admin.
    sqlx::query("SELECT pg_advisory_xact_lock(7_271_001)").execute(&mut **tx).await?;
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(&mut **tx).await?;
    let role = if users == 0 { "admin" } else { "user" };
    let mut use_invite = None;
    if users > 0 {
        match &*s.registration {
            "open" => {}
            "closed" => return Err(ApiError::forbidden("sign-up is closed on this server")),
            _ => {
                // Lock the invite row so two sign-ups cannot use the same code.
                let valid: Option<String> = sqlx::query_scalar(
                    "SELECT code FROM invites WHERE code = $1 AND used_by IS NULL AND revoked_at IS NULL AND expires_at > now() FOR UPDATE",
                )
                .bind(invite.unwrap_or(""))
                .fetch_optional(&mut **tx)
                .await?;
                use_invite = Some(valid.ok_or_else(|| ApiError::forbidden("invite code is invalid, used or expired"))?);
            }
        }
    }
    let user = sqlx::query_as::<_, AuthUser>(
        "INSERT INTO users (id, username, display_name, role, password_hash) VALUES ($1, $2, $3, $4, $5)
         RETURNING id, username, display_name, role",
    )
    .bind(id)
    .bind(username)
    .bind(display_name)
    .bind(role)
    .bind(password_hash)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("username is taken"),
        e => e.into(),
    })?;
    if let Some(code) = use_invite {
        sqlx::query("UPDATE invites SET used_by = $2 WHERE code = $1").bind(code).bind(user.id).execute(&mut **tx).await?;
    }
    Ok(user)
}

#[derive(Serialize)]
pub struct AuthConfig {
    /// `open` | `invite` | `closed`
    registration: String,
    /// No account exists yet: the next sign-up becomes admin and needs no invite.
    first_user: bool,
}

/// Public: lets the sign-in page show or hide the invite field.
pub async fn auth_config(State(s): State<AppState>) -> ApiResult<Json<AuthConfig>> {
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(&s.pool).await?;
    Ok(Json(AuthConfig { registration: s.registration.to_string(), first_user: users == 0 }))
}

// ── username + password ────────────────────────────────────────────────────

const PASSWORD_MIN: usize = 10;
const PASSWORD_MAX: usize = 128;
const MAX_FAILURES: u32 = 5;
const LOCKOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// Upper bound on usernames tracked for lockout (unknown names are tracked too, so a 429
/// does not reveal which usernames exist).
const MAX_TRACKED: usize = 10_000;
/// How long after a recovery-code sign-in the password may be set without the current one.
const RECOVERY_GRANT_SECS: i64 = 10 * 60;

type Failures = std::collections::HashMap<String, (u32, std::time::Instant)>;

/// Reserve one attempt for `username` before the slow verify, so parallel requests cannot all
/// pass the check. Returns the minutes to wait when the limit is reached.
fn reserve_attempt(f: &mut Failures, username: &str, now: std::time::Instant) -> Result<(), u64> {
    if f.len() >= MAX_TRACKED {
        f.retain(|_, (_, since)| now.duration_since(*since) <= LOCKOUT);
        if f.len() >= MAX_TRACKED
            && let Some(oldest) = f.iter().min_by_key(|(_, (_, since))| *since).map(|(k, _)| k.clone())
        {
            f.remove(&oldest);
        }
    }
    let e = f.entry(username.to_owned()).or_insert((0, now));
    if now.duration_since(e.1) > LOCKOUT {
        *e = (0, now);
    }
    if e.0 >= MAX_FAILURES {
        return Err((LOCKOUT.saturating_sub(now.duration_since(e.1))).as_secs() / 60 + 1);
    }
    e.0 += 1;
    Ok(())
}

fn check_password_policy(p: &str) -> ApiResult<()> {
    let n = p.chars().count();
    if n < PASSWORD_MIN {
        return Err(ApiError::bad_request(format!("password: at least {PASSWORD_MIN} characters")));
    }
    if n > PASSWORD_MAX {
        return Err(ApiError::bad_request(format!("password: at most {PASSWORD_MAX} characters")));
    }
    Ok(())
}

/// Argon2id (default params), off the async runtime because it is deliberately slow.
async fn hash_password(p: String) -> ApiResult<String> {
    tokio::task::spawn_blocking(move || {
        use argon2::password_hash::PasswordHasher;
        argon2::Argon2::default().hash_password(p.as_bytes()).map(|h| h.to_string())
    })
    .await
    .map_err(|e| ApiError::from(anyhow::anyhow!("hash task: {e}")))?
    .map_err(|e| ApiError::from(anyhow::anyhow!("hash: {e}")))
}

async fn verify_password(p: String, phc: String) -> bool {
    tokio::task::spawn_blocking(move || {
        use argon2::password_hash::{PasswordVerifier, phc::PasswordHash};
        PasswordHash::new(&phc).map(|h| argon2::Argon2::default().verify_password(p.as_bytes(), &h).is_ok()).unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// Hash compared against when the user does not exist, so timing does not reveal valid usernames.
static DUMMY_HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    use argon2::password_hash::PasswordHasher;
    argon2::Argon2::default().hash_password(b"taowatch-timing-equalizer").map(|h| h.to_string()).unwrap_or_default()
});

#[derive(Deserialize)]
pub struct PasswordRegister {
    username: String,
    password: String,
    display_name: Option<String>,
    invite: Option<String>,
}

pub async fn password_register(
    State(s): State<AppState>,
    session: Session,
    Json(req): Json<PasswordRegister>,
) -> ApiResult<Json<RegisterDone>> {
    let username = req.username.trim().to_ascii_lowercase();
    if !valid_username(&username) {
        return Err(ApiError::bad_request("username: 2–32 characters, a–z 0–9 _ . -"));
    }
    check_password_policy(&req.password)?;
    let invite = normalize_invite(req.invite);
    check_can_register(&s, invite.as_deref()).await?;
    let hash = hash_password(req.password).await?;
    let mut tx = s.pool.begin().await?;
    let user = create_user(&s, &mut tx, Uuid::new_v4(), &username, req.display_name.as_deref(), invite.as_deref(), Some(&hash)).await?;
    let recovery_codes = new_recovery_codes(&mut tx, user.id).await?;
    tx.commit().await?;
    start_session(&session, user.id).await?;
    Ok(Json(RegisterDone { user, recovery_codes }))
}

#[derive(Deserialize)]
pub struct PasswordLogin {
    username: String,
    password: String,
}

pub async fn password_login(State(s): State<AppState>, session: Session, Json(req): Json<PasswordLogin>) -> ApiResult<Json<AuthUser>> {
    let username = req.username.trim().to_ascii_lowercase();
    let reserved = reserve_attempt(&mut s.login_failures.lock().expect("lock"), &username, std::time::Instant::now());
    if let Err(wait) = reserved {
        return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, format!("too many failed attempts; try again in {wait} min or use a passkey")));
    }
    let row: Option<(Uuid, Option<String>)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE username = $1").bind(&username).fetch_optional(&s.pool).await?;
    let (id, phc) = match row {
        Some((id, Some(phc))) => (Some(id), phc),
        _ => (None, DUMMY_HASH.clone()),
    };
    let ok = verify_password(req.password, phc).await && id.is_some();
    if !ok {
        // The attempt was already counted by `reserve_attempt`.
        return Err(ApiError(StatusCode::UNAUTHORIZED, "username or password is wrong".into()));
    }
    s.login_failures.lock().expect("lock").remove(&username);
    let id = id.expect("checked");
    start_session(&session, id).await?;
    Ok(Json(
        sqlx::query_as::<_, AuthUser>("SELECT id, username, display_name, role FROM users WHERE id = $1")
            .bind(id)
            .fetch_one(&s.pool)
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct SetPassword {
    /// Required when the account already has a password.
    current: Option<String>,
    new: String,
}

pub async fn set_password(
    State(s): State<AppState>,
    session: Session,
    user: AuthUser,
    Json(req): Json<SetPassword>,
) -> ApiResult<StatusCode> {
    check_password_policy(&req.new)?;
    let existing: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1").bind(user.id).fetch_one(&s.pool).await?;
    let recovered = session
        .remove_value(SESSION_RECOVERED)
        .await?
        .and_then(|v| v.as_i64())
        .is_some_and(|at| OffsetDateTime::now_utc().unix_timestamp() - at <= RECOVERY_GRANT_SECS);
    if let Some(phc) = existing
        && !recovered
        && !verify_password(req.current.unwrap_or_default(), phc).await
    {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "current password is wrong".into()));
    }
    let hash = hash_password(req.new).await?;
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1").bind(user.id).bind(hash).execute(&s.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct Methods {
    password: bool,
    passkeys: i64,
}

/// Which sign-in methods the current user has (Settings).
pub async fn methods(State(s): State<AppState>, user: AuthUser) -> ApiResult<Json<Methods>> {
    let (password, passkeys): (bool, i64) =
        sqlx::query_as("SELECT password_hash IS NOT NULL, (SELECT count(*) FROM passkeys WHERE user_id = $1) FROM users WHERE id = $1")
            .bind(user.id)
            .fetch_one(&s.pool)
            .await?;
    Ok(Json(Methods { password, passkeys }))
}

// ── login ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LoginStart {
    /// Empty / missing ⇒ discoverable login (pick a saved passkey, conditional UI).
    username: Option<String>,
}

#[derive(Serialize, Deserialize)]
enum PendingLogin {
    User { user_id: Uuid, state: PasskeyAuthentication },
    Discoverable { state: DiscoverableAuthentication },
}

pub async fn login_start(
    State(s): State<AppState>,
    session: Session,
    Json(req): Json<LoginStart>,
) -> ApiResult<Json<RequestChallengeResponse>> {
    let username = req.username.map(|u| u.trim().to_ascii_lowercase()).filter(|u| !u.is_empty());
    let (rcr, pending) = match username {
        Some(username) => {
            let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
                .bind(&username)
                .fetch_optional(&s.pool)
                .await?
                .ok_or_else(|| ApiError::bad_request("no passkey for this username"))?;
            let keys: Vec<Passkey> = load_passkeys(&s.pool, user_id).await?.into_iter().map(|(_, k)| k).collect();
            if keys.is_empty() {
                return Err(ApiError::bad_request("no passkey for this username"));
            }
            let (rcr, state) = s.webauthn.start_passkey_authentication(&keys)?;
            (rcr, PendingLogin::User { user_id, state })
        }
        None => {
            let (rcr, state) = s.webauthn.start_discoverable_authentication()?;
            (rcr, PendingLogin::Discoverable { state })
        }
    };
    session.insert("login", pending).await?;
    Ok(Json(rcr))
}

pub async fn login_finish(State(s): State<AppState>, session: Session, Json(cred): Json<PublicKeyCredential>) -> ApiResult<Json<AuthUser>> {
    let pending: PendingLogin = session.remove("login").await?.ok_or_else(|| ApiError::bad_request("no sign-in in progress"))?;
    let user_id = match pending {
        PendingLogin::User { user_id, state } => {
            let res = s.webauthn.finish_passkey_authentication(&cred, &state)?;
            touch_passkey(&s.pool, load_passkeys(&s.pool, user_id).await?, &res).await?;
            user_id
        }
        PendingLogin::Discoverable { state } => {
            let (user_id, cred_id) = s.webauthn.identify_discoverable_authentication(&cred)?;
            let stored = load_passkeys(&s.pool, user_id).await?;
            let key = stored
                .iter()
                .find(|(id, _)| id.as_slice() == cred_id)
                .map(|(_, k)| k)
                .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "unknown passkey".into()))?;
            let res = s.webauthn.finish_discoverable_authentication_v2(&cred, state, key)?;
            touch_passkey(&s.pool, stored, &res).await?;
            user_id
        }
    };
    start_session(&session, user_id).await?;
    let user = sqlx::query_as::<_, AuthUser>("SELECT id, username, display_name, role FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&s.pool)
        .await?;
    Ok(Json(user))
}

pub async fn logout(session: Session) -> ApiResult<StatusCode> {
    session.flush().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── recovery ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct Recover {
    username: String,
    code: String,
}

/// Signs the user in with a one-time code; the client then adds a new passkey
/// via `POST /api/passkeys/register/start`.
pub async fn recover(State(s): State<AppState>, session: Session, Json(req): Json<Recover>) -> ApiResult<Json<AuthUser>> {
    let user: Option<AuthUser> = sqlx::query_as::<_, AuthUser>(
        "UPDATE recovery_codes r SET used_at = now() FROM users u
         WHERE u.username = $1 AND r.user_id = u.id AND r.code_hash = $2 AND r.used_at IS NULL
         RETURNING u.id, u.username, u.display_name, u.role",
    )
    .bind(req.username.trim().to_ascii_lowercase())
    .bind(hash_code(&req.code))
    .fetch_optional(&s.pool)
    .await?;
    // Same message for unknown user and wrong code.
    let user = user.ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "username or recovery code is wrong".into()))?;
    start_session(&session, user.id).await?;
    // Allows one password reset without the old password for a few minutes (see `set_password`).
    session.insert(SESSION_RECOVERED, OffsetDateTime::now_utc().unix_timestamp()).await?;
    Ok(Json(user))
}

pub async fn regenerate_codes(State(s): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<String>>> {
    let mut tx = s.pool.begin().await?;
    let codes = new_recovery_codes(&mut tx, user.id).await?;
    tx.commit().await?;
    Ok(Json(codes))
}

#[derive(Serialize)]
pub struct CodesStatus {
    unused: i64,
    total: i64,
}

pub async fn codes_status(State(s): State<AppState>, user: AuthUser) -> ApiResult<Json<CodesStatus>> {
    let (unused, total): (i64, i64) =
        sqlx::query_as("SELECT count(*) FILTER (WHERE used_at IS NULL), count(*) FROM recovery_codes WHERE user_id = $1")
            .bind(user.id)
            .fetch_one(&s.pool)
            .await?;
    Ok(Json(CodesStatus { unused, total }))
}

// ── passkey management ─────────────────────────────────────────────────────

#[derive(Serialize, sqlx::FromRow)]
pub struct PasskeyInfo {
    id: String,
    name: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    last_used_at: Option<OffsetDateTime>,
}

pub async fn list_passkeys(State(s): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<PasskeyInfo>>> {
    Ok(Json(
        sqlx::query_as::<_, PasskeyInfo>(
            "SELECT encode(id, 'hex') AS id, name, created_at, last_used_at FROM passkeys WHERE user_id = $1 ORDER BY created_at",
        )
        .bind(user.id)
        .fetch_all(&s.pool)
        .await?,
    ))
}

pub async fn add_passkey_start(State(s): State<AppState>, session: Session, user: AuthUser) -> ApiResult<Json<CreationChallengeResponse>> {
    let existing: Vec<CredentialID> = load_passkeys(&s.pool, user.id).await?.into_iter().map(|(_, k)| k.cred_id().clone()).collect();
    let display = user.display_name.clone().unwrap_or_else(|| user.username.clone());
    let (ccr, state) = s.webauthn.start_passkey_registration(user.id, &user.username, &display, Some(existing))?;
    session.insert("add_passkey", state).await?;
    Ok(Json(ccr))
}

pub async fn add_passkey_finish(
    State(s): State<AppState>,
    session: Session,
    user: AuthUser,
    Json(req): Json<RegisterFinish>,
) -> ApiResult<StatusCode> {
    let state: PasskeyRegistration =
        session.remove("add_passkey").await?.ok_or_else(|| ApiError::bad_request("no passkey registration in progress"))?;
    let passkey = s.webauthn.finish_passkey_registration(&req.credential, &state)?;
    sqlx::query("INSERT INTO passkeys (id, user_id, credential, name) VALUES ($1, $2, $3, $4)")
        .bind(passkey.cred_id().as_slice())
        .bind(user.id)
        .bind(serde_json::to_value(&passkey).map_err(anyhow::Error::from)?)
        .bind(req.name.unwrap_or_else(|| "passkey".into()))
        .execute(&s.pool)
        .await?;
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize)]
pub struct Rename {
    name: String,
}

pub async fn rename_passkey(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<Rename>,
) -> ApiResult<StatusCode> {
    let n = sqlx::query("UPDATE passkeys SET name = $3 WHERE user_id = $1 AND id = decode($2, 'hex')")
        .bind(user.id)
        .bind(&id)
        .bind(req.name.trim())
        .execute(&s.pool)
        .await?
        .rows_affected();
    if n == 0 { Err(ApiError::not_found("passkey not found")) } else { Ok(StatusCode::NO_CONTENT) }
}

pub async fn delete_passkey(State(s): State<AppState>, user: AuthUser, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let mut tx = s.pool.begin().await?;
    // Lock this user's passkeys, then count, so two parallel deletes cannot remove the last one.
    let ids: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT id FROM passkeys WHERE user_id = $1 FOR UPDATE").bind(user.id).fetch_all(&mut *tx).await?;
    let has_password: bool =
        sqlx::query_scalar("SELECT password_hash IS NOT NULL FROM users WHERE id = $1").bind(user.id).fetch_one(&mut *tx).await?;
    if ids.len() <= 1 && !has_password {
        return Err(ApiError::conflict("cannot remove your last passkey — set a password first, or add another passkey"));
    }
    let n = sqlx::query("DELETE FROM passkeys WHERE user_id = $1 AND id = decode($2, 'hex')")
        .bind(user.id)
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    if n == 0 { Err(ApiError::not_found("passkey not found")) } else { Ok(StatusCode::NO_CONTENT) }
}

// ── development login ──────────────────────────────────────────────────────

/// Passkeys need a real authenticator, so local testing with curl uses this.
/// Only routed when TAOWATCH_DEV_LOGIN=1, RP_ID=localhost and the origin is not HTTPS (see main.rs).
#[derive(Deserialize)]
pub struct DevLogin {
    username: String,
}

pub async fn dev_login(State(s): State<AppState>, session: Session, Json(req): Json<DevLogin>) -> ApiResult<Json<AuthUser>> {
    let username = req.username.trim().to_ascii_lowercase();
    if !valid_username(&username) {
        return Err(ApiError::bad_request("bad username"));
    }
    let user = sqlx::query_as::<_, AuthUser>(
        "INSERT INTO users (id, username, role) VALUES ($1, $2, CASE WHEN (SELECT count(*) FROM users) = 0 THEN 'admin' ELSE 'user' END)
         ON CONFLICT (username) DO UPDATE SET username = EXCLUDED.username
         RETURNING id, username, display_name, role",
    )
    .bind(Uuid::new_v4())
    .bind(&username)
    .fetch_one(&s.pool)
    .await?;
    start_session(&session, user.id).await?;
    Ok(Json(user))
}

// ── admin: invites & users ─────────────────────────────────────────────────

#[derive(Serialize, sqlx::FromRow)]
pub struct InviteInfo {
    code: String,
    used_by: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    revoked_at: Option<OffsetDateTime>,
}

pub async fn list_invites(State(s): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<InviteInfo>>> {
    user.require_admin()?;
    Ok(Json(
        sqlx::query_as::<_, InviteInfo>(
            "SELECT i.code, u.username AS used_by, i.expires_at, i.revoked_at FROM invites i LEFT JOIN users u ON u.id = i.used_by ORDER BY i.expires_at DESC",
        )
        .fetch_all(&s.pool)
        .await?,
    ))
}

pub async fn create_invite(State(s): State<AppState>, user: AuthUser) -> ApiResult<Json<InviteInfo>> {
    user.require_admin()?;
    let code = random_code(2);
    Ok(Json(
        sqlx::query_as::<_, InviteInfo>(
            "INSERT INTO invites (code, created_by, expires_at) VALUES ($1, $2, now() + interval '7 days')
             RETURNING code, NULL::text AS used_by, expires_at, revoked_at",
        )
        .bind(&code)
        .bind(user.id)
        .fetch_one(&s.pool)
        .await?,
    ))
}

pub async fn revoke_invite(State(s): State<AppState>, user: AuthUser, Path(code): Path<String>) -> ApiResult<StatusCode> {
    user.require_admin()?;
    let n = sqlx::query("UPDATE invites SET revoked_at = now() WHERE code = $1 AND used_by IS NULL AND revoked_at IS NULL")
        .bind(code.to_ascii_uppercase())
        .execute(&s.pool)
        .await?
        .rows_affected();
    if n == 0 { Err(ApiError::not_found("no unused invite with this code")) } else { Ok(StatusCode::NO_CONTENT) }
}

#[derive(Serialize, sqlx::FromRow)]
pub struct UserInfo {
    username: String,
    role: String,
    wallets: i64,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

pub async fn list_users(State(s): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<UserInfo>>> {
    user.require_admin()?;
    Ok(Json(
        sqlx::query_as::<_, UserInfo>(
            "SELECT u.username, u.role, count(w.address) FILTER (WHERE w.deleted_at IS NULL) AS wallets, u.created_at
             FROM users u LEFT JOIN user_wallets w ON w.user_id = u.id GROUP BY u.id ORDER BY u.created_at",
        )
        .fetch_all(&s.pool)
        .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_grouped_and_unambiguous() {
        let c = random_code(3);
        assert_eq!(c.len(), 14);
        assert_eq!(c.matches('-').count(), 2);
        assert!(c.chars().filter(|c| *c != '-').all(|c| ALPHABET.contains(&(c as u8))));
        assert_ne!(random_code(3), random_code(3));
    }

    #[test]
    fn code_hash_ignores_case_spaces_and_dashes() {
        assert_eq!(hash_code("abcd-efgh-jkmn"), hash_code(" ABCD EFGH JKMN "));
        assert_ne!(hash_code("ABCD-EFGH-JKMN"), hash_code("ABCD-EFGH-JKMP"));
    }

    #[test]
    fn password_policy() {
        assert!(check_password_policy("short").is_err());
        assert!(check_password_policy("long enough pw").is_ok());
        assert!(check_password_policy(&"x".repeat(129)).is_err());
    }

    #[tokio::test]
    async fn argon2_roundtrip() {
        let h = hash_password("correct horse battery".into()).await.expect("hash");
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery".into(), h.clone()).await);
        assert!(!verify_password("wrong password!!".into(), h).await);
        assert!(!verify_password("anything at all".into(), "not a phc string".into()).await);
    }

    #[test]
    fn usernames() {
        assert!(valid_username("alice"));
        assert!(valid_username("m.n_2-x"));
        assert!(!valid_username("M"));
        assert!(!valid_username("bad name"));
        assert!(!valid_username(&"a".repeat(33)));
    }

    #[test]
    fn lockout_counts_attempts_before_verifying() {
        let mut f = Failures::new();
        let t0 = std::time::Instant::now();
        for _ in 0..MAX_FAILURES {
            assert!(reserve_attempt(&mut f, "alice", t0).is_ok());
        }
        // The 6th parallel attempt is refused even though none has finished yet.
        assert!(reserve_attempt(&mut f, "alice", t0).is_err());
        assert!(reserve_attempt(&mut f, "bob", t0).is_ok());
        // After the lockout window the counter starts over.
        assert!(reserve_attempt(&mut f, "alice", t0 + LOCKOUT + std::time::Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn lockout_map_is_bounded() {
        let mut f = Failures::new();
        let t0 = std::time::Instant::now();
        for i in 0..MAX_TRACKED + 50 {
            let _ = reserve_attempt(&mut f, &format!("u{i}"), t0);
        }
        assert!(f.len() <= MAX_TRACKED);
    }
}
