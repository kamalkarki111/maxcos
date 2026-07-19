//! Maxcos — multi-user macOS simulator (Rust SSR)
mod apps;
mod proxy;
mod security;
mod store;
mod terminal;

use askama::Template;
use axum::{
    body::Body,
    extract::{ConnectInfo, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Json, Router,
};
use chrono::{Datelike, Local, Timelike};
use serde::Deserialize;
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

use security::SharedRateLimit;
use store::{
    Note, Reminder, SafariBookmark, SafariTab, Store, UserSettings,
};

#[derive(Clone)]
struct AppState {
    store: Arc<Store>,
    lock: Arc<Mutex<()>>,
    rate_limit: SharedRateLimit,
}

#[derive(Debug, Clone)]
struct SessionData {
    id: String,
    user_id: String,
    username: String,
}

const SESSION_COOKIE: &str = "maxcos_session";
const SESSION_MAX_AGE: i64 = store::Store::SESSION_TTL_SECS;

struct HtmlTemplate<T>(T);
impl<T: Template> IntoResponse for HtmlTemplate<T> {
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(body) => Html(body).into_response(),
            Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{err}")).into_response(),
        }
    }
}

#[derive(Template)]
#[template(path = "boot.html")]
struct BootTemplate {}

#[derive(Template)]
#[template(path = "setup.html")]
struct SetupTemplate {
    error: Option<String>,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: Option<String>,
    time: String,
    date: String,
    users_json: String,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "desktop.html")]
struct DesktopTemplate {
    username: String,
    user_id: String,
    time: String,
    date: String,
    wallpaper: String,
    settings_json: String,
    dock_apps_json: String,
    desktop_icons_json: String,
    all_apps_json: String,
}

fn session_cookie_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| {
            c.split(';').find_map(|p| {
                p.trim()
                    .strip_prefix(&format!("{SESSION_COOKIE}="))
                    .map(|s| s.to_string())
            })
        })
}

fn set_session_cookie(session_id: &str) -> String {
    // HttpOnly + SameSite=Lax + Secure (omit Secure only if MAXCOS_INSECURE_COOKIES=1)
    format!(
        "{SESSION_COOKIE}={session_id}; Path=/; HttpOnly; SameSite=Lax{}; Max-Age={SESSION_MAX_AGE}",
        security::cookie_secure_attr()
    )
}

fn clear_session_cookie() -> String {
    format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax{}; Max-Age=0",
        security::cookie_secure_attr()
    )
}

fn with_csrf_cookie(mut res: Response, token: &str) -> Response {
    if let Ok(val) = header::HeaderValue::from_str(&security::set_csrf_cookie(token)) {
        res.headers_mut().append(header::SET_COOKIE, val);
    }
    res
}

fn rate_check(state: &AppState, kind: &str, ip: &str) -> Result<(), String> {
    state
        .rate_limit
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .check(kind, ip)
}

fn rate_reset(state: &AppState, kind: &str, ip: &str) {
    state
        .rate_limit
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .reset(kind, ip);
}

fn redirect_with_session(location: &str, session_id: &str) -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::SET_COOKIE, set_session_cookie(session_id))
        .header(header::LOCATION, location)
        .body(Body::empty())
        .unwrap()
}

