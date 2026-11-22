//! Value scanner (like Cheat Engine): an exact first scan or a full memory snapshot,
//! then narrowing down with "increased / decreased / changed / unchanged / exact".

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::mem::{for_each_chunk, Process, Region};

/// Hard cap on stored candidates (16 bytes each).
pub const MAX_HITS: usize = 10_000_000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VType {
    I32,
    I64,
    F32,
    F64,
    /// One byte (game flags). Profiles only; the scanner UI does not offer it.
    U8,
}

fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace() && *c != '_').collect::<String>().replace(',', ".")
}

impl VType {
    pub fn size(self) -> usize {
        match self {
            VType::U8 => 1,
            VType::I32 | VType::F32 => 4,
            VType::I64 | VType::F64 => 8,
        }
    }

    pub fn decode(self, b: &[u8]) -> f64 {
        match self {
            VType::I32 => i32::from_le_bytes(b[..4].try_into().unwrap()) as f64,
            VType::I64 => i64::from_le_bytes(b[..8].try_into().unwrap()) as f64,
            VType::F32 => f32::from_le_bytes(b[..4].try_into().unwrap()) as f64,
            VType::F64 => f64::from_le_bytes(b[..8].try_into().unwrap()),
            VType::U8 => b[0] as f64,
        }
    }

    pub fn encode(self, s: &str) -> Result<Vec<u8>, String> {
        let t = clean(s);
        let err = || format!("«{}» — неверное значение для типа {}", s.trim(), self.label());
        Ok(match self {
            VType::I32 => t.parse::<i32>().map_err(|_| err())?.to_le_bytes().to_vec(),
            VType::I64 => t.parse::<i64>().map_err(|_| err())?.to_le_bytes().to_vec(),
            VType::F32 => t.parse::<f32>().map_err(|_| err())?.to_le_bytes().to_vec(),
            VType::F64 => t.parse::<f64>().map_err(|_| err())?.to_le_bytes().to_vec(),
            VType::U8 => vec![t.parse::<u8>().map_err(|_| err())?],
        })
    }

