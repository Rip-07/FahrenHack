//! re-probe: research helper for writing game profiles.
//!
//!   re-probe watch <pid> <addr> [w|rw] [seconds] [len]  which instructions write (or access) an address
//!   re-probe dis   <pid> <addr> [count]                 disassemble from an address
//!   re-probe read  <pid> <addr> [len]                   hex dump
//!
//! `watch` attaches as a debugger for the given time, puts a hardware breakpoint on every
//! thread, reports each instruction that touched the address, then detaches without
//! killing the game. It never patches anything. A game with anti-debug protection may
//! notice it; nothing here tries to hide from that.

mod tdb;

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::time::{Duration, Instant};

use iced_x86::{Decoder, DecoderOptions, Formatter, Instruction, IntelFormatter};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::Debug::{
    ContinueDebugEvent, DebugActiveProcess, DebugActiveProcessStop, DebugSetProcessKillOnExit, GetThreadContext,
    ReadProcessMemory, SetThreadContext, WriteProcessMemory, WaitForDebugEvent, CONTEXT, DEBUG_EVENT,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Thread32First, Thread32Next, MODULEENTRY32W,
    TH32CS_SNAPMODULE, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::Memory::{VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_PRIVATE};
use windows_sys::Win32::System::Threading::{OpenProcess, OpenThread, ResumeThread, SuspendThread};

const DBG_CONTINUE: i32 = 0x0001_0002;
const DBG_EXCEPTION_NOT_HANDLED: i32 = 0x8001_0001u32 as i32;
const EXCEPTION_SINGLE_STEP: i32 = 0x8000_0004u32 as i32;
const EXCEPTION_BREAKPOINT: i32 = 0x8000_0003u32 as i32;
const CTX_DEBUG_CONTROL_INTEGER: u32 = 0x0010_0013;
const THREAD_ACCESS: u32 = 0x0002 | 0x0008 | 0x0010 | 0x0040; // suspend/resume, get/set context, query
const PROCESS_VM_READ_QUERY: u32 = 0x0010 | 0x0400;

#[repr(C, align(16))]
struct Ctx(CONTEXT);

fn new_ctx(flags: u32) -> Box<Ctx> {
    let mut c: Box<Ctx> = Box::new(unsafe { zeroed() });
    c.0.ContextFlags = flags;
    c
}

fn parse_hex(s: &str) -> usize {
    usize::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).expect("hex address")
}

pub struct Target {
    h: HANDLE,
    mods: Vec<(String, usize, usize)>,
}

impl Target {
    fn open(pid: u32) -> Target {
        let h = unsafe { OpenProcess(PROCESS_VM_READ_QUERY, 0, pid) };
        assert!(!h.is_null(), "OpenProcess failed (run as administrator?)");
        Target { h, mods: modules(pid) }
    }

    pub fn read(&self, addr: usize, len: usize) -> Vec<u8> {
        let mut b = vec![0u8; len];
        let mut n = 0usize;
        unsafe { ReadProcessMemory(self.h, addr as *const c_void, b.as_mut_ptr().cast(), len, &mut n) };
        b.truncate(n);
        b
    }

    fn name(&self, addr: usize) -> String {
        match self.mods.iter().find(|(_, b, s)| addr >= *b && addr < b + s) {
            Some((n, b, _)) => format!("{n}+{:X}", addr - b),
            None => format!("{addr:X}"),
        }
    }

    fn fmt_ins(&self, ins: &Instruction) -> String {
        let mut out = String::new();
        let mut f = IntelFormatter::new();
        f.options_mut().set_hex_prefix("0x");
        f.options_mut().set_hex_suffix("");
        f.format(ins, &mut out);
        let bytes = self.read(ins.ip() as usize, ins.len());
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
        format!("{:<22} {:<36} {}", self.name(ins.ip() as usize), hex.join(" "), out)
    }

    fn disasm(&self, addr: usize, count: usize) {
        let code = self.read(addr, count * 15 + 16);
        let mut d = Decoder::with_ip(64, &code, addr as u64, DecoderOptions::NONE);
        for _ in 0..count {
            if !d.can_decode() {
                break;
            }
            let ins = d.decode();
            println!("  {}", self.fmt_ins(&ins));
        }
    }

    /// Instructions that end exactly at `end` (a data breakpoint traps after the access).
    fn ending_at(&self, end: usize) -> Vec<Instruction> {
        let code = self.read(end - 15, 15);
        let mut out = Vec::new();
        for start in 0..15 {
            let mut d = Decoder::with_ip(64, &code[start..], (end - 15 + start) as u64, DecoderOptions::NONE);
            let ins = d.decode();
            if !ins.is_invalid() && ins.next_ip() as usize == end {
                out.push(ins);
            }
        }
        out
    }
}

