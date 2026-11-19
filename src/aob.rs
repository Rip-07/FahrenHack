//! Byte-signature (AOB) search inside a module, e.g. `48 8B 05 ?? ?? ?? ?? 48 85 C0`.
//! Used by profiles with verified signatures (code patches, RIP-relative statics).

use crate::mem::{Module, Process, RegionKind};

pub fn parse(pattern: &str) -> Option<Vec<Option<u8>>> {
    let v = pattern
        .split_whitespace()
        .map(|t| {
            if t.chars().all(|c| c == '?') {
                Some(None)
            } else {
                u8::from_str_radix(t, 16).ok().map(Some)
            }
        })
        .collect::<Option<Vec<_>>>()?;
    (!v.is_empty() && v.iter().any(|b| b.is_some())).then_some(v)
}

pub fn parse_bytes(s: &str) -> Option<Vec<u8>> {
    let v = s.split_whitespace().map(|t| u8::from_str_radix(t, 16).ok()).collect::<Option<Vec<_>>>()?;
    (!v.is_empty()).then_some(v)
}

/// First address in the module's readable memory that matches the pattern.
pub fn find(p: &Process, m: &Module, pat: &[Option<u8>]) -> Option<usize> {
    const CHUNK: usize = 8 << 20;
    let (k, first) = pat.iter().enumerate().find_map(|(i, b)| b.map(|b| (i, b)))?;
    let n = pat.len();
    let mut buf = vec![0u8; CHUNK + n];
    for r in p.regions_in(RegionKind::Readable, m.base, m.base + m.size) {
        let mut off = 0;
        while off < r.size {
            // Overlap chunks by n-1 bytes so matches on a boundary are not lost.
            let len = (r.size - off).min(CHUNK + n - 1);
            let b = &mut buf[..len];
            if len >= n && p.read(r.base + off, b) {
                for i in 0..=len - n {
                    if b[i + k] == first && pat.iter().enumerate().all(|(j, pb)| pb.map_or(true, |x| b[i + j] == x)) {
                        return Some(r.base + off + i);
                    }
                }
            }
            off += CHUNK;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert_eq!(parse("48 8B ?? 05"), Some(vec![Some(0x48), Some(0x8B), None, Some(0x05)]));
        assert_eq!(parse("?? ??"), None);
        assert_eq!(parse("ZZ"), None);
        assert_eq!(parse_bytes("90 90"), Some(vec![0x90, 0x90]));
    }
}
