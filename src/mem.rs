//! Access to another process's memory through WinAPI.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW, Thread32First,
    Thread32Next, MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
    TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::Memory::{
    VirtualAllocEx, VirtualProtectEx, VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_FREE, MEM_MAPPED,
    MEM_RESERVE,
    PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_READONLY,
    PAGE_READWRITE, PAGE_WRITECOPY,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, IsWow64Process, OpenProcess, OpenThread, PROCESS_QUERY_INFORMATION,
    PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE, THREAD_QUERY_INFORMATION,
};

const STILL_ACTIVE: u32 = 259;

#[repr(C)]
struct ThreadBasicInformation {
    exit_status: i32,
    teb: *mut c_void,
    client_id: [usize; 2],
    affinity_mask: usize,
    priority: i32,
    base_priority: i32,
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationThread(thread: HANDLE, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Module {
    pub name: String,
    pub base: usize,
    pub size: usize,
}

impl Module {
    pub fn contains(&self, addr: usize) -> bool {
        addr >= self.base && addr < self.base + self.size
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub base: usize,
    pub size: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// Writable data: where game values and pointers live.
    Writable,
    /// Anything readable: used for signature (AOB) search in code.
    Readable,
}

fn wstr(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

pub fn list_processes() -> Vec<ProcInfo> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                if e.th32ProcessID != 0 {
                    out.push(ProcInfo { pid: e.th32ProcessID, name: wstr(&e.szExeFile) });
                }
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

pub struct Process {
    handle: HANDLE,
    pub pid: u32,
    pub name: String,
    /// 32-bit game (4-byte pointers).
    pub is32: bool,
}

// A process handle may be used from any thread.
unsafe impl Send for Process {}
unsafe impl Sync for Process {}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

impl Process {
    pub fn open(pid: u32, name: &str) -> Result<Self, String> {
        unsafe {
            let h = OpenProcess(
                PROCESS_VM_READ | PROCESS_VM_WRITE | PROCESS_VM_OPERATION | PROCESS_QUERY_INFORMATION,
                0,
                pid,
            );
            if h.is_null() {
                return Err(format!(
                    "Не удалось открыть процесс (ошибка {}). Запустите FahrenHack от имени администратора.",
                    GetLastError()
                ));
            }
            let mut wow = 0;
            IsWow64Process(h, &mut wow);
            Ok(Process { handle: h, pid, name: name.to_string(), is32: wow != 0 })
        }
    }

    pub fn is_alive(&self) -> bool {
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(self.handle, &mut code) != 0 && code == STILL_ACTIVE }
    }

    pub fn ptr_size(&self) -> usize {
        if self.is32 {
            4
        } else {
            8
        }
    }

    pub fn read(&self, addr: usize, buf: &mut [u8]) -> bool {
        let mut n = 0usize;
        unsafe {
            ReadProcessMemory(
                self.handle,
                addr as *const c_void,
                buf.as_mut_ptr() as *mut c_void,
                buf.len(),
                &mut n,
            ) != 0
                && n == buf.len()
        }
    }

    pub fn read_ptr(&self, addr: usize) -> Option<usize> {
        if self.is32 {
            let mut b = [0u8; 4];
            self.read(addr, &mut b).then(|| u32::from_le_bytes(b) as usize)
        } else {
            let mut b = [0u8; 8];
            self.read(addr, &mut b).then(|| u64::from_le_bytes(b) as usize)
        }
    }

    pub fn read_u32(&self, addr: usize) -> Option<u32> {
        let mut b = [0u8; 4];
        self.read(addr, &mut b).then(|| u32::from_le_bytes(b))
    }

    /// Allocates executable memory within ±1.75 GB of `near`, so a 5-byte `jmp rel32` reaches it.
    /// The memory is never freed: a game thread might still be running inside it.
    pub fn alloc_near(&self, near: usize, size: usize) -> Option<usize> {
        const GRANULARITY: usize = 0x10000;
        const RANGE: usize = 0x7000_0000;
        let lo = near.saturating_sub(RANGE).max(0x10000);
        let hi = near.saturating_add(RANGE);
        let mut addr = lo;
        unsafe {
            while addr < hi {
                let mut mbi: MEMORY_BASIC_INFORMATION = zeroed();
                if VirtualQueryEx(self.handle, addr as *const c_void, &mut mbi, size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
                    return None;
                }
                let base = mbi.BaseAddress as usize;
                let end = base.saturating_add(mbi.RegionSize);
                if mbi.State == MEM_FREE {
                    let start = (base + GRANULARITY - 1) & !(GRANULARITY - 1);
                    if start + size <= end && start + size <= hi {
                        let p = VirtualAllocEx(
                            self.handle,
                            start as *const c_void,
                            size,
                            MEM_COMMIT | MEM_RESERVE,
                            PAGE_EXECUTE_READWRITE,
                        );
                        if !p.is_null() {
                            return Some(p as usize);
                        }
                    }
                }
                if end <= addr {
                    return None;
                }
                addr = end;
            }
        }
        None
    }

    pub fn read_u64(&self, addr: usize) -> Option<u64> {
        let mut b = [0u8; 8];
        self.read(addr, &mut b).then(|| u64::from_le_bytes(b))
    }

    /// Committed stack ranges `[limit, base)` of the game's threads. Values there are
    /// short-lived copies, so scans and pointer search skip them.
    pub fn thread_stacks(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut e: THREADENTRY32 = zeroed();
            e.dwSize = size_of::<THREADENTRY32>() as u32;
            if Thread32First(snap, &mut e) != 0 {
                loop {
                    if e.th32OwnerProcessID == self.pid {
                        let th = OpenThread(THREAD_QUERY_INFORMATION, 0, e.th32ThreadID);
                        if !th.is_null() {
                            let mut tbi: ThreadBasicInformation = zeroed();
                            let ok = NtQueryInformationThread(
                                th,
                                0,
                                &mut tbi as *mut _ as *mut c_void,
                                size_of::<ThreadBasicInformation>() as u32,
                                std::ptr::null_mut(),
                            ) == 0;
                            if ok {
                                let teb = tbi.teb as usize;
                                // NT_TIB: StackBase, StackLimit. A WOW64 thread keeps its 32-bit TEB 0x2000 further.
                                let range = if self.is32 {
                                    let t = teb + 0x2000;
                                    self.read_u32(t + 4).zip(self.read_u32(t + 8)).map(|(b, l)| (l as usize, b as usize))
                                } else {
                                    self.read_u64(teb + 8).zip(self.read_u64(teb + 16)).map(|(b, l)| (l as usize, b as usize))
                                };
                                if let Some((limit, base)) = range.filter(|(l, b)| b > l) {
                                    out.push((limit, base));
                                }
                            }
                            CloseHandle(th);
                        }
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

    pub fn write(&self, addr: usize, data: &[u8]) -> bool {
        let raw = || unsafe {
            let mut n = 0usize;
            WriteProcessMemory(
                self.handle,
                addr as *const c_void,
                data.as_ptr() as *const c_void,
                data.len(),
                &mut n,
            ) != 0
        };
        if raw() {
            return true;
        }
        // The page may be write-protected (code): lift protection for the write.
        unsafe {
            let mut old = 0u32;
            if VirtualProtectEx(self.handle, addr as *const c_void, data.len(), PAGE_EXECUTE_READWRITE, &mut old) == 0 {
                return false;
            }
            let ok = raw();
            let mut tmp = 0u32;
            VirtualProtectEx(self.handle, addr as *const c_void, data.len(), old, &mut tmp);
            ok
        }
    }

    pub fn modules(&self) -> Vec<Module> {
        let mut out = Vec::new();
        unsafe {
            let mut snap = INVALID_HANDLE_VALUE;
            for _ in 0..5 {
                snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, self.pid);
                if snap != INVALID_HANDLE_VALUE {
                    break;
                }
            }
            if snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut e: MODULEENTRY32W = zeroed();
            e.dwSize = size_of::<MODULEENTRY32W>() as u32;
            if Module32FirstW(snap, &mut e) != 0 {
                loop {
                    out.push(Module {
                        name: wstr(&e.szModule),
                        base: e.modBaseAddr as usize,
                        size: e.modBaseSize as usize,
                    });
                    if Module32NextW(snap, &mut e) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snap);
        }
        out
    }

    /// Build fingerprint of a loaded module: PE TimeDateStamp + image size.
    /// Changes with every game patch, so calibrations are kept per build.
    pub fn build_key(&self, m: &Module) -> String {
        let stamp = self
            .read_u32(m.base + 0x3C)
            .and_then(|lfanew| self.read_u32(m.base + lfanew as usize + 8))
            .unwrap_or(0);
        format!("{stamp:08x}-{:x}", m.size)
    }

    /// Committed memory regions of the requested kind, optionally limited to [start, end).
    pub fn regions_in(&self, kind: RegionKind, start: usize, end: usize) -> Vec<Region> {
        let writable = PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY;
        let readable = writable | PAGE_READONLY | PAGE_EXECUTE_READ;
        let want = if kind == RegionKind::Writable { writable } else { readable };
        let mut out = Vec::new();
        let mut addr = start.max(0x10000);
        let limit: usize = if self.is32 { 0xFFFF_0000 } else { 0x7FFF_FFFF_FFFF };
        let end = end.min(limit);
        unsafe {
            while addr < end {
                let mut mbi: MEMORY_BASIC_INFORMATION = zeroed();
                if VirtualQueryEx(self.handle, addr as *const c_void, &mut mbi, size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
                    break;
                }
                let base = mbi.BaseAddress as usize;
                let size = mbi.RegionSize;
                if mbi.State == MEM_COMMIT
                    && mbi.Protect & want != 0
                    && mbi.Protect & PAGE_GUARD == 0
                    && (kind == RegionKind::Readable || mbi.Type != MEM_MAPPED)
                {
                    let b = base.max(start);
                    let e = (base + size).min(end);
                    if e > b {
                        out.push(Region { base: b, size: e - b });
                    }
                }
                let next = base.saturating_add(size);
                if next <= addr {
                    break;
                }
                addr = next;
            }
        }
        out
    }

    /// Writable game data: heap and static memory, without thread stacks.
    pub fn regions(&self) -> Vec<Region> {
        let stacks = self.thread_stacks();
        self.regions_in(RegionKind::Writable, 0, usize::MAX)
            .into_iter()
            .filter(|r| !stacks.iter().any(|&(lo, hi)| r.base < hi && r.base + r.size > lo))
            .collect()
    }
}

/// Reads big memory ranges in chunks and hands them to `f(address, bytes)`.
pub fn for_each_chunk(p: &Process, regions: &[Region], mut f: impl FnMut(usize, &[u8]) -> bool) {
    const CHUNK: usize = 16 << 20;
    let mut buf = vec![0u8; CHUNK];
    for r in regions {
        let mut off = 0;
        while off < r.size {
            let len = (r.size - off).min(CHUNK);
            let b = &mut buf[..len];
            if p.read(r.base + off, b) && !f(r.base + off, b) {
                return;
            }
            off += len;
        }
    }
}
