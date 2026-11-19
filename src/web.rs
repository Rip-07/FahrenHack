//! HTTP server: serves the UI to the desktop window and to phones on the LAN.
//! Every `/api` call needs the session token (header `x-token` or `?t=`).

use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{self, lock, ScanReq, Shared};
use crate::profile::Kind;
use crate::scan::{self, NextMode, ScanState, VType};
use crate::{mem, pointer, steam};

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

pub async fn serve(app: Shared, ready: std::sync::mpsc::Sender<Result<u16, String>>) {
    let first = lock(&app).settings.port;
    let mut bound = None;
    for port in first..first.saturating_add(10) {
        if let Ok(l) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            bound = Some((l, port));
            break;
        }
    }
    let Some((listener, port)) = bound else {
        let _ = ready.send(Err(format!("Порты {first}–{} заняты", first.saturating_add(9))));
        return;
    };

    let api = Router::new()
        .route("/state", get(state))
        .route("/job", get(job_state))
        .route("/games", get(games))
        .route("/games/{id}/launch", post(launch))
        .route("/games/{gid}/cheats/{cid}", put(set_cheat_settings).delete(reset_cheat_settings))
        .route("/processes", get(processes))
        .route("/attach", post(attach))
        .route("/detach", post(detach))
        .route("/cheats/off", post(all_off))
        .route("/cheats/{id}/toggle", post(toggle))
        .route("/cheats/{id}/calibration", delete(reset_calibration))
        .route("/calib/{id}/bind", post(bind))
        .route("/custom", post(add_custom))
        .route("/custom/{id}", delete(remove_custom))
        .route("/scan/first", post(scan_first))
        .route("/scan/unknown", post(scan_unknown))
        .route("/scan/next", post(scan_next))
        .route("/scan/reset", post(scan_reset))
        .route("/scan/results", get(scan_results))
        .route("/write", post(write_mem))
        .route("/settings", put(put_settings))
        .route("/token/reset", post(reset_token))
        .route("/open-data", post(open_data))
        .route("/qr", get(qr))
        .route("/art/{game}/{kind}", get(art))
        .layer(middleware::from_fn_with_state(app.clone(), auth));

    let router = Router::new()
        .route("/", get(|| async { asset("text/html; charset=utf-8", include_str!("ui/index.html")) }))
        .route("/app.css", get(|| async { asset("text/css; charset=utf-8", include_str!("ui/app.css")) }))
        .route("/app.js", get(|| async { asset("text/javascript; charset=utf-8", include_str!("ui/app.js")) }))
        .route("/logo.svg", get(|| async { asset("image/svg+xml", include_str!("../assets/logo.svg")) }))
        .nest("/api", api)
        .with_state(app.clone());

    let _ = ready.send(Ok(port));
    tokio::spawn(lan_listener(app, router.clone(), port));
    let _ = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await;
}

/// Listens on the LAN address only while the phone remote is switched on.
async fn lan_listener(app: Shared, router: Router, port: u16) {
    let mut running: Option<(String, tokio::task::JoinHandle<()>)> = None;
    let mut last_error = String::new();
    loop {
        let want = lock(&app).settings.remote;
        if want {
            let ip = app::detect_lan_ip();
            lock(&app).lan_ip = ip.clone();
            let same = running.as_ref().is_some_and(|(r, _)| *r == ip);
            if !same && ip != "127.0.0.1" {
                if let Some((_, h)) = running.take() {
                    h.abort();
                }
                match tokio::net::TcpListener::bind((ip.as_str(), port)).await {
                    Ok(l) => {
                        println!("[+] Пульт для телефона: http://{ip}:{port}");
                        let r = router.clone();
                        let h = tokio::spawn(async move {
                            let _ = axum::serve(l, r.into_make_service_with_connect_info::<SocketAddr>()).await;
                        });
                        running = Some((ip, h));
                        last_error.clear();
                    }
                    Err(e) => {
                        let msg = format!("[!] Пульт: не удалось открыть {ip}:{port}: {e}");
                        if msg != last_error {
                            eprintln!("{msg}");
                            last_error = msg;
                        }
                    }
                }
            }
        } else if let Some((_, h)) = running.take() {
            h.abort();
            println!("[-] Пульт выключен");
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "no-cache")], body).into_response()
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
        return (StatusCode::FORBIDDEN, "Пульт выключен в настройках FahrenHack").into_response();
    }
    let by_header = req.headers().get("x-token").and_then(|v| v.to_str().ok()) == Some(token.as_str());
    let by_query = req
        .uri()
        .query()
        .is_some_and(|q| q.split('&').any(|kv| kv.strip_prefix("t=") == Some(token.as_str())));
    if by_header || by_query {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "Неверный токен: откройте ссылку или QR-код из FahrenHack").into_response()
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
        .profiles
        .iter()
        .find(|g| g.id == id)
        .and_then(|g| g.steam_appid)
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
    lock(&app).set_override(&gid, &cid, b.value, b.hotkey).map_err(bad)?;
    ok()
}

