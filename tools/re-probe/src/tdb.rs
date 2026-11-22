//! Research on the game's own type database ("TDB", RE Engine), read from outside the
//! process. Layout found empirically on this game: header counts at +0x0C.., table pointers
//! at +0x58.., element sizes from table gaps divided by counts.

use crate::Target;

pub struct Tdb {
    #[allow(dead_code)]
    pub hdr: usize,
    pub h: Vec<u8>,
    pub strings: Vec<u8>,
    pub types: Vec<u8>,
}

pub const TYPE_SIZE: usize = 72;

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

impl Tdb {
    pub fn load(t: &Target, hdr: usize) -> Tdb {
        let h = t.read(hdr, 0x100);
        assert!(&h[..4] == b"TDB\0", "no TDB header at {hdr:X}");
        let n_types = u32_at(&h, 0x0C) as usize;
        let strings = t.read(u64_at(&h, 0xD0) as usize, u32_at(&h, 0x50) as usize);
        let types = t.read(u64_at(&h, 0x60) as usize, n_types * TYPE_SIZE);
        Tdb { hdr, h, strings, types }
    }

    pub fn string(&self, off: usize) -> &str {
        let s = &self.strings[off.min(self.strings.len())..];
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        std::str::from_utf8(&s[..end]).unwrap_or("?")
    }

    /// Offsets of `name` as a whole NUL-terminated string in the string pool.
    pub fn string_offsets(&self, name: &str) -> Vec<usize> {
        let n = name.as_bytes();
        memchr::memmem::find_iter(&self.strings, n)
            .filter(|&i| (i == 0 || self.strings[i - 1] == 0) && self.strings.get(i + n.len()) == Some(&0))
            .collect()
    }

    pub fn type_record(&self, i: usize) -> &[u8] {
        &self.types[i * TYPE_SIZE..(i + 1) * TYPE_SIZE]
    }

    pub fn count_types(&self) -> usize {
        self.types.len() / TYPE_SIZE
    }
}

/// Research command: where `name` sits in the string pool and which type records hold
/// that offset (as an aligned u32, or inside a u32 as the low bits of a bit field).
pub fn find(t: &Target, hdr: usize, name: &str) {
    let db = Tdb::load(t, hdr);
    let offs = db.string_offsets(name);
    println!("{} type records, string pool {} bytes; '{name}' at {:X?}", db.count_types(), db.strings.len(), offs);
    for &off in &offs {
        for i in 0..db.count_types() {
            let r = db.type_record(i);
            for pos in (0..TYPE_SIZE).step_by(4) {
                let v = u32_at(r, pos) as usize;
                // whole u32, or a 30-bit or smaller field in the low bits
                let hit = v == off || (v & 0x3FFF_FFFF) == off || (v & 0x3F_FFFF) == off;
                if hit {
                    let hex: Vec<String> = r.chunks(4).map(|c| format!("{:08X}", u32_at(c, 0))).collect();
                    println!("type #{i} pos +{pos:02X}: {}", hex.join(" "));
                }
            }
        }
    }
}

/// Research command: dump type records `from..from+count`, showing each u32 and, where it
/// is a valid string offset, the string.
pub fn dump(t: &Target, hdr: usize, from: usize, count: usize) {
    let db = Tdb::load(t, hdr);
    for i in from..(from + count).min(db.count_types()) {
        let r = db.type_record(i);
        let cells: Vec<String> = (0..TYPE_SIZE)
            .step_by(4)
            .map(|p| {
                let v = u32_at(r, p) as usize;
                let s = if v > 0 && v < db.strings.len() && db.strings[v - 1] == 0 { db.string(v) } else { "" };
                if !s.is_empty() && s.len() < 60 && s.is_ascii() { format!("{v:08X}[{s}]") } else { format!("{v:08X}") }
            })
            .collect();
        println!("#{i}: {}", cells.join(" "));
    }
}
fn looks_like_name(db: &Tdb, off: usize) -> bool {
    if off == 0 || off >= db.strings.len() || db.strings[off - 1] != 0 {
        return false;
    }
    let s = db.string(off);
    !s.is_empty() && s.len() < 200 && s.bytes().all(|c| c.is_ascii_graphic())
}

