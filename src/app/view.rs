//! JSON projections shared by the desktop UI and the phone remote.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::mem;
use crate::profile::{self, CheatDef, GameProfile, Kind, Locator};
use crate::steam;

use super::App;

impl App {
    pub(crate) fn state_json(&mut self) -> Value {
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
        let Some(g) = self.game.as_ref() else {
            return out;
        };
        let gid = g.profile.id.clone();
        let defs = g.profile.cheats.clone();
        let extra: Vec<(Option<String>, Option<String>)> = defs
            .iter()
            .map(|d| {
                (
                    self.settings.cheat_value(&gid, d),
                    self.settings.cheat_hotkey(&gid, d),
                )
            })
            .collect();
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
                    Locator::Calibrate {
                        mode,
                        prompt,
                        steps,
                    } => {
                        let e = g.calib.cheats.get(&d.id);
                        json!({
                            "mode": mode,
                            "prompt": prompt,
                            "steps": steps,
                            "chains": g.chain_count(&d.id),
                            "confirmations": e.map(|e| e.confirmations).unwrap_or(0),
                            "session": g.session_addresses.contains_key(&d.id),
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
                    "kind": cheat_kind(d),
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

    pub(crate) fn games_json(&self) -> Value {
        let procs: HashSet<String> = mem::list_processes()
            .into_iter()
            .map(|p| p.name.to_lowercase())
            .collect();
        let attached = self.game.as_ref().map(|g| g.profile.id.clone());
        let mut list: Vec<Value> = self
            .profiles
            .iter()
            .map(|g| self.game_json(g, true, &procs, attached.as_deref()))
            .collect();
        if let Some(g) = self.game.as_ref().filter(|g| !g.known) {
            list.push(self.game_json(&g.profile, false, &procs, attached.as_deref()));
        }
        Value::Array(list)
    }

    fn game_json(
        &self,
        g: &GameProfile,
        known: bool,
        procs: &HashSet<String>,
        attached: Option<&str>,
    ) -> Value {
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
                "kind": cheat_kind(c),
                "vtype": c.vtype,
                "experimental": c.experimental,
                "custom": c.custom,
                "value": self.settings.cheat_value(&g.id, c),
                "hotkey": self.settings.cheat_hotkey(&g.id, c),
            })).collect::<Vec<_>>(),
        })
    }
}

fn cheat_kind(definition: &CheatDef) -> &'static str {
    if matches!(definition.locator, Locator::Patch { .. }) {
        "patch"
    } else {
        match definition.kind {
            Kind::Set => "set",
            Kind::Freeze => "freeze",
        }
    }
}
