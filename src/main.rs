// No console window in release builds; `--headless` / `--browser` attach one.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! FahrenHack: open-source trainer for single-player PC games.
//! A console-style desktop UI, ready game profiles and a phone remote over the LAN.

mod aob;
mod app;
mod hotkey;
mod inject;
mod logo;
mod mem;
mod pointer;
mod profile;
mod scan;
mod steam;
mod web;
mod window;

use std::sync::{Arc, Mutex};

use windows_sys::Win32::System::Console::{AllocConsole, AttachConsole, ATTACH_PARENT_PROCESS};
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

const HELP: &str = "FahrenHack — трейнер для одиночных игр

  fahrenhack              окно приложения
  fahrenhack --browser    открыть интерфейс в браузере вместо окна
  fahrenhack --headless   только сервер для телефона (ссылка и QR в консоли)

Данные: %APPDATA%\\FahrenHack (можно переопределить переменной FAHRENHACK_DATA)";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn fatal(msg: &str) -> ! {
    eprintln!("[x] {msg}");
    let (text, title) = (wide(msg), wide("FahrenHack"));
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
    }
    std::process::exit(1);
}

fn console() {
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            AllocConsole();
        }
    }
}

fn banner(local: &str, lan: Option<&str>, data: &str) {
    println!("=== FahrenHack {} ===", env!("CARGO_PKG_VERSION"));
    println!("ПК:      {local}");
    println!("Данные:  {data}");
    let Some(lan) = lan else {
        println!("Пульт для телефона выключен: включите его в разделе «Пульт».\n");
        return;
    };
    println!("Телефон: {lan}");
    println!("(телефон должен быть в той же сети Wi-Fi: отсканируйте QR-код)\n");
    if let Ok(code) = qrcode::QrCode::new(lan.as_bytes()) {
        use qrcode::render::unicode::Dense1x2;
        let s = code.render::<Dense1x2>().dark_color(Dense1x2::Light).light_color(Dense1x2::Dark).build();
        println!("{s}\n");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    if flag("--help") || flag("-h") {
        console();
        println!("{HELP}");
        return;
    }
    let headless = flag("--headless");
    let browser = flag("--browser");
    if headless || browser {
        console();
    }

    let mut state = app::App::new();
    state.lan_ip = app::detect_lan_ip();
    let app: app::Shared = Arc::new(Mutex::new(state));
    app::spawn_workers(&app);

    let (tx, rx) = std::sync::mpsc::channel();
    {
        let app = app.clone();
        std::thread::spawn(move || {
            match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
                Ok(rt) => rt.block_on(web::serve(app, tx)),
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                }
            }
        });
    }
    let port = match rx.recv() {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => fatal(&format!("Не удалось запустить сервер: {e}")),
        Err(_) => fatal("Не удалось запустить сервер"),
    };

    let (local, lan, data_dir, fullscreen) = {
        let mut a = app::lock(&app);
        a.port = port;
        (
            format!("http://127.0.0.1:{port}/?t={}", a.settings.token),
            a.settings.remote.then(|| a.lan_url()),
            a.data_dir.clone(),
            a.settings.fullscreen,
        )
    };
    banner(&local, lan.as_deref(), &data_dir.display().to_string());

    if headless {
        loop {
            std::thread::park();
        }
    }
    if browser {
        let _ = steam::open(&local);
        loop {
            std::thread::park();
        }
    }

    let exit_app = app.clone();
    let on_exit: Box<dyn FnOnce()> = Box::new(move || app::lock(&exit_app).detach());
    if let Err(e) = window::run(format!("{local}&desktop=1"), data_dir, fullscreen, on_exit) {
        eprintln!("[!] Окно не открылось ({e}), открываю интерфейс в браузере");
        let _ = steam::open(&local);
        loop {
            std::thread::park();
        }
    }
}
