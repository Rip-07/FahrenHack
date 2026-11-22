//! Attached game, its compatibility guard and address resolution.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::aob;
use crate::inject;
use crate::mem::{Module, Process};
use crate::pointer::{self, Chain, Parsed};
use crate::profile::{self, Calibration, CheatDef, GameProfile, Locator};
use crate::scan::VType;

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
        ANTI_CHEAT
            .iter()
            .find(|(k, _)| n.contains(k))
            .map(|(_, name)| *name)
    })
}

pub(super) enum Active {
    Freeze(Vec<u8>),
    Patch { addr: usize, original: Vec<u8> },
    /// Code injection: the site jumps into `cave`; `original` is restored when switched off.
    /// `links` are the cheat ids whose addresses the cave reads (`{addr:ID}`).
    Inject { site: usize, original: Vec<u8>, cave: usize, links: Vec<String> },
}

impl Active {
    /// Code range this effect has overwritten, to refuse overlapping patches.
    pub(super) fn code_range(&self) -> Option<(usize, usize)> {
        match self {
            Active::Patch { addr, original } => Some((*addr, addr + original.len())),
            Active::Inject { site, original, .. } => Some((*site, site + original.len())),
            Active::Freeze(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Unresolved {
    NeedsCalibration,
    Waiting,
    Missing,
    Invalid,
    /// `Item` locator: the item is not in the inventory right now.
    Absent,
}

impl Unresolved {
    pub(super) fn code(self) -> &'static str {
        match self {
            Unresolved::NeedsCalibration => "calibrate",
            Unresolved::Waiting => "waiting",
            Unresolved::Missing => "missing",
            Unresolved::Invalid => "invalid",
            Unresolved::Absent => "waiting",
        }
    }

    pub(super) fn message(self) -> &'static str {
        match self {
            Unresolved::NeedsCalibration => "Чит ещё не настроен: нажмите «Настроить»",
            Unresolved::Waiting => "Адрес пока недоступен: загрузите сохранение и войдите в игру",
            Unresolved::Missing => "Сигнатура не найдена: профиль не подходит к этой версии игры",
            Unresolved::Invalid => "Ошибка в описании чита",
            Unresolved::Absent => "Этого предмета сейчас нет в инвентаре: подберите хотя бы один",
        }
    }
}

pub(super) struct GameSession {
    pub(super) proc: Arc<Process>,
    pub(super) modules: Vec<Module>,
    pub(super) main: Module,
    pub(super) build: String,
    pub(super) profile: GameProfile,
    /// The game has a ready-made profile.
    pub(super) known: bool,
    pub(super) calib: Calibration,
    /// Addresses found by calibration in this session (used when no pointer chain is known).
    pub(super) session_addresses: HashMap<String, usize>,
    pub(super) active: HashMap<String, Active>,
    /// Code caves allocated in the game for injection cheats (kept for the whole session).
    pub(super) caves: HashMap<String, usize>,
    chains: HashMap<String, Vec<Parsed>>,
    pick: HashMap<String, (usize, Instant)>,
    aob_cache: HashMap<String, Option<usize>>,
}

impl GameSession {
    pub(super) fn open(
        pid: u32,
        exe: &str,
        profiles: &[GameProfile],
        data_dir: &Path,
    ) -> Result<Self, String> {
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
        let found = profiles
            .iter()
            .find(|profile| {
                profile
                    .processes
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(exe))
            })
            .cloned();
        let known = found.is_some();
        let mut profile = found.unwrap_or_else(|| GameProfile::generic(exe));
        profile
            .cheats
            .extend(profile::load_custom(data_dir, &profile.id));
        let calib = Calibration::load(data_dir, &profile.id, &build);
        let mut game = Self {
            proc: Arc::new(p),
            modules,
            main,
            build,
            profile,
            known,
            calib,
            session_addresses: HashMap::new(),
            active: HashMap::new(),
            caves: HashMap::new(),
            chains: HashMap::new(),
            pick: HashMap::new(),
            aob_cache: HashMap::new(),
        };
        game.rebuild_chains();
        Ok(game)
    }

    pub(super) fn restore_patches(&mut self) {
        let ids: Vec<String> = self.active.keys().cloned().collect();
        for id in ids {
            self.effect_off(&id);
        }
    }

    /// A frozen flag (e.g. "invincible") is written back to the profile's `off_value`
    /// when released, so the game is not left with the cheat's value.
    fn release_freeze(&mut self, id: &str) {
        let Some(def) = self.def(id).cloned() else { return };
        let Some(off) = def.off_value.as_deref() else { return };
        let Ok(bytes) = self.vtype(&def).encode(off) else { return };
        if let Ok(addr) = self.resolve(id) {
            self.proc.write(addr, &bytes);
        }
    }

    /// Name of another active cheat that already rewrote code in `[site, site+len)`.
    pub(super) fn code_conflict(&self, site: usize, len: usize, except: &str) -> Option<String> {
        self.active
            .iter()
            .filter(|(id, _)| id.as_str() != except)
            .find(|(_, a)| a.code_range().is_some_and(|(s, e)| site < e && site + len > s))
            .map(|(id, _)| self.def(id).map_or_else(|| id.clone(), |d| d.name.clone()))
    }

    /// Switches a code injection on. `param` is written to the cave's data slot first,
    /// so the injected code never runs with a stale parameter.
    pub(super) fn inject_on(&mut self, def: &CheatDef, param: Option<&[u8]>) -> Result<(), String> {
        let Locator::Inject { size, code, .. } = &def.locator else {
            return Err("Этот чит не использует инъекцию кода".into());
        };
        if let Some(Active::Inject { cave, .. }) = self.active.get(&def.id) {
            if let Some(p) = param {
                self.proc.write(cave + inject::PARAM_OFFSET, p);
            }
            return Ok(());
        }
        let site = self.resolve(&def.id).map_err(|e| e.message().to_string())?;
        if let Some(other) = self.code_conflict(site, *size, &def.id) {
            return Err(format!("Этот чит меняет тот же код, что и «{other}»: сначала выключите его"));
        }
        let parts = inject::parse(code).ok_or("Ошибка в коде инъекции профиля")?;
        let original = self.read_bytes(site, *size).ok_or("Не удалось прочитать код игры")?;
        let cave = match self.caves.get(&def.id) {
            Some(&c) => c,
            None => {
                let c = self
                    .proc
                    .alloc_near(site, inject::CAVE_SIZE)
                    .ok_or("Не удалось выделить память рядом с кодом игры")?;
                self.caves.insert(def.id.clone(), c);
                c
            }
        };
        let cave_code = inject::assemble(&parts, site, *size, &original, cave)?;
        let links = inject::links(&parts);
        if let Some(p) = param {
            self.proc.write(cave + inject::PARAM_OFFSET, p);
        }
        // Linked addresses go in before the jump, like the parameter.
        self.write_links(cave, &links);
        if !self.proc.write(cave, &cave_code) {
            return Err("Не удалось записать код инъекции".into());
        }
        let jump = inject::site_jump(site, *size, cave)?;
        if !self.proc.write(site, &jump) {
            return Err("Не удалось изменить код игры".into());
        }
        self.active
            .insert(def.id.clone(), Active::Inject { site, original, cave, links });
        Ok(())
    }

    /// Writes the current address of each linked cheat (0 if unknown) into the cave slots.
    fn write_links(&mut self, cave: usize, links: &[String]) {
        for (k, id) in links.iter().enumerate() {
            let addr = self.resolve(id).unwrap_or(0) as u64;
            self.proc.write(cave + inject::LINK_OFFSET + 8 * k, &addr.to_le_bytes());
        }
    }

    /// Keeps `{addr:ID}` slots of active injections current (objects move on area loads).
    pub(super) fn refresh_links(&mut self) {
        let caves: Vec<(usize, Vec<String>)> = self
            .active
            .values()
            .filter_map(|a| match a {
                Active::Inject { cave, links, .. } if !links.is_empty() => Some((*cave, links.clone())),
                _ => None,
            })
            .collect();
        for (cave, links) in caves {
            self.write_links(cave, &links);
        }
    }

    /// Switches any active effect of a cheat off, restoring patched code.
    pub(super) fn effect_off(&mut self, id: &str) -> bool {
        match self.active.remove(id) {
            Some(Active::Patch { addr, original }) => {
                self.proc.write(addr, &original);
                true
            }
            Some(Active::Inject { site, original, .. }) => {
                self.proc.write(site, &original);
                true
            }
            Some(Active::Freeze(_)) => {
                self.release_freeze(id);
                true
            }
            None => false,
        }
    }

    pub(super) fn chain_count(&self, cheat: &str) -> usize {
        self.chains.get(cheat).map_or(0, Vec::len)
    }

    pub(super) fn def(&self, id: &str) -> Option<&CheatDef> {
        self.profile.cheats.iter().find(|c| c.id == id)
    }

    pub(super) fn rebuild_chains(&mut self) {
        self.chains.clear();
        self.pick.clear();
        for c in &self.profile.cheats {
            let list: Vec<Chain> = match self.calib.cheats.get(&c.id) {
                Some(e) if !e.chains.is_empty() => e.chains.clone(),
                _ => self
                    .profile
                    .builds
                    .get(&self.build)
                    .and_then(|b| b.get(&c.id))
                    .cloned()
                    .unwrap_or_default(),
            };
            let parsed: Vec<Parsed> = list
                .iter()
                .filter_map(|ch| ch.parse(&self.modules))
                .collect();
            if !parsed.is_empty() {
                self.chains.insert(c.id.clone(), parsed);
            }
        }
    }

    pub(super) fn vtype(&self, def: &CheatDef) -> VType {
        self.calib
            .cheats
            .get(&def.id)
            .map(|e| e.vtype)
            .unwrap_or(def.vtype)
    }

    pub(super) fn read_bytes(&self, addr: usize, len: usize) -> Option<Vec<u8>> {
        let mut b = vec![0u8; len];
        self.proc.read(addr, &mut b).then_some(b)
    }

    fn aob_site(
        &mut self,
        key: &str,
        module: Option<&str>,
        pattern: &str,
    ) -> Result<usize, Unresolved> {
        if let Some(v) = self.aob_cache.get(key) {
            return v.ok_or(Unresolved::Missing);
        }
        let m = match module {
            Some(n) if !n.trim().is_empty() => self
                .modules
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(n.trim()))
                .cloned(),
            _ => Some(self.main.clone()),
        };
        let m = m.ok_or(Unresolved::Waiting)?;
        let pat = aob::parse(pattern).ok_or(Unresolved::Invalid)?;
        let site = aob::find(&self.proc, &m, &pat);
        self.aob_cache.insert(key.to_string(), site);
        site.ok_or(Unresolved::Missing)
    }

