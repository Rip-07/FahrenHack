//! Application state: the attached game, cheats, calibrations, scanner jobs and background workers.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::aob;
use crate::hotkey::Hotkey;
use crate::mem::{self, Module, Process};
use crate::pointer::{self, Chain, Parsed};
use crate::profile::{self, CalEntry, CalMode, Calibration, CheatDef, GameProfile, Kind, Locator};
use crate::scan::{self, NextMode, ScanState, VType};
use crate::steam;

pub type Shared = Arc<Mutex<App>>;

pub fn lock(app: &Shared) -> MutexGuard<'_, App> {
    app.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------- settings

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub token: String,
    pub port: u16,
    pub auto_attach: bool,
    pub hotkeys: bool,
    /// Phone remote over the LAN (off by default: only this PC can open the UI).
    pub remote: bool,
    pub sounds: bool,
    pub fullscreen: bool,
    /// "game/cheat" -> user's value and hotkey.
    pub overrides: HashMap<String, Override>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            token: gen_token(),
            port: 7777,
            auto_attach: true,
            hotkeys: true,
            remote: false,
            sounds: true,
            fullscreen: false,
            overrides: HashMap::new(),
        }
    }
}

/// Address of this PC in the local network (no packets are sent; the socket only picks a route).
pub fn detect_lan_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("192.168.0.1:9")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}

/// Modules of anti-cheat systems. FahrenHack refuses to touch such games:
/// it is for single-player games only and never tries to get around protection.
const ANTI_CHEAT: &[(&str, &str)] = &[
    ("easyanticheat", "Easy Anti-Cheat"),
    ("beclient", "BattlEye"),
    ("battleye", "BattlEye"),
    ("eaanticheat", "EA AntiCheat"),
    ("gameguard", "nProtect GameGuard"),
    ("npggnt", "nProtect GameGuard"),
    ("xigncode", "XIGNCODE3"),
    ("x3.xem", "XIGNCODE3"),
    ("faceit", "FACEIT"),
    ("acesdk", "ACE"),
    ("anticheatexpert", "ACE"),
];

fn anti_cheat(modules: &[Module]) -> Option<&'static str> {
    modules.iter().find_map(|m| {
        let n = m.name.to_lowercase();
        ANTI_CHEAT.iter().find(|(k, _)| n.contains(k)).map(|(_, name)| *name)
    })
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Override {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// `Some("")` disables the profile's hotkey.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<String>,
}

pub fn gen_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut out = String::new();
    for salt in 0..2u32 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(nanos);
        h.write_u32(std::process::id() ^ salt);
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out[..20].to_string()
}

pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FAHRENHACK_DATA") {
        return PathBuf::from(d);
    }
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_else(|| ".".into())).join("FahrenHack")
}

// ---------------------------------------------------------------- jobs

/// A long operation (scan, pointer search) polled by the UI.
#[derive(Clone, Serialize)]
pub struct Job {
    pub id: u64,
    pub kind: String,
    pub label: String,
    pub progress: f32,
    pub done: bool,
    pub error: Option<String>,
    pub result: Option<Value>,
}

pub fn job_progress(app: &Shared, id: u64, progress: f32, label: Option<&str>) {
    let mut a = lock(app);
    if let Some(j) = a.job.as_mut().filter(|j| j.id == id) {
        j.progress = progress.clamp(0.0, 1.0);
        if let Some(l) = label {
            j.label = l.to_string();
        }
    }
}

fn job_done(a: &mut App, id: u64, r: Result<Value, String>) {
    if let Some(j) = a.job.as_mut().filter(|j| j.id == id) {
        j.done = true;
        j.progress = 1.0;
        match r {
            Ok(v) => j.result = Some(v),
            Err(e) => j.error = Some(e),
        }
    }
}

// ---------------------------------------------------------------- attached game

