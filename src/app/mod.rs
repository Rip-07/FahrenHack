//! Application state and game-session lifecycle.

mod cheats;
mod jobs;
mod session;
mod settings;
mod view;
mod workers;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::profile::{self, GameProfile};
use crate::scan::ScanState;

use self::jobs::Job;
use self::session::GameSession;
use self::settings::{data_dir, Settings};

pub(crate) use self::jobs::{start_bind, start_scan, ScanReq};
pub(crate) use self::settings::{detect_lan_ip, gen_token};
pub(crate) use self::workers::spawn_workers;

pub(crate) type Shared = Arc<Mutex<App>>;

pub(crate) fn lock(app: &Shared) -> MutexGuard<'_, App> {
    app.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) struct App {
    pub(crate) data_dir: PathBuf,
    profiles: Vec<GameProfile>,
    pub(crate) settings: Settings,
    game: Option<GameSession>,
    scan: Option<ScanState>,
    pub(crate) job: Option<Job>,
    next_job: u64,
    /// The user detached from this pid: don't auto-attach to it again.
    skip_pid: Option<u32>,
    pub(crate) lan_ip: String,
    pub(crate) port: u16,
    attach_failures: HashMap<u32, u32>,
}

impl App {
    pub(crate) fn new() -> Self {
        let data_dir = data_dir();
        let _ = std::fs::create_dir_all(&data_dir);
        let settings = Settings::load(&data_dir);
        let app = Self {
            profiles: profile::load_all(&data_dir),
            data_dir,
            settings,
            game: None,
            scan: None,
            job: None,
            next_job: 0,
            skip_pid: None,
            lan_ip: "127.0.0.1".into(),
            port: 0,
            attach_failures: HashMap::new(),
        };
        app.save_settings();
        app
    }

    pub(crate) fn steam_appid(&self, game_id: &str) -> Option<u32> {
        self.profiles
            .iter()
            .find(|profile| profile.id == game_id)
            .and_then(|profile| profile.steam_appid)
    }

    pub(crate) fn attach(&mut self, pid: u32, exe: &str) -> Result<(), String> {
        self.detach();
        let game = GameSession::open(pid, exe, &self.profiles, &self.data_dir)?;
        println!("[+] Подключено: {exe} (pid {pid}, сборка {})", game.build);
        self.game = Some(game);
        self.skip_pid = None;
        Ok(())
    }

    /// Detach from the current process without auto-attaching to the same PID again.
    pub(crate) fn detach_manually(&mut self) {
        self.skip_pid = self.game.as_ref().map(|game| game.proc.pid);
        self.detach();
    }

    /// End the session, restoring any patches and discarding scanner results.
    pub(crate) fn detach(&mut self) {
        if let Some(mut game) = self.game.take() {
            game.restore_patches();
            println!("[-] Отключено: {}", game.proc.name);
        }
        self.scan = None;
    }
}
