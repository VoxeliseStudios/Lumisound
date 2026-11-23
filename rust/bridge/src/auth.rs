//! Tokens and request authentication (port of auth.py + main.py's
//! check_auth / get_current_user / check_admin_or_operator).

use sha2::Digest as _;
use crate::error::AppError;
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::{Duration as ChronoDuration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

pub const TOKEN_EXPIRE_DAYS: i64 = 30;
pub const OPERATOR_USER_ID: &str = "ca8a4c53-5603-472e-9287-5fb879f28090";
const SESSION_CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Claims {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    pub exp: i64,
    pub iat: i64,
}

pub struct Keys {
    enc: EncodingKey,
    dec: DecodingKey,
    validation: Validation,
}

impl Keys {
    pub fn new(secret: &str) -> Self {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.leeway = 0;
        validation.required_spec_claims.clear();
        validation.required_spec_claims.insert("exp".into());
        Self {
            enc: EncodingKey::from_secret(secret.as_bytes()),
            dec: DecodingKey::from_secret(secret.as_bytes()),
            validation,
        }
    }

    fn encode(&self, sub: Option<&str>, jti: Option<&str>, purpose: Option<&str>, ttl: ChronoDuration) -> String {
        let now = Utc::now();
        let claims = Claims {
            sub: sub.map(str::to_string),
            jti: jti.map(str::to_string),
            purpose: purpose.map(str::to_string),
            exp: (now + ttl).timestamp(),
            iat: now.timestamp(),
        };
        jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims, &self.enc).expect("jwt encode")
    }

    pub fn decode(&self, token: &str) -> Option<Claims> {
        jsonwebtoken::decode::<Claims>(token, &self.dec, &self.validation).ok().map(|d| d.claims)
    }

    pub fn create_token(&self, user_id: &str, token_id: &str, expire_days: i64) -> String {
        self.encode(Some(user_id), Some(token_id), None, ChronoDuration::days(expire_days))
    }

    pub fn create_purpose_token(&self, user_id: Option<&str>, purpose: &str, seconds: i64) -> String {
        self.encode(user_id, None, Some(purpose), ChronoDuration::seconds(seconds))
    }

    /// Returns the subject of a token minted for `purpose` (totp_pending,
    /// discord_oauth_state, ...).
    pub fn decode_purpose(&self, token: &str, purpose: &str) -> Option<Claims> {
        self.decode(token).filter(|c| c.purpose.as_deref() == Some(purpose))
    }
}

pub async fn hash_password(password: String) -> anyhow::Result<String> {
    Ok(tokio::task::spawn_blocking(move || bcrypt::hash(password, bcrypt::DEFAULT_COST)).await??)
}

pub async fn verify_password(password: String, hashed: String) -> bool {
    tokio::task::spawn_blocking(move || bcrypt::verify(password, &hashed).unwrap_or(false))
        .await
        .unwrap_or(false)
}

fn bearer(parts: &Parts) -> Option<&str> {
    parts.headers.get("authorization")?.to_str().ok()?.strip_prefix("Bearer ")
}

impl AppState {
    /// Session row still live? Cached for 60s per token id, like the Python
    /// `_session_cache`, so revocation takes effect within a minute.
    pub async fn session_alive(&self, token_id: &str) -> Result<bool, AppError> {
        let now = Instant::now();
        if let Some(until) = self.session_cache.get(token_id) {
            if now < *until {
                return Ok(true);
            }
        }
        let alive: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM ios_user_sessions WHERE token_id = $1 AND expires_at > NOW()",
        )
        .bind(token_id)
        .fetch_optional(&self.db)
        .await?;
        if alive.is_some() {
            self.session_cache.insert(token_id.to_string(), now + SESSION_CACHE_TTL);
            Ok(true)
        } else {
            self.session_cache.remove(token_id);
            Ok(false)
        }
    }

    pub fn forget_session(&self, token_id: &str) {
        self.session_cache.remove(token_id);
    }
}

/// A signed-in account (`Depends(get_current_user)`).
#[derive(Debug, Clone)]
pub struct User {
    pub id: String,
    pub token_id: Option<String>,
    pub claims: Claims,
}

impl User {
    pub fn payload(&self) -> Value {
        serde_json::to_value(&self.claims).unwrap_or(Value::Null)
    }
}

