//! Configuration, per-cheat preferences and settings persistence.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::profile::{self, CheatDef};

use super::App;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) token: String,
    pub(crate) port: u16,
    pub(crate) auto_attach: bool,
    pub(crate) hotkeys: bool,
    /// Phone remote over the LAN (off by default: only this PC can open the UI).
    pub(crate) remote: bool,
    pub(crate) sounds: bool,
    pub(crate) fullscreen: bool,
    /// "game/cheat" -> user's value and hotkey.
    overrides: HashMap<String, Override>,
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
pub(crate) fn detect_lan_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("192.168.0.1:9")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Override {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    /// `Some("")` disables the profile's hotkey.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hotkey: Option<String>,
}

pub(crate) fn gen_token() -> String {
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

pub(super) fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FAHRENHACK_DATA") {
        return PathBuf::from(d);
    }
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_else(|| ".".into())).join("FahrenHack")
}

impl Settings {
    pub(super) fn load(data_dir: &Path) -> Self {
        let mut settings: Self =
            profile::read_json(&data_dir.join("settings.json")).unwrap_or_default();
        if settings.token.len() < 12 {
            settings.token = gen_token();
        }
        settings
    }

    fn override_key(game: &str, cheat: &str) -> String {
        format!("{game}/{cheat}")
    }

    /// The user's value, else the profile's. `None` = freeze the current value.
    pub(super) fn cheat_value(&self, game: &str, def: &CheatDef) -> Option<String> {
        let o = self.overrides.get(&Self::override_key(game, &def.id));
        o.and_then(|o| o.value.clone())
            .or_else(|| def.value.clone())
            .filter(|s| !s.trim().is_empty() && !s.trim().eq_ignore_ascii_case("current"))
    }

    pub(super) fn cheat_hotkey(&self, game: &str, def: &CheatDef) -> Option<String> {
        match self
            .overrides
            .get(&Self::override_key(game, &def.id))
            .and_then(|o| o.hotkey.clone())
        {
            Some(h) => (!h.trim().is_empty()).then_some(h),
            None => def.hotkey.clone(),
        }
    }

    pub(super) fn set_cheat_override(
        &mut self,
        game: &str,
        cheat: &str,
        value: String,
        hotkey: String,
    ) {
        let entry = self
            .overrides
            .entry(Self::override_key(game, cheat))
            .or_default();
        entry.value = (!value.is_empty()).then_some(value);
        entry.hotkey = Some(hotkey);
    }

    pub(super) fn clear_cheat_override(&mut self, game: &str, cheat: &str) {
        self.overrides.remove(&Self::override_key(game, cheat));
    }
}

impl App {
    pub(crate) fn save_settings(&self) {
        if let Ok(s) = serde_json::to_string_pretty(&self.settings) {
            let _ = std::fs::write(self.data_dir.join("settings.json"), s);
        }
    }

    pub(crate) fn lan_url(&self) -> String {
        format!(
            "http://{}:{}/?t={}",
            self.lan_ip, self.port, self.settings.token
        )
    }
}