    pub fn fmt(self, b: &[u8]) -> String {
        match self {
            VType::I32 | VType::I64 | VType::U8 => format!("{}", self.decode(b) as i64),
            VType::F32 | VType::F64 => {
                let s = format!("{:.3}", self.decode(b));
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            VType::I32 => "целое 4 байта",
            VType::I64 => "целое 8 байт",
            VType::F32 => "дробное 4 байта",
            VType::F64 => "дробное 8 байт",
            VType::U8 => "байт 0–255",
        }
    }

    fn is_float(self) -> bool {
        matches!(self, VType::F32 | VType::F64)
    }
}

pub fn parse_number(s: &str) -> Result<f64, String> {
    clean(s).parse::<f64>().map_err(|_| format!("«{}» — не число", s.trim()))
}

fn matches_exact(vt: VType, v: f64, t: f64) -> bool {
    if vt.is_float() {
        // Games often keep 100 as 100.37 — compare with a tolerance.
        let tol = if t.fract() == 0.0 { 0.5 } else { 0.005 };
        (v - t).abs() <= tol
    } else {
        v == t
    }
}

fn to8(b: &[u8]) -> [u8; 8] {
    let mut a = [0u8; 8];
    a[..b.len()].copy_from_slice(b);
    a
}

#[derive(Clone, Copy)]
pub struct Hit {
    pub addr: usize,
    pub prev: [u8; 8],
}

pub struct HitScan {
    pub vtype: VType,
    pub hits: Vec<Hit>,
    pub truncated: bool,
}

struct Seg {
    addr: usize,
    len: usize,
}

/// Copy of all writable memory on disk — the start of an "unknown value" search.
pub struct Snapshot {
    pub vtype: VType,
    path: PathBuf,
    segs: Vec<Seg>,
    pub bytes: u64,
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub enum ScanState {
    Hits(HitScan),
    Snapshot(Snapshot),
}

impl ScanState {
    pub fn json(&self) -> Value {
        match self {
            ScanState::Hits(h) => json!({"count": h.hits.len(), "vtype": h.vtype, "truncated": h.truncated, "snapshot": false}),
            ScanState::Snapshot(s) => json!({"count": null, "vtype": s.vtype, "truncated": false, "snapshot": true, "snapshot_mb": s.bytes >> 20}),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NextMode {
    Exact,
    Increased,
    Decreased,
    Changed,
    Unchanged,
}

fn total(regions: &[Region]) -> f32 {
    regions.iter().map(|r| r.size as f64).sum::<f64>().max(1.0) as f32
}

pub fn first_scan(p: &Process, vt: VType, target: f64, progress: &dyn Fn(f32)) -> HitScan {
    let regions = p.regions();
    let total = total(&regions);
    let size = vt.size();
    let mut hits = Vec::new();
    let mut truncated = false;
    let mut done = 0f32;
    for_each_chunk(p, &regions, |addr, b| {
        let mut i = 0;
        while i + size <= b.len() {
            let chunk = &b[i..i + size];
            if matches_exact(vt, vt.decode(chunk), target) {
                hits.push(Hit { addr: addr + i, prev: to8(chunk) });
                if hits.len() >= MAX_HITS {
                    truncated = true;
                    return false;
                }
            }
            i += 4;
        }
        done += b.len() as f32;
        progress(done / total);
        true
    });
    HitScan { vtype: vt, hits, truncated }
}

pub fn snapshot(p: &Process, vt: VType, path: PathBuf, progress: &dyn Fn(f32)) -> Result<Snapshot, String> {
    let regions = p.regions();
    let total = total(&regions);
    let mut snap = Snapshot { vtype: vt, path, segs: Vec::new(), bytes: 0 };
    let file = File::create(&snap.path).map_err(|e| format!("Не удалось создать файл снимка: {e}"))?;
    let mut w = BufWriter::with_capacity(1 << 20, file);
    let mut err = None;
    let mut done = 0f32;
    for_each_chunk(p, &regions, |addr, b| {
        if let Err(e) = w.write_all(b) {
            err = Some(e.to_string());
            return false;
        }
        snap.segs.push(Seg { addr, len: b.len() });
        snap.bytes += b.len() as u64;
        done += b.len() as f32;
        progress(done / total);
        true
    });
    if let Err(e) = w.flush() {
        err.get_or_insert(e.to_string());
    }
    drop(w);
    match err {
        Some(e) => Err(format!("Не удалось записать снимок памяти (мало места на диске?): {e}")),
        None => Ok(snap),
    }
}

/// First filter after a snapshot. Errors (keeping the snapshot) if far too many values match.
pub fn compare_snapshot(p: &Process, snap: &Snapshot, mode: NextMode, target: Option<f64>, progress: &dyn Fn(f32)) -> Result<HitScan, String> {
    if mode == NextMode::Exact && target.is_none() {
        return Err("Введите значение для точного поиска".into());
    }
    let vt = snap.vtype;
    let size = vt.size();
    let file = File::open(&snap.path).map_err(|e| format!("Снимок недоступен: {e}"))?;
    let mut r = BufReader::with_capacity(1 << 20, file);
    let mut old = Vec::new();
    let mut cur = Vec::new();
    let mut hits = Vec::new();
    let total = snap.bytes.max(1) as f32;
    let mut done = 0f32;
    for seg in &snap.segs {
        old.resize(seg.len, 0);
        r.read_exact(&mut old).map_err(|e| format!("Ошибка чтения снимка: {e}"))?;
        cur.resize(seg.len, 0);
        done += seg.len as f32;
        if !p.read(seg.addr, &mut cur) {
            continue;
        }
        let mut i = 0;
        while i + size <= seg.len {
            let c = &cur[i..i + size];
            let o = &old[i..i + size];
            let keep = match mode {
                NextMode::Exact => matches_exact(vt, vt.decode(c), target.unwrap()),
                NextMode::Increased => vt.decode(c) > vt.decode(o),
                NextMode::Decreased => vt.decode(c) < vt.decode(o),
                NextMode::Changed => c != o,
                NextMode::Unchanged => c == o,
            };
            if keep {
                hits.push(Hit { addr: seg.addr + i, prev: to8(c) });
                if hits.len() >= MAX_HITS {
                    return Err("Слишком много совпадений. Сначала измените значение в игре и выберите «Уменьшилось», «Увеличилось» или «Изменилось».".into());
                }
            }
            i += 4;
        }
        progress(done / total);
    }
    Ok(HitScan { vtype: vt, hits, truncated: false })
}

/// Reads values page by page instead of one WinAPI call per address.
struct PageReader<'a> {
    p: &'a Process,
    page: usize,
    valid: bool,
    buf: [u8; 4096],
}

impl<'a> PageReader<'a> {
    fn new(p: &'a Process) -> Self {
        PageReader { p, page: usize::MAX, valid: false, buf: [0; 4096] }
    }

    fn read(&mut self, addr: usize, size: usize) -> Option<[u8; 8]> {
        let off = addr & 0xFFF;
        if off + size > 4096 {
            let mut b = [0u8; 8];
            return self.p.read(addr, &mut b[..size]).then_some(b);
        }
        let page = addr & !0xFFF;
        if page != self.page {
            self.page = page;
            self.valid = self.p.read(page, &mut self.buf);
        }
        self.valid.then(|| to8(&self.buf[off..off + size]))
    }
}

pub fn next_scan(p: &Process, st: &mut HitScan, mode: NextMode, target: Option<f64>, progress: &dyn Fn(f32)) -> Result<(), String> {
    if mode == NextMode::Exact && target.is_none() {
        return Err("Введите значение для точного поиска".into());
    }
    let vt = st.vtype;
    let size = vt.size();
    let total = st.hits.len().max(1) as f32;
    let mut reader = PageReader::new(p);
    let mut n = 0usize;
    st.hits.retain_mut(|h| {
        n += 1;
        if n % 262_144 == 0 {
            progress(n as f32 / total);
        }
        let Some(cur) = reader.read(h.addr, size) else { return false };
        let c = vt.decode(&cur);
        let pv = vt.decode(&h.prev);
        let keep = match mode {
            NextMode::Exact => matches_exact(vt, c, target.unwrap()),
            NextMode::Increased => c > pv,
            NextMode::Decreased => c < pv,
            NextMode::Changed => cur[..size] != h.prev[..size],
            NextMode::Unchanged => cur[..size] == h.prev[..size],
        };
        if keep {
            h.prev = cur;
        }
        keep
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_roundtrip() {
        assert_eq!(VType::I32.fmt(&VType::I32.encode("1 000 000").unwrap()), "1000000");
        assert_eq!(VType::F32.fmt(&VType::F32.encode("12,5").unwrap()), "12.5");
        assert!(VType::I32.encode("abc").is_err());
        assert_eq!(VType::U8.encode("1").unwrap(), vec![1]);
        assert!(VType::U8.encode("256").is_err());
        assert_eq!(VType::U8.fmt(&[7]), "7");
    }

    #[test]
    fn float_tolerance() {
        assert!(matches_exact(VType::F32, 100.37, 100.0));
        assert!(!matches_exact(VType::F32, 101.0, 100.0));
        assert!(matches_exact(VType::I32, 5.0, 5.0));
    }
}
