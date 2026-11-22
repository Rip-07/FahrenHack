//! Code injection through a code cave.
//!
//! `size` bytes at the patch site are replaced by `jmp cave` (E9 rel32) plus NOPs. The cave is
//! allocated within ±2 GB of the site and holds the profile's code; the cheat's parameter
//! (for sliders, e.g. a multiplier) lives in a data slot at `cave + PARAM_OFFSET`.
//!
//! The `code` template is hex bytes separated by spaces, plus placeholders:
//! - `{orig}`      the original bytes overwritten at the site (must not be RIP-relative);
//! - `{return}`    `jmp` back to the first instruction after the overwritten bytes (5 bytes);
//! - `{param}`     rel32 displacement to the parameter slot, for an instruction whose
//!                 displacement is its last field, e.g. `F3 0F 59 05 {param}` = `mulss xmm0,[param]`;
//! - `{param_abs}` 8-byte absolute address of the parameter, e.g. `48 B8 {param_abs}`;
//! - `{addr:ID}`   rel32 displacement to a cave slot that holds the current address of
//!                 another cheat's value (cheat `ID` of the same profile), or 0 while that
//!                 address is unknown. The app refreshes the slot while the injection is on,
//!                 e.g. `48 8B 05 {addr:health}` = `mov rax,[address of the player's health]`.

pub const CAVE_SIZE: usize = 0x1000;
pub const PARAM_OFFSET: usize = 0xF00;
/// First slot for `{addr:ID}` values, after the 8-byte parameter.
pub const LINK_OFFSET: usize = 0xF08;
pub const MAX_LINKS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Bytes(Vec<u8>),
    Orig,
    Return,
    Param,
    ParamAbs,
    Addr(String),
}

/// Cheat ids referenced by `{addr:ID}`, in slot order (slot k at `LINK_OFFSET + 8k`).
pub fn links(parts: &[Part]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in parts {
        if let Part::Addr(id) = p {
            if !out.contains(id) {
                out.push(id.clone());
            }
        }
    }
    out
}

pub fn parse(code: &str) -> Option<Vec<Part>> {
    let mut parts = Vec::new();
    for token in code.split_whitespace() {
        let part = match token {
            "{orig}" => Part::Orig,
            "{return}" => Part::Return,
            "{param}" => Part::Param,
            "{param_abs}" => Part::ParamAbs,
            t if t.starts_with("{addr:") && t.ends_with('}') && t.len() > 7 => Part::Addr(t[6..t.len() - 1].to_string()),
            hex => Part::Bytes(vec![u8::from_str_radix(hex, 16).ok().filter(|_| hex.len() == 2)?]),
        };
        match (parts.last_mut(), part) {
            (Some(Part::Bytes(prev)), Part::Bytes(b)) => prev.extend(b),
            (_, p) => parts.push(p),
        }
    }
    (!parts.is_empty()).then_some(parts)
}

fn rel32(from_end: usize, to: usize) -> Result<[u8; 4], String> {
    let d = to as i64 - from_end as i64;
    i32::try_from(d)
        .map(|v| v.to_le_bytes())
        .map_err(|_| "Код-пещера слишком далеко от кода игры".to_string())
}

/// Machine code for the cave.
pub fn assemble(parts: &[Part], site: usize, size: usize, original: &[u8], cave: usize) -> Result<Vec<u8>, String> {
    let param = cave + PARAM_OFFSET;
    let ids = links(parts);
    if ids.len() > MAX_LINKS {
        return Err("Слишком много ссылок {addr:…} в коде инъекции".into());
    }
    let mut out = Vec::new();
    for p in parts {
        match p {
            Part::Bytes(b) => out.extend_from_slice(b),
            Part::Orig => out.extend_from_slice(original),
            Part::Return => {
                out.push(0xE9);
                let end = cave + out.len() + 4;
                out.extend_from_slice(&rel32(end, site + size)?);
            }
            Part::Param => {
                let end = cave + out.len() + 4;
                out.extend_from_slice(&rel32(end, param)?);
            }
            Part::ParamAbs => out.extend_from_slice(&(param as u64).to_le_bytes()),
            Part::Addr(id) => {
                let k = ids.iter().position(|x| x == id).unwrap();
                let end = cave + out.len() + 4;
                out.extend_from_slice(&rel32(end, cave + LINK_OFFSET + 8 * k)?);
            }
        }
    }
    if out.len() > PARAM_OFFSET {
        return Err("Код инъекции не помещается в код-пещеру".into());
    }
    Ok(out)
}

/// Bytes written over the site: `jmp cave` padded with NOPs.
pub fn site_jump(site: usize, size: usize, cave: usize) -> Result<Vec<u8>, String> {
    if size < 5 {
        return Err("Для перехода нужно минимум 5 байт".into());
    }
    let mut out = vec![0xE9];
    out.extend_from_slice(&rel32(site + 5, cave)?);
    out.resize(size, 0x90);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_template() {
        let p = parse("8B 15 {param} F0 29 51 18 {orig} {return}").unwrap();
        assert_eq!(
            p,
            vec![Part::Bytes(vec![0x8B, 0x15]), Part::Param, Part::Bytes(vec![0xF0, 0x29, 0x51, 0x18]), Part::Orig, Part::Return]
        );
        assert!(parse("ZZ").is_none());
        assert!(parse("123").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn assemble_offsets() {
        let site = 0x1_4000_1000;
        let cave = 0x1_4100_0000;
        let code = assemble(&parse("8B 15 {param} {return}").unwrap(), site, 5, &[0x90; 5], cave).unwrap();
        // mov edx,[rip+d]: d = param - (cave + 6)
        let d = i32::from_le_bytes(code[2..6].try_into().unwrap()) as i64;
        assert_eq!(cave as i64 + 6 + d, (cave + PARAM_OFFSET) as i64);
        // jmp back: target = cave + 11 + rel
        assert_eq!(code[6], 0xE9);
        let r = i32::from_le_bytes(code[7..11].try_into().unwrap()) as i64;
        assert_eq!(cave as i64 + 11 + r, (site + 5) as i64);
        let j = site_jump(site, 7, cave).unwrap();
        assert_eq!(j.len(), 7);
        assert_eq!(&j[5..], &[0x90, 0x90]);
        let r = i32::from_le_bytes(j[1..5].try_into().unwrap()) as i64;
        assert_eq!(site as i64 + 5 + r, cave as i64);
        assert!(site_jump(site, 4, cave).is_err());
        assert!(site_jump(site, 5, site + (3 << 30)).is_err());
    }

    #[test]
    fn addr_links_get_their_own_slots() {
        let parts = parse("48 8B 05 {addr:health} 48 8B 0D {addr:ammo} 48 8B 15 {addr:health} {return}").unwrap();
        assert_eq!(links(&parts), vec!["health".to_string(), "ammo".to_string()]);
        let cave = 0x1_4100_0000;
        let code = assemble(&parts, 0x1_4000_1000, 5, &[0x90; 5], cave).unwrap();
        // each `mov r64,[rip+d]` is 7 bytes; d is relative to the end of the instruction
        let target = |at: usize| (cave + at + 7) as i64 + i32::from_le_bytes(code[at + 3..at + 7].try_into().unwrap()) as i64;
        assert_eq!(target(0), (cave + LINK_OFFSET) as i64);
        assert_eq!(target(7), (cave + LINK_OFFSET + 8) as i64);
        assert_eq!(target(14), (cave + LINK_OFFSET) as i64);
        assert!(parse("{addr:}").is_none());
    }
}