fn modules(pid: u32) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, pid);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: MODULEENTRY32W = zeroed();
        e.dwSize = size_of::<MODULEENTRY32W>() as u32;
        if Module32FirstW(snap, &mut e) != 0 {
            loop {
                let len = e.szModule.iter().position(|&c| c == 0).unwrap_or(0);
                out.push((String::from_utf16_lossy(&e.szModule[..len]), e.modBaseAddr as usize, e.modBaseSize as usize));
                if Module32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

fn threads(pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        let mut e: THREADENTRY32 = zeroed();
        e.dwSize = size_of::<THREADENTRY32>() as u32;
        if Thread32First(snap, &mut e) != 0 {
            loop {
                if e.th32OwnerProcessID == pid {
                    out.push(e.th32ThreadID);
                }
                if Thread32Next(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

/// Sets (dr7 != 0) or clears the hardware breakpoints in slots 0..addrs.len() of one thread.
fn set_bp(tid: u32, addrs: &[usize], dr7: u64) -> bool {
    unsafe {
        let h = OpenThread(THREAD_ACCESS, 0, tid);
        if h.is_null() {
            return false;
        }
        SuspendThread(h);
        let mut c = new_ctx(CTX_DEBUG_CONTROL_INTEGER);
        let ok = GetThreadContext(h, &mut c.0) != 0 && {
            let a = |i: usize| addrs.get(i).copied().unwrap_or(0) as u64;
            c.0.Dr0 = a(0);
            c.0.Dr1 = a(1);
            c.0.Dr2 = a(2);
            c.0.Dr3 = a(3);
            c.0.Dr6 = 0;
            c.0.Dr7 = (c.0.Dr7 & !0xFFFF_00FF) | dr7;
            SetThreadContext(h, &c.0) != 0
        };
        ResumeThread(h);
        CloseHandle(h);
        ok
    }
}

struct Hit {
    count: u32,
    regs: String,
}

fn watch(pid: u32, addrs: &[usize], mode: &str, seconds: u64, len: usize) {
    let t = Target::open(pid);
    assert!(!addrs.is_empty() && addrs.len() <= 4, "1 to 4 addresses");
    let rw: u64 = if mode == "rw" { 0b11 } else { 0b01 };
    let ln: u64 = if len == 8 { 0b10 } else if len == 2 { 0b01 } else if len == 1 { 0b00 } else { 0b11 };
    let mut dr7 = 0u64;
    for i in 0..addrs.len() {
        dr7 |= (1 << (2 * i)) | (rw << (16 + 4 * i)) | (ln << (18 + 4 * i));
    }
    let list: Vec<String> = addrs.iter().map(|a| format!("{a:X}")).collect();
    println!("watch {} for {seconds} s, {} access, {len} bytes", list.join(","), if rw == 1 { "write" } else { "read/write" });
    let main_mod = t.mods.first().map(|m| (m.1, m.1 + m.2)).unwrap_or((0, 0));
    unsafe {
        assert!(DebugActiveProcess(pid) != 0, "DebugActiveProcess failed (run as administrator?)");
        DebugSetProcessKillOnExit(0);
    }
    let mut hits: BTreeMap<usize, Hit> = BTreeMap::new();
    let mut armed = false;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut ev: DEBUG_EVENT = unsafe { zeroed() };
    while Instant::now() < deadline {
        if unsafe { WaitForDebugEvent(&mut ev, 100) } == 0 {
            continue;
        }
        let mut status = DBG_CONTINUE;
        unsafe {
            match ev.dwDebugEventCode {
                1 => {
                    let rec = &ev.u.Exception.ExceptionRecord;
                    if rec.ExceptionCode == EXCEPTION_SINGLE_STEP {
                        let h = OpenThread(THREAD_ACCESS, 0, ev.dwThreadId);
                        let mut c = new_ctx(CTX_DEBUG_CONTROL_INTEGER);
                        if !h.is_null() && GetThreadContext(h, &mut c.0) != 0 {
                            if c.0.Dr6 & 0xF != 0 {
                                let rip = c.0.Rip as usize;
                                let e = hits.entry(rip).or_insert(Hit { count: 0, regs: String::new() });
                                e.count += 1;
                                if e.regs.is_empty() {
                                    let r = &c.0;
                                    let slot = (c.0.Dr6 & 0xF).trailing_zeros() as usize;
                                    let stack = t.read(c.0.Rsp as usize, 0x400);
                                    let callers: Vec<String> = stack
                                        .chunks_exact(8)
                                        .map(|q| u64::from_le_bytes(q.try_into().unwrap()) as usize)
                                        .filter(|&v| v >= main_mod.0 && v < main_mod.1)
                                        .take(12)
                                        .map(|v| t.name(v))
                                        .collect();
                                    let val = t.read(addrs[slot], 8);
                                    e.regs = format!(
                                        "slot {slot} ({:X}) now {:02X?}\n      stack refs: {}\n      rax={:X} rbx={:X} rcx={:X} rdx={:X} rsi={:X} rdi={:X} rbp={:X} rsp={:X}\n      r8={:X} r9={:X} r10={:X} r11={:X} r12={:X} r13={:X} r14={:X} r15={:X}",
                                        addrs[slot], val, callers.join(" "), r.Rax, r.Rbx, r.Rcx, r.Rdx, r.Rsi, r.Rdi, r.Rbp, r.Rsp, r.R8, r.R9, r.R10, r.R11, r.R12, r.R13, r.R14, r.R15
                                    );
                                }
                                c.0.Dr6 = 0;
                                SetThreadContext(h, &c.0);
                            } else {
                                status = DBG_EXCEPTION_NOT_HANDLED;
                            }
                        }
                        if !h.is_null() {
                            CloseHandle(h);
                        }
                    } else if rec.ExceptionCode == EXCEPTION_BREAKPOINT && !armed {
                        // The attach breakpoint: the process is stopped, arm every thread now.
                        let n = threads(pid).into_iter().filter(|&tid| set_bp(tid, addrs, dr7)).count();
                        println!("armed {n} threads");
                        armed = true;
                    } else {
                        status = DBG_EXCEPTION_NOT_HANDLED;
                    }
                }
                2 => {
                    if armed {
                        set_bp(ev.dwThreadId, addrs, dr7);
                    }
                }
                3 => {
                    let f = ev.u.CreateProcessInfo.hFile;
                    if !f.is_null() {
                        CloseHandle(f);
                    }
                }
                5 => {
                    println!("the game exited");
                    return;
                }
                6 => {
                    let f = ev.u.LoadDll.hFile;
                    if !f.is_null() {
                        CloseHandle(f);
                    }
                }
                _ => {}
            }
            ContinueDebugEvent(ev.dwProcessId, ev.dwThreadId, status);
        }
    }
    for tid in threads(pid) {
        set_bp(tid, &[], 0);
    }
    // Drain events that arrived while disarming, then detach.
    unsafe {
        while WaitForDebugEvent(&mut ev, 50) != 0 {
            ContinueDebugEvent(ev.dwProcessId, ev.dwThreadId, DBG_CONTINUE);
        }
        DebugActiveProcessStop(pid);
    }
    println!("detached; {} distinct instruction(s)", hits.len());
    for (rip, h) in &hits {
        println!("\n== {} hits, next instruction at {}", h.count, t.name(*rip));
        for ins in t.ending_at(*rip) {
            println!("  candidate: {}", t.fmt_ins(&ins));
        }
        println!("      {}", h.regs);
        println!("  context:");
        t.disasm(rip.saturating_sub(0x30), 1);
        t.disasm(*rip, 8);
    }
}

/// Committed private read/write regions (the game's heap).
pub fn heap_regions(t: &Target) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut addr = 0usize;
    loop {
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
        if unsafe { VirtualQueryEx(t.h, addr as *const c_void, &mut mbi, size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
            break;
        }
        let rw = mbi.Protect & 0x04 != 0 && mbi.Protect & 0x100 == 0;
        if mbi.State == MEM_COMMIT && mbi.Type == MEM_PRIVATE && rw {
            out.push((mbi.BaseAddress as usize, mbi.RegionSize));
        }
        addr = mbi.BaseAddress as usize + mbi.RegionSize;
    }
    out
}

/// Candidate list: (address, last value) pairs, little-endian u64 + i32.
fn save_list(path: &str, list: &[(usize, i32)]) {
    let mut b = Vec::with_capacity(list.len() * 12);
    for (a, v) in list {
        b.extend_from_slice(&(*a as u64).to_le_bytes());
        b.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, b).expect("write list");
    eprintln!("{} candidate(s)", list.len());
    for (a, v) in list.iter().take(30) {
        if f32_mode() {
            println!("{a:X} = {}", f32::from_bits(*v as u32));
        } else {
            println!("{a:X} = {v}");
        }
    }
}

fn load_list(path: &str) -> Vec<(usize, i32)> {
    let b = std::fs::read(path).expect("read list");
    b.chunks_exact(12)
        .map(|c| (u64::from_le_bytes(c[..8].try_into().unwrap()) as usize, i32::from_le_bytes(c[8..].try_into().unwrap())))
        .collect()
}

/// `RP_F32=1` makes snap/filt treat the 4 bytes as f32 (stored as raw bits).
fn f32_mode() -> bool {
    std::env::var("RP_F32").is_ok_and(|v| v == "1")
}

fn keep_mode(mode: &str, old: i32, new: i32) -> bool {
    if f32_mode() {
        let (o, n) = (f32::from_bits(old as u32), f32::from_bits(new as u32));
        return match mode {
            "dec" => n < o,
            "inc" => n > o,
            "same" => n == o,
            "changed" => n != o,
            "plausible" => n.is_finite() && (0.5..=100_000.0).contains(&n),
            m => m.parse::<f32>().map(|v| (v - n).abs() < 0.5).expect("mode"),
        };
    }
    match mode {
        "dec" => new < old,
        "inc" => new > old,
        "same" => new == old,
        "changed" => new != old,
        // Drop values that left the plausible range.
        "plausible" => (1..=100_000).contains(&new),
        m => m.parse::<i32>().map(|v| v == new).expect("dec|inc|same|changed|<value>"),
    }
}

/// `snap`: candidates = every aligned i32 in [lo, hi] in the heap.
fn snap(pid: u32, out: &str, lo: i32, hi: i32) {
    let t = Target::open(pid);
    let mut list = Vec::new();
    for (base, size) in heap_regions(&t) {
        let b = t.read(base, size);
        for (i, c) in b.chunks_exact(4).enumerate() {
            let v = i32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            let ok = if f32_mode() {
                let x = f32::from_bits(v as u32);
                x.is_finite() && x >= lo as f32 && x <= hi as f32
            } else {
                v >= lo && v <= hi
            };
            if ok {
                list.push((base + i * 4, v));
            }
        }
    }
    save_list(out, &list);
}

/// `filt`: keep candidates whose value changed as `mode` says; stores the new values.
fn filt(pid: u32, file: &str, mode: &str) {
    let t = Target::open(pid);
    let list = load_list(file);
    let mut out = Vec::new();
    // Read page by page: candidates are sorted by address.
    let mut page_addr = usize::MAX;
    let mut page = Vec::new();
    for (a, old) in list {
        if mode == "plausible" && a < 0x100_0000_0000 {
            continue; // thread stacks live low in this game; keep heap only
        }
        let p = a & !0xFFF;
        if p != page_addr {
            page_addr = p;
            page = t.read(p, 0x1000);
        }
        let o = a - p;
        if page.len() >= o + 4 {
            let new = i32::from_le_bytes(page[o..o + 4].try_into().unwrap());
            if keep_mode(mode, old, new) {
                out.push((a, new));
            }
        }
    }
    save_list(file, &out);
}

/// Every 4-byte-aligned i32 equal to `value` in committed private read/write memory.
fn find(pid: u32, value: i32) {
    let t = Target::open(pid);
    let mut addr = 0usize;
    let mut found = 0;
    loop {
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
        if unsafe { VirtualQueryEx(t.h, addr as *const c_void, &mut mbi, size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
            break;
        }
        let base = mbi.BaseAddress as usize;
        let size = mbi.RegionSize;
        let rw = mbi.Protect & 0x04 != 0 && mbi.Protect & 0x100 == 0; // PAGE_READWRITE, not PAGE_GUARD
        if mbi.State == MEM_COMMIT && mbi.Type == MEM_PRIVATE && rw {
            let b = t.read(base, size);
            for (i, c) in b.chunks_exact(4).enumerate() {
                if i32::from_le_bytes([c[0], c[1], c[2], c[3]]) == value {
                    println!("{:X}", base + i * 4);
                    found += 1;
                }
            }
        }
        addr = base + size;
    }
    eprintln!("{found} match(es)");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pid: u32 = a.get(2).and_then(|s| s.parse().ok()).expect("pid");
    match a.get(1).map(String::as_str) {
        Some("watch") => watch(
            pid,
            &a[3].split(',').map(parse_hex).collect::<Vec<_>>(),
            a.get(4).map_or("w", String::as_str),
            a.get(5).and_then(|s| s.parse().ok()).unwrap_or(20),
            a.get(6).and_then(|s| s.parse().ok()).unwrap_or(4),
        ),
        Some("dis") => {
            let t = Target::open(pid);
            t.disasm(parse_hex(&a[3]), a.get(4).and_then(|s| s.parse().ok()).unwrap_or(30));
        }
        Some("read") => {
            let t = Target::open(pid);
            let addr = parse_hex(&a[3]);
            let b = t.read(addr, a.get(4).and_then(|s| s.parse().ok()).unwrap_or(64));
            for (i, row) in b.chunks(16).enumerate() {
                let hex: Vec<String> = row.iter().map(|x| format!("{x:02X}")).collect();
                println!("{:X}  {}", addr + i * 16, hex.join(" "));
            }
        }
        Some("write") => {
            let addr = parse_hex(&a[3]);
            let v: i32 = a[4].parse().expect("i32 value");
            let h = unsafe { OpenProcess(0x0008 | 0x0020 | 0x0400, 0, pid) };
            let mut n = 0usize;
            let ok = unsafe { WriteProcessMemory(h, addr as *const c_void, v.to_le_bytes().as_ptr().cast(), 4, &mut n) };
            println!("write {v} to {addr:X}: {}", if ok != 0 { "ok" } else { "failed" });
        }
        Some("aob") => {
            // Every match of a signature (`??` = any byte) in the main module.
            let t = Target::open(pid);
            let pat: Vec<Option<u8>> = a[3].split_whitespace().map(|x| u8::from_str_radix(x, 16).ok()).collect();
            let (_, base, size) = t.mods[0].clone();
            let mut hits = 0;
            let mut off = 0;
            while off < size {
                let len = (size - off).min(16 << 20);
                let b = t.read(base + off, len + pat.len());
                for i in 0..b.len().saturating_sub(pat.len()) {
                    if pat.iter().enumerate().all(|(j, p)| p.map_or(true, |x| b[i + j] == x)) {
                        println!("{}", t.name(base + off + i));
                        hits += 1;
                    }
                }
                off += len;
            }
            println!("{hits} match(es)");
        }
        Some("ptr") => {
            // Where the qword a[3] is stored: in the main module (statics) and in the heap.
            let t = Target::open(pid);
            let want = parse_hex(&a[3]) as u64;
            let range = a.get(4).map(|s| parse_hex(s) as u64).unwrap_or(0);
            let (_, mb, ms) = t.mods[0].clone();
            let mut regions = heap_regions(&t);
            let mut addr = mb;
            while addr < mb + ms {
                let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
                if unsafe { VirtualQueryEx(t.h, addr as *const c_void, &mut mbi, size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
                    break;
                }
                if mbi.State == MEM_COMMIT && mbi.Protect & 0x66 != 0 && mbi.Protect & 0x100 == 0 {
                    regions.push((mbi.BaseAddress as usize, mbi.RegionSize));
                }
                addr = mbi.BaseAddress as usize + mbi.RegionSize;
            }
            let mut n = 0;
            for (base, size) in regions {
                let b = t.read(base, size);
                for (i, c) in b.chunks_exact(8).enumerate() {
                    let v = u64::from_le_bytes(c.try_into().unwrap());
                    if v >= want.saturating_sub(range) && v <= want {
                        let at = base + i * 8;
                        let tag = if at >= mb && at < mb + ms { "STATIC" } else { "heap" };
                        println!("{tag} {} -> {v:X} (+{:X})", t.name(at), want - v);
                        n += 1;
                    }
                }
            }
            eprintln!("{n} reference(s)");
        }
        Some("xref") => {
            // `mov r64,[rip+disp]` instructions in the main module that load the static at module+a[3].
            let t = Target::open(pid);
            let (_, mb, ms) = t.mods[0].clone();
            let targets: Vec<usize> = a[3].split(',').map(|x| mb + parse_hex(x)).collect();
            let mut off = 0;
            while off < ms {
                let len = (ms - off).min(16 << 20);
                let b = t.read(mb + off, len + 8);
                for i in 0..b.len().saturating_sub(7) {
                    let rex = b[i];
                    if (rex == 0x48 || rex == 0x4C) && b[i + 1] == 0x8B && b[i + 2] & 0xC7 == 0x05 {
                        let d = i32::from_le_bytes(b[i + 3..i + 7].try_into().unwrap()) as i64;
                        let at = mb + off + i;
                        let tgt = (at as i64 + 7 + d) as usize;
                        if targets.contains(&tgt) {
                            let hex: Vec<String> = b[i..(i + 20).min(b.len())].iter().map(|x| format!("{x:02X}")).collect();
                            println!("{} -> {}  {}", t.name(at), t.name(tgt), hex.join(" "));
                        }
                    }
                }
                off += len;
            }
        }
        Some("objs") => {
            // Heap objects whose first qword (type) equals a[3]; prints the first a[4] bytes as i32s.
            let t = Target::open(pid);
            let ty = parse_hex(&a[3]) as u64;
            let n: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(0x60);
            for (base, size) in heap_regions(&t) {
                let b = t.read(base, size);
                let mut i = 0;
                while i + n <= b.len() {
                    if u64::from_le_bytes(b[i..i + 8].try_into().unwrap()) == ty {
                        let vals: Vec<String> = b[i + 8..i + n]
                            .chunks_exact(4)
                            .map(|c| i32::from_le_bytes(c.try_into().unwrap()).to_string())
                            .collect();
                        println!("{:X}: {}", base + i, vals.join(" "));
                    }
                    i += 16;
                }
            }
        }
        Some("str") => {
            // Every occurrence of an ASCII string (a[3]) in readable committed memory.
            let t = Target::open(pid);
            // "hex:54 44 42 00" searches raw bytes instead of text.
            let owned: Vec<u8> = match a[3].strip_prefix("hex:") {
                Some(h) => h.split_whitespace().map(|x| u8::from_str_radix(x, 16).expect("hex byte")).collect(),
                None => a[3].as_bytes().to_vec(),
            };
            let needle = &owned[..];
            let max: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(40);
            // Regions first (to report progress), then a SIMD substring search per 64 MB chunk.
            let mut regions = Vec::new();
            let mut addr = 0usize;
            loop {
                let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
                if unsafe { VirtualQueryEx(t.h, addr as *const c_void, &mut mbi, size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
                    break;
                }
                let readable = mbi.Protect & 0xEE != 0 && mbi.Protect & 0x100 == 0;
                if mbi.State == MEM_COMMIT && readable {
                    regions.push((mbi.BaseAddress as usize, mbi.RegionSize));
                }
                addr = mbi.BaseAddress as usize + mbi.RegionSize;
            }
            let total: usize = regions.iter().map(|r| r.1).sum();
            let finder = memchr::memmem::Finder::new(needle);
            let started = Instant::now();
            let (mut done, mut n, mut last) = (0usize, 0, Instant::now());
            const CHUNK: usize = 64 << 20;
            'outer: for (base, size) in regions {
                let mut off = 0;
                while off < size {
                    let len = (size - off).min(CHUNK + needle.len() - 1);
                    let b = t.read(base + off, len);
                    for i in finder.find_iter(&b) {
                        let ctx: String = b[i..(i + 64).min(b.len())]
                            .iter()
                            .map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { '.' })
                            .collect();
                        println!("{} {}", t.name(base + off + i), ctx);
                        n += 1;
                        if n >= max {
                            break 'outer;
                        }
                    }
                    off += CHUNK;
                    done += len.min(CHUNK);
                    if last.elapsed() > Duration::from_secs(5) {
                        eprintln!("... {}% of {} MB", done * 100 / total.max(1), total >> 20);
                        last = Instant::now();
                    }
                }
            }
            eprintln!("{n} match(es) in {:.0} s", started.elapsed().as_secs_f32());
        }
        Some("tdbdump") => tdb::dump(&Target::open(pid), parse_hex(&a[3]), a[4].parse().unwrap(), a[5].parse().unwrap()),
        Some("tdbstats") => tdb::stats(&Target::open(pid), parse_hex(&a[3])),
        Some("tdbtab") => tdb::table_stats(&Target::open(pid), parse_hex(&a[3]), parse_hex(&a[4]), a[5].parse().unwrap(), a[6].parse().unwrap()),
        Some("tdbimpl") => tdb::impl_find(&Target::open(pid), parse_hex(&a[3]), &a[4]),
        Some("tdblink") => tdb::link(&Target::open(pid), parse_hex(&a[3]), a[4].parse().unwrap(), a[5].parse().unwrap()),
        Some("qfind") => {
            // Aligned qword a[5] inside [a[3], a[3]+a[4]): prints index (offset/8).
            let t = Target::open(pid);
            let (base, len, want) = (parse_hex(&a[3]), parse_hex(&a[4]), parse_hex(&a[5]) as u64);
            let b = t.read(base, len);
            for (i, c) in b.chunks_exact(8).enumerate() {
                if u64::from_le_bytes(c.try_into().unwrap()) == want {
                    println!("{:X} index {i} (offset {:X})", base + i * 8, i * 8);
                }
            }
        }
        Some("tdbgrep") => tdb::grep(&Target::open(pid), parse_hex(&a[3]), &a[4]),
        Some("tdbref") => tdb::refs(&Target::open(pid), parse_hex(&a[3]), parse_hex(&a[4]) as u32),
        Some("tdbfields") => tdb::fields(&Target::open(pid), parse_hex(&a[3]), a[4].parse().unwrap(), a[5].parse().unwrap()),
        Some("tdbpair") => tdb::pair(&Target::open(pid), parse_hex(&a[3]), a[4].parse().unwrap(), a[5].parse().unwrap(), a[6].parse().unwrap(), a[7].parse().unwrap()),
        Some("tdbtlink") => tdb::table_link(&Target::open(pid), parse_hex(&a[3]), parse_hex(&a[4]), a[5].parse().unwrap(), a[6].parse().unwrap(), a[7].parse().unwrap(), a[8].parse().unwrap()),
        Some("tdbrun") => tdb::run(&Target::open(pid), parse_hex(&a[3]), parse_hex(&a[4]), a[5].parse().unwrap(), a[6].parse().unwrap(), a[7].parse().unwrap(), a[8].parse().unwrap()),
        Some("tdbfield") => tdb::field_search(&Target::open(pid), parse_hex(&a[3]), &a[4], a.get(5).and_then(|s| s.parse().ok()).unwrap_or(3)),
        Some("slots") => {
            // Research: managed array of slot objects at a[3]: count at +0x1C, elements from +0x20;
            // slot +0x10 -> item; item +0x28 id, +0x30 durability, +0x44 count.
            let t = Target::open(pid);
            let arr = parse_hex(&a[3]);
            let hdr = t.read(arr, 0x20);
            let n = u32::from_le_bytes(hdr[0x1C..0x20].try_into().unwrap()) as usize;
            let els = t.read(arr + 0x20, n * 8);
            let q = |b: &[u8], o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap()) as usize;
            let d = |b: &[u8], o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
            for i in 0..n {
                let slot = q(&els, i * 8);
                if slot == 0 {
                    continue;
                }
                let s = t.read(slot, 0x30);
                if s.len() < 0x30 {
                    continue;
                }
                let item = q(&s, 0x10);
                if item == 0 {
                    continue;
                }
                let it = t.read(item, 0x50);
                if it.len() < 0x50 {
                    continue;
                }
                println!("slot[{i}] {slot:X} item {item:X} id {:08X} dur {} count {} | slot +18 {} +1C {} +28 {}", d(&it, 0x28), d(&it, 0x30), d(&it, 0x44), d(&s, 0x18), d(&s, 0x1C), d(&s, 0x28));
            }
        }
        Some("dispfind") => {
            // Code in the main module that writes memory at displacement a[3] and, within a[5]
            // bytes, at displacement a[4] (e.g. two neighbouring fields of one object).
            let t = Target::open(pid);
            let (d1, d2, win) = (parse_hex(&a[3]) as i64, parse_hex(&a[4]) as i64, parse_hex(&a[5]));
            let (_, mb, ms) = t.mods[0].clone();
            let code = t.read(mb, ms);
            let mut d = Decoder::with_ip(64, &code, mb as u64, DecoderOptions::NONE);
            let mut w1: Vec<u64> = Vec::new();
            let mut w2: Vec<u64> = Vec::new();
            let mut ins = Instruction::default();
            while d.can_decode() {
                d.decode_out(&mut ins);
                if ins.is_invalid() || ins.op0_kind() != iced_x86::OpKind::Memory {
                    continue;
                }
                let disp = ins.memory_displacement64() as i64;
                let base = ins.memory_base();
                if base == iced_x86::Register::RSP || base == iced_x86::Register::RBP || base == iced_x86::Register::None {
                    continue;
                }
                use iced_x86::Code::*;
                let ptr_store = matches!(ins.code(), Mov_rm64_r64);
                let float_store = matches!(ins.code(), Movss_xmmm32_xmm | VEX_Vmovss_m32_xmm | Mov_rm32_imm32);
                if disp == d1 && ptr_store {
                    w1.push(ins.ip());
                } else if disp == d2 && float_store {
                    w2.push(ins.ip());
                }
            }
            let mut n = 0;
            for &a1 in &w1 {
                if let Some(&a2) = w2.iter().find(|&&x| x.abs_diff(a1) <= win as u64) {
                    let from = a1.min(a2) as usize;
                    println!("== {} / {}", t.name(a1 as usize), t.name(a2 as usize));
                    t.disasm(from.saturating_sub(0x10), 10);
                    n += 1;
                    if n >= 25 {
                        break;
                    }
                }
            }
            eprintln!("{} writes at {d1:X}, {} at {d2:X}, {n} pairs", w1.len(), w2.len());
        }
        Some("recoilscan") => {
            // Research: heap objects shaped like CameraRecoilParam (see tdbfield): four small
            // floats at +0x18..+0x24, a pointer at +0x28, small positive floats at +0x30/+0x34.
            // Groups them by their type pointer (+0).
            let t = Target::open(pid);
            let small = |v: f32| v.is_finite() && v.abs() <= 45.0;
            let mut groups: BTreeMap<u64, Vec<(usize, [f32; 6])>> = BTreeMap::new();
            for (base, size) in heap_regions(&t) {
                let b = t.read(base, size);
                let mut i = 0;
                while i + 0x38 <= b.len() {
                    let q = |o: usize| u64::from_le_bytes(b[i + o..i + o + 8].try_into().unwrap());
                    let f = |o: usize| f32::from_le_bytes(b[i + o..i + o + 4].try_into().unwrap());
                    let ty = q(0);
                    let ptr = q(0x28);
                    let heapish = |p: u64| p > 0x100_0000_0000 && p < 0x7000_0000_0000 && p % 8 == 0;
                    if heapish(ty) && heapish(ptr) {
                        let v = [f(0x18), f(0x1C), f(0x20), f(0x24), f(0x30), f(0x34)];
                        let nonzero = v[..4].iter().filter(|x| **x != 0.0).count();
                        if v[..4].iter().all(|x| small(*x)) && nonzero >= 2 && v[4] > 0.0 && v[4] < 10.0 && v[5] >= 0.0 && v[5] < 10.0 {
                            groups.entry(ty).or_default().push((base + i, v));
                        }
                    }
                    i += 16;
                }
            }
            for (ty, list) in groups.iter().filter(|(_, l)| l.len() >= 4) {
                println!("type {ty:X}: {} objects", list.len());
                for (a, v) in list.iter().take(6) {
                    println!("   {a:X}: yaw {:.2}..{:.2} pitch {:.2}..{:.2} time {:.2} cancel {:.2}", v[0], v[1], v[2], v[3], v[4], v[5]);
                }
            }
        }
        Some("ptrtable") => {
            // Run of consecutive code pointers (into the main module) around a[3].
            let t = Target::open(pid);
            let at = parse_hex(&a[3]);
            let span = 0x400_0000usize;
            let lo = at.saturating_sub(span);
            let b = t.read(lo, span * 2);
            let (_, mb, ms) = t.mods[0].clone();
            let code = |k: usize| {
                let v = u64::from_le_bytes(b[k..k + 8].try_into().unwrap()) as usize;
                v >= mb && v < mb + ms
            };
            let mid = at - lo;
            let mut s0 = mid;
            while s0 >= 8 && code(s0 - 8) {
                s0 -= 8;
            }
            let mut e0 = mid;
            while e0 + 8 <= b.len() && code(e0) {
                e0 += 8;
            }
            println!("table {} .. {} ({} entries); {} is index {}", t.name(lo + s0), t.name(lo + e0), (e0 - s0) / 8, t.name(at), (mid - s0) / 8);
        }
        Some("tdbmethods") => tdb::methods(&Target::open(pid), parse_hex(&a[3]), &a[4], a.get(5).and_then(|s| s.parse().ok()).unwrap_or(8)),
        Some("tdbreg") => tdb::registrations(&Target::open(pid), parse_hex(&a[3]), parse_hex(&a[4]), a.get(5).map_or("", String::as_str)),
        Some("tdbregall") => tdb::registrations_all(&Target::open(pid), parse_hex(&a[3]), a.get(4).map_or("", String::as_str)),
        Some("ftable") => {
            // Function table a[3] with a[4] entries: first instructions of each function.
            let t = Target::open(pid);
            let at = parse_hex(&a[3]);
            let n: usize = a[4].parse().unwrap();
            let b = t.read(at, n * 8);
            for k in 0..n {
                let f = u64::from_le_bytes(b[k * 8..k * 8 + 8].try_into().unwrap()) as usize;
                let code = t.read(f, 32);
                let mut d = Decoder::with_ip(64, &code, f as u64, DecoderOptions::NONE);
                let mut parts = Vec::new();
                let mut fm = IntelFormatter::new();
                for _ in 0..3 {
                    if !d.can_decode() { break; }
                    let ins = d.decode();
                    let mut s = String::new();
                    fm.format(&ins, &mut s);
                    parts.push(s);
                }
                println!("[{k}] {} : {}", t.name(f), parts.join(" ; "));
            }
        }
        Some("fuses") => {
            // For each function of table a[3] (a[4] entries): which of the displacements
            // a[5] (comma-separated hex) it touches, and whether it reads or writes them.
            let t = Target::open(pid);
            let at = parse_hex(&a[3]);
            let n: usize = a[4].parse().unwrap();
            let want: Vec<i64> = a[5].split(',').map(|x| parse_hex(x) as i64).collect();
            let b = t.read(at, n * 8);
            for k in 0..n {
                let f = u64::from_le_bytes(b[k * 8..k * 8 + 8].try_into().unwrap()) as usize;
                let code = t.read(f, 0x1000);
                let mut d = Decoder::with_ip(64, &code, f as u64, DecoderOptions::NONE);
                let mut hits = Vec::new();
                let mut last_ret = false;
                while d.can_decode() {
                    let ins = d.decode();
                    if ins.is_invalid() || (last_ret && ins.code() == iced_x86::Code::Int3) {
                        break;
                    }
                    last_ret = ins.mnemonic() == iced_x86::Mnemonic::Ret;
                    for op in 0..ins.op_count() {
                        if ins.op_kind(op) == iced_x86::OpKind::Memory {
                            let disp = ins.memory_displacement64() as i64;
                            if want.contains(&disp) && ins.memory_base() != iced_x86::Register::RSP {
                                let rw = if op == 0 && ins.mnemonic() != iced_x86::Mnemonic::Cmp && ins.mnemonic() != iced_x86::Mnemonic::Test { "w" } else { "r" };
                                hits.push(format!("{disp:X}{rw}"));
                            }
                        }
                    }
                }
                if !hits.is_empty() {
                    hits.dedup();
                    println!("[{k}] {} : {}", t.name(f), hits.join(" "));
                }
            }
        }
        Some("tdbprefix") => tdb::prefix(&Target::open(pid), parse_hex(&a[3]), &a[4].split(',').map(|x| x.parse().unwrap()).collect::<Vec<usize>>()),
        Some("tdbobj") => tdb::object(&Target::open(pid), parse_hex(&a[3]), parse_hex(&a[4]), a.get(5).map_or(0x200, |s| parse_hex(s))),
        Some("tdbinst") => tdb::instances(&Target::open(pid), parse_hex(&a[3]), &a[4], a.get(5).and_then(|s| s.parse().ok()).unwrap_or(4)),
        Some("listitems") => {
            // A managed List<T> at a[3] (+0x10 array, +0x18 size) whose elements hold a
            // chainsaw.Item at +0x10: print each item id (+0x28), +0x30 and count (+0x44).
            let t = Target::open(pid);
            let list = parse_hex(&a[3]);
            let l = t.read(list, 0x20);
            let q = |b: &[u8], o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap()) as usize;
            let d = |b: &[u8], o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
            let (arr, n) = (q(&l, 0x10), d(&l, 0x18) as usize);
            let els = t.read(arr + 0x20, n * 8);
            for i in 0..n {
                let e = q(&els, i * 8);
                let eb = t.read(e, 0x20);
                if eb.len() < 0x20 { continue; }
                let item = q(&eb, 0x10);
                let ib = t.read(item, 0x50);
                if ib.len() < 0x50 { continue; }
                println!("[{i}] elem {e:X} item {item:X} id {:08X} +30 {} +34 {} +44 {}", d(&ib, 0x28), d(&ib, 0x30), d(&ib, 0x34), d(&ib, 0x44));
            }
        }
        Some("tdbfind") => tdb::find(&Target::open(pid), parse_hex(&a[3]), &a[4]),
        Some("tdbhdr") => {
            // Research: header counts and table pointers of the type database at a[3].
            let t = Target::open(pid);
            let at = parse_hex(&a[3]);
            let h = t.read(at, 0x100);
            let u32s: Vec<String> = (0x08..0x58).step_by(4).map(|o| format!("+{o:02X}={}", u32::from_le_bytes(h[o..o + 4].try_into().unwrap()))).collect();
            println!("{}", u32s.join(" "));
            let mut ptrs: Vec<(usize, u64)> = (0x58..0x100).step_by(8).map(|o| (o, u64::from_le_bytes(h[o..o + 8].try_into().unwrap()))).collect();
            ptrs.sort_by_key(|p| p.1);
            for i in 0..ptrs.len() {
                let (o, p) = ptrs[i];
                let gap = ptrs.get(i + 1).map_or(0, |n| n.1 - p);
                let peek = t.read(p as usize, 48);
                let hex: Vec<String> = peek.iter().map(|b| format!("{b:02X}")).collect();
                let txt: String = peek.iter().map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { '.' }).collect();
                println!("hdr+{o:02X} -> {p:X} gap {gap:>11}  {}  |{txt}|", hex.join(" "));
            }
        }
        Some("snap") => snap(pid, &a[3], a.get(4).and_then(|s| s.parse().ok()).unwrap_or(1), a.get(5).and_then(|s| s.parse().ok()).unwrap_or(100_000)),
        Some("filt") => filt(pid, &a[3], &a[4]),
        Some("keep") => {
            // Addresses (one hex per line in a[4]) that now hold value a[3].
            let t = Target::open(pid);
            let v: i32 = a[3].parse().expect("i32 value");
            let list = std::fs::read_to_string(&a[4]).expect("address list");
            let mut kept = 0;
            for line in list.lines().filter(|l| !l.trim().is_empty()) {
                let addr = parse_hex(line.trim());
                let b = t.read(addr, 4);
                if b.len() == 4 && i32::from_le_bytes([b[0], b[1], b[2], b[3]]) == v {
                    println!("{addr:X}");
                    kept += 1;
                }
            }
            eprintln!("{kept} kept");
        }
        Some("find") => find(pid, a[3].parse().expect("i32 value")),
        _ => eprintln!("usage: re-probe watch|dis|read|find <pid> ..."),
    }
}
