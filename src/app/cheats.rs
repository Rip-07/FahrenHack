//! Cheat commands and application of active effects.

use crate::aob;
use crate::hotkey::Hotkey;
use crate::profile::{self, CalMode, CheatDef, Kind, Locator};
use crate::scan::VType;

use super::session::Active;
use super::App;

impl App {
    fn find_def(&self, game: &str, cheat: &str) -> Option<CheatDef> {
        if let Some(g) = self.game.as_ref().filter(|g| g.profile.id == game) {
            return g.def(cheat).cloned();
        }
        let p = self.profiles.iter().find(|p| p.id == game)?;
        p.cheats
            .iter()
            .find(|c| c.id == cheat)
            .cloned()
            .or_else(|| {
                profile::load_custom(&self.data_dir, game)
                    .into_iter()
                    .find(|c| c.id == cheat)
            })
    }

    /// Switch a cheat (hotkey or button). Returns whether it is on now.
    pub(crate) fn toggle(&mut self, id: &str) -> Result<bool, String> {
        let g = self.game.as_mut().ok_or("Игра не подключена")?;
        let def = g.def(id).cloned().ok_or("Нет такого чита")?;
        let value = self.settings.cheat_value(&g.profile.id, &def);
        if let Locator::Patch { bytes, .. } = &def.locator {
            if let Some(Active::Patch { addr, original }) = g.active.remove(id) {
                g.proc.write(addr, &original);
                return Ok(false);
            }
            let addr = g.resolve(id).map_err(|e| e.message().to_string())?;
            let patch = aob::parse_bytes(bytes).ok_or("Неверные байты патча в профиле")?;
            let original = g
                .read_bytes(addr, patch.len())
                .ok_or("Не удалось прочитать код игры")?;
            if !g.proc.write(addr, &patch) {
                return Err("Запись в память не удалась".into());
            }
            g.active
                .insert(id.to_string(), Active::Patch { addr, original });
            return Ok(true);
        }
        match def.kind {
            Kind::Set => {
                let v = value.ok_or("Укажите значение в параметрах чита (⋯)")?;
                let addr = g.resolve(id).map_err(|e| e.message().to_string())?;
                let bytes = g.vtype(&def).encode(&v)?;
                g.proc
                    .write(addr, &bytes)
                    .then_some(false)
                    .ok_or_else(|| "Запись в память не удалась".to_string())
            }
            Kind::Freeze => {
                if g.active.remove(id).is_some() {
                    return Ok(false);
                }
                let addr = g.resolve(id).map_err(|e| e.message().to_string())?;
                let vt = g.vtype(&def);
                let bytes = match &value {
                    Some(v) => vt.encode(v)?,
                    None => g
                        .read_bytes(addr, vt.size())
                        .ok_or("Не удалось прочитать значение")?,
                };
                if !g.proc.write(addr, &bytes) {
                    return Err("Запись в память не удалась".into());
                }
                g.active.insert(id.to_string(), Active::Freeze(bytes));
                Ok(true)
            }
        }
    }

    pub(crate) fn all_off(&mut self) {
        if let Some(g) = self.game.as_mut() {
            g.restore_patches();
        }
    }

    /// value: "" = profile default; hotkey: "" = no hotkey.
    pub(crate) fn set_override(
        &mut self,
        game: &str,
        cheat: &str,
        value: String,
        hotkey: String,
    ) -> Result<(), String> {
        let def = self.find_def(game, cheat).ok_or("Нет такого чита")?;
        let value = value.trim().to_string();
        let hotkey = hotkey.trim().to_uppercase();
        if !value.is_empty() && !value.eq_ignore_ascii_case("current") {
            let vt = self
                .game
                .as_ref()
                .filter(|g| g.profile.id == game)
                .map(|g| g.vtype(&def))
                .unwrap_or(def.vtype);
            vt.encode(&value)?;
        }
        if !hotkey.is_empty() && Hotkey::parse(&hotkey).is_none() {
            return Err("Неверная клавиша. Примеры: F6, CTRL+F2, SHIFT+NUM5, ALT+K".into());
        }
        self.settings.set_cheat_override(game, cheat, value, hotkey);
        self.save_settings();
        self.refreeze(game, &def);
        Ok(())
    }

    pub(crate) fn clear_override(&mut self, game: &str, cheat: &str) {
        self.settings.clear_cheat_override(game, cheat);
        self.save_settings();
        if let Some(def) = self.find_def(game, cheat) {
            self.refreeze(game, &def);
        }
    }

    /// Apply a changed value to a cheat that is currently frozen.
    fn refreeze(&mut self, game: &str, def: &CheatDef) {
        let value = self.settings.cheat_value(game, def);
        let Some(g) = self.game.as_mut().filter(|g| g.profile.id == game) else {
            return;
        };
        if !matches!(g.active.get(&def.id), Some(Active::Freeze(_))) {
            return;
        }
        let vt = g.vtype(def);
        let bytes = match value {
            Some(v) => vt.encode(&v).ok(),
            None => g
                .resolve(&def.id)
                .ok()
                .and_then(|a| g.read_bytes(a, vt.size())),
        };
        if let Some(b) = bytes {
            g.active.insert(def.id.clone(), Active::Freeze(b));
        }
    }

    pub(crate) fn reset_calibration(&mut self, id: &str) -> Result<(), String> {
        let g = self.game.as_mut().ok_or("Игра не подключена")?;
        g.calib.cheats.remove(id);
        g.session_addresses.remove(id);
        g.active.remove(id);
        g.calib.save(&self.data_dir, &g.profile.id, &g.build)?;
        g.rebuild_chains();
        Ok(())
    }

    pub(crate) fn add_custom(
        &mut self,
        name: String,
        vtype: VType,
        kind: Kind,
        value: Option<String>,
    ) -> Result<String, String> {
        let g = self.game.as_mut().ok_or("Игра не подключена")?;
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("Введите название".into());
        }
        if let Some(v) = value.as_deref().filter(|v| !v.trim().is_empty()) {
            vtype.encode(v)?;
        }
        let id = (1..)
            .map(|n| format!("my{n}"))
            .find(|id| g.def(id).is_none())
            .unwrap();
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
            locator: Locator::Calibrate {
                mode: CalMode::Exact,
                prompt: String::new(),
                steps: vec![],
            },
        });
        profile::save_custom(&self.data_dir, &g.profile.id, &g.profile.cheats)?;
        Ok(id)
    }

    pub(crate) fn remove_custom(&mut self, id: &str) -> Result<(), String> {
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
    pub(super) fn tick(&mut self) {
        let alive = match &self.game {
            Some(g) => g.proc.is_alive(),
            None => return,
        };
        if !alive {
            self.detach();
            return;
        }
        let g = self.game.as_mut().unwrap();
        let ids: Vec<String> = g
            .active
            .iter()
            .filter(|(_, a)| matches!(a, Active::Freeze(_)))
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            if let Ok(addr) = g.resolve(&id) {
                if let Some(Active::Freeze(bytes)) = g.active.get(&id) {
                    g.proc.write(addr, bytes);
                }
            }
        }
    }
}