/// Research command: for every 64-bit window in a type record and every (shift, width),
/// the share of records whose field is the start of a name-like string.
pub fn stats(t: &Target, hdr: usize) {
    let db = Tdb::load(t, hdr);
    let n = db.count_types().min(20000);
    let mut best: Vec<(f32, usize, u32, u32)> = Vec::new();
    for pos in (0..TYPE_SIZE - 7).step_by(4) {
        for shift in 0..32u32 {
            for width in 16..=32u32 {
                let mask = if width == 64 { u64::MAX } else { (1u64 << width) - 1 };
                let mut ok = 0;
                let mut distinct = std::collections::HashSet::new();
                for i in (0..n).step_by(7) {
                    let v = ((u64_at(db.type_record(i), pos) >> shift) & mask) as usize;
                    if v != 0 && looks_like_name(&db, v) {
                        ok += 1;
                        distinct.insert(v);
                    }
                }
                let share = ok as f32 / ((n / 7).max(1)) as f32;
                if share > 0.5 && distinct.len() * 4 > n / 7 {
                    best.push((share, pos, shift, width));
                }
            }
        }
    }
    best.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    for (share, pos, shift, width) in best.iter().take(25) {
        let sample: Vec<&str> = (1..6).map(|i| db.string(((u64_at(db.type_record(i * 1000), *pos) >> shift) & ((1u64 << width) - 1)) as usize)).collect();
        println!("{:.3} pos +{pos:02X} shift {shift} width {width}: {:?}", share, sample);
    }
}
/// Research command: the same statistic for any table (`slot` = header offset of its
/// pointer, `size` = record size, `count` = records).
pub fn table_stats(t: &Target, hdr: usize, slot: usize, size: usize, count: usize) {
    let db = Tdb::load(t, hdr);
    let base = u64_at(&db.h, slot) as usize;
    let n = count.min(40000);
    let tab = t.read(base, n * size);
    let rec = |i: usize| &tab[i * size..(i + 1) * size];
    let mut best: Vec<(f32, usize, u32, u32, usize)> = Vec::new();
    for pos in (0..size.saturating_sub(7)).step_by(4).chain(std::iter::once(size.saturating_sub(8))) {
        for shift in 0..32u32 {
            for width in 16..=32u32 {
                let mask = (1u64 << width) - 1;
                let mut ok = 0;
                let mut distinct = std::collections::HashSet::new();
                let mut samples = 0;
                for i in (1..n).step_by(5) {
                    samples += 1;
                    let v = ((u64_at(rec(i), pos) >> shift) & mask) as usize;
                    if v != 0 && looks_like_name(&db, v) {
                        ok += 1;
                        distinct.insert(v);
                    }
                }
                let share = ok as f32 / samples.max(1) as f32;
                if share > 0.6 && distinct.len() * 5 > samples {
                    best.push((share, pos, shift, width, distinct.len()));
                }
            }
        }
    }
    best.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(a.3.cmp(&b.3)));
    for (share, pos, shift, width, d) in best.iter().take(12) {
        let sample: Vec<&str> = (1..7).map(|k| db.string(((u64_at(rec(k * n / 8), *pos) >> shift) & ((1u64 << width) - 1)) as usize)).collect();
        println!("{:.3} pos +{pos:02X} shift {shift} width {width} distinct {d}: {:?}", share, sample);
    }
}
pub const IMPL_SIZE: usize = 48;