fn public_users_json(store: &Store) -> String {
    let users = store.list_users();
    serde_json::to_string(
        &users
            .iter()
            .map(|u| {
                serde_json::json!({
                    "id": u.id,
                    "name": u.name,
                    "avatar": u.avatar,
                    "color": u.color,
                    "has_password": !u.password_hash.is_empty(),
                    "password_hint": u.password_hint,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".into())
}

/// Resolve cookie → persistent session (touch on disk).
fn require_user(state: &AppState, headers: &HeaderMap) -> Result<SessionData, Response> {
    let Some(sid) = session_cookie_id(headers) else {
        return Err(StatusCode::UNAUTHORIZED.into_response());
    };
    let _g = state.lock.lock().unwrap();
    match state.store.touch_session(&sid) {
        Ok(Some(s)) => Ok(SessionData {
            id: s.id,
            user_id: s.user_id,
            username: s.username,
        }),
        Ok(None) => Err(StatusCode::UNAUTHORIZED.into_response()),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

fn create_login_session(state: &AppState, user_id: &str, username: &str) -> Result<String, String> {
    let _g = state.lock.lock().unwrap();
    state
        .store
        .create_session(user_id, username)
        .map(|s| s.id)
        .map_err(|e| e.to_string())
}

// ─── Pages ──────────────────────────────────────────────────────────────────

async fn boot() -> impl IntoResponse {
    HtmlTemplate(BootTemplate {})
}

/// After boot: valid session → desktop; no users → setup; else login.
async fn entry(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Ok(sess) = require_user(&state, &headers) {
        let _ = sess;
        return Redirect::to("/desktop").into_response();
    }
    let has = state.store.has_users();
    if !has {
        Redirect::to("/setup").into_response()
    } else {
        Redirect::to("/login").into_response()
    }
}

async fn setup_page(State(state): State<AppState>) -> impl IntoResponse {
    if state.store.has_users() {
        return Redirect::to("/login").into_response();
    }
    let csrf = security::new_csrf_token();
    let res = HtmlTemplate(SetupTemplate {
        error: None,
        csrf_token: csrf.clone(),
    })
    .into_response();
    with_csrf_cookie(res, &csrf)
}

#[derive(Deserialize)]
struct SetupForm {
    full_name: String,
    account_name: Option<String>,
    password: String,
    password_confirm: String,
    password_hint: Option<String>,
    avatar: Option<String>,
    color: Option<String>,
    csrf_token: Option<String>,
}

async fn setup_submit(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<SetupForm>,
) -> impl IntoResponse {
    let ip = security::client_ip(&headers, Some(addr));
    let csrf = security::new_csrf_token();
    let fail = |msg: &str| {
        let res = HtmlTemplate(SetupTemplate {
            error: Some(msg.into()),
            csrf_token: csrf.clone(),
        })
        .into_response();
        with_csrf_cookie(res, &csrf)
    };

    if let Err(e) = security::verify_csrf(&headers, form.csrf_token.as_deref()) {
        return fail(&e);
    }
    if state.store.has_users() {
        return Redirect::to("/login").into_response();
    }
    if let Err(msg) = rate_check(&state, "create", &ip) {
        state.store.audit(
            "rate_limit_lockout",
            "",
            &ip,
            &format!("user-create (setup form): {msg}"),
        );
        return fail(&msg);
    }
    let name = form.full_name.trim().to_string();
    if name.is_empty() {
        return fail("Enter your name.");
    }
    if form.password != form.password_confirm {
        return fail("Passwords do not match.");
    }
    if let Err(e) = security::validate_password(&form.password) {
        return fail(&e);
    }
    let _ = form.account_name;
    let _g = state.lock.lock().unwrap();
    let created = state.store.create_user(
        name.clone(),
        form.avatar.unwrap_or_default(),
        form.color.unwrap_or_else(|| "#0A84FF".into()),
        form.password,
        form.password_hint.unwrap_or_default(),
    );
    drop(_g);
    match created {
        Ok(u) => {
            rate_reset(&state, "create", &ip);
            match create_login_session(&state, &u.id, &u.name) {
                Ok(sid) => redirect_with_session("/desktop", &sid),
                Err(e) => fail(&format!("Account created but sign-in failed: {e}")),
            }
        }
        Err(e) => fail(&e.to_string()),
    }
}

async fn login_page(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !state.store.has_users() {
        return Redirect::to("/setup").into_response();
    }
    if require_user(&state, &headers).is_ok() {
        return Redirect::to("/desktop").into_response();
    }
    let now = Local::now();
    let csrf = security::new_csrf_token();
    let res = HtmlTemplate(LoginTemplate {
        error: None,
        time: now.format("%-I:%M").to_string(),
        date: now.format("%A, %B %-d").to_string(),
        users_json: public_users_json(&state.store),
        csrf_token: csrf.clone(),
    })
    .into_response();
    with_csrf_cookie(res, &csrf)
}

#[derive(Deserialize)]
struct LoginForm {
    user_id: Option<String>,
    username: Option<String>,
    password: Option<String>,
    csrf_token: Option<String>,
}

async fn login_submit(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    if !state.store.has_users() {
        return Redirect::to("/setup").into_response();
    }
    let ip = security::client_ip(&headers, Some(addr));
    let now = Local::now();
    let csrf = security::new_csrf_token();
    let fail = |msg: &str| {
        let res = HtmlTemplate(LoginTemplate {
            error: Some(msg.into()),
            time: now.format("%-I:%M").to_string(),
            date: now.format("%A, %B %-d").to_string(),
            users_json: public_users_json(&state.store),
            csrf_token: csrf.clone(),
        })
        .into_response();
        with_csrf_cookie(res, &csrf)
    };

    if let Err(e) = security::verify_csrf(&headers, form.csrf_token.as_deref()) {
        return fail(&e);
    }
    if let Err(msg) = rate_check(&state, "login", &ip) {
        state.store.audit(
            "rate_limit_lockout",
            "",
            &ip,
            &format!("login (form): {msg}"),
        );
        return fail(&msg);
    }

    let account = if let Some(id) = form.user_id.as_ref().filter(|s| !s.is_empty()) {
        state.store.find_user(id)
    } else if let Some(name) = form.username.as_ref().filter(|s| !s.trim().is_empty()) {
        state.store.find_user_by_name(name)
    } else {
        None
    };

    let Some(account) = account else {
        state.store.audit(
            "login_fail",
            "",
            form.username.as_deref().unwrap_or(""),
            "User not found (form login)",
        );
        return fail("Select a user to continue.");
    };

    let pw = form.password.unwrap_or_default();
    if !Store::verify_password(&pw, &account.password_hash) {
        state.store.audit(
            "login_fail",
            &account.id,
            &account.name,
            "Incorrect password (form login)",
        );
        return fail("Incorrect password.");
    }

    match create_login_session(&state, &account.id, &account.name) {
        Ok(sid) => {
            rate_reset(&state, "login", &ip);
            state.store.audit(
                "login_success",
                &account.id,
                &account.name,
                "Signed in via form login",
            );
            redirect_with_session("/desktop", &sid)
        }
        Err(e) => fail(&format!("Could not create session: {e}")),
    }
}

async fn desktop(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !state.store.has_users() {
        return Redirect::to("/setup").into_response();
    }
    let Ok(sess) = require_user(&state, &headers) else {
        return Redirect::to("/login").into_response();
    };
    let settings = {
        let _g = state.lock.lock().unwrap();
        state.store.settings_get(&sess.user_id)
    };
    let wallpaper = settings
        .spaces
        .get(settings.active_space)
        .map(|s| s.wallpaper.clone())
        .unwrap_or_else(|| settings.wallpaper.clone());
    let now = Local::now();
    HtmlTemplate(DesktopTemplate {
        username: sess.username,
        user_id: sess.user_id,
        time: now.format("%-I:%M %p").to_string(),
        date: now.format("%a %b %-d").to_string(),
        wallpaper,
        settings_json: serde_json::to_string(&settings).unwrap_or_else(|_| "{}".into()),
        dock_apps_json: serde_json::to_string(&apps::dock_apps()).unwrap(),
        desktop_icons_json: serde_json::to_string(&apps::desktop_icons()).unwrap(),
        all_apps_json: serde_json::to_string(&apps::all_apps()).unwrap(),
    })
    .into_response()
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(sid) = session_cookie_id(&headers) {
        let _g = state.lock.lock().unwrap();
        let _ = state.store.destroy_session(&sid);
    }
    let dest = if state.store.has_users() {
        "/login"
    } else {
        "/setup"
    };
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::SET_COOKIE, clear_session_cookie())
        .header(header::LOCATION, dest)
        .body(Body::empty())
        .unwrap()
}

async fn health() -> &'static str {
    "ok"
}

async fn api_time() -> Json<serde_json::Value> {
    let now = Local::now();
    Json(serde_json::json!({
        "time": now.format("%-I:%M %p").to_string(),
        "time_short": now.format("%-I:%M").to_string(),
        "date": now.format("%a %b %-d").to_string(),
        "date_full": now.format("%A, %B %-d, %Y").to_string(),
        "hour": now.hour(), "minute": now.minute(), "second": now.second(),
        "day": now.day(), "month": now.month(), "year": now.year(),
    }))
}

async fn api_apps() -> Json<Vec<apps::AppMeta>> {
    Json(apps::all_apps())
}

// ── Session API ─────────────────────────────────────────────────────────────

async fn api_session_me(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    match require_user(&state, &headers) {
        Ok(sess) => {
            let _g = state.lock.lock().unwrap();
            if let Some(rec) = state.store.get_session(&sess.id) {
                Json(serde_json::json!({
                    "authenticated": true,
                    "session": Store::session_info(&rec),
                }))
                .into_response()
            } else {
                Json(serde_json::json!({ "authenticated": false })).into_response()
            }
        }
        Err(_) => Json(serde_json::json!({ "authenticated": false })).into_response(),
    }
}

#[derive(Deserialize)]
struct ApiLoginBody {
    user_id: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

async fn api_session_login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ApiLoginBody>,
) -> impl IntoResponse {
    let ip = security::client_ip(&headers, Some(addr));
    if let Err(msg) = rate_check(&state, "login", &ip) {
        state.store.audit(
            "rate_limit_lockout",
            "",
            &ip,
            &format!("login (API): {msg}"),
        );
        return security::rate_limited_json(&msg);
    }
    if !state.store.has_users() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "no_users", "setup_required": true })),
        )
            .into_response();
    }
    let account = if let Some(id) = body.user_id.as_ref().filter(|s| !s.is_empty()) {
        state.store.find_user(id)
    } else if let Some(name) = body.username.as_ref().filter(|s| !s.trim().is_empty()) {
        state.store.find_user_by_name(name)
    } else {
        None
    };
    let Some(account) = account else {
        state.store.audit(
            "login_fail",
            "",
            body.username.as_deref().unwrap_or(""),
            "User not found (API login)",
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "user_not_found" })),
        )
            .into_response();
    };
    let pw = body.password.unwrap_or_default();
    if !Store::verify_password(&pw, &account.password_hash) {
        state.store.audit(
            "login_fail",
            &account.id,
            &account.name,
            "Incorrect password (API login)",
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "invalid_password" })),
        )
            .into_response();
    }
    match create_login_session(&state, &account.id, &account.name) {
        Ok(sid) => {
            rate_reset(&state, "login", &ip);
            state.store.audit(
                "login_success",
                &account.id,
                &account.name,
                "Signed in via API login",
            );
            let _g = state.lock.lock().unwrap();
            let info = state
                .store
                .get_session(&sid)
                .map(|s| Store::session_info(&s));
            (
                StatusCode::OK,
                [(header::SET_COOKIE, set_session_cookie(&sid))],
                Json(serde_json::json!({
                    "ok": true,
                    "session": info,
                    "user": { "id": account.id, "name": account.name, "avatar": account.avatar, "color": account.color }
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn api_session_logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(sid) = session_cookie_id(&headers) {
        let _g = state.lock.lock().unwrap();
        let _ = state.store.destroy_session(&sid);
    }
    (
        StatusCode::OK,
        [(header::SET_COOKIE, clear_session_cookie())],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

async fn api_setup_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "has_users": state.store.has_users(),
        "user_count": state.store.user_count(),
        "setup_required": !state.store.has_users(),
    }))
}


// ── Users API ───────────────────────────────────────────────────────────────

async fn api_users_list(State(state): State<AppState>) -> Json<serde_json::Value> {
    let users = state.store.list_users();
    Json(serde_json::json!({
        "users": users.iter().map(|u| serde_json::json!({
            "id": u.id, "name": u.name, "avatar": u.avatar, "color": u.color,
            "has_password": !u.password_hash.is_empty(),
            "password_hint": u.password_hint,
        })).collect::<Vec<_>>()
    }))
}



#[derive(Deserialize)]
struct CreateUserBody {
    name: String,
    avatar: Option<String>,
    color: Option<String>,
    password: Option<String>,
    password_hint: Option<String>,
    /// If true, also create a session cookie for the new user (setup / add account)
    sign_in: Option<bool>,
}

async fn api_users_create(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<CreateUserBody>,
) -> impl IntoResponse {
    let ip = security::client_ip(&headers, Some(addr));
    if let Err(msg) = rate_check(&state, "create", &ip) {
        state.store.audit(
            "rate_limit_lockout",
            "",
            &ip,
            &format!("user-create (API): {msg}"),
        );
        return security::rate_limited_json(&msg);
    }
    // Allow create without auth (login sheet / setup). Settings → Users when signed in.
    let _ = require_user(&state, &headers);
    let _g = state.lock.lock().unwrap();
    let created = state.store.create_user(
        body.name.trim().to_string(),
        body.avatar.unwrap_or_default(),
        body.color.unwrap_or_else(|| "#0A84FF".into()),
        body.password.unwrap_or_default(),
        body.password_hint.unwrap_or_default(),
    );
    drop(_g);
    match created {
        Ok(u) => {
            rate_reset(&state, "create", &ip);
            if body.sign_in.unwrap_or(false) {
                match create_login_session(&state, &u.id, &u.name) {
                    Ok(sid) => {
                        return (
                            StatusCode::OK,
                            [(header::SET_COOKIE, set_session_cookie(&sid))],
                            Json(serde_json::json!({
                                "id": u.id, "name": u.name, "avatar": u.avatar, "color": u.color,
                                "session_id": sid, "signed_in": true
                            })),
                        )
                            .into_response();
                    }
                    Err(e) => {
                        return (
                            StatusCode::OK,
                            Json(serde_json::json!({
                                "id": u.id, "name": u.name, "avatar": u.avatar, "color": u.color,
                                "signed_in": false, "session_error": e
                            })),
                        )
                            .into_response();
                    }
                }
            }
            Json(serde_json::json!({
                "id": u.id, "name": u.name, "avatar": u.avatar, "color": u.color, "signed_in": false
            }))
            .into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn api_users_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if sess.user_id == id {
        return (StatusCode::BAD_REQUEST, "cannot delete the signed-in user").into_response();
    }
    let _g = state.lock.lock().unwrap();
    match state.store.delete_user(&id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

// ── Settings / Spaces ───────────────────────────────────────────────────────

async fn api_settings_get(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let Ok(sess) = require_user(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = state.lock.lock().unwrap();
    Json(state.store.settings_get(&sess.user_id)).into_response()
}

async fn api_settings_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(settings): Json<UserSettings>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = state.lock.lock().unwrap();
    match state.store.settings_save(&sess.user_id, &settings) {
        Ok(()) => Json(settings).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Admin / audit ───────────────────────────────────────────────────────────

async fn api_admin_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let Ok(_sess) = require_user(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let limit = q
        .get("limit")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(100);
    let _g = state.lock.lock().unwrap();
    let entries = state.store.audit_list(limit);
    Json(serde_json::json!({ "entries": entries })).into_response()
}

// ── Notes ───────────────────────────────────────────────────────────────────

async fn api_notes_list(State(s): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    Json(s.store.notes_list(&sess.user_id)).into_response()
}
#[derive(Deserialize)]
struct NoteInput {
    title: String,
    body: String,
}
async fn api_notes_create(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(i): Json<NoteInput>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.note_create(&sess.user_id, i.title, i.body) {
        Ok(n) => {
            let _ = s.store.notification_push(
                &sess.user_id,
                "Note Created".into(),
                n.title.clone(),
                "Notes".into(),
                "notes".into(),
            );
            Json(n).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
async fn api_notes_update(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(i): Json<NoteInput>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.note_update(&sess.user_id, &id, i.title, i.body) {
        Ok(Some(n)) => Json(n).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
async fn api_notes_delete(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.note_delete(&sess.user_id, &id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Reminders ───────────────────────────────────────────────────────────────

async fn api_reminders_list(State(s): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    Json(s.store.reminders_list(&sess.user_id)).into_response()
}
#[derive(Deserialize)]
struct ReminderInput {
    text: String,
}
async fn api_reminders_create(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(i): Json<ReminderInput>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.reminder_create(&sess.user_id, i.text) {
        Ok(r) => {
            let _ = s.store.notification_push(
                &sess.user_id,
                "Reminder Added".into(),
                r.text.clone(),
                "Reminders".into(),
                "reminders".into(),
            );
            Json(r).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
async fn api_reminders_toggle(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.reminder_toggle(&sess.user_id, &id) {
        Ok(Some(r)) => Json(r).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
async fn api_reminders_delete(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.reminder_delete(&sess.user_id, &id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Notifications ───────────────────────────────────────────────────────────

async fn api_notifications_list(State(s): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    let list = s.store.notifications_list(&sess.user_id);
    let unread = list.iter().filter(|n| !n.read).count();
    Json(serde_json::json!({ "notifications": list, "unread": unread })).into_response()
}
#[derive(Deserialize)]
struct NotifInput {
    title: String,
    body: String,
    app: Option<String>,
    app_id: Option<String>,
}
async fn api_notifications_push(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(i): Json<NotifInput>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.notification_push(
        &sess.user_id,
        i.title,
        i.body,
        i.app.unwrap_or_else(|| "Maxcos".into()),
        i.app_id.unwrap_or_else(|| "systemsettings".into()),
    ) {
        Ok(n) => Json(n).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
async fn api_notifications_read(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.notification_mark_read(&sess.user_id, &id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
async fn api_notifications_read_all(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.notifications_mark_all_read(&sess.user_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
async fn api_notifications_clear(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.notifications_clear(&sess.user_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
async fn api_notifications_delete(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.notification_delete(&sess.user_id, &id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── FS ──────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct FsPathQuery {
    path: Option<String>,
}
async fn api_fs_list(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<FsPathQuery>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let path = q.path.unwrap_or_else(|| "~".into());
    let _g = s.lock.lock().unwrap();
    match s.store.fs_list(&sess.user_id, &path) {
        Ok(entries) => Json(serde_json::json!({ "path": path, "entries": entries })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}
async fn api_fs_read(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<FsPathQuery>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let path = q.path.unwrap_or_default();
    let _g = s.lock.lock().unwrap();
    match s.store.fs_read(&sess.user_id, &path) {
        Ok((virt, content)) => {
            Json(serde_json::json!({ "path": virt, "content": content })).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}
#[derive(Deserialize)]
struct FsWriteBody {
    path: String,
    content: String,
}
async fn api_fs_write(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<FsWriteBody>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.fs_write(&sess.user_id, &b.path, &b.content) {
        Ok(e) => {
            let _ = s.store.notification_push(
                &sess.user_id,
                "Document Saved".into(),
                e.name.clone(),
                "TextEdit".into(),
                "textedit".into(),
            );
            Json(e).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}
#[derive(Deserialize)]
struct FsMkdirBody {
    path: String,
}
async fn api_fs_mkdir(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<FsMkdirBody>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.fs_mkdir(&sess.user_id, &b.path) {
        Ok(e) => Json(e).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}
#[derive(Deserialize)]
struct FsCreateBody {
    path: String,
    content: Option<String>,
}
async fn api_fs_create(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<FsCreateBody>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s
        .store
        .fs_create_file(&sess.user_id, &b.path, b.content.as_deref().unwrap_or(""))
    {
        Ok(e) => Json(e).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}
async fn api_fs_delete(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<FsPathQuery>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let path = q.path.unwrap_or_default();
    let _g = s.lock.lock().unwrap();
    match s.store.fs_delete(&sess.user_id, &path) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}
#[derive(Deserialize)]
struct FsRenameBody {
    from: String,
    to: String,
}
async fn api_fs_rename(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<FsRenameBody>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    match s.store.fs_rename(&sess.user_id, &b.from, &b.to) {
        Ok(e) => Json(e).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    limit: Option<usize>,
}
async fn api_search(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let query = q.q.unwrap_or_default();
    let limit = q.limit.unwrap_or(25).min(50);
    let _g = s.lock.lock().unwrap();
    Json(serde_json::json!({ "q": query, "results": s.store.search(&sess.user_id, &query, limit) }))
        .into_response()
}

// ── Safari ──────────────────────────────────────────────────────────────────

async fn api_safari_state(State(s): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    Json(s.store.safari_get(&sess.user_id)).into_response()
}
#[derive(Deserialize)]
struct SafariStateUpdate {
    tabs: Option<Vec<SafariTab>>,
    bookmarks: Option<Vec<SafariBookmark>>,
}
async fn api_safari_save(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(u): Json<SafariStateUpdate>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    let mut st = s.store.safari_get(&sess.user_id);
    if let Some(tabs) = u.tabs {
        st.tabs = tabs;
    }
    if let Some(bm) = u.bookmarks {
        st.bookmarks = bm;
    }
    match s.store.safari_save(&sess.user_id, &st) {
        Ok(()) => Json(st).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
#[derive(Deserialize)]
struct SafariNav {
    url: String,
    title: Option<String>,
    tab_id: Option<String>,
}
async fn api_safari_navigate(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(n): Json<SafariNav>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let _g = s.lock.lock().unwrap();
    let title = n.title.unwrap_or_else(|| n.url.clone());
    let _ = s.store.safari_add_history(&sess.user_id, &title, &n.url);
    let mut st = s.store.safari_get(&sess.user_id);
    if let Some(tid) = n.tab_id {
        if let Some(tab) = st.tabs.iter_mut().find(|t| t.id == tid) {
            tab.url = n.url.clone();
            tab.title = title;
        }
    }
    let _ = s.store.safari_save(&sess.user_id, &st);
    Json(serde_json::json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
struct ProxyQuery {
    url: Option<String>,
}
async fn proxy_handler(State(s): State<AppState>, headers: HeaderMap, Query(q): Query<ProxyQuery>) -> impl IntoResponse {
    let raw = q.url.unwrap_or_default();
    if raw.is_empty() || raw == "about:start" || raw == "about:blank" {
        return start_page().into_response();
    }
    // history only if logged in
    let user_id = require_user(&s, &headers).ok().map(|x| x.user_id);
    match proxy::fetch_proxied(&raw).await {
        Ok(p) => {
            if let Some(uid) = user_id {
                let _g = s.lock.lock().unwrap();
                let title = p.title.clone().unwrap_or_else(|| p.final_url.clone());
                let _ = s.store.safari_add_history(&uid, &title, &p.final_url);
            }
            let mut res = Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(p.bytes))
                .unwrap();
            let headers = res.headers_mut();
            if let Ok(v) = HeaderValue::from_str(&p.content_type) {
                headers.insert(header::CONTENT_TYPE, v);
            }
            headers.insert(
                header::HeaderName::from_static("x-frame-options"),
                HeaderValue::from_static("SAMEORIGIN"),
            );
            if let Ok(v) = HeaderValue::from_str(&p.final_url) {
                headers.insert(header::HeaderName::from_static("x-proxied-url"), v);
            }
            res
        }
        Err(e) => {
            let html = format!(
                r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>Error</title>
<style>body{{font-family:-apple-system,sans-serif;padding:40px;background:#f5f5f7}}
.box{{max-width:520px;margin:40px auto;background:#fff;border-radius:12px;padding:24px}}</style></head>
<body><div class="box"><h1>Safari couldn’t open the page</h1><p>{}</p>
<p><a href="/proxy?url=about:start">Start Page</a></p></div></body></html>"#,
                html_escape(&e)
            );
            Html(html).into_response()
        }
    }
}

fn start_page() -> Html<String> {
    Html(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>Start Page</title>
<style>body{margin:0;font-family:-apple-system,sans-serif;background:#f5f5f7}
h1{text-align:center;margin:48px 0 24px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(100px,1fr));gap:16px;max-width:640px;margin:0 auto;padding:16px}
a{display:flex;flex-direction:column;align-items:center;gap:8px;text-decoration:none;color:inherit;padding:12px;border-radius:12px}
a:hover{background:rgba(0,0,0,.05)}
.icon{width:56px;height:56px;border-radius:12px;display:grid;place-items:center;color:#fff;font-weight:700}
</style></head><body><h1>Favorites</h1><div class="grid">
<a href="/proxy?url=https://www.apple.com"><div class="icon" style="background:#555"></div><span>Apple</span></a>
<a href="/proxy?url=https://www.wikipedia.org"><div class="icon" style="background:#000">W</div><span>Wikipedia</span></a>
<a href="/proxy?url=https://duckduckgo.com"><div class="icon" style="background:#de5833">DD</div><span>DuckDuckGo</span></a>
<a href="/proxy?url=https://github.com"><div class="icon" style="background:#24292e">GH</div><span>GitHub</span></a>
</div></body></html>"#
            .into(),
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

// ── Terminal ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TerminalCmd {
    cmd: String,
    cwd: Option<String>,
}
async fn api_terminal(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TerminalCmd>,
) -> impl IntoResponse {
    let Ok(sess) = require_user(&s, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let cwd = body.cwd.unwrap_or_else(|| "~".into());
    let store = s.store.clone();
    let uid = sess.user_id.clone();
    let result = terminal::run_command(&store, &uid, &body.cmd, &cwd).await;
    Json(serde_json::json!({
        "output": result.output, "cwd": result.cwd, "exit_code": result.exit_code
    }))
    .into_response()
}

#[derive(Deserialize)]
struct CalcQuery {
    expr: Option<String>,
}
async fn api_calc(Query(q): Query<CalcQuery>) -> Json<serde_json::Value> {
    let expr = q.expr.unwrap_or_default();
    match eval_simple(&expr) {
        Ok(v) => Json(serde_json::json!({"ok":true,"result":v,"expr":expr})),
        Err(e) => Json(serde_json::json!({"ok":false,"error":e,"expr":expr})),
    }
}

fn eval_simple(expr: &str) -> Result<f64, String> {
    let cleaned: String = expr.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return Err("empty".into());
    }
    if !cleaned
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '*' | '/' | '.' | '(' | ')'))
    {
        return Err("invalid".into());
    }
    fn pe(chars: &[char], pos: &mut usize) -> Result<f64, String> {
        let mut v = pt(chars, pos)?;
        while *pos < chars.len() {
            match chars[*pos] {
                '+' => {
                    *pos += 1;
                    v += pt(chars, pos)?;
                }
                '-' => {
                    *pos += 1;
                    v -= pt(chars, pos)?;
                }
                _ => break,
            }
        }
        Ok(v)
    }
    fn pt(chars: &[char], pos: &mut usize) -> Result<f64, String> {
        let mut v = pf(chars, pos)?;
        while *pos < chars.len() {
            match chars[*pos] {
                '*' => {
                    *pos += 1;
                    v *= pf(chars, pos)?;
                }
                '/' => {
                    *pos += 1;
                    let d = pf(chars, pos)?;
                    if d == 0.0 {
                        return Err("div0".into());
                    }
                    v /= d;
                }
                _ => break,
            }
        }
        Ok(v)
    }
    fn pf(chars: &[char], pos: &mut usize) -> Result<f64, String> {
        if *pos >= chars.len() {
            return Err("end".into());
        }
        match chars[*pos] {
            '+' => {
                *pos += 1;
                pf(chars, pos)
            }
            '-' => {
                *pos += 1;
                Ok(-pf(chars, pos)?)
            }
            '(' => {
                *pos += 1;
                let v = pe(chars, pos)?;
                if *pos >= chars.len() || chars[*pos] != ')' {
                    return Err(")".into());
                }
                *pos += 1;
                Ok(v)
            }
            c if c.is_ascii_digit() || c == '.' => {
                let st = *pos;
                while *pos < chars.len() && (chars[*pos].is_ascii_digit() || chars[*pos] == '.') {
                    *pos += 1;
                }
                chars[st..*pos]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .map_err(|_| "num".into())
            }
            _ => Err("tok".into()),
        }
    }
    let chars: Vec<char> = cleaned.chars().collect();
    let mut pos = 0;
    let v = pe(&chars, &mut pos)?;
    if pos != chars.len() {
        return Err("trail".into());
    }
    Ok(v)
}

async fn api_mail() -> Json<serde_json::Value> {
    Json(serde_json::json!({"inbox":[
        {"id":"1","from":"Tim Cook","email":"tim@apple.com","subject":"Welcome","preview":"Multi-user Maxcos","body":"Each account is isolated.","time":"9:41 AM","unread":true}
    ]}))
}
async fn api_messages() -> Json<serde_json::Value> {
    Json(serde_json::json!({"conversations":[
        {"id":"1","name":"Siri","avatar":"S","color":"#0A84FF","last":"Hi","time":"Now","messages":[{"from":"them","text":"Welcome","time":"Now"}]}
    ]}))
}
async fn api_photos() -> Json<serde_json::Value> {
    Json(serde_json::json!({"albums":[{"id":"recents","name":"Recents","count":2}],"photos":[
        {"id":"1","album":"recents","title":"Sky","gradient":"linear-gradient(135deg,#4facfe,#00f2fe)"},
        {"id":"2","album":"recents","title":"Dusk","gradient":"linear-gradient(135deg,#fa709a,#fee140)"}
    ]}))
}
async fn api_music() -> Json<serde_json::Value> {
    Json(serde_json::json!({"playlists":[{"id":"1","name":"Favorites","count":1}],"tracks":[
        {"id":"1","title":"Midnight Drive","artist":"Neon","album":"Night","duration":"3:42","color":"#FF2D55"}
    ]}))
}
async fn api_calendar() -> Json<serde_json::Value> {
    let now = Local::now();
    Json(serde_json::json!({"year":now.year(),"month":now.month(),"day":now.day(),"events":[
        {"id":"1","title":"Standup","day":now.day(),"time":"10:00 AM","color":"#0A84FF"}
    ]}))
}

// silence unused imports used in type paths
#[allow(dead_code)]
fn _keep(_: &Note, _: &Reminder) {}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let store = Store::connect(base.clone())
        .await
        .expect("mongodb connect (set MONGODB_URI, default mongodb://127.0.0.1:27017/maxcos)");
    tracing::info!("MongoDB is sole source of truth; disk cache under data/cache only");

    let state = AppState {
        store: Arc::new(store),
        lock: Arc::new(Mutex::new(())),
        rate_limit: security::new_rate_limit(),
    };

    let static_dir = base.join("static");
    let app = Router::new()
        .route("/", get(boot))
        .route("/entry", get(entry))
        .route("/setup", get(setup_page).post(setup_submit))
        .route("/login", get(login_page).post(login_submit))
        .route("/desktop", get(desktop))
        .route("/logout", post(logout).get(logout))
        .route("/health", get(health))
        .route("/api/time", get(api_time))
        .route("/api/apps", get(api_apps))
        .route("/api/setup/status", get(api_setup_status))
        .route("/api/session/me", get(api_session_me))
        .route("/api/session/login", post(api_session_login))
        .route("/api/session/logout", post(api_session_logout))
        .route("/api/users", get(api_users_list).post(api_users_create))
        .route("/api/users/{id}", axum::routing::delete(api_users_delete))
        .route("/api/settings", get(api_settings_get).post(api_settings_save))
        .route("/api/admin/audit", get(api_admin_audit))
        .route("/api/notes", get(api_notes_list).post(api_notes_create))
        .route(
            "/api/notes/{id}",
            axum::routing::put(api_notes_update).delete(api_notes_delete),
        )
        .route(
            "/api/reminders",
            get(api_reminders_list).post(api_reminders_create),
        )
        .route("/api/reminders/{id}/toggle", post(api_reminders_toggle))
        .route(
            "/api/reminders/{id}",
            axum::routing::delete(api_reminders_delete),
        )
        .route(
            "/api/notifications",
            get(api_notifications_list).post(api_notifications_push),
        )
        .route("/api/notifications/read-all", post(api_notifications_read_all))
        .route("/api/notifications/clear", post(api_notifications_clear))
        .route("/api/notifications/{id}/read", post(api_notifications_read))
        .route(
            "/api/notifications/{id}",
            axum::routing::delete(api_notifications_delete),
        )
        .route("/api/fs/list", get(api_fs_list))
        .route("/api/fs/read", get(api_fs_read))
        .route("/api/fs/write", axum::routing::put(api_fs_write))
        .route("/api/fs/mkdir", post(api_fs_mkdir))
        .route("/api/fs/create", post(api_fs_create))
        .route("/api/fs/delete", axum::routing::delete(api_fs_delete))
        .route("/api/fs/rename", post(api_fs_rename))
        .route("/api/search", get(api_search))
        .route("/api/safari", get(api_safari_state).post(api_safari_save))
        .route("/api/safari/navigate", post(api_safari_navigate))
        .route("/proxy", get(proxy_handler))
        .route("/api/terminal", post(api_terminal))
        .route("/api/calc", get(api_calc))
        .route("/api/mail", get(api_mail))
        .route("/api/messages", get(api_messages))
        .route("/api/photos", get(api_photos))
        .route("/api/music", get(api_music))
        .route("/api/calendar", get(api_calendar))
        .nest_service("/static", ServeDir::new(static_dir))
        // No CORS * — same-origin only
        .layer(middleware::from_fn(security::security_headers_middleware))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::info!("🍎 Maxcos multi-user (hardened) on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .expect("serve");
}