pub enum Active {
    Freeze(Vec<u8>),
    Patch { addr: usize, original: Vec<u8> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unresolved {
    NeedsCalibration,
    Waiting,
    Missing,
    Invalid,
}

impl Unresolved {
    pub fn code(self) -> &'static str {
        match self {
            Unresolved::NeedsCalibration => "calibrate",
            Unresolved::Waiting => "waiting",
            Unresolved::Missing => "missing",
            Unresolved::Invalid => "invalid",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Unresolved::NeedsCalibration => "Чит ещё не настроен: нажмите «Настроить»",
            Unresolved::Waiting => "Адрес пока недоступен: загрузите сохранение и войдите в игру",
            Unresolved::Missing => "Сигнатура не найдена: профиль не подходит к этой версии игры",
            Unresolved::Invalid => "Ошибка в описании чита",
        }
    }
}

pub struct Attached {
    pub proc: Arc<Process>,
    pub modules: Vec<Module>,
    pub main: Module,
    pub build: String,
    pub profile: GameProfile,
    /// The game has a ready-made profile.
    pub known: bool,
    pub calib: Calibration,
    /// Addresses found by calibration in this session (used when no pointer chain is known).
    pub session: HashMap<String, usize>,
    pub active: HashMap<String, Active>,
    chains: HashMap<String, Vec<Parsed>>,
    pick: HashMap<String, (usize, Instant)>,
    aob_cache: HashMap<String, Option<usize>>,
}

impl Attached {
    pub fn def(&self, id: &str) -> Option<&CheatDef> {
        self.profile.cheats.iter().find(|c| c.id == id)
    }

    fn rebuild_chains(&mut self) {
        self.chains.clear();
        self.pick.clear();
        for c in &self.profile.cheats {
            let list: Vec<Chain> = match self.calib.cheats.get(&c.id) {
                Some(e) if !e.chains.is_empty() => e.chains.clone(),
                _ => self.profile.builds.get(&self.build).and_then(|b| b.get(&c.id)).cloned().unwrap_or_default(),
            };
            let parsed: Vec<Parsed> = list.iter().filter_map(|ch| ch.parse(&self.modules)).collect();
            if !parsed.is_empty() {
                self.chains.insert(c.id.clone(), parsed);
            }
        }
    }

    pub fn vtype(&self, def: &CheatDef) -> VType {
        self.calib.cheats.get(&def.id).map(|e| e.vtype).unwrap_or(def.vtype)
    }

    pub fn read_bytes(&self, addr: usize, len: usize) -> Option<Vec<u8>> {
        let mut b = vec![0u8; len];
        self.proc.read(addr, &mut b).then_some(b)
    }

    fn aob_site(&mut self, key: &str, module: Option<&str>, pattern: &str) -> Result<usize, Unresolved> {
        if let Some(v) = self.aob_cache.get(key) {
            return v.ok_or(Unresolved::Missing);
        }
        let m = match module {
            Some(n) if !n.trim().is_empty() => self.modules.iter().find(|m| m.name.eq_ignore_ascii_case(n.trim())).cloned(),
            _ => Some(self.main.clone()),
        };
        let m = m.ok_or(Unresolved::Waiting)?;
        let pat = aob::parse(pattern).ok_or(Unresolved::Invalid)?;
        let site = aob::find(&self.proc, &m, &pat);
        self.aob_cache.insert(key.to_string(), site);
        site.ok_or(Unresolved::Missing)
    }

    /// Address of the cheat's value (or code patch site) in this session.
    pub fn resolve(&mut self, id: &str) -> Result<usize, Unresolved> {
        let loc = self.def(id).ok_or(Unresolved::Invalid)?.locator.clone();
        match loc {
            Locator::Pointer { module, offsets } => {
                pointer::resolve_chain(&self.proc, &self.modules, module.as_deref(), &offsets).ok_or(Unresolved::Waiting)
            }
            Locator::Aob { module, pattern, rip, add, offsets } => {
                let site = self.aob_site(id, module.as_deref(), &pattern)?;
                let mut addr = site as i64;
                if let Some([disp, len]) = rip {
                    let d = self.proc.read_u32(site + disp).ok_or(Unresolved::Waiting)? as i32;
                    addr = site as i64 + len as i64 + d as i64;
                }
                let mut addr = (addr + add) as usize;
                for o in &offsets {
                    let off = pointer::parse_hex(o).ok_or(Unresolved::Invalid)?;
                    addr = (self.proc.read_ptr(addr).ok_or(Unresolved::Waiting)? as i64 + off) as usize;
                }
                Ok(addr)
            }
            Locator::Patch { module, pattern, offset, .. } => {
                Ok((self.aob_site(id, module.as_deref(), &pattern)? as i64 + offset) as usize)
            }
            Locator::Calibrate { .. } => {
                if self.chains.contains_key(id) {
                    if let Ok(a) = self.pick_chain(id) {
                        return Ok(a);
                    }
                }
                if let Some(&a) = self.session.get(id) {
                    return Ok(a);
                }
                if self.chains.contains_key(id) {
                    Err(Unresolved::Waiting)
                } else {
                    Err(Unresolved::NeedsCalibration)
                }
            }
        }
    }