/// Research command: type-impl records (table at hdr+0x68) whose name is `name`, with every
/// u32 decoded as a string where it looks like one.
pub fn impl_find(t: &Target, hdr: usize, name: &str) {
    let db = Tdb::load(t, hdr);
    let n = u32_at(&db.h, 0x18) as usize;
    let tab = t.read(u64_at(&db.h, 0x68) as usize, n * IMPL_SIZE);
    let offs = db.string_offsets(name);
    for i in 0..n {
        let r = &tab[i * IMPL_SIZE..(i + 1) * IMPL_SIZE];
        if offs.contains(&((u32_at(r, 0) & 0x3F_FFFF) as usize)) {
            let cells: Vec<String> = (0..IMPL_SIZE)
                .step_by(4)
                .map(|p| {
                    let v = u32_at(r, p) as usize;
                    let lo = v & 0x3F_FFFF;
                    if looks_like_name(&db, lo) { format!("{v:08X}[{}]", db.string(lo)) } else { format!("{v:08X}") }
                })
                .collect();
            println!("impl #{i}: {}", cells.join(" "));
        }
    }
}
/// Research command: bit fields of type records that equal impl index `a` in some record
/// and impl index `b` in another, at the same position, shift and width.
pub fn link(t: &Target, hdr: usize, a: u64, b: u64) {
    let db = Tdb::load(t, hdr);
    let n = db.count_types();
    for pos in (0..TYPE_SIZE - 7).step_by(4) {
        for shift in 0..32u32 {
            for width in 10..=24u32 {
                let mask = (1u64 << width) - 1;
                let (mut ha, mut hb) = (Vec::new(), Vec::new());
                for i in 0..n {
                    let v = (u64_at(db.type_record(i), pos) >> shift) & mask;
                    if v == a {
                        ha.push(i);
                    } else if v == b {
                        hb.push(i);
                    }
                    if ha.len() > 4 || hb.len() > 4 {
                        break;
                    }
                }
                if !ha.is_empty() && !hb.is_empty() && ha.len() <= 4 && hb.len() <= 4 {
                    println!("pos +{pos:02X} shift {shift} width {width}: {a} in {ha:?}, {b} in {hb:?}");
                }
            }
        }
    }
}
/// Research command: strings in the pool that contain `part`.
pub fn grep(t: &Target, hdr: usize, part: &str) {
    let db = Tdb::load(t, hdr);
    let mut n = 0;
    for i in memchr::memmem::find_iter(&db.strings, part.as_bytes()) {
        let start = db.strings[..i].iter().rposition(|&c| c == 0).map_or(0, |p| p + 1);
        println!("{start:X}: {}", db.string(start));
        n += 1;
        if n > 40 {
            break;
        }
    }
}
/// Header slots with table address and size (size = distance to the next table).
pub fn tables(db: &Tdb) -> Vec<(usize, usize, usize)> {
    let mut p: Vec<(usize, usize)> = (0x58..0x100).step_by(8).map(|o| (o, u64_at(&db.h, o) as usize)).collect();
    p.sort_by_key(|x| x.1);
    (0..p.len())
        .map(|i| {
            let size = p.get(i + 1).map_or(0x100_0000, |n| n.1 - p[i].1).min(0x400_0000);
            (p[i].0, p[i].1, size)
        })
        .collect()
}

/// Research command: every table position holding `value` as 3 little-endian bytes.
pub fn refs(t: &Target, hdr: usize, value: u32) {
    let db = Tdb::load(t, hdr);
    let pat = &value.to_le_bytes()[..3];
    for (slot, addr, size) in tables(&db) {
        let b = t.read(addr, size);
        for i in memchr::memmem::find_iter(&b, pat).take(8) {
            let from = i & !7;
            let hex: Vec<String> = b[from..(from + 24).min(b.len())].iter().map(|x| format!("{x:02X}")).collect();
            println!("hdr+{slot:02X} +{i:X} (byte {}): {}", i % 8, hex.join(" "));
        }
    }
}
pub const FIELD_SIZE: usize = 12;

/// Research command: field records (table hdr+0x88) `from..from+count` with names.
pub fn fields(t: &Target, hdr: usize, from: usize, count: usize) {
    let db = Tdb::load(t, hdr);
    let tab = t.read(u64_at(&db.h, 0x88) as usize + from * FIELD_SIZE, count * FIELD_SIZE);
    for k in 0..count {
        let r = &tab[k * FIELD_SIZE..(k + 1) * FIELD_SIZE];
        let (a, n, c) = (u32_at(r, 0), u32_at(r, 4), u32_at(r, 8));
        println!("field #{}: flags {a:08X} offset {n:>5X} name {c:08X}  {}", from + k, db.string((c & 0x3F_FFFF) as usize));
    }
}
fn bitfields_equal(r: &[u8], value: u64) -> Vec<(usize, u32, u32)> {
    let mut out = Vec::new();
    for pos in (0..TYPE_SIZE - 7).step_by(4) {
        for shift in 0..32u32 {
            for width in 10..=24u32 {
                if (u64_at(r, pos) >> shift) & ((1u64 << width) - 1) == value {
                    out.push((pos, shift, width));
                }
            }
        }
    }
    out
}

