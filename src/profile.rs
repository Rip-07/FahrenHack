//! Game profiles (which cheats a game has and how to find them), per-build calibrations
//! and the user's own cheats.
//!
//! Built-in profiles live in `games/*.json` and are compiled into the binary.
//! Files in `%APPDATA%\FahrenHack\games\` override or add profiles without rebuilding.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::pointer::{self, Chain};
use crate::scan::VType;

const BUILTIN: &[(&str, &str)] = &[("re4r.json", include_str!("../games/re4r.json"))];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameProfile {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub processes: Vec<String>,
    #[serde(default)]
    pub steam_appid: Option<u32>,
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub pointer_scan: pointer::Opts,
    #[serde(default)]
    pub cheats: Vec<CheatDef>,
    /// Verified pointer chains shipped with the profile: game build -> cheat id -> chains.
    #[serde(default)]
    pub builds: HashMap<String, HashMap<String, Vec<Chain>>>,
}

fn default_accent() -> String {
    "#2b4c6f".into()
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Keep the value while the cheat is on.
    Freeze,
    /// Write the value once per press.
    Set,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheatDef {
    pub id: String,
    #[serde(default)]
    pub category: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    pub kind: Kind,
    pub vtype: VType,
    /// Default value. For `freeze`, `null` means "keep the value it had when switched on".
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub hotkey: Option<String>,
    #[serde(default)]
    pub experimental: bool,
    #[serde(default)]
    pub custom: bool,
    pub locator: Locator,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CalMode {
    /// The number is visible in the game (money, ammo): exact search.
    #[default]
    Exact,
    /// Only a bar is visible (health): memory snapshot + increased/decreased.
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Locator {
    /// Fixed pointer chain `[[module+o0]+o1]+…`.
    Pointer {
        #[serde(default)]
        module: Option<String>,
        offsets: Vec<String>,
    },
    /// Byte signature; optional RIP-relative [displacement offset, instruction length], then a chain.
    Aob {
        #[serde(default)]
        module: Option<String>,
        pattern: String,
        #[serde(default)]
        rip: Option<[usize; 2]>,
        #[serde(default)]
        add: i64,
        #[serde(default)]
        offsets: Vec<String>,
    },
    /// Code patch toggled on/off (original bytes are restored).
    Patch {
        #[serde(default)]
        module: Option<String>,
        pattern: String,
        #[serde(default)]
        offset: i64,
        bytes: String,
    },
    /// Found once on the user's machine by a guided search, then kept as pointer chains.
    Calibrate {
        #[serde(default)]
        mode: CalMode,
        #[serde(default)]
        prompt: String,
        #[serde(default)]
        steps: Vec<String>,
    },
}

impl GameProfile {
    /// Profile for any process without a ready-made profile.
    pub fn generic(exe: &str) -> GameProfile {
        GameProfile {
            id: exe.to_lowercase(),
            title: exe.trim_end_matches(".exe").trim_end_matches(".EXE").to_string(),
            subtitle: "Произвольный процесс".into(),
            processes: vec![exe.to_string()],
            steam_appid: None,
            accent: default_accent(),
            notes: "Для этой игры нет готового профиля. Найдите значения во вкладке «Сканер» и добавьте их кнопкой «В читы».".into(),
            pointer_scan: pointer::Opts::default(),
            cheats: vec![],
            builds: HashMap::new(),
        }
    }
}

/// Reads a JSON file; tolerates the UTF-8 BOM that Notepad and PowerShell add.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let s = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()
}

fn parse_profile(name: &str, s: &str) -> Option<GameProfile> {
    match serde_json::from_str::<GameProfile>(s.trim_start_matches('\u{feff}')) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("[!] Профиль {name} не загружен: {e}");
            None
        }
    }
}

pub fn builtin() -> Vec<GameProfile> {
    BUILTIN.iter().filter_map(|(n, s)| parse_profile(n, s)).collect()
}

pub fn load_all(data: &Path) -> Vec<GameProfile> {
    let mut list = builtin();
    let dir = data.join("games");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_some_and(|x| x == "json") {
                let name = path.display().to_string();
                if let Some(p) = std::fs::read_to_string(&path).ok().and_then(|s| parse_profile(&name, &s)) {
                    list.retain(|x| x.id != p.id);
                    list.push(p);
                }
            }
        }
    }
    list
}

fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Calibration {
    #[serde(default)]
    pub cheats: HashMap<String, CalEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CalEntry {
    pub vtype: VType,
    #[serde(default)]
    pub chains: Vec<Chain>,
    /// In how many game sessions the chains led to the right value.
    #[serde(default)]
    pub confirmations: u32,
    #[serde(default)]
    pub last_pid: u32,
}

impl Calibration {
    fn path(data: &Path, game: &str, build: &str) -> PathBuf {
        data.join("calibration").join(safe(game)).join(format!("{}.json", safe(build)))
    }

    pub fn load(data: &Path, game: &str, build: &str) -> Calibration {
        read_json(&Self::path(data, game, build)).unwrap_or_default()
    }

    pub fn save(&self, data: &Path, game: &str, build: &str) -> Result<(), String> {
        let path = Self::path(data, game, build);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let s = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, s).map_err(|e| e.to_string())
    }
}

fn custom_path(data: &Path, game: &str) -> PathBuf {
    data.join("custom").join(format!("{}.json", safe(game)))
}

pub fn load_custom(data: &Path, game: &str) -> Vec<CheatDef> {
    read_json::<Vec<CheatDef>>(&custom_path(data, game))
        .unwrap_or_default()
        .into_iter()
        .map(|mut c| {
            c.custom = true;
            c
        })
        .collect()
}

pub fn save_custom(data: &Path, game: &str, cheats: &[CheatDef]) -> Result<(), String> {
    let path = custom_path(data, game);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mine: Vec<&CheatDef> = cheats.iter().filter(|c| c.custom).collect();
    let s = serde_json::to_string_pretty(&mine).map_err(|e| e.to_string())?;
    std::fs::write(path, s).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_profiles_parse() {
        let list = builtin();
        assert_eq!(list.len(), BUILTIN.len(), "every built-in profile must parse");
        for g in &list {
            assert!(!g.processes.is_empty(), "{}: no process names", g.id);
            let mut ids = std::collections::HashSet::new();
            for c in &g.cheats {
                assert!(ids.insert(&c.id), "{}: duplicate cheat id {}", g.id, c.id);
                if let Some(h) = &c.hotkey {
                    assert!(crate::hotkey::Hotkey::parse(h).is_some(), "{}: bad hotkey {h}", c.id);
                }
                if let Some(v) = &c.value {
                    assert!(c.vtype.encode(v).is_ok(), "{}: bad value {v}", c.id);
                }
            }
        }
    }
}