    /// Chains are stored best first (shortest, smallest offsets; confirmed ones survive
    /// re-calibration), so the first chain that leads to readable memory wins.
    /// Majority votes are not used: many long chains can agree on the same wrong address.
    fn pick_chain(&mut self, id: &str) -> Result<usize, Unresolved> {
        let chains = self.chains.get(id).ok_or(Unresolved::NeedsCalibration)?;
        if let Some(&(i, t)) = self.pick.get(id) {
            if t.elapsed() < Duration::from_secs(3) {
                if let Some(a) = chains.get(i).and_then(|c| c.resolve(&self.proc)) {
                    return Ok(a);
                }
            }
        }
        let mut probe = [0u8; 4];
        for (i, c) in chains.iter().enumerate() {
            if let Some(a) = c.resolve(&self.proc).filter(|&a| self.proc.read(a, &mut probe)) {
                self.pick.insert(id.to_string(), (i, Instant::now()));
                return Ok(a);
            }
        }
        Err(Unresolved::Waiting)
    }
}

// ---------------------------------------------------------------- app

pub struct App {
    pub data_dir: PathBuf,
    pub profiles: Vec<GameProfile>,
    pub settings: Settings,
    pub game: Option<Attached>,
    pub scan: Option<ScanState>,
    pub job: Option<Job>,
    next_job: u64,
    /// The user detached from this pid: don't auto-attach to it again.
    pub skip_pid: Option<u32>,
    pub lan_ip: String,
    pub port: u16,
    attach_failures: HashMap<u32, u32>,
}

impl App {
    pub fn new() -> App {
        let data_dir = data_dir();
        let _ = std::fs::create_dir_all(&data_dir);
        let mut settings: Settings = profile::read_json(&data_dir.join("settings.json")).unwrap_or_default();
        if settings.token.len() < 12 {
            settings.token = gen_token();
        }
        let app = App {
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

    pub fn save_settings(&self) {
        if let Ok(s) = serde_json::to_string_pretty(&self.settings) {
            let _ = std::fs::write(self.data_dir.join("settings.json"), s);
        }
    }

    pub fn lan_url(&self) -> String {
        format!("http://{}:{}/?t={}", self.lan_ip, self.port, self.settings.token)
    }

    pub fn attach(&mut self, pid: u32, exe: &str) -> Result<(), String> {
        self.detach();
        let p = Process::open(pid, exe)?;
        let modules = p.modules();
        if let Some(ac) = anti_cheat(&modules) {
            return Err(format!(
                "В игре работает античит ({ac}). FahrenHack подключается только к одиночным играм без античита: иначе можно получить блокировку."
            ));
        }
        let main = modules
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(exe))
            .or(modules.first())
            .cloned()
            .ok_or("Не удалось прочитать модули процесса: игра ещё загружается или это 32-битная система")?;
        let build = p.build_key(&main);
        let found = self.profiles.iter().find(|g| g.processes.iter().any(|x| x.eq_ignore_ascii_case(exe))).cloned();
        let known = found.is_some();
        let mut profile = found.unwrap_or_else(|| GameProfile::generic(exe));
        profile.cheats.extend(profile::load_custom(&self.data_dir, &profile.id));
        let calib = Calibration::load(&self.data_dir, &profile.id, &build);
        let mut g = Attached {
            proc: Arc::new(p),
            modules,
            main,
            build,
            profile,
            known,
            calib,
            session: HashMap::new(),
            active: HashMap::new(),
            chains: HashMap::new(),
            pick: HashMap::new(),
            aob_cache: HashMap::new(),
        };
        g.rebuild_chains();
        println!("[+] Подключено: {exe} (pid {pid}, сборка {})", g.build);
        self.game = Some(g);
        self.skip_pid = None;
        Ok(())
    }

    /// Detaches and restores any patched code.
    pub fn detach(&mut self) {
        if let Some(g) = self.game.take() {
            let Attached { proc, active, .. } = g;
            for a in active.into_values() {
                if let Active::Patch { addr, original } = a {
                    proc.write(addr, &original);
                }
            }
            println!("[-] Отключено: {}", proc.name);
        }
        self.scan = None;
    }

    fn okey(game: &str, cheat: &str) -> String {
        format!("{game}/{cheat}")
    }

    /// The user's value, else the profile's. `None` = freeze the current value.
    pub fn value_of(&self, game: &str, def: &CheatDef) -> Option<String> {
        let o = self.settings.overrides.get(&Self::okey(game, &def.id));
        o.and_then(|o| o.value.clone())
            .or_else(|| def.value.clone())
            .filter(|s| !s.trim().is_empty() && !s.trim().eq_ignore_ascii_case("current"))
    }

    pub fn hotkey_of(&self, game: &str, def: &CheatDef) -> Option<String> {
        match self.settings.overrides.get(&Self::okey(game, &def.id)).and_then(|o| o.hotkey.clone()) {
            Some(h) => (!h.trim().is_empty()).then_some(h),
            None => def.hotkey.clone(),
        }
    }

    fn find_def(&self, game: &str, cheat: &str) -> Option<CheatDef> {
        if let Some(g) = self.game.as_ref().filter(|g| g.profile.id == game) {
            return g.def(cheat).cloned();
        }
        let p = self.profiles.iter().find(|p| p.id == game)?;
        p.cheats
            .iter()
            .find(|c| c.id == cheat)
            .cloned()
            .or_else(|| profile::load_custom(&self.data_dir, game).into_iter().find(|c| c.id == cheat))
    }

    /// Switch a cheat (hotkey or button). Returns whether it is on now.
    pub fn toggle(&mut self, id: &str) -> Result<bool, String> {
        let g = self.game.as_ref().ok_or("Игра не подключена")?;
        let def = g.def(id).cloned().ok_or("Нет такого чита")?;
        let value = self.value_of(&g.profile.id.clone(), &def);
        let g = self.game.as_mut().unwrap();
        if let Locator::Patch { bytes, .. } = &def.locator {
            if let Some(Active::Patch { addr, original }) = g.active.remove(id) {
                g.proc.write(addr, &original);
                return Ok(false);
            }
            let addr = g.resolve(id).map_err(|e| e.message().to_string())?;
            let patch = aob::parse_bytes(bytes).ok_or("Неверные байты патча в профиле")?;
            let original = g.read_bytes(addr, patch.len()).ok_or("Не удалось прочитать код игры")?;
            if !g.proc.write(addr, &patch) {
                return Err("Запись в память не удалась".into());
            }
            g.active.insert(id.to_string(), Active::Patch { addr, original });
            return Ok(true);
        }
        match def.kind {
            Kind::Set => {
                let v = value.ok_or("Укажите значение в параметрах чита (⋯)")?;
                let addr = g.resolve(id).map_err(|e| e.message().to_string())?;
                let bytes = g.vtype(&def).encode(&v)?;
                g.proc.write(addr, &bytes).then_some(false).ok_or_else(|| "Запись в память не удалась".to_string())
            }
            Kind::Freeze => {
                if g.active.remove(id).is_some() {
                    return Ok(false);
                }
                let addr = g.resolve(id).map_err(|e| e.message().to_string())?;
                let vt = g.vtype(&def);
                let bytes = match &value {
                    Some(v) => vt.encode(v)?,
                    None => g.read_bytes(addr, vt.size()).ok_or("Не удалось прочитать значение")?,
                };
                if !g.proc.write(addr, &bytes) {
                    return Err("Запись в память не удалась".into());
                }
                g.active.insert(id.to_string(), Active::Freeze(bytes));
                Ok(true)
            }
        }
    }

    pub fn all_off(&mut self) {
        if let Some(g) = self.game.as_mut() {
            for a in std::mem::take(&mut g.active).into_values() {
                if let Active::Patch { addr, original } = a {
                    g.proc.write(addr, &original);
                }
            }
        }
    }

    /// value: "" = profile default; hotkey: "" = no hotkey.
    pub fn set_override(&mut self, game: &str, cheat: &str, value: String, hotkey: String) -> Result<(), String> {
        let def = self.find_def(game, cheat).ok_or("Нет такого чита")?;
        let value = value.trim().to_string();
        let hotkey = hotkey.trim().to_uppercase();
        if !value.is_empty() && !value.eq_ignore_ascii_case("current") {
            let vt = self.game.as_ref().filter(|g| g.profile.id == game).map(|g| g.vtype(&def)).unwrap_or(def.vtype);
            vt.encode(&value)?;
        }
        if !hotkey.is_empty() && Hotkey::parse(&hotkey).is_none() {
            return Err("Неверная клавиша. Примеры: F6, CTRL+F2, SHIFT+NUM5, ALT+K".into());
        }
        let e = self.settings.overrides.entry(Self::okey(game, cheat)).or_default();
        e.value = (!value.is_empty()).then_some(value);
        e.hotkey = Some(hotkey);
        self.save_settings();
        self.refreeze(game, &def);
        Ok(())
    }

    pub fn clear_override(&mut self, game: &str, cheat: &str) {
        self.settings.overrides.remove(&Self::okey(game, cheat));
        self.save_settings();
        if let Some(def) = self.find_def(game, cheat) {
            self.refreeze(game, &def);
        }
    }

    /// Apply a changed value to a cheat that is currently frozen.
    fn refreeze(&mut self, game: &str, def: &CheatDef) {
        let value = self.value_of(game, def);
        let Some(g) = self.game.as_mut().filter(|g| g.profile.id == game) else { return };
        if !matches!(g.active.get(&def.id), Some(Active::Freeze(_))) {
            return;
        }
        let vt = g.vtype(def);
        let bytes = match value {
            Some(v) => vt.encode(&v).ok(),
            None => g.resolve(&def.id).ok().and_then(|a| g.read_bytes(a, vt.size())),
        };
        if let Some(b) = bytes {
            g.active.insert(def.id.clone(), Active::Freeze(b));
        }
    }

    pub fn reset_calibration(&mut self, id: &str) -> Result<(), String> {
        let g = self.game.as_mut().ok_or("Игра не подключена")?;
        g.calib.cheats.remove(id);
        g.session.remove(id);
        g.active.remove(id);
        g.calib.save(&self.data_dir, &g.profile.id, &g.build)?;
        g.rebuild_chains();
        Ok(())
    }

    pub fn add_custom(&mut self, name: String, vtype: VType, kind: Kind, value: Option<String>) -> Result<String, String> {
        let g = self.game.as_mut().ok_or("Игра не подключена")?;
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("Введите название".into());
        }
        if let Some(v) = value.as_deref().filter(|v| !v.trim().is_empty()) {
            vtype.encode(v)?;
        }
        let id = (1..).map(|n| format!("my{n}")).find(|id| g.def(id).is_none()).unwrap();
        g.profile.cheats.push(CheatDef {
            id: id.clone(),
            category: "Мои читы".into(),
            name,
            description: String::new(),
            icon: "star".into(),
            kind,
            vtype,
            value: value.filter(|v| !v.trim().is_empty()),
            hotkey: None,
            experimental: false,
            custom: true,
            locator: Locator::Calibrate { mode: CalMode::Exact, prompt: String::new(), steps: vec![] },
        });
        profile::save_custom(&self.data_dir, &g.profile.id, &g.profile.cheats)?;
        Ok(id)
    }

