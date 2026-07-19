//! Production security helpers: rate limits, CSRF, password policy, headers.

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{header, HeaderMap, HeaderValue, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    sync::Mutex,
    time::{Duration, Instant},
};
use uuid::Uuid;

/// Max attempts per IP per window for login / user-create.
pub const RATE_MAX: usize = 5;
pub const RATE_WINDOW: Duration = Duration::from_secs(60);
/// Lockout duration after exceeding RATE_MAX.
pub const RATE_LOCKOUT: Duration = Duration::from_secs(60);

const CSRF_COOKIE: &str = "maxcos_csrf";
const CSRF_MAX_AGE: i64 = 60 * 60 * 8;

#[derive(Default)]
struct Bucket {
    attempts: VecDeque<Instant>,
    locked_until: Option<Instant>,
}

#[derive(Default)]
pub struct RateLimiter {
    buckets: HashMap<String, Bucket>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an attempt. Ok if allowed; Err with human message if locked / over limit.
    pub fn check(&mut self, kind: &str, ip: &str) -> Result<(), String> {
        let key = format!("{kind}:{ip}");
        let now = Instant::now();
        let b = self.buckets.entry(key).or_default();

        if let Some(until) = b.locked_until {
            if now < until {
                let secs = until.saturating_duration_since(now).as_secs().max(1);
                return Err(format!(
                    "Too many attempts from this IP. Locked out for {secs}s (max {}/min).",
                    RATE_MAX
                ));
            }
            b.locked_until = None;
            b.attempts.clear();
        }

        while b
            .attempts
            .front()
            .is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW)
        {
            b.attempts.pop_front();
        }

        if b.attempts.len() >= RATE_MAX {
            b.locked_until = Some(now + RATE_LOCKOUT);
            return Err(format!(
                "Rate limit exceeded (max {} tries/min per IP). Locked out for {}s.",
                RATE_MAX,
                RATE_LOCKOUT.as_secs()
            ));
        }

        b.attempts.push_back(now);
        Ok(())
    }

    /// Clear attempts after a successful auth (optional UX).
    pub fn reset(&mut self, kind: &str, ip: &str) {
        let key = format!("{kind}:{ip}");
        self.buckets.remove(&key);
    }
}

pub type SharedRateLimit = std::sync::Arc<Mutex<RateLimiter>>;

pub fn new_rate_limit() -> SharedRateLimit {
    std::sync::Arc::new(Mutex::new(RateLimiter::new()))
}

pub fn client_ip(headers: &HeaderMap, addr: Option<SocketAddr>) -> String {
    if let Some(xff) = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        if let Some(first) = xff.split(',').next() {
            let t = first.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    if let Some(real) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let t = real.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    addr.map(|a| a.ip().to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub fn extract_connect_ip(headers: &HeaderMap, connect: Option<ConnectInfo<SocketAddr>>) -> String {
    client_ip(headers, connect.map(|c| c.0))
}

// ── Password policy ─────────────────────────────────────────────────────────

pub fn validate_password(password: &str) -> Result<(), String> {
    if password.len() < 8 {
        return Err("Password must be at least 8 characters".into());
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        return Err("Password must contain at least one number".into());
    }
    Ok(())
}

// ── CSRF (double-submit cookie) ─────────────────────────────────────────────

pub fn new_csrf_token() -> String {
    Uuid::new_v4().to_string().replace('-', "")
}

/// When `MAXCOS_INSECURE_COOKIES=1`, omit the Secure flag (local HTTP only).
/// Production default keeps Secure on all cookies.
pub fn cookie_secure_attr() -> &'static str {
    match std::env::var("MAXCOS_INSECURE_COOKIES") {
        Ok(v) if v == "1" || v.eq_ignore_ascii_case("true") => "",
        _ => "; Secure",
    }
}

pub fn set_csrf_cookie(token: &str) -> String {
    // Double-submit cookie: form field must match. SameSite=Lax; Secure in production.
    format!(
        "{CSRF_COOKIE}={token}; Path=/; SameSite=Lax{}; Max-Age={CSRF_MAX_AGE}",
        cookie_secure_attr()
    )
}

pub fn csrf_from_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| {
            c.split(';').find_map(|p| {
                p.trim()
                    .strip_prefix(&format!("{CSRF_COOKIE}="))
                    .map(|s| s.to_string())
            })
        })
}

pub fn verify_csrf(headers: &HeaderMap, form_token: Option<&str>) -> Result<(), String> {
    let cookie = csrf_from_cookie(headers).ok_or_else(|| "Missing CSRF cookie".to_string())?;
    let form = form_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing CSRF token".to_string())?;
    if cookie.len() < 16 || form.len() < 16 || cookie != form {
        return Err("Invalid CSRF token".into());
    }
    Ok(())
}

// ── Security response headers ───────────────────────────────────────────────

pub async fn security_headers_middleware(req: Request<Body>, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        header::HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    h.insert(
        header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    h.insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
    );
    // Same-origin app: scripts/styles from self; inline allowed for existing templates.
    h.insert(
        header::HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; \
             script-src 'self' 'unsafe-inline'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob:; \
             font-src 'self' data:; \
             connect-src 'self'; \
             frame-src 'self'; \
             frame-ancestors 'none'; \
             base-uri 'self'; \
             form-action 'self'; \
             object-src 'none'",
        ),
    );
    res
}

pub fn rate_limited_response(msg: &str) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, "60")],
        msg.to_string(),
    )
        .into_response()
}

pub fn rate_limited_json(msg: &str) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, "60")],
        axum::Json(serde_json::json!({ "error": "rate_limited", "message": msg })),
    )
        .into_response()
}
