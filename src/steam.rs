//! Steam integration: install detection, the user's local library art cache, launching games.
//! Art is read from the user's own Steam cache at runtime and is never bundled with FahrenHack.

use std::path::{Path, PathBuf};

use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn root() -> Option<PathBuf> {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let key = wide("Software\\Valve\\Steam");
    let val = wide("SteamPath");
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            val.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if r == 0 {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let p = PathBuf::from(String::from_utf16_lossy(&buf[..len]).replace('/', "\\"));
        if p.exists() {
            return Some(p);
        }
    }
    let p = PathBuf::from(r"C:\Program Files (x86)\Steam");
    p.exists().then_some(p)
}

/// Strings inside double quotes of a VDF/ACF line.
fn quoted(line: &str) -> Vec<String> {
    line.split('"').skip(1).step_by(2).map(|s| s.replace("\\\\", "\\")).collect()
}

pub fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    if let Ok(s) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) {
        for line in s.lines() {
            let q = quoted(line);
            if q.len() == 2 && q[0] == "path" {
                let p = PathBuf::from(&q[1]);
                if !out.iter().any(|x| x.to_string_lossy().eq_ignore_ascii_case(&p.to_string_lossy())) {
                    out.push(p);
                }
            }
        }
    }
    out
}

pub fn install_dir(appid: u32) -> Option<PathBuf> {
    let root = root()?;
    for lib in libraries(&root) {
        let apps = lib.join("steamapps");
        let Ok(s) = std::fs::read_to_string(apps.join(format!("appmanifest_{appid}.acf"))) else { continue };
        for line in s.lines() {
            let q = quoted(line);
            if q.len() == 2 && q[0].eq_ignore_ascii_case("installdir") {
                let d = apps.join("common").join(&q[1]);
                if d.exists() {
                    return Some(d);
                }
            }
        }
    }
    None
}

pub fn art(appid: u32, kind: &str) -> Option<PathBuf> {
    let name = match kind {
        "cover" => "library_600x900.jpg",
        "hero" => "library_hero.jpg",
        "logo" => "logo.png",
        "header" => "header.jpg",
        _ => return None,
    };
    let cache = root()?.join("appcache").join("librarycache");
    let legacy = cache.join(format!("{appid}_{name}"));
    if legacy.exists() {
        return Some(legacy);
    }
    find_file(&cache.join(appid.to_string()), name, 2)
}

fn find_file(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.exists() {
        return Some(direct);
    }
    if depth == 0 {
        return None;
    }
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        if e.path().is_dir() {
            if let Some(p) = find_file(&e.path(), name, depth - 1) {
                return Some(p);
            }
        }
    }
    None
}

pub fn launch(appid: u32) -> Result<(), String> {
    open(&format!("steam://rungameid/{appid}"))
}

/// Opens a URL or folder with its default handler, without a console window.
pub fn open(target: &str) -> Result<(), String> {
    std::process::Command::new("explorer.exe").arg(target).spawn().map(|_| ()).map_err(|e| e.to_string())
}