    pub fn remove_custom(&mut self, id: &str) -> Result<(), String> {
        let g = self.game.as_mut().ok_or("Игра не подключена")?;
        if !g.def(id).is_some_and(|c| c.custom) {
            return Err("Удалять можно только свои читы".into());
        }
        g.profile.cheats.retain(|c| c.id != id);
        g.active.remove(id);
        g.calib.cheats.remove(id);
        let _ = g.calib.save(&self.data_dir, &g.profile.id, &g.build);
        profile::save_custom(&self.data_dir, &g.profile.id, &g.profile.cheats)?;
        g.rebuild_chains();
        Ok(())
    }

    /// Re-write frozen values; detach when the game exits. Runs every 40 ms.
    pub fn tick(&mut self) {
        let alive = match &self.game {
            Some(g) => g.proc.is_alive(),
            None => return,
        };
        if !alive {
            self.detach();
            return;
        }
        let g = self.game.as_mut().unwrap();
        let ids: Vec<String> = g.active.iter().filter(|(_, a)| matches!(a, Active::Freeze(_))).map(|(k, _)| k.clone()).collect();
        for id in ids {
            if let Ok(addr) = g.resolve(&id) {
                if let Some(Active::Freeze(bytes)) = g.active.get(&id) {
                    g.proc.write(addr, bytes);
                }
            }
        }
    }

