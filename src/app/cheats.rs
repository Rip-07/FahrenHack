//! Cheat commands and application of active effects.

use crate::aob;
use crate::hotkey::Hotkey;
use crate::profile::{self, CalMode, CheatDef, Kind, Locator};
use crate::scan::{parse_number, VType};

use super::session::Active;
use super::App;

/// Text form of a number for a value type: integers are rounded, floats keep up to 4 decimals.
pub(super) fn format_number(vt: VType, n: f64) -> String {
    match vt {
        VType::I32 | VType::I64 | VType::U8 => format!("{}", n.round() as i64),
        VType::F32 | VType::F64 => {
            let s = format!("{n:.4}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
    }
}

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
    /// For a select it applies the chosen option; numbers and sliders use `set_value`/`step`.
    pub(crate) fn toggle(&mut self, id: &str) -> Result<bool, String> {
        let g = self.game.as_mut().ok_or("Игра не подключена")?;
        let def = g.def(id).cloned().ok_or("Нет такого чита")?;
        let value = self.settings.cheat_value(&g.profile.id, &def);
        match def.kind {
            Kind::Select => {
                let v = value
                    .or_else(|| def.options.first().map(|o| o.value.clone()))
                    .ok_or("Не выбран вариант")?;
                return self.set_value(id, &v);
            }
            Kind::Number => return Err("Измените значение кнопками − и +".into()),
            Kind::Slider => {
                // Switch between off and the remembered (or default) position.
                if g.active.contains_key(id) {
                    g.effect_off(id);
                    return Ok(false);
                }
                let v = value
                    .filter(|v| !def.is_off_value(v))
                    .or_else(|| def.max.map(|m| format_number(g.vtype(&def), def.clamp(m.min(2.0)))))
                    .ok_or("Укажите значение ползунком")?;
                return self.set_value(id, &v);
            }
            Kind::Freeze | Kind::Set => {}
        }
        if let Locator::Inject { .. } = &def.locator {
            if g.effect_off(id) {
                return Ok(false);
            }
            let vt = g.vtype(&def);
            let param = value.as_deref().map(|v| vt.encode(v)).transpose()?;
            g.inject_on(&def, param.as_deref())?;
            return Ok(true);
        }
        if let Locator::Patch { bytes, .. } = &def.locator {
            if let Some(Active::Patch { addr, original }) = g.active.remove(id) {
                g.proc.write(addr, &original);
                return Ok(false);
            }
            let addr = g.resolve(id).map_err(|e| e.message().to_string())?;
            let patch = aob::parse_bytes(bytes).ok_or("Неверные байты патча в профиле")?;
            if let Some(other) = g.code_conflict(addr, patch.len(), id) {
                return Err(format!("Этот чит меняет тот же код, что и «{other}»: сначала выключите его"));
            }
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
            Kind::Number | Kind::Slider | Kind::Select => unreachable!("handled above"),
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
                if g.effect_off(id) {
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

    /// Sets the value of a number, slider or select (and the stored value of other kinds).
    /// Returns whether the cheat is active afterwards.
    pub(crate) fn set_value(&mut self, id: &str, raw: &str) -> Result<bool, String> {
        let g = self.game.as_mut().ok_or("Игра не подключена")?;
        let def = g.def(id).cloned().ok_or("Нет такого чита")?;
        let gid = g.profile.id.clone();
        let vt = g.vtype(&def);
        match def.kind {
            Kind::Number => {
                let n = def.clamp(parse_number(raw)?);
                let bytes = vt.encode(&format_number(vt, n))?;
                let addr = g.resolve(id).map_err(|e| e.message().to_string())?;
                if !g.proc.write(addr, &bytes) {
                    return Err("Запись в память не удалась".into());
                }
                Ok(false)
            }
            Kind::Slider => {
                let value = format_number(vt, def.clamp(parse_number(raw)?));
                self.settings.remember_value(&gid, id, value.clone());
                self.save_settings();
                let g = self.game.as_mut().unwrap();
                if def.is_off_value(&value) {
                    g.effect_off(id);
                    return Ok(false);
                }
                let bytes = vt.encode(&value)?;
                if let Locator::Inject { .. } = def.locator {
                    g.inject_on(&def, Some(&bytes))?;
                } else {
                    let addr = g.resolve(id).map_err(|e| e.message().to_string())?;
                    if !g.proc.write(addr, &bytes) {
                        return Err("Запись в память не удалась".into());
                    }
                    g.active.insert(id.to_string(), Active::Freeze(bytes));
                }
                Ok(true)
            }
            Kind::Select => {
                let option = def
                    .options
                    .iter()
                    .find(|o| o.value == raw.trim())
                    .ok_or("Такого варианта нет в профиле")?;
                let value = option.value.clone();
                let bytes = vt.encode(&value)?;
                self.settings.remember_value(&gid, id, value);
                self.save_settings();
                let g = self.game.as_mut().unwrap();
                if let Locator::Inject { .. } = def.locator {
                    g.inject_on(&def, Some(&bytes))?;
                    return Ok(true);
                }
                let addr = g.resolve(id).map_err(|e| e.message().to_string())?;
                if !g.proc.write(addr, &bytes) {
                    return Err("Запись в память не удалась".into());
                }
                Ok(false)
            }
            Kind::Freeze | Kind::Set => {
                vt.encode(raw)?;
                self.settings.remember_value(&gid, id, raw.trim().to_string());
                self.save_settings();
                self.refreeze(&gid, &def);
                Ok(self.game.as_ref().is_some_and(|g| g.active.contains_key(id)))
            }
        }
    }

    /// "Increase" (dir > 0) or "decrease" (dir < 0): one step for numbers and sliders,
    /// the next or previous option for selects, on/off for switches and buttons.
    pub(crate) fn step(&mut self, id: &str, dir: i32) -> Result<bool, String> {
        let g = self.game.as_mut().ok_or("Игра не подключена")?;
        let def = g.def(id).cloned().ok_or("Нет такого чита")?;
        let vt = g.vtype(&def);
        let dir = dir.signum() as f64;
        let next = match def.kind {
            Kind::Freeze | Kind::Set => {
                return if dir > 0.0 { self.toggle(id) } else { Ok(g.active.contains_key(id)) };
            }
            Kind::Number => {
                let addr = g.resolve(id).map_err(|e| e.message().to_string())?;
                let cur = g
                    .read_bytes(addr, vt.size())
                    .map(|b| vt.decode(&b))
                    .ok_or("Не удалось прочитать значение")?;
                format_number(vt, cur + dir * def.step())
            }
            Kind::Slider => {
                let cur = self
                    .settings
                    .cheat_value(&g.profile.id, &def)
                    .or_else(|| def.off_value.clone())
                    .and_then(|v| parse_number(&v).ok())
                    .or(def.min)
                    .unwrap_or(0.0);
                format_number(vt, cur + dir * def.step())
            }
            Kind::Select => {
                if def.options.is_empty() {
                    return Err("В профиле нет вариантов".into());
                }
                let cur = self.settings.cheat_value(&g.profile.id, &def);
                let n = def.options.len() as i64;
                let i = cur
                    .and_then(|v| def.options.iter().position(|o| o.value == v))
                    .map_or(if dir > 0.0 { -1 } else { 0 }, |i| i as i64);
                def.options[(i + dir as i64).rem_euclid(n) as usize].value.clone()
            }
        };
        self.set_value(id, &next)
    }

    pub(crate) fn toggle_pin(&mut self, game: &str, cheat: &str) -> bool {
        let pinned = self.settings.toggle_pin(game, cheat);
        self.save_settings();
        pinned
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
        hotkey_down: Option<String>,
    ) -> Result<(), String> {
        let def = self.find_def(game, cheat).ok_or("Нет такого чита")?;
        let value = value.trim().to_string();
        let hotkey = hotkey.trim().to_uppercase();
        let hotkey_down = hotkey_down.map(|h| h.trim().to_uppercase());
        if let Some(h) = hotkey_down.as_deref().filter(|h| !h.is_empty()) {
            if Hotkey::parse(h).is_none() {
                return Err("Неверная вторая клавиша. Примеры: SHIFT+F3, CTRL+SHIFT+F2".into());
            }
        }
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
        self.settings
            .set_cheat_override(game, cheat, value, hotkey, hotkey_down);
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
            hotkey_down: None,
            info: String::new(),
            warn: String::new(),
            step: None,
            min: None,
            max: None,
            off_value: None,
            options: vec![],
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
        g.refresh_links();
    }
}
