//! Scan and calibration commands and the lifecycle of background jobs.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use serde::Serialize;
use serde_json::{json, Value};

use crate::mem::{Module, Process};
use crate::pointer::{self, Chain};
use crate::profile::{CalEntry, Locator};
use crate::scan::{self, NextMode, ScanState, VType};

use super::{lock, App, Shared};

/// A long operation (scan, pointer search) polled by the UI.
#[derive(Clone, Serialize)]
pub(crate) struct Job {
    id: u64,
    kind: String,
    label: String,
    progress: f32,
    done: bool,
    error: Option<String>,
    result: Option<Value>,
}

fn job_progress(app: &Shared, id: u64, progress: f32, label: Option<&str>) {
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

pub(crate) enum ScanReq {
    First { vtype: VType, value: f64 },
    Unknown { vtype: VType },
    Next { mode: NextMode, value: Option<f64> },
}

pub(crate) fn start_scan(app: &Shared, req: ScanReq) -> Result<u64, String> {
    let (id, proc, prev) = {
        let mut a = lock(app);
        let proc = a.game.as_ref().ok_or("Игра не подключена")?.proc.clone();
        if matches!(req, ScanReq::Next { .. }) && a.scan.is_none() {
            return Err("Сначала сделайте первый поиск".into());
        }
        let label = if matches!(req, ScanReq::Unknown { .. }) {
            "Делаю снимок памяти…"
        } else {
            "Сканирую память…"
        };
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
                Some(ScanState::Snapshot(s)) => {
                    match scan::compare_snapshot(&proc, &s, mode, value, &prog) {
                        Ok(h) => {
                            let n = h.hits.len();
                            (Some(ScanState::Hits(h)), Ok(json!({"count": n})))
                        }
                        Err(e) => (Some(ScanState::Snapshot(s)), Err(e)),
                    }
                }
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
pub(crate) fn start_bind(
    app: &Shared,
    cheat: String,
    addr: usize,
    vtype: VType,
) -> Result<u64, String> {
    let (id, target) = {
        let mut a = lock(app);
        let g = a.game.as_ref().ok_or("Игра не подключена")?;
        let def = g.def(&cheat).ok_or("Нет такого чита")?;
        if !matches!(def.locator, Locator::Calibrate { .. }) {
            return Err("Этот чит не требует настройки".into());
        }
        let target = BindTarget {
            proc: Arc::clone(&g.proc),
            modules: g.modules.clone(),
            main: g.main.clone(),
            opts: g.profile.pointer_scan,
            existing: g.calib.cheats.get(&cheat).cloned(),
            game_id: g.profile.id.clone(),
            build: g.build.clone(),
            data_dir: a.data_dir.clone(),
        };
        let id = a.begin_job("pointer", "Проверяю адрес…")?;
        (id, target)
    };
    let app = app.clone();
    thread::spawn(move || {
        let BindTarget {
            proc,
            modules,
            main,
            opts,
            existing,
            game_id,
            build,
            data_dir,
        } = target;
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
                    let conf = if e.last_pid != proc.pid {
                        e.confirmations + 1
                    } else {
                        e.confirmations.max(1)
                    };
                    return Ok((kept, conf, "confirmed"));
                }
            }
            let chains = pointer::scan(
                &proc,
                &modules,
                std::slice::from_ref(&main),
                addr,
                opts,
                &|f, l| job_progress(&app, id, f, Some(l)),
            )?;
            Ok((chains, 1, "scanned"))
        })();
        let mut a = lock(&app);
        let r = match found {
            Ok((chains, confirmations, how)) => {
                if let Some(g) = a.game.as_mut().filter(|g| g.proc.pid == proc.pid) {
                    g.session_addresses.insert(cheat.clone(), addr);
                    g.calib.cheats.insert(
                        cheat.clone(),
                        CalEntry {
                            vtype,
                            chains: chains.clone(),
                            confirmations,
                            last_pid: proc.pid,
                        },
                    );
                    let _ = g.calib.save(&data_dir, &game_id, &build);
                    g.rebuild_chains();
                }
                Ok(json!({"chains": chains.len(), "confirmations": confirmations, "how": how}))
            }
            Err(e) => {
                // The address still works until the game is restarted.
                if let Some(g) = a.game.as_mut().filter(|g| g.proc.pid == proc.pid) {
                    g.session_addresses.insert(cheat.clone(), addr);
                    g.calib.cheats.insert(
                        cheat.clone(),
                        CalEntry {
                            vtype,
                            chains: vec![],
                            confirmations: 0,
                            last_pid: proc.pid,
                        },
                    );
                    g.rebuild_chains();
                }
                Ok(json!({"chains": 0, "confirmations": 0, "how": "session", "warning": e}))
            }
        };
        job_done(&mut a, id, r);
    });
    Ok(id)
}

struct BindTarget {
    proc: Arc<Process>,
    modules: Vec<Module>,
    main: Module,
    opts: pointer::Opts,
    existing: Option<CalEntry>,
    game_id: String,
    build: String,
    data_dir: PathBuf,
}

impl App {
    fn begin_job(&mut self, kind: &str, label: &str) -> Result<u64, String> {
        if self.job.as_ref().is_some_and(|j| !j.done) {
            return Err("Подождите: предыдущая операция ещё не закончилась".into());
        }
        self.next_job += 1;
        self.job = Some(Job {
            id: self.next_job,
            kind: kind.into(),
            label: label.into(),
            progress: 0.0,
            done: false,
            error: None,
            result: None,
        });
        Ok(self.next_job)
    }

    pub(crate) fn reset_scan(&mut self) -> Result<(), String> {
        if self
            .job
            .as_ref()
            .is_some_and(|job| !job.done && job.kind == "scan")
        {
            return Err("Дождитесь окончания сканирования".into());
        }
        self.scan = None;
        Ok(())
    }

    pub(crate) fn scan_results(&self) -> Vec<Value> {
        let (Some(g), Some(ScanState::Hits(h))) = (self.game.as_ref(), self.scan.as_ref()) else {
            return Vec::new();
        };
        let size = h.vtype.size();
        h
            .hits
            .iter()
            .take(200)
            .map(|hit| {
                let mut b = [0u8; 8];
                let cur = g.proc.read(hit.addr, &mut b[..size]).then(|| h.vtype.fmt(&b));
                let module = g
                    .modules
                    .iter()
                    .find(|m| m.contains(hit.addr))
                    .map(|m| format!("{}+{:X}", m.name, hit.addr - m.base));
                json!({"addr": format!("{:X}", hit.addr), "value": cur, "prev": h.vtype.fmt(&hit.prev), "static": module})
            })
            .collect()
    }

    pub(crate) fn write_scan_value(
        &self,
        addr: &str,
        vtype: VType,
        value: &str,
    ) -> Result<(), String> {
        let g = self.game.as_ref().ok_or("Игра не подключена")?;
        let addr = pointer::parse_hex(addr)
            .filter(|a| *a > 0)
            .ok_or("Неверный адрес")? as usize;
        let bytes = vtype.encode(value)?;
        if !g.proc.write(addr, &bytes) {
            return Err("Запись в память не удалась".into());
        }
        Ok(())
    }
}