    pub fn begin_job(&mut self, kind: &str, label: &str) -> Result<u64, String> {
        if self.job.as_ref().is_some_and(|j| !j.done) {
            return Err("Подождите: предыдущая операция ещё не закончилась".into());
        }
        self.next_job += 1;
        self.job = Some(Job { id: self.next_job, kind: kind.into(), label: label.into(), progress: 0.0, done: false, error: None, result: None });
        Ok(self.next_job)
    }

    // ------------------------------------------------------------ JSON for the UI

    pub fn state_json(&mut self) -> Value {
        let settings = json!({
            "auto_attach": self.settings.auto_attach,
            "hotkeys": self.settings.hotkeys,
            "remote": self.settings.remote,
            "sounds": self.settings.sounds,
            "fullscreen": self.settings.fullscreen,
            "data_dir": self.data_dir.display().to_string(),
        });
        let mut out = json!({
            "version": env!("CARGO_PKG_VERSION"),
            "lan_url": self.lan_url(),
            "settings": settings,
            "scan": self.scan.as_ref().map(|s| s.json()),
            "job": self.job.clone(),
            "attached": null,
            "cheats": [],
        });
        let Some(g) = self.game.as_ref() else { return out };
        let gid = g.profile.id.clone();
        let defs = g.profile.cheats.clone();
        let extra: Vec<(Option<String>, Option<String>)> =
            defs.iter().map(|d| (self.value_of(&gid, d), self.hotkey_of(&gid, d))).collect();
        let g = self.game.as_mut().unwrap();
        out["attached"] = json!({
            "pid": g.proc.pid,
            "exe": g.proc.name,
            "game": gid,
            "title": g.profile.title,
            "known": g.known,
            "build": g.build,
            "x86": g.proc.is32,
        });
        let cheats: Vec<Value> = defs
            .iter()
            .zip(extra)
            .map(|(d, (value, hotkey))| {
                let r = g.resolve(&d.id);
                let vt = g.vtype(d);
                let patch = matches!(d.locator, Locator::Patch { .. });
                let current = match r {
                    Ok(a) if !patch => g.read_bytes(a, vt.size()).map(|b| vt.fmt(&b)),
                    _ => None,
                };
                let calibration = match &d.locator {
                    Locator::Calibrate { mode, prompt, steps } => {
                        let e = g.calib.cheats.get(&d.id);
                        json!({
                            "mode": mode,
                            "prompt": prompt,
                            "steps": steps,
                            "chains": g.chains.get(&d.id).map(|c| c.len()).unwrap_or(0),
                            "confirmations": e.map(|e| e.confirmations).unwrap_or(0),
                            "session": g.session.contains_key(&d.id),
                        })
                    }
                    _ => Value::Null,
                };
                json!({
                    "id": d.id,
                    "name": d.name,
                    "category": d.category,
                    "description": d.description,
                    "icon": d.icon,
                    "kind": if patch { "patch" } else if d.kind == Kind::Set { "set" } else { "freeze" },
                    "vtype": vt,
                    "value": value,
                    "hotkey": hotkey,
                    "experimental": d.experimental,
                    "custom": d.custom,
                    "status": match r { Ok(_) => "ready", Err(e) => e.code() },
                    "active": g.active.contains_key(&d.id),
                    "current": current,
                    "addr": r.ok().map(|a| format!("{a:X}")),
                    "calibration": calibration,
                })
            })
            .collect();
        out["cheats"] = Value::Array(cheats);
        out
    }

