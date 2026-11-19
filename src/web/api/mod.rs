//! Authenticated HTTP routes and request/response adapters.
mod scanner;

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{self, lock, Shared};
use crate::profile::Kind;
use crate::scan::VType;
use crate::{mem, pointer, steam};

pub(super) fn router(app: Shared) -> Router<Shared> {
    Router::new()
        .route("/state", get(state))
        .route("/job", get(job_state))
        .route("/games", get(games))
        .route("/games/{id}/launch", post(launch))
        .route(
            "/games/{gid}/cheats/{cid}",
            put(set_cheat_settings).delete(reset_cheat_settings),
        )
        .route("/processes", get(processes))
        .route("/attach", post(attach))
        .route("/detach", post(detach))
        .route("/cheats/off", post(all_off))
        .route("/cheats/{id}/toggle", post(toggle))
        .route("/cheats/{id}/calibration", delete(reset_calibration))
        .route("/calib/{id}/bind", post(bind))
        .route("/custom", post(add_custom))
        .route("/custom/{id}", delete(remove_custom))
        .merge(scanner::router())
        .route("/settings", put(put_settings))
        .route("/token/reset", post(reset_token))
        .route("/open-data", post(open_data))
        .route("/qr", get(qr))
        .route("/art/{game}/{kind}", get(art))
        .layer(middleware::from_fn_with_state(app, auth))
}

type ApiResult = Result<Json<Value>, (StatusCode, String)>;

fn bad(e: impl ToString) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, e.to_string())
}

fn ok() -> ApiResult {
    Ok(Json(json!({"ok": true})))
}

fn job(id: Result<u64, String>) -> ApiResult {
    Ok(Json(json!({"job": id.map_err(bad)?})))
}

async fn auth(State(app): State<Shared>, req: Request, next: Next) -> Response {
    let (token, remote) = {
        let a = lock(&app);
        (a.settings.token.clone(), a.settings.remote)
    };
    let local = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|c| c.0.ip().is_loopback());
    if !local && !remote {
        return (
            StatusCode::FORBIDDEN,
            "Пульт выключен в настройках FahrenHack",
        )
            .into_response();
    }
    let by_header =
        req.headers().get("x-token").and_then(|v| v.to_str().ok()) == Some(token.as_str());
    let by_query = req.uri().query().is_some_and(|q| {
        q.split('&')
            .any(|kv| kv.strip_prefix("t=") == Some(token.as_str()))
    });
    if by_header || by_query {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            "Неверный токен: откройте ссылку или QR-код из FahrenHack",
        )
            .into_response()
    }
}

async fn state(State(app): State<Shared>) -> ApiResult {
    Ok(Json(lock(&app).state_json()))
}

async fn job_state(State(app): State<Shared>) -> ApiResult {
    Ok(Json(json!(lock(&app).job.clone())))
}

async fn games(State(app): State<Shared>) -> ApiResult {
    Ok(Json(lock(&app).games_json()))
}

async fn launch(State(app): State<Shared>, Path(id): Path<String>) -> ApiResult {
    let appid = lock(&app)
        .steam_appid(&id)
        .ok_or_else(|| bad("Игру нельзя запустить через Steam"))?;
    steam::launch(appid).map_err(bad)?;
    ok()
}

#[derive(Deserialize)]
struct CheatSettings {
    #[serde(default)]
    value: String,
    #[serde(default)]
    hotkey: String,
}

async fn set_cheat_settings(
    State(app): State<Shared>,
    Path((gid, cid)): Path<(String, String)>,
    Json(b): Json<CheatSettings>,
) -> ApiResult {
    lock(&app)
        .set_override(&gid, &cid, b.value, b.hotkey)
        .map_err(bad)?;
    ok()
}

async fn reset_cheat_settings(
    State(app): State<Shared>,
    Path((gid, cid)): Path<(String, String)>,
) -> ApiResult {
    lock(&app).clear_override(&gid, &cid);
    ok()
}

async fn processes() -> ApiResult {
    let mut list = mem::list_processes();
    list.sort_by_key(|p| p.name.to_lowercase());
    Ok(Json(json!(list)))
}

#[derive(Deserialize)]
struct AttachReq {
    pid: u32,
}

