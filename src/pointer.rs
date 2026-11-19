//! Pointer chains `[[module+o0]+o1]+o2…` and a pointer scanner that finds chains
//! from the game's static memory to a value, so a cheat keeps working after a restart.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::mem::{for_each_chunk, Module, Process};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Chain {
    pub module: String,
    pub offsets: Vec<String>,
}

/// A chain with the module base and offsets already resolved for this session.
#[derive(Clone, Debug)]
pub struct Parsed {
    base: usize,
    offsets: Vec<i64>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Opts {
    pub max_depth: usize,
    pub max_offset: usize,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { max_depth: 5, max_offset: 0x1000 }
    }
}

pub fn parse_hex(s: &str) -> Option<i64> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r.trim()),
        None => (false, s),
    };
    let s = s.trim_start_matches("0x").trim_start_matches("0X");
    let v = i64::from_str_radix(s, 16).ok()?;
    Some(if neg { -v } else { v })
}

pub fn hex(v: i64) -> String {
    if v < 0 {
        format!("-0x{:X}", -v)
    } else {
        format!("0x{v:X}")
    }
}

impl Chain {
    /// A value that lives directly in a module's static memory.
    pub fn static_at(m: &Module, addr: usize) -> Chain {
        Chain { module: m.name.clone(), offsets: vec![hex((addr - m.base) as i64)] }
    }

    pub fn parse(&self, modules: &[Module]) -> Option<Parsed> {
        let base = modules.iter().find(|m| m.name.eq_ignore_ascii_case(&self.module))?.base;
        let offsets = self.offsets.iter().map(|o| parse_hex(o)).collect::<Option<Vec<_>>>()?;
        (!offsets.is_empty()).then_some(Parsed { base, offsets })
    }
}

impl Parsed {
    pub fn resolve(&self, p: &Process) -> Option<usize> {
        let mut addr = (self.base as i64).wrapping_add(self.offsets[0]) as usize;
        for o in &self.offsets[1..] {
            let ptr = p.read_ptr(addr)?;
            if ptr < 0x10000 {
                return None;
            }
            addr = (ptr as i64).wrapping_add(*o) as usize;
        }
        Some(addr)
    }
}

/// `module = None` means the first offset is an absolute address.
pub fn resolve_chain(p: &Process, modules: &[Module], module: Option<&str>, offsets: &[String]) -> Option<usize> {
    let base = match module {
        Some(m) if !m.trim().is_empty() => modules.iter().find(|x| x.name.eq_ignore_ascii_case(m.trim()))?.base,
        _ => 0,
    };
    let offsets = offsets.iter().map(|s| parse_hex(s)).collect::<Option<Vec<_>>>()?;
    if offsets.is_empty() {
        return None;
    }
    Parsed { base, offsets }.resolve(p)
}

const MAX_PAIRS: usize = 120_000_000;
const MAX_FRONTIER: usize = 400_000;
const MAX_FOUND: usize = 30_000;
/// How many chains are kept per cheat.
pub const KEEP: usize = 64;