impl FromRequestParts<AppState> for User {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = bearer(parts).ok_or_else(|| AppError::unauthorized("Authentication required"))?;
        let claims = state.keys.decode(token).ok_or_else(|| {
            tracing::warn!("JWT decode failed for token (invalid or expired)");
            AppError::unauthorized("Invalid or expired token")
        })?;
        // Short-lived purpose tokens (2FA pending, OAuth state) are not
        // sessions. The Python bridge accepted them here; that let a
        // half-finished 2FA login act as the account.
        if claims.purpose.is_some() {
            return Err(AppError::unauthorized("Invalid or expired token"));
        }
        let id = claims.sub.clone().ok_or_else(|| AppError::unauthorized("Invalid or expired token"))?;
        if let Some(jti) = &claims.jti {
            if !state.session_alive(jti).await? {
                return Err(AppError::unauthorized("Session has been revoked"));
            }
        }
        state.note_session_ip(&parts.headers, &id);
        Ok(User { id, token_id: claims.jti.clone(), claims })
    }
}

/// Optional account: `Some` for a valid session, `None` otherwise.
pub struct MaybeUser(pub Option<User>);

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        Ok(MaybeUser(User::from_request_parts(parts, state).await.ok()))
    }
}

/// Constant-time string comparison (only the length can leak).
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl AppState {
    /// The account behind a session token: a validly signed, non-purpose JWT
    /// whose session row is still live (logout/revocation ends it).
    pub async fn session_user(&self, token: &str) -> Option<String> {
        let c = self.keys.decode(token).filter(|c| c.purpose.is_none())?;
        let jti = c.jti.as_deref()?;
        if !self.session_alive(jti).await.ok()? {
            return None;
        }
        c.sub
    }

    /// Session from `X-Account-Token`, or a user JWT in `Authorization`.
    pub async fn header_session_user(&self, headers: &axum::http::HeaderMap) -> Option<String> {
        let u = self.header_session_user_inner(headers).await?;
        self.note_session_ip(headers, &u);
        Some(u)
    }

    async fn header_session_user_inner(&self, headers: &axum::http::HeaderMap) -> Option<String> {
        if let Some(t) = headers.get("x-account-token").and_then(|v| v.to_str().ok()).filter(|t| !t.is_empty()) {
            if let Some(u) = self.session_user(t).await {
                return Some(u);
            }
        }
        let b = headers.get("authorization")?.to_str().ok()?.strip_prefix("Bearer ")?;
        if !self.cfg.api_key.is_empty() && ct_eq(b, &self.cfg.api_key) {
            return None;
        }
        self.session_user(b).await
    }

    /// The account a stream ticket belongs to. Tickets are kept in memory
    /// and in `ios_stream_tickets` (by SHA-256), so a bridge restart no
    /// longer turns every playing track's ticket into a 401.
    pub async fn ticket_user(&self, ticket: &str) -> Option<String> {
        if ticket.is_empty() || ticket.len() > 64 {
            return None;
        }
        if let Some(u) = self.stream_tickets.get(&ticket.to_string()) {
            return Some(u);
        }
        let hash = hex::encode(sha2::Sha256::digest(ticket.as_bytes()));
        let row: Option<(String, chrono::NaiveDateTime)> =
            sqlx::query_as("SELECT user_id, expires_at FROM ios_stream_tickets WHERE ticket_hash = $1 AND expires_at > NOW() AT TIME ZONE 'UTC'")
                .bind(&hash)
                .fetch_optional(&self.db)
                .await
                .ok()
                .flatten();
        let (user, expires) = row?;
        let left = (expires - chrono::Utc::now().naive_utc()).num_seconds().max(1) as u64;
        self.stream_tickets.set_ttl(ticket.to_string(), user.clone(), std::time::Duration::from_secs(left));
        Some(user)
    }

    /// Remember which account is signed in from this client IP (see
    /// `session_ips`). Only forwarded addresses count: without
    /// X-Forwarded-For every client would share one key.
    pub fn note_session_ip(&self, headers: &axum::http::HeaderMap, user: &str) {
        let ip = client_ip(headers, None);
        if ip != "unknown" {
            self.session_ips.set(ip, user.to_string());
        }
    }

    /// The account signed in from this IP recently, for a request that
    /// carries the shared app key as its bearer (old app builds).
    pub fn ip_grace_user(&self, headers: &axum::http::HeaderMap) -> Option<String> {
        let b = headers.get("authorization")?.to_str().ok()?.strip_prefix("Bearer ")?;
        if self.cfg.api_key.is_empty() || !ct_eq(b, &self.cfg.api_key) {
            return None;
        }
        let ip = client_ip(headers, None);
        if ip == "unknown" {
            return None;
        }
        self.session_ips.get(&ip)
    }
}

