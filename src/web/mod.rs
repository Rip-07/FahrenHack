//! HTTP listeners for the desktop window and the optional phone remote.
mod api;
mod assets;

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;

use crate::app::{self, lock, Shared};

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
        let _ = ready.send(Err(format!(
            "Порты {first}–{} заняты",
            first.saturating_add(9)
        )));
        return;
    };

    let router = assets::router()
        .nest("/api", api::router(app.clone()))
        .with_state(app.clone());
    let _ = ready.send(Ok(port));
    tokio::spawn(lan_listener(app, router.clone(), port));
    let _ = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await;
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
                            let _ = axum::serve(
                                l,
                                r.into_make_service_with_connect_info::<SocketAddr>(),
                            )
                            .await;
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