    pub fn games_json(&self) -> Value {
        let procs: HashSet<String> = mem::list_processes().into_iter().map(|p| p.name.to_lowercase()).collect();
        let attached = self.game.as_ref().map(|g| g.profile.id.clone());
        let mut list: Vec<Value> = self.profiles.iter().map(|g| self.game_json(g, true, &procs, attached.as_deref())).collect();
        if let Some(g) = self.game.as_ref().filter(|g| !g.known) {
            list.push(self.game_json(&g.profile, false, &procs, attached.as_deref()));
        }
        Value::Array(list)
    }

    fn game_json(&self, g: &GameProfile, known: bool, procs: &HashSet<String>, attached: Option<&str>) -> Value {
        let appid = g.steam_appid;
        let art = |k: &str| appid.and_then(|a| steam::art(a, k)).is_some();
        let mut cheats = g.cheats.clone();
        if known && attached != Some(g.id.as_str()) {
            cheats.extend(profile::load_custom(&self.data_dir, &g.id));
        }
        json!({
            "id": g.id,
            "title": g.title,
            "subtitle": g.subtitle,
            "accent": g.accent,
            "notes": g.notes,
            "known": known,
            "steam_appid": appid,
            "installed": appid.is_some_and(|a| steam::install_dir(a).is_some()),
            "running": g.processes.iter().any(|p| procs.contains(&p.to_lowercase())),
            "attached": attached == Some(g.id.as_str()),
            "art": { "cover": art("cover"), "hero": art("hero"), "logo": art("logo") },
            "cheats": cheats.iter().map(|c| json!({
                "id": c.id,
                "name": c.name,
                "category": c.category,
                "description": c.description,
                "icon": c.icon,
                "kind": if matches!(c.locator, Locator::Patch { .. }) { "patch" } else if c.kind == Kind::Set { "set" } else { "freeze" },
                "vtype": c.vtype,
                "experimental": c.experimental,
                "custom": c.custom,
                "value": self.value_of(&g.id, c),
                "hotkey": self.hotkey_of(&g.id, c),
            })).collect::<Vec<_>>(),
        })
    }
}

// ---------------------------------------------------------------- scanner jobs

pub enum ScanReq {
    First { vtype: VType, value: f64 },
    Unknown { vtype: VType },
    Next { mode: NextMode, value: Option<f64> },
}

pub fn start_scan(app: &Shared, req: ScanReq) -> Result<u64, String> {
    let (id, proc, prev) = {
        let mut a = lock(app);
        let proc = a.game.as_ref().ok_or("Игра не подключена")?.proc.clone();
        if matches!(req, ScanReq::Next { .. }) && a.scan.is_none() {
            return Err("Сначала сделайте первый поиск".into());
        }
        let label = if matches!(req, ScanReq::Unknown { .. }) { "Делаю снимок памяти…" } else { "Сканирую память…" };
        let id = a.begin_job("scan", label)?;
        let prev = a.scan.take();
        (id, proc, prev)
    };
    let app = app.clone();
    thread::spawn(move || {
        let prog = |f: f32| job_progress(&app, id, f, None);
        let (state, result): (Option<ScanState>, Result<Value, String>) = match req {
            ScanReq::First { vtype, value } => {
                drop(prev);
                let h = scan::first_scan(&proc, vtype, value, &prog);
                let v = json!({"count": h.hits.len(), "truncated": h.truncated});
                (Some(ScanState::Hits(h)), Ok(v))
            }
            ScanReq::Unknown { vtype } => {
                drop(prev);
                let path = std::env::temp_dir().join(format!("fahrenhack-{}-{id}.snap", proc.pid));
                match scan::snapshot(&proc, vtype, path, &prog) {
                    Ok(s) => {
                        let v = json!({"count": null, "snapshot_mb": s.bytes >> 20});
                        (Some(ScanState::Snapshot(s)), Ok(v))
                    }
                    Err(e) => (None, Err(e)),
                }
            }
            ScanReq::Next { mode, value } => match prev {
                Some(ScanState::Hits(mut h)) => {
                    let r = scan::next_scan(&proc, &mut h, mode, value, &prog);
                    let n = h.hits.len();
                    (Some(ScanState::Hits(h)), r.map(|_| json!({"count": n})))
                }
                Some(ScanState::Snapshot(s)) => match scan::compare_snapshot(&proc, &s, mode, value, &prog) {
                    Ok(h) => {
                        let n = h.hits.len();
                        (Some(ScanState::Hits(h)), Ok(json!({"count": n})))
                    }
                    Err(e) => (Some(ScanState::Snapshot(s)), Err(e)),
                },
                None => (None, Err("Сначала сделайте первый поиск".into())),
            },
        };
        let mut a = lock(&app);
        if a.game.as_ref().is_some_and(|g| g.proc.pid == proc.pid) {
            a.scan = state;
        }
        job_done(&mut a, id, result);
    });
    Ok(id)
}

/// Calibration: bind a cheat to the address the user found, then look for pointer
/// chains so it is found automatically after the game restarts.
pub fn start_bind(app: &Shared, cheat: String, addr: usize, vtype: VType) -> Result<u64, String> {
    let (id, proc, modules, main, opts, existing, game_id, build, data_dir) = {
        let mut a = lock(app);
        let g = a.game.as_ref().ok_or("Игра не подключена")?;
        let def = g.def(&cheat).ok_or("Нет такого чита")?;
        if !matches!(def.locator, Locator::Calibrate { .. }) {
            return Err("Этот чит не требует настройки".into());
        }
        let t = (
            g.proc.clone(),
            g.modules.clone(),
            g.main.clone(),
            g.profile.pointer_scan,
            g.calib.cheats.get(&cheat).cloned(),
            g.profile.id.clone(),
            g.build.clone(),
        );
        let data_dir = a.data_dir.clone();
        let id = a.begin_job("pointer", "Проверяю адрес…")?;
        (id, t.0, t.1, t.2, t.3, t.4, t.5, t.6, data_dir)
    };
    let app = app.clone();
    thread::spawn(move || {
        let found: Result<(Vec<Chain>, u32, &str), String> = (|| {
            // A value in a module's static memory needs no pointers at all.
            if let Some(m) = modules.iter().find(|m| m.contains(addr)) {
                return Ok((vec![Chain::static_at(m, addr)], 1, "static"));
            }
            // Calibrated in an earlier session: keep the chains that still lead here.
            if let Some(e) = &existing {
                let kept: Vec<Chain> = e
                    .chains
                    .iter()
                    .filter(|c| c.parse(&modules).and_then(|p| p.resolve(&proc)) == Some(addr))
                    .cloned()
                    .collect();
                if !kept.is_empty() {
                    let conf = if e.last_pid != proc.pid { e.confirmations + 1 } else { e.confirmations.max(1) };
                    return Ok((kept, conf, "confirmed"));
                }
            }
            let chains = pointer::scan(&proc, &modules, std::slice::from_ref(&main), addr, opts, &|f, l| {
                job_progress(&app, id, f, Some(l))
            })?;
            Ok((chains, 1, "scanned"))
        })();
        let mut a = lock(&app);
        let data_dir = data_dir.clone();
        let r = match found {
            Ok((chains, confirmations, how)) => {
                if let Some(g) = a.game.as_mut().filter(|g| g.proc.pid == proc.pid) {
                    g.session.insert(cheat.clone(), addr);
                    g.calib.cheats.insert(
                        cheat.clone(),
                        CalEntry { vtype, chains: chains.clone(), confirmations, last_pid: proc.pid },
                    );
                    let _ = g.calib.save(&data_dir, &game_id, &build);
                    g.rebuild_chains();
                }
                Ok(json!({"chains": chains.len(), "confirmations": confirmations, "how": how}))
            }
            Err(e) => {
                // The address still works until the game is restarted.
                if let Some(g) = a.game.as_mut().filter(|g| g.proc.pid == proc.pid) {
                    g.session.insert(cheat.clone(), addr);
                    g.calib.cheats.insert(cheat.clone(), CalEntry { vtype, chains: vec![], confirmations: 0, last_pid: proc.pid });
                    g.rebuild_chains();
                }
                Ok(json!({"chains": 0, "confirmations": 0, "how": "session", "warning": e}))
            }
        };
        job_done(&mut a, id, r);
    });
    Ok(id)
}

// ---------------------------------------------------------------- workers

pub fn spawn_workers(app: &Shared) {
    let a = app.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(40));
        lock(&a).tick();
    });

    let a = app.clone();
    thread::spawn(move || hotkey_loop(a));

    let a = app.clone();
    thread::spawn(move || auto_attach_loop(a));
}