    /// Signature (+ optional RIP-relative displacement) + add, then a pointer chain.
    fn aob_chain(
        &mut self,
        key: &str,
        module: Option<&str>,
        pattern: &str,
        rip: Option<[usize; 2]>,
        add: i64,
        offsets: &[String],
    ) -> Result<usize, Unresolved> {
        let site = self.aob_site(key, module, pattern)?;
        let mut addr = site as i64;
        if let Some([disp, len]) = rip {
            let d = self.proc.read_u32(site + disp).ok_or(Unresolved::Waiting)? as i32;
            addr = site as i64 + len as i64 + d as i64;
        }
        let mut addr = (addr + add) as usize;
        for o in offsets {
            let off = pointer::parse_hex(o).ok_or(Unresolved::Invalid)?;
            addr = (self.proc.read_ptr(addr).ok_or(Unresolved::Waiting)? as i64 + off) as usize;
        }
        Ok(addr)
    }

    /// Address of the cheat's value (or code patch site) in this session.
    pub(super) fn resolve(&mut self, id: &str) -> Result<usize, Unresolved> {
        let loc = self.def(id).ok_or(Unresolved::Invalid)?.locator.clone();
        match loc {
            Locator::Pointer { module, offsets } => {
                pointer::resolve_chain(&self.proc, &self.modules, module.as_deref(), &offsets)
                    .ok_or(Unresolved::Waiting)
            }
            Locator::Aob {
                module,
                pattern,
                rip,
                add,
                offsets,
            } => self.aob_chain(id, module.as_deref(), &pattern, rip, add, &offsets),
            Locator::Item {
                module,
                pattern,
                rip,
                add,
                offsets,
                item,
                id_at,
                id: item_id,
                field,
            } => {
                let list = self.aob_chain(id, module.as_deref(), &pattern, rip, add, &offsets)?;
                let hex = |s: &str| pointer::parse_hex(s).ok_or(Unresolved::Invalid);
                let (item_off, id_off, field_off) = (hex(&item)?, hex(&id_at)?, hex(&field)?);
                let want = u32::from_str_radix(item_id.trim().trim_start_matches("0x"), 16)
                    .map_err(|_| Unresolved::Invalid)?;
                let array = self.proc.read_ptr(list + 0x10).ok_or(Unresolved::Waiting)?;
                let size = self.proc.read_u32(list + 0x18).ok_or(Unresolved::Waiting)? as usize;
                for i in 0..size.min(4096) {
                    let Some(elem) = self.proc.read_ptr(array + 0x20 + 8 * i) else { continue };
                    let Some(obj) = self.proc.read_ptr((elem as i64 + item_off) as usize) else { continue };
                    if obj != 0 && self.proc.read_u32((obj as i64 + id_off) as usize) == Some(want) {
                        return Ok((obj as i64 + field_off) as usize);
                    }
                }
                Err(Unresolved::Absent)
            }
            Locator::Patch {
                module,
                pattern,
                offset,
                ..
            } => Ok((self.aob_site(id, module.as_deref(), &pattern)? as i64 + offset) as usize),
            Locator::Inject {
                module,
                pattern,
                offset,
                ..
            } => Ok((self.aob_site(id, module.as_deref(), &pattern)? as i64 + offset) as usize),
            Locator::Calibrate { .. } => {
                if self.chains.contains_key(id) {
                    if let Ok(a) = self.pick_chain(id) {
                        return Ok(a);
                    }
                }
                if let Some(&a) = self.session_addresses.get(id) {
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
            if let Some(a) = c
                .resolve(&self.proc)
                .filter(|&a| self.proc.read(a, &mut probe))
            {
                self.pick.insert(id.to_string(), (i, Instant::now()));
                return Ok(a);
            }
        }
        Err(Unresolved::Waiting)
    }
}
