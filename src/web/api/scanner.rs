//! HTTP adapters for the scanner and its result list.
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::app::{self, lock, ScanReq, Shared};
use crate::scan::{self, NextMode, VType};

use super::{bad, job, ok, ApiResult};

pub(super) fn router() -> Router<Shared> {
    Router::new()
        .route("/scan/first", post(scan_first))
        .route("/scan/unknown", post(scan_unknown))
        .route("/scan/next", post(scan_next))
        .route("/scan/reset", post(scan_reset))
        .route("/scan/results", get(scan_results))
        .route("/write", post(write_mem))
}

#[derive(Deserialize)]
struct FirstReq {
    vtype: VType,
    value: String,
}

async fn scan_first(State(app): State<Shared>, Json(r): Json<FirstReq>) -> ApiResult {
    let value = scan::parse_number(&r.value).map_err(bad)?;
    job(app::start_scan(
        &app,
        ScanReq::First {
            vtype: r.vtype,
            value,
        },
    ))
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
    job(app::start_scan(
        &app,
        ScanReq::Next {
            mode: r.mode,
            value,
        },
    ))
}

async fn scan_reset(State(app): State<Shared>) -> ApiResult {
    lock(&app).reset_scan().map_err(bad)?;
    ok()
}

async fn scan_results(State(app): State<Shared>) -> ApiResult {
    Ok(Json(json!(lock(&app).scan_results())))
}

#[derive(Deserialize)]
struct WriteReq {
    addr: String,
    vtype: VType,
    value: String,
}

async fn write_mem(State(app): State<Shared>, Json(r): Json<WriteReq>) -> ApiResult {
    lock(&app)
        .write_scan_value(&r.addr, r.vtype, &r.value)
        .map_err(bad)?;
    ok()
}