fn hotkey_loop(app: Shared) {
    let mut prev: HashMap<String, bool> = HashMap::new();
    loop {
        thread::sleep(Duration::from_millis(30));
        let mut a = lock(&app);
        if !a.settings.hotkeys {
            continue;
        }
        let Some(g) = a.game.as_ref() else { continue };
        let gid = g.profile.id.clone();
        let keys: Vec<(String, String, Hotkey)> = g
            .profile
            .cheats
            .iter()
            .filter_map(|c| Some((c.id.clone(), c.name.clone(), Hotkey::parse(&a.hotkey_of(&gid, c)?)?)))
            .collect();
        for (id, name, hk) in keys {
            let down = hk.pressed();
            let was = prev.insert(id.clone(), down).unwrap_or(false);
            if down && !was {
                match a.toggle(&id) {
                    Ok(on) => println!("[hotkey] {name}{}", if on { ": ВКЛ" } else { "" }),
                    Err(e) => println!("[hotkey] {name}: {e}"),
                }
            }
        }
    }
}

/// Attach by itself when a game with a profile is running.
fn auto_attach_loop(app: Shared) {
    loop {
        thread::sleep(Duration::from_secs(2));
        let (wanted, skip) = {
            let a = lock(&app);
            if a.game.is_some() || !a.settings.auto_attach {
                continue;
            }
            let w: Vec<String> = a.profiles.iter().flat_map(|p| p.processes.iter().map(|s| s.to_lowercase())).collect();
            (w, a.skip_pid)
        };
        if wanted.is_empty() {
            continue;
        }
        let Some(p) = mem::list_processes()
            .into_iter()
            .find(|p| wanted.contains(&p.name.to_lowercase()) && Some(p.pid) != skip)
        else {
            continue;
        };
        let mut a = lock(&app);
        if a.game.is_some() {
            continue;
        }
        let n = a.attach_failures.get(&p.pid).copied().unwrap_or(0);
        if n >= 5 {
            continue;
        }
        if let Err(e) = a.attach(p.pid, &p.name) {
            println!("[!] Автоподключение к {}: {e}", p.name);
            a.attach_failures.insert(p.pid, n + 1);
        }
    }
}