/// Shared-service routes (search, stream, download, lyrics, ...). The shared
/// app key ships inside the public IPAs/APK, so it proves nothing on its own:
/// these routes need a signed-in, unrevoked session as well (sent as
/// `X-Account-Token` beside the key, or as the bearer itself). `user` is that
/// account.
pub struct ApiKey {
    pub user: String,
}

impl FromRequestParts<AppState> for ApiKey {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        match state.header_session_user(&parts.headers).await {
            Some(user) => Ok(ApiKey { user }),
            None => Err(AppError::unauthorized("Sign in to use this feature")),
        }
    }
}

/// A live session from the headers, or a stream ticket (`?ticket=`) for URLs
/// handed to a media player that cannot carry headers.
pub struct SessionOrTicket {
    pub user: String,
}

impl FromRequestParts<AppState> for SessionOrTicket {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(user) = state.header_session_user(&parts.headers).await {
            return Ok(SessionOrTicket { user });
        }
        let ticket = parts.uri.query().unwrap_or("").split('&').find_map(|kv| kv.strip_prefix("ticket=")).filter(|t| !t.is_empty());
        if let Some(t) = ticket {
            if let Some(user) = state.ticket_user(t).await {
                return Ok(SessionOrTicket { user });
            }
        }
        Err(AppError::unauthorized("Sign in to use this feature"))
    }
}

/// `Depends(check_admin_or_operator)`: the operator's own session, or ADMIN_TOKEN.
pub struct Admin;

impl FromRequestParts<AppState> for Admin {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        admin_check(state, bearer(parts)).await
    }
}

/// Admin credentials: the operator's live session token, or ADMIN_TOKEN.
/// Shared by the `Admin` extractor (Bearer) and the dashboard page (cookie).
pub async fn admin_check(state: &AppState, token: Option<&str>) -> Result<Admin, AppError> {
    if let Some(c) = token.and_then(|t| state.keys.decode(t)) {
        if c.sub.as_deref() == Some(OPERATOR_USER_ID) && c.purpose.is_none() {
            let jti = c.jti.ok_or_else(|| AppError::unauthorized("Malformed session token"))?;
            let alive: Option<i32> = sqlx::query_scalar(
                "SELECT 1 FROM ios_user_sessions WHERE token_id = $1 AND expires_at > NOW()",
            )
            .bind(&jti)
            .fetch_optional(&state.db)
            .await?;
            return if alive.is_some() { Ok(Admin) } else { Err(AppError::unauthorized("Session has been revoked")) };
        }
    }
    if state.cfg.admin_token.is_empty() {
        return Err(AppError::new(503, "Admin dashboard not configured (set ADMIN_TOKEN)"));
    }
    if token.is_some_and(|t| ct_eq(t, &state.cfg.admin_token)) {
        Ok(Admin)
    } else {
        Err(AppError::unauthorized("Invalid admin credentials"))
    }
}

/// `X-Account-Token` header's account id (used by API-key routes that can
/// optionally attribute work to an account).
pub fn account_token_user_id(state: &AppState, headers: &axum::http::HeaderMap) -> Option<String> {
    let t = headers.get("x-account-token")?.to_str().ok()?;
    if t.is_empty() {
        return None;
    }
    state.keys.decode(t).filter(|c| c.purpose.is_none())?.sub
}

/// Client IP as the Python bridge computed it: last X-Forwarded-For hop
/// (cloudflared appends the real client), else the socket peer.
pub fn client_ip(headers: &axum::http::HeaderMap, peer: Option<std::net::SocketAddr>) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(last) = xff.split(',').map(str::trim).filter(|p| !p.is_empty()).last() {
            return last.to_string();
        }
    }
    peer.map(|p| p.ip().to_string()).unwrap_or_else(|| "unknown".into())
}