/// Research command: positions where type `ta` holds `va` and type `tb` holds `vb`.
pub fn pair(t: &Target, hdr: usize, ta: usize, va: u64, tb: usize, vb: u64) {
    let db = Tdb::load(t, hdr);
    let a = bitfields_equal(db.type_record(ta), va);
    let b = bitfields_equal(db.type_record(tb), vb);
    for x in a.iter().filter(|x| b.contains(x)) {
        println!("type #{ta}={va} and #{tb}={vb} at pos +{:02X} shift {} width {}", x.0, x.1, x.2);
    }
}
/// Research command: like `link`, for any table (`slot`, record `size`, `count`).
pub fn table_link(t: &Target, hdr: usize, slot: usize, size: usize, count: usize, a: u64, b: u64) {
    let db = Tdb::load(t, hdr);
    let tab = t.read(u64_at(&db.h, slot) as usize, count * size);
    let n = tab.len() / size;
    let last = size.saturating_sub(8);
    let mut positions: Vec<usize> = (0..=last).step_by(4).collect();
    if !positions.contains(&last) {
        positions.push(last);
    }
    for pos in positions {
        for shift in 0..32u32 {
            for width in 12..=24u32 {
                let mask = (1u64 << width) - 1;
                let (mut ha, mut hb) = (Vec::new(), Vec::new());
                for i in 0..n {
                    let v = (u64_at(&tab[i * size..], pos) >> shift) & mask;
                    if v == a {
                        ha.push(i);
                    } else if v == b {
                        hb.push(i);
                    }
                    if ha.len() > 3 || hb.len() > 3 {
                        break;
                    }
                }
                if !ha.is_empty() && !hb.is_empty() && ha.len() <= 3 && hb.len() <= 3 {
                    println!("pos +{pos:02X} shift {shift} width {width}: {a} in {ha:?}, {b} in {hb:?}");
                }
            }
        }
    }
}
/// Research command: a run of `len` consecutive records in table (slot,size) whose bit
/// field counts up from `start` (start, start+1, ...).
pub fn run(t: &Target, hdr: usize, slot: usize, size: usize, count: usize, start: u64, len: usize) {
    let db = Tdb::load(t, hdr);
    let tab = t.read(u64_at(&db.h, slot) as usize, count * size);
    let n = tab.len() / size;
    for pos in (0..=size - 8).step_by(4) {
        for shift in 0..32u32 {
            for width in 12..=24u32 {
                let mask = (1u64 << width) - 1;
                let f = |i: usize| (u64_at(&tab[i * size..], pos) >> shift) & mask;
                for i in 0..n.saturating_sub(len) {
                    if f(i) == start && (1..len).all(|k| f(i + k) == start + k as u64) {
                        let hex: Vec<String> = (0..len).map(|k| format!("{:016X}", u64_at(&tab[(i + k) * size..], 0))).collect();
                        println!("records {i}.. pos +{pos:02X} shift {shift} width {width}: {}", hex.join(" "));
                    }
                }
            }
        }
    }
}
/// Profile-writing command: fields whose name contains `part`, with the offset inside the
/// object (data offset + 0x10 object header) and neighbouring fields of the same group.
pub fn field_search(t: &Target, hdr: usize, part: &str, context: usize) {
    let db = Tdb::load(t, hdr);
    let n = u32_at(&db.h, 0x1C) as usize;
    let tab = t.read(u64_at(&db.h, 0x88) as usize, n * FIELD_SIZE);
    let name = |i: usize| db.string((u32_at(&tab[i * FIELD_SIZE..], 8) & 0x3F_FFFF) as usize);
    let off = |i: usize| u32_at(&tab[i * FIELD_SIZE..], 4);
    let mut shown = 0;
    for i in 0..n {
        if !name(i).contains(part) {
            continue;
        }
        println!("== field #{i}: +0x{:X} {}", off(i) + 0x10, name(i));
        for k in i.saturating_sub(context)..(i + context + 1).min(n) {
            let mark = if k == i { ">" } else { " " };
            println!("  {mark} #{k} +0x{:X} {}", off(k) + 0x10, name(k));
        }
        shown += 1;
        if shown >= 30 {
            break;
        }
    }
}
/// Research command: method-impl records (table hdr+0x78, 12 bytes) whose name contains
/// `part`, with neighbours.
pub fn methods(t: &Target, hdr: usize, part: &str, context: usize) {
    let db = Tdb::load(t, hdr);
    let n = u32_at(&db.h, 0x20) as usize;
    // records start 4 bytes into the table
    let tab = t.read(u64_at(&db.h, 0x78) as usize + 4, n * 12);
    let name = |i: usize| db.string((u32_at(&tab[i * 12..], 4) & 0x3F_FFFF) as usize);
    let mut shown = 0;
    for i in 0..n {
        if name(i) != part {
            continue;
        }
        println!("== method impl #{i}");
        for k in i.saturating_sub(context)..(i + context + 1).min(n) {
            let r = &tab[k * 12..k * 12 + 12];
            println!("  {} #{k} {:08X} {:08X} {:08X} {}", if k == i { ">" } else { " " }, u32_at(r, 0), u32_at(r, 4), u32_at(r, 8), name(k));
        }
        shown += 1;
        if shown >= 5 {
            break;
        }
    }
}
/// Full name ("namespace.Name") of type `ti`: type record +0x0C bits 6.. = impl index.
pub fn type_name(db: &Tdb, impls: &[u8], ti: usize) -> String {
    if ti >= db.count_types() {
        return "?".into();
    }
    let imp = ((u32_at(db.type_record(ti), 0x0C) >> 6) & 0x3FFFF) as usize;
    if (imp + 1) * IMPL_SIZE > impls.len() {
        return format!("type#{ti}");
    }
    let r = &impls[imp * IMPL_SIZE..];
    let ns = db.string((u32_at(r, 4) & 0x3F_FFFF) as usize);
    let n = db.string((u32_at(r, 0) & 0x3F_FFFF) as usize);
    if ns.is_empty() { n.to_string() } else { format!("{ns}.{n}") }
}