async fn reset_cheat_settings(State(app): State<Shared>, Path((gid, cid)): Path<(String, String)>) -> ApiResult {
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
    let mut a = lock(&app);
    a.skip_pid = a.game.as_ref().map(|g| g.proc.pid);
    a.detach();
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

async fn bind(State(app): State<Shared>, Path(id): Path<String>, Json(r): Json<BindReq>) -> ApiResult {
    let addr = pointer::parse_hex(&r.addr).filter(|a| *a > 0).ok_or_else(|| bad("Неверный адрес"))? as usize;
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
    let id = lock(&app).add_custom(r.name, r.vtype, r.kind, r.value).map_err(bad)?;
    Ok(Json(json!({"id": id})))
}

async fn remove_custom(State(app): State<Shared>, Path(id): Path<String>) -> ApiResult {
    lock(&app).remove_custom(&id).map_err(bad)?;
    ok()
}

#[derive(Deserialize)]
struct FirstReq {
    vtype: VType,
    value: String,
}

async fn scan_first(State(app): State<Shared>, Json(r): Json<FirstReq>) -> ApiResult {
    let value = scan::parse_number(&r.value).map_err(bad)?;
    job(app::start_scan(&app, ScanReq::First { vtype: r.vtype, value }))
}

#[derive(Deserialize)]
struct UnknownReq {
    vtype: VType,
}

async fn scan_unknown(State(app): State<Shared>, Json(r): Json<UnknownReq>) -> ApiResult {
    job(app::start_scan(&app, ScanReq::Unknown { vtype: r.vtype }))
}

#[derive(Deserialize)]
struct NextReq {
    mode: NextMode,
    #[serde(default)]
    value: Option<String>,
}

async fn scan_next(State(app): State<Shared>, Json(r): Json<NextReq>) -> ApiResult {
    let value = match r.value.as_deref().map(str::trim) {
        Some(v) if !v.is_empty() => Some(scan::parse_number(v).map_err(bad)?),
        _ => None,
    };
    job(app::start_scan(&app, ScanReq::Next { mode: r.mode, value }))
}

async fn scan_reset(State(app): State<Shared>) -> ApiResult {
    let mut a = lock(&app);
    if a.job.as_ref().is_some_and(|j| !j.done && j.kind == "scan") {
        return Err(bad("Дождитесь окончания сканирования"));
    }
    a.scan = None;
    ok()
}

async fn scan_results(State(app): State<Shared>) -> ApiResult {
    let a = lock(&app);
    let (Some(g), Some(ScanState::Hits(h))) = (a.game.as_ref(), a.scan.as_ref()) else {
        return Ok(Json(json!([])));
    };
    let size = h.vtype.size();
    let list: Vec<Value> = h
        .hits
        .iter()
        .take(200)
        .map(|hit| {
            let mut b = [0u8; 8];
            let cur = g.proc.read(hit.addr, &mut b[..size]).then(|| h.vtype.fmt(&b));
            let module = g
                .modules
                .iter()
                .find(|m| m.contains(hit.addr))
                .map(|m| format!("{}+{:X}", m.name, hit.addr - m.base));
            json!({"addr": format!("{:X}", hit.addr), "value": cur, "prev": h.vtype.fmt(&hit.prev), "static": module})
        })
        .collect();
    Ok(Json(json!(list)))
}

#[derive(Deserialize)]
struct WriteReq {
    addr: String,
    vtype: VType,
    value: String,
}

async fn write_mem(State(app): State<Shared>, Json(r): Json<WriteReq>) -> ApiResult {
    let a = lock(&app);
    let g = a.game.as_ref().ok_or_else(|| bad("Игра не подключена"))?;
    let addr = pointer::parse_hex(&r.addr).filter(|a| *a > 0).ok_or_else(|| bad("Неверный адрес"))? as usize;
    let bytes = r.vtype.encode(&r.value).map_err(bad)?;
    if !g.proc.write(addr, &bytes) {
        return Err(bad("Запись в память не удалась"));
    }
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
    Ok(Json(json!({"token": a.settings.token, "lan_url": a.lan_url()})))
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

#[cfg(test)]
mod tests {
    /// wry defines `window.ipc` as a non-configurable global inside the desktop window.
    /// A global `ipc` binding in app.js makes the whole script fail with a SyntaxError there
    /// (the page stays empty and the window cannot be dragged), while a normal browser works.
    #[test]
    fn ui_script_cannot_clash_with_webview_globals() {
        let html = include_str!("ui/index.html");
        assert!(html.contains(r#"<script type="module" src="/app.js">"#), "app.js must load as a module");
        let js = include_str!("ui/app.js");
        for decl in ["const ipc", "let ipc", "var ipc", "function ipc"] {
            assert!(!js.contains(decl), "app.js must not declare `{decl}`");
        }
    }
}

async fn art(State(app): State<Shared>, Path((game, kind)): Path<(String, String)>) -> Response {
    let appid = lock(&app).profiles.iter().find(|g| g.id == game).and_then(|g| g.steam_appid);
    let Some(path) = appid.and_then(|a| steam::art(a, &kind)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match std::fs::read(&path) {
        Ok(bytes) => {
            let ct = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) { "image/png" } else { "image/jpeg" };
            ([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "max-age=3600")], bytes).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