async fn attach(State(app): State<Shared>, Json(r): Json<AttachReq>) -> ApiResult {
    let name = mem::list_processes()
        .into_iter()
        .find(|p| p.pid == r.pid)
        .map(|p| p.name)
        .ok_or_else(|| bad("Процесс не найден"))?;
    lock(&app).attach(r.pid, &name).map_err(bad)?;
    ok()
}

async fn detach(State(app): State<Shared>) -> ApiResult {
    lock(&app).detach_manually();
    ok()
}

async fn all_off(State(app): State<Shared>) -> ApiResult {
    lock(&app).all_off();
    ok()
}

async fn toggle(State(app): State<Shared>, Path(id): Path<String>) -> ApiResult {
    let on = lock(&app).toggle(&id).map_err(bad)?;
    Ok(Json(json!({"active": on})))
}

async fn reset_calibration(State(app): State<Shared>, Path(id): Path<String>) -> ApiResult {
    lock(&app).reset_calibration(&id).map_err(bad)?;
    ok()
}

#[derive(Deserialize)]
struct BindReq {
    addr: String,
    vtype: VType,
}

async fn bind(
    State(app): State<Shared>,
    Path(id): Path<String>,
    Json(r): Json<BindReq>,
) -> ApiResult {
    let addr = pointer::parse_hex(&r.addr)
        .filter(|a| *a > 0)
        .ok_or_else(|| bad("Неверный адрес"))? as usize;
    job(app::start_bind(&app, id, addr, r.vtype))
}

#[derive(Deserialize)]
struct CustomReq {
    name: String,
    vtype: VType,
    kind: Kind,
    #[serde(default)]
    value: Option<String>,
}

async fn add_custom(State(app): State<Shared>, Json(r): Json<CustomReq>) -> ApiResult {
    let id = lock(&app)
        .add_custom(r.name, r.vtype, r.kind, r.value)
        .map_err(bad)?;
    Ok(Json(json!({"id": id})))
}

async fn remove_custom(State(app): State<Shared>, Path(id): Path<String>) -> ApiResult {
    lock(&app).remove_custom(&id).map_err(bad)?;
    ok()
}

#[derive(Deserialize)]
struct SettingsReq {
    auto_attach: Option<bool>,
    hotkeys: Option<bool>,
    remote: Option<bool>,
    sounds: Option<bool>,
    fullscreen: Option<bool>,
}

async fn put_settings(State(app): State<Shared>, Json(r): Json<SettingsReq>) -> ApiResult {
    let mut a = lock(&app);
    if let Some(v) = r.auto_attach {
        a.settings.auto_attach = v;
    }
    if let Some(v) = r.hotkeys {
        a.settings.hotkeys = v;
    }
    if let Some(v) = r.remote {
        a.settings.remote = v;
    }
    if let Some(v) = r.sounds {
        a.settings.sounds = v;
    }
    if let Some(v) = r.fullscreen {
        a.settings.fullscreen = v;
    }
    a.save_settings();
    ok()
}

async fn reset_token(State(app): State<Shared>) -> ApiResult {
    let mut a = lock(&app);
    a.settings.token = app::gen_token();
    a.save_settings();
    Ok(Json(
        json!({"token": a.settings.token, "lan_url": a.lan_url()}),
    ))
}

async fn open_data(State(app): State<Shared>) -> ApiResult {
    let dir = lock(&app).data_dir.display().to_string();
    steam::open(&dir).map_err(bad)?;
    ok()
}

async fn qr(State(app): State<Shared>) -> Response {
    let url = lock(&app).lan_url();
    match qrcode::QrCode::new(url.as_bytes()) {
        Ok(code) => {
            let svg = code
                .render::<qrcode::render::svg::Color>()
                .min_dimensions(240, 240)
                .dark_color(qrcode::render::svg::Color("#0b1626"))
                .light_color(qrcode::render::svg::Color("#ffffff"))
                .build();
            ([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response()
        }
        Err(e) => bad(e).into_response(),
    }
}

async fn art(State(app): State<Shared>, Path((game, kind)): Path<(String, String)>) -> Response {
    let appid = lock(&app).steam_appid(&game);
    let Some(path) = appid.and_then(|a| steam::art(a, &kind)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match std::fs::read(&path) {
        Ok(bytes) => {
            let ct = if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png"))
            {
                "image/png"
            } else {
                "image/jpeg"
            };
            (
                [
                    (header::CONTENT_TYPE, ct),
                    (header::CACHE_CONTROL, "max-age=3600"),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