/// Research command: walk the native type registration array (0x28-byte records:
/// +0 static slot, +8 type index, +0xC size, +0x14 method count, +0x20 function table)
/// around `at`, printing records whose name contains `filter`.
pub fn registrations(t: &Target, hdr: usize, at: usize, filter: &str) {
    let db = Tdb::load(t, hdr);
    let impls = t.read(u64_at(&db.h, 0x68) as usize, u32_at(&db.h, 0x18) as usize * IMPL_SIZE);
    let (_, mb, ms) = t.mods[0].clone();
    let inmod = |p: u64| (p as usize) >= mb && (p as usize) < mb + ms;
    let span = 0x200_0000;
    let lo = at - span;
    let b = t.read(lo, span * 2);
    let ok = |o: usize| o + 0x28 <= b.len() && inmod(u64_at(&b, o)) && (u32_at(&b, o + 8) as usize) < db.count_types() && inmod(u64_at(&b, o + 0x20));
    let mut start = at - lo;
    while start >= 0x28 && ok(start - 0x28) {
        start -= 0x28;
    }
    let mut end = at - lo;
    while ok(end) {
        end += 0x28;
    }
    println!("{} records from {}", (end - start) / 0x28, t.name(lo + start));
    let mut o = start;
    while o < end {
        let ti = u32_at(&b, o + 8) as usize;
        let name = type_name(&db, &impls, ti);
        if name.contains(filter) {
            println!("  {} type #{ti} {name} size {:X} methods {} table {}", t.name(lo + o), u32_at(&b, o + 0xC), u32_at(&b, o + 0x14), t.name(u64_at(&b, o + 0x20) as usize));
        }
        o += 0x28;
    }
}
/// Research command: every registration record in the main module (strict check: the size
/// in the record equals the size in the type record), filtered by name.
pub fn registrations_all(t: &Target, hdr: usize, filter: &str) {
    let db = Tdb::load(t, hdr);
    let impls = t.read(u64_at(&db.h, 0x68) as usize, u32_at(&db.h, 0x18) as usize * IMPL_SIZE);
    let (_, mb, ms) = t.mods[0].clone();
    let inmod = |p: u64| (p as usize) >= mb && (p as usize) < mb + ms;
    let mut found = 0;
    let mut shown = 0;
    let chunk = 0x100_0000;
    let mut off = 0;
    while off < ms {
        let b = t.read(mb + off, (ms - off).min(chunk + 0x28));
        let mut o = 0;
        while o + 0x28 <= b.len() && o < chunk {
            let ti = u32_at(&b, o + 8) as usize;
            if ti > 0 && ti < db.count_types() && inmod(u64_at(&b, o)) && inmod(u64_at(&b, o + 0x20)) {
                let size = u32_at(&b, o + 0xC);
                if size != 0 && size == u32_at(db.type_record(ti), 0x14) {
                    found += 1;
                    let name = type_name(&db, &impls, ti);
                    if name.contains(filter) && shown < 40 {
                        shown += 1;
                        println!("  {} type #{ti} {name} size {size:X} methods {} table {}", t.name(mb + off + o), u32_at(&b, o + 0x14), t.name(u64_at(&b, o + 0x20) as usize));
                    }
                }
            }
            o += 8;
        }
        off += chunk;
    }
    println!("{found} registration records");
}
/// Research command: prefix sums of method counts (impl +0x18 low 16 bits) and of
/// (impl +0x14 low 16 bits), printed for the given impl indices.
pub fn prefix(t: &Target, hdr: usize, which: &[usize]) {
    let db = Tdb::load(t, hdr);
    let n = u32_at(&db.h, 0x18) as usize;
    let impls = t.read(u64_at(&db.h, 0x68) as usize, n * IMPL_SIZE);
    let (mut m, mut f) = (0usize, 0usize);
    for i in 0..n {
        let r = &impls[i * IMPL_SIZE..];
        if which.contains(&i) {
            println!("impl #{i} {}: methods before {m}, (+0x14 low) before {f}", db.string((u32_at(r, 0) & 0x3F_FFFF) as usize));
        }
        m += (u32_at(r, 0x18) & 0xFFFF) as usize;
        f += (u32_at(r, 0x14) & 0xFFFF) as usize;
    }
    println!("total methods {m} (+0x14) {f}");
}
/// Profile-writing command: class name of the managed object at `obj` (its first qword is the
/// vtable stored in its type record at +0x40), plus each pointer field's class.
pub fn object(t: &Target, hdr: usize, obj: usize, fields: usize) {
    let db = Tdb::load(t, hdr);
    let impls = t.read(u64_at(&db.h, 0x68) as usize, u32_at(&db.h, 0x18) as usize * IMPL_SIZE);
    let mut by_vt = std::collections::HashMap::new();
    for i in 0..db.count_types() {
        let vt = u64_at(db.type_record(i), 0x40);
        if vt != 0 {
            by_vt.entry(vt).or_insert(i);
        }
    }
    let class = |addr: usize| -> Option<String> {
        let b = t.read(addr, 8);
        if b.len() < 8 {
            return None;
        }
        by_vt.get(&u64_at(&b, 0)).map(|&ti| format!("{} (type #{ti}, size {:X})", type_name(&db, &impls, ti), u32_at(db.type_record(ti), 0x14)))
    };
    println!("{obj:X}: {}", class(obj).unwrap_or_else(|| "unknown".into()));
    let b = t.read(obj, fields);
    for o in (0x10..b.len().saturating_sub(7)).step_by(8) {
        let v = u64_at(&b, o) as usize;
        if let Some(c) = (v > 0x10000).then(|| class(v)).flatten() {
            println!("  +{o:X} -> {v:X} {c}");
        }
    }
}
/// Profile-writing command: live heap objects of every class whose full name contains `part`
/// (object's first qword == the class vtable from its type record).
pub fn instances(t: &Target, hdr: usize, part: &str, max: usize) {
    let db = Tdb::load(t, hdr);
    let impls = t.read(u64_at(&db.h, 0x68) as usize, u32_at(&db.h, 0x18) as usize * IMPL_SIZE);
    let mut want = std::collections::HashMap::new();
    for i in 0..db.count_types() {
        let vt = u64_at(db.type_record(i), 0x40);
        if vt != 0 {
            let n = type_name(&db, &impls, i);
            if n.contains(part) {
                want.insert(vt, (n, u32_at(db.type_record(i), 0x14)));
            }
        }
    }
    let mut found: std::collections::BTreeMap<String, Vec<usize>> = Default::default();
    for (base, size) in crate::heap_regions(t) {
        let b = t.read(base, size);
        let mut i = 0;
        while i + 8 <= b.len() {
            if let Some((n, sz)) = want.get(&u64_at(&b, i)) {
                found.entry(format!("{n} size {sz:X}")).or_default().push(base + i);
            }
            i += 16;
        }
    }
    for (n, list) in &found {
        let shown: Vec<String> = list.iter().take(max).map(|a| format!("{a:X}")).collect();
        println!("{n}: {} object(s) {}", list.len(), shown.join(" "));
    }
}