/// Finds pointer chains from `statics` (usually the game's exe) to `target`.
pub fn scan(
    p: &Process,
    modules: &[Module],
    statics: &[Module],
    target: usize,
    opts: Opts,
    progress: &dyn Fn(f32, &str),
) -> Result<Vec<Chain>, String> {
    let regions = p.regions();
    if regions.is_empty() {
        return Err("Не удалось прочитать память игры".into());
    }
    let starts: Vec<usize> = regions.iter().map(|r| r.base).collect();
    let ends: Vec<usize> = regions.iter().map(|r| r.base + r.size).collect();
    let (lo, hi) = (starts[0], *ends.last().unwrap());
    let inside = |v: usize| {
        if v < lo || v >= hi {
            return false;
        }
        let i = starts.partition_point(|&s| s <= v);
        i > 0 && v < ends[i - 1]
    };

    // 1. Every value in writable memory that looks like a pointer into writable memory.
    let total = regions.iter().map(|r| r.size as f64).sum::<f64>().max(1.0) as f32;
    let psz = p.ptr_size();
    let mut pairs: Vec<(u64, u64)> = Vec::new();
    let mut capped = false;
    let mut done = 0f32;
    progress(0.0, "Читаю память игры…");
    for_each_chunk(p, &regions, |addr, b| {
        let mut i = 0;
        while i + psz <= b.len() {
            let v = if psz == 8 {
                u64::from_le_bytes(b[i..i + 8].try_into().unwrap()) as usize
            } else {
                u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as usize
            };
            if inside(v) {
                pairs.push((v as u64, (addr + i) as u64));
            }
            i += psz;
        }
        if pairs.len() > MAX_PAIRS {
            capped = true;
            return false;
        }
        done += b.len() as f32;
        progress(0.55 * done / total, "Читаю память игры…");
        true
    });
    progress(0.56, "Сортирую указатели…");
    pairs.sort_unstable();

    // 2. Walk backwards from the target: who points at (target - offset)?
    struct Node {
        addr: usize,
        off: u32,
        parent: u32,
    }
    const ROOT: u32 = u32::MAX;
    let mut nodes = vec![Node { addr: target, off: 0, parent: ROOT }];
    let mut frontier: Vec<u32> = vec![0];
    let mut seen: HashSet<usize> = HashSet::from([target]);
    let mut found: Vec<(u32, usize, usize, u32)> = Vec::new(); // (node, static location, module, offset)
    'levels: for depth in 1..=opts.max_depth {
        progress(
            0.62 + 0.33 * (depth - 1) as f32 / opts.max_depth as f32,
            &format!("Ищу цепочки указателей: уровень {depth} из {}…", opts.max_depth),
        );
        let mut next: Vec<u32> = Vec::new();
        for &ni in &frontier {
            let t = nodes[ni as usize].addr;
            let from = t.saturating_sub(opts.max_offset) as u64;
            let mut k = pairs.partition_point(|&(v, _)| v < from);
            while k < pairs.len() && pairs[k].0 <= t as u64 {
                let (v, loc) = pairs[k];
                k += 1;
                let loc = loc as usize;
                let off = (t as u64 - v) as u32;
                if let Some(mi) = statics.iter().position(|m| m.contains(loc)) {
                    found.push((ni, loc, mi, off));
                    if found.len() >= MAX_FOUND {
                        break 'levels;
                    }
                } else if depth < opts.max_depth && seen.insert(loc) {
                    nodes.push(Node { addr: loc, off, parent: ni });
                    next.push((nodes.len() - 1) as u32);
                }
            }
        }
        if next.len() > MAX_FRONTIER {
            next.sort_by_key(|&i| nodes[i as usize].off);
            next.truncate(MAX_FRONTIER);
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    drop(pairs);

    // 3. Shortest chains with the smallest offsets first; keep those that resolve right now.
    let mut chains: Vec<(usize, i64, Chain)> = found
        .iter()
        .map(|&(ni, loc, mi, off)| {
            let m = &statics[mi];
            let mut offs = vec![(loc - m.base) as i64, off as i64];
            let mut cur = ni;
            while nodes[cur as usize].parent != ROOT {
                offs.push(nodes[cur as usize].off as i64);
                cur = nodes[cur as usize].parent;
            }
            let sum: i64 = offs[1..].iter().sum();
            (offs.len(), sum, Chain { module: m.name.clone(), offsets: offs.iter().map(|&o| hex(o)).collect() })
        })
        .collect();
    chains.sort_by_key(|(len, sum, _)| (*len, *sum));
    progress(0.97, "Проверяю найденные цепочки…");
    let mut out = Vec::new();
    for (_, _, c) in chains {
        if c.parse(modules).and_then(|pc| pc.resolve(p)) == Some(target) {
            out.push(c);
            if out.len() >= KEEP {
                break;
            }
        }
    }
    if out.is_empty() && capped {
        return Err("Игра использует слишком много памяти для поиска указателей".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        assert_eq!(parse_hex("0x1A"), Some(26));
        assert_eq!(parse_hex("-0x10"), Some(-16));
        assert_eq!(parse_hex("ff"), Some(255));
        assert_eq!(hex(-16), "-0x10");
        assert_eq!(hex(4096), "0x1000");
        assert_eq!(parse_hex("zz"), None);
    }
}
