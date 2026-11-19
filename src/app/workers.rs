//! Periodic freezing, hotkey polling and automatic game attachment.

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use crate::hotkey::Hotkey;
use crate::mem;

use super::{lock, Shared};

pub(crate) fn spawn_workers(app: &Shared) {
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
            .filter_map(|c| {
                Some((
                    c.id.clone(),
                    c.name.clone(),
                    Hotkey::parse(&a.settings.cheat_hotkey(&gid, c)?)?,
                ))
            })
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
            let w: Vec<String> = a
                .profiles
                .iter()
                .flat_map(|p| p.processes.iter().map(|s| s.to_lowercase()))
                .collect();
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
