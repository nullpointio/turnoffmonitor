//! Thin Win32 layer: message loop, tray icon, hotkey, registry, CLI actions.
//!
//! Only `windows-sys` is used; no GUI toolkit. The whole file is Windows-only
//! and is compiled away on other hosts so `cargo test` runs on Linux.
//!
//! The crate is `no_std` in release, so everything here uses `core`/`alloc`
//! primitives (`String`, `Vec`) and Win32 calls instead of `std::io`,
//! `std::sync` or `std::thread`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ffi::c_void;
use core::mem::{size_of, zeroed};
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicU32, AtomicU8, AtomicUsize, Ordering};

use windows_sys::Win32::Foundation::{
    GetLastError, BOOL, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, FILETIME,
    HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, WriteFile, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS};
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_SZ,
};
use windows_sys::Win32::System::Shutdown::LockWorkStation;
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, GetProcessTimes, Sleep,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, KillTimer, LoadCursorW,
    LoadIconW, LoadImageW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
    SetForegroundWindow, SetTimer, TrackPopupMenu, TranslateMessage, HICON, IDI_APPLICATION,
    IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED, MSG, SM_CXSMICON, WNDCLASSW,
};

use crate::app::{self, Command, Hotkey};

// Messages and constants we rely on. Declared locally so the imported surface
// stays explicit and minimal.
const WM_NULL: u32 = 0x0000;
const WM_DESTROY: u32 = 0x0002;
const WM_CLOSE: u32 = 0x0010;
const WM_ENDSESSION: u32 = 0x0016;
const WM_TIMER: u32 = 0x0113;
const WM_SYSCOMMAND: u32 = 0x0112;
const WM_HOTKEY: u32 = 0x0312;
const WM_APP: u32 = 0x8000;
const WM_TRAY: u32 = WM_APP + 1;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_CONTEXTMENU: u32 = 0x007B;
const WM_LBUTTONDBLCLK: u32 = 0x0203;

const SC_MONITORPOWER: usize = 0xF170;
const HWND_BROADCAST_H: HWND = 0xFFFF as HWND;

const MONITOR_OFF: isize = 2;
const MONITOR_ON: isize = -1;

const WS_POPUP: u32 = 0x8000_0000;
const IDC_ARROW_PTR: usize = 32512;
const MF_STRING: u32 = 0x0000;
const MF_SEPARATOR: u32 = 0x0800;
const MF_CHECKED: u32 = 0x0008;
const TPM_RETURNCMD: u32 = 0x0100;
const TPM_RIGHTBUTTON: u32 = 0x0002;
const TPM_NONOTIFY: u32 = 0x0080;

const HOTKEY_ID: i32 = 1;
const TIMER_ID: usize = 1;
/// Second timer id, used only by `-selftest` to close the idle window.
const IDLE_TIMER_ID: usize = 2;
/// Length of the idle sample in `-selftest`, as the issue specifies.
const IDLE_WINDOW_MS: u32 = 60_000;
/// Acceptance ceiling for private bytes (issue #4).
const PRIVATE_BUDGET_BYTES: usize = 3 * 1024 * 1024;
/// Let the key-up be processed before the display is switched off, otherwise
/// the release itself wakes the monitor again.
const OFF_DELAY_MS: u32 = 250;

const CLASS_NAME: &str = "TurnOffMonitorWnd";
const MUTEX_NAME: &str = r"Local\TurnOffMonitor.SingleInstance";
const SELFTEST_MUTEX: &str = r"Local\TurnOffMonitor.Selftest";

static PENDING_LOCK: AtomicU32 = AtomicU32::new(0);
static ICON_HANDLE: AtomicUsize = AtomicUsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static LABEL_BYTES: [AtomicU8; 64] = [const { AtomicU8::new(0) }; 64];
static LABEL_LEN: AtomicUsize = AtomicUsize::new(0);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

fn set_label(s: &str) {
    let mut n = 0;
    for b in s.bytes().take(64) {
        LABEL_BYTES[n].store(b, Ordering::SeqCst);
        n += 1;
    }
    LABEL_LEN.store(n, Ordering::SeqCst);
}

fn label() -> String {
    let n = LABEL_LEN.load(Ordering::SeqCst).min(64);
    let mut s = String::with_capacity(n);
    for slot in LABEL_BYTES.iter().take(n) {
        s.push(slot.load(Ordering::SeqCst) as char);
    }
    s
}

/// Console output that never panics: the shipped exe is a GUI binary, so
/// writes are simply dropped when there is no console.
pub fn print(msg: impl AsRef<str>) {
    static ATTACH_TRIED: AtomicU32 = AtomicU32::new(0);
    let msg = msg.as_ref();
    unsafe {
        let std = windows_sys::Win32::System::Console::STD_OUTPUT_HANDLE;
        let mut handle = GetStdHandle(std);
        let invalid = |h: windows_sys::Win32::Foundation::HANDLE| h.is_null() || h == -1isize as _;
        if invalid(handle) && ATTACH_TRIED.swap(1, Ordering::SeqCst) == 0 {
            // GUI build launched from a terminal: borrow the parent console.
            AttachConsole(ATTACH_PARENT_PROCESS);
            handle = GetStdHandle(std);
        }
        if invalid(handle) {
            handle = CreateFileW(
                wide("CONOUT$").as_ptr(),
                windows_sys::Win32::Foundation::GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            );
        }
        let mut written = 0u32;
        WriteFile(
            handle,
            msg.as_ptr(),
            msg.len() as u32,
            &mut written,
            null_mut(),
        );
    }
}

/// GUI-subsystem binaries are not given a console; borrow the parent one so
/// `-version` / `-selftest` print when run from cmd.exe.
fn attach_console() {
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// The raw command line (`GetCommandLineW`), used instead of `std::env::args`.
///
/// The manual NUL scan below is recognised by LLVM as `wcslen` and emitted as a
/// call to it. A `no_std` binary links no CRT, so without this definition the
/// MSVC linker fails with LNK2019 on `wcslen` (MinGW resolves it from libmingwex).
#[no_mangle]
pub unsafe extern "C" fn wcslen(mut s: *const u16) -> usize {
    let start = s;
    while *s != 0 {
        s = s.add(1);
    }
    s.offset_from(start) as usize
}

pub fn command_line() -> String {
    unsafe {
        let ptr = windows_sys::Win32::System::Environment::GetCommandLineW();
        if ptr.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(core::slice::from_raw_parts(ptr, len))
    }
}

// ---------------------------------------------------------------------------
// Monitor power
// ---------------------------------------------------------------------------

/// The one API call that actually blanks every display. `PostMessage` is used
/// instead of `SendMessage` so a hung window cannot stall us.
fn post_monitor(state: isize) -> bool {
    unsafe {
        PostMessageW(
            HWND_BROADCAST_H,
            WM_SYSCOMMAND,
            SC_MONITORPOWER,
            state as LPARAM,
        ) != 0
    }
}

fn post_monitor_off() -> bool {
    post_monitor(MONITOR_OFF)
}

fn turn_off_now() -> bool {
    let ok = post_monitor_off();
    write_state(1);
    ok
}

// ---------------------------------------------------------------------------
// Registry (HKCU only: no admin, no machine-wide changes)
// ---------------------------------------------------------------------------

fn current_exe() -> Option<String> {
    let mut buf = [0u16; 1024];
    let len = unsafe { GetModuleFileNameW(null_mut(), buf.as_mut_ptr(), buf.len() as u32) };
    if len == 0 || len as usize >= buf.len() {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

fn open_key(path: &str, access: u32) -> Option<HKEY> {
    let mut key: HKEY = null_mut();
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide(path).as_ptr(), 0, access, &mut key) };
    if rc == ERROR_SUCCESS {
        Some(key)
    } else {
        None
    }
}

/// Is the Run entry present? Read-only probe, never creates the key.
pub fn run_entry_present() -> bool {
    let Some(key) = open_key(app::RUN_KEY, KEY_QUERY_VALUE) else {
        return false;
    };
    let rc = unsafe {
        RegQueryValueExW(
            key,
            wide(app::RUN_VALUE).as_ptr(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    unsafe { RegCloseKey(key) };
    rc == ERROR_SUCCESS
}

pub fn install() -> i32 {
    let Some(exe) = current_exe() else {
        print("error: cannot determine own executable path\n");
        return 1;
    };
    let data = match app::run_value_data(&exe) {
        Ok(d) => d,
        Err(e) => {
            print(format!("error: {e}\n"));
            return 1;
        }
    };
    let data = wide(&data);
    unsafe {
        let mut key: HKEY = null_mut();
        let rc = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(app::RUN_KEY).as_ptr(),
            0,
            null(),
            0,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        );
        if rc != ERROR_SUCCESS {
            print(format!("error: cannot open Run key (code {rc})\n"));
            return 1;
        }
        let rc = RegSetValueExW(
            key,
            wide(app::RUN_VALUE).as_ptr(),
            0,
            REG_SZ,
            data.as_ptr() as *const u8,
            (data.len() * size_of::<u16>()) as u32,
        );
        RegCloseKey(key);
        if rc != ERROR_SUCCESS {
            print(format!("error: cannot write Run value (code {rc})\n"));
            return 1;
        }
    }
    print("installed: starts with Windows\n");
    0
}

pub fn uninstall() -> i32 {
    let Some(key) = open_key(app::RUN_KEY, KEY_SET_VALUE | KEY_QUERY_VALUE) else {
        print("not installed: no Run entry\n");
        return 0;
    };
    let rc = unsafe { RegDeleteValueW(key, wide(app::RUN_VALUE).as_ptr()) };
    unsafe { RegCloseKey(key) };
    if rc == ERROR_SUCCESS {
        print("uninstalled: starts with Windows\n");
        0
    } else if rc == ERROR_FILE_NOT_FOUND {
        print("not installed: no Run entry\n");
        0
    } else {
        print(format!("error: cannot delete Run value (code {rc})\n"));
        1
    }
}

/// Toggle the Run entry, reporting the new state.
fn toggle_startup() -> bool {
    if run_entry_present() {
        uninstall() == 0
    } else {
        install() == 0
    }
}

fn read_state() -> u32 {
    let Some(key) = open_key(app::APP_KEY, KEY_QUERY_VALUE) else {
        return 0;
    };
    let mut value: u32 = 0;
    let mut cb = size_of::<u32>() as u32;
    let mut kind: u32 = 0;
    let rc = unsafe {
        RegQueryValueExW(
            key,
            wide(app::STATE_VALUE).as_ptr(),
            null_mut(),
            &mut kind,
            &mut value as *mut u32 as *mut u8,
            &mut cb,
        )
    };
    unsafe { RegCloseKey(key) };
    if rc == ERROR_SUCCESS && kind == REG_DWORD && cb == 4 {
        value
    } else {
        0
    }
}

fn write_state(value: u32) {
    unsafe {
        let mut key: HKEY = null_mut();
        let rc = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(app::APP_KEY).as_ptr(),
            0,
            null(),
            0,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        );
        if rc != ERROR_SUCCESS {
            return;
        }
        let bytes = value.to_le_bytes();
        RegSetValueExW(
            key,
            wide(app::STATE_VALUE).as_ptr(),
            0,
            REG_DWORD,
            bytes.as_ptr(),
            4,
        );
        RegCloseKey(key);
    }
}

// ---------------------------------------------------------------------------
// Tray icon
// ---------------------------------------------------------------------------

/// Icon resource ID declared in `build.rs` (`1 ICON "assets/icon.ico"`).
const ICON_RESOURCE: usize = 1;
const ICON_RESOURCE_NAME: *const u16 = ICON_RESOURCE as *const u16;

/// Load the embedded multi-size icon; fall back to the system default if the
/// resource is somehow missing. `LR_SHARED` means the handle must not be
/// destroyed, which is why `WM_DESTROY` only forgets it. `cx == 0` asks for the
/// system icon size (window class), otherwise an exact small-icon size (tray).
unsafe fn load_icon(hinst: HINSTANCE, cx: i32) -> HICON {
    let flags = if cx == 0 {
        LR_DEFAULTSIZE | LR_SHARED
    } else {
        LR_SHARED
    };
    let icon = LoadImageW(hinst, ICON_RESOURCE_NAME, IMAGE_ICON, cx, cx, flags) as HICON;
    if icon.is_null() {
        LoadIconW(null_mut(), IDI_APPLICATION)
    } else {
        icon
    }
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = ICON_HANDLE.load(Ordering::SeqCst) as HICON;
    let tip = format!("Turn Off Monitor ({})", label());
    for (slot, unit) in nid.szTip.iter_mut().zip(tip.encode_utf16().take(127)) {
        *slot = unit;
    }
    nid
}

fn tray_add(hwnd: HWND) -> bool {
    let nid = tray_data(hwnd);
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) != 0 }
}

fn tray_remove(hwnd: HWND) {
    let nid = tray_data(hwnd);
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn show_menu(hwnd: HWND) {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        AppendMenuW(menu, MF_STRING, 1, wide("Turn off monitors").as_ptr());
        AppendMenuW(menu, MF_STRING, 2, wide("Turn off and lock").as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        let flags = if run_entry_present() {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        AppendMenuW(menu, flags, 3, wide("Start with Windows").as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, 4, wide("Quit").as_ptr());

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            pt.x,
            pt.y,
            0,
            hwnd,
            null(),
        );
        DestroyMenu(menu);
        // Documented workaround so the menu closes when clicking elsewhere.
        PostMessageW(hwnd, WM_NULL, 0, 0);

        match cmd {
            1 => request_off(hwnd, false),
            2 => request_off(hwnd, true),
            3 => {
                toggle_startup();
            }
            4 => {
                DestroyWindow(hwnd);
            }
            _ => {}
        }
    }
}

fn request_off(hwnd: HWND, lock: bool) {
    PENDING_LOCK.store(u32::from(lock), Ordering::SeqCst);
    unsafe {
        SetTimer(hwnd, TIMER_ID, OFF_DELAY_MS, None);
    }
}

// ---------------------------------------------------------------------------
// Window + message loop
// ---------------------------------------------------------------------------

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let event = l as u32;
            if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                show_menu(hwnd);
            } else if event == WM_LBUTTONDBLCLK {
                request_off(hwnd, false);
            }
            0
        }
        WM_HOTKEY => {
            request_off(hwnd, false);
            0
        }
        WM_TIMER => {
            KillTimer(hwnd, TIMER_ID);
            if PENDING_LOCK.swap(0, Ordering::SeqCst) != 0 {
                LockWorkStation();
            }
            turn_off_now();
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY | WM_ENDSESSION => {
            UnregisterHotKey(hwnd, HOTKEY_ID);
            tray_remove(hwnd);
            // The icon is a shared resource handle (LR_SHARED); never destroy it.
            ICON_HANDLE.store(0, Ordering::SeqCst);
            PostQuitMessage(0);
            0
        }
        _ => {
            if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::SeqCst) {
                // Explorer restarted: put the icon back.
                tray_add(hwnd);
                return 0;
            }
            DefWindowProcW(hwnd, msg, w, l)
        }
    }
}

unsafe fn create_window(hinst: HINSTANCE) -> Option<HWND> {
    let class = wide(CLASS_NAME);
    let icon = load_icon(hinst, 0);
    let small = load_icon(hinst, GetSystemMetrics(SM_CXSMICON));
    if !small.is_null() {
        ICON_HANDLE.store(small as usize, Ordering::SeqCst);
    }
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: icon,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW_PTR as *const u16),
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class.as_ptr(),
    };
    RegisterClassW(&wc);
    let hwnd = CreateWindowExW(
        0,
        class.as_ptr(),
        wide("Turn Off Monitor").as_ptr(),
        WS_POPUP,
        0,
        0,
        0,
        0,
        null_mut(),
        null_mut(),
        hinst,
        null(),
    );
    if hwnd.is_null() {
        None
    } else {
        Some(hwnd)
    }
}

fn register_hotkey(hwnd: HWND, hk: &Hotkey) -> bool {
    let with_norepeat = hk.mods | app::MOD_NOREPEAT;
    unsafe {
        if RegisterHotKey(hwnd, HOTKEY_ID, with_norepeat, hk.vk) != 0 {
            return true;
        }
        // Pre-Windows 7 or a shell that rejects MOD_NOREPEAT.
        RegisterHotKey(hwnd, HOTKEY_ID, hk.mods, hk.vk) != 0
    }
}

/// Resident mode: tray icon + global hotkey + message loop.
pub fn run_resident(hk: &Hotkey) -> i32 {
    attach_console();
    set_label(&app::format_hotkey(hk));

    unsafe {
        let name = wide(MUTEX_NAME);
        let _mutex = CreateMutexW(null_mut(), 0, name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return 0;
        }

        let hinst = GetModuleHandleW(null());
        let Some(hwnd) = create_window(hinst) else {
            print("error: cannot create window\n");
            return 1;
        };
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            Ordering::SeqCst,
        );

        tray_add(hwnd);

        if !register_hotkey(hwnd, hk) {
            print(format!(
                "warning: {} is already taken; use the tray menu or -hotkey\n",
                label()
            ));
        }

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        0
    }
}

// ---------------------------------------------------------------------------
// CLI actions
// ---------------------------------------------------------------------------

pub fn dispatch(cmd: Command) -> i32 {
    attach_console();
    match cmd {
        Command::Run(hk) => run_resident(&hk),
        Command::Off => {
            unsafe { Sleep(OFF_DELAY_MS) };
            if turn_off_now() {
                0
            } else {
                1
            }
        }
        Command::Lock => {
            unsafe {
                LockWorkStation();
            }
            unsafe { Sleep(OFF_DELAY_MS) };
            if turn_off_now() {
                0
            } else {
                1
            }
        }
        Command::Toggle => {
            if read_state() == 1 {
                post_monitor(MONITOR_ON);
                write_state(0);
            } else {
                unsafe { Sleep(OFF_DELAY_MS) };
                turn_off_now();
            }
            0
        }
        Command::Install => install(),
        Command::Uninstall => uninstall(),
        Command::Selftest => selftest(),
        Command::Version => {
            print(format!("turnoffmonitor {}\n", app::VERSION));
            0
        }
        Command::Help => {
            print(app::usage());
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Selftest
// ---------------------------------------------------------------------------

// Footprint probes (issue #4): working set, private bytes, CPU time.

/// Working set and private bytes (`PagefileUsage`, what Task Manager calls
/// "Commit size") for this process, in bytes.
fn process_memory() -> Option<(usize, usize)> {
    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS = zeroed();
        counters.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) == 0 {
            return None;
        }
        Some((counters.WorkingSetSize, counters.PagefileUsage))
    }
}

/// Kernel + user CPU time for this process, in 100 ns ticks.
fn process_cpu_ticks() -> Option<u64> {
    unsafe {
        let mut created: FILETIME = zeroed();
        let mut exited: FILETIME = zeroed();
        let mut kernel: FILETIME = zeroed();
        let mut user: FILETIME = zeroed();
        if GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        ) == 0
        {
            return None;
        }
        let ticks = |f: &FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
        Some(ticks(&kernel) + ticks(&user))
    }
}

fn mem_text(mem: (usize, usize)) -> String {
    format!("{} K working set, {} K private", mem.0 / 1024, mem.1 / 1024)
}

pub fn selftest() -> i32 {
    let mut passed = 0u32;
    let mut failed = 0u32;

    print(format!(
        "turnoffmonitor {} selftest [{}]\n",
        app::VERSION,
        if cfg!(feature = "console") {
            "console"
        } else {
            "gui"
        }
    ));

    let startup = process_memory();
    print(match startup {
        Some(mem) => format!("MEM startup: {}\n", mem_text(mem)),
        None => "MEM startup: unavailable\n".to_string(),
    });

    let mut line = |ok: bool, name: &str, detail: String| {
        if ok {
            passed += 1;
            print(format!("PASS {name} ({detail})\n"));
        } else {
            failed += 1;
            print(format!("FAIL {name} ({detail})\n"));
        }
    };

    let args = app::check_parse_table();
    line(
        args.failures.is_empty(),
        "arg-parse",
        match args.failures.first() {
            Some(f) => format!("{} cases, first failure: {f}", args.cases),
            None => format!("{} cases", args.cases),
        },
    );

    let hk = app::default_hotkey();
    let keys = app::check_hotkey_table();
    line(
        keys.failures.is_empty(),
        "hotkey-parse",
        match keys.failures.first() {
            Some(f) => format!("{} cases, first failure: {f}", keys.cases),
            None => format!(
                "{} cases, {} -> mods=0x{:04X} vk=0x{:02X}",
                keys.cases,
                app::format_hotkey(&hk),
                hk.mods,
                hk.vk
            ),
        },
    );

    let exe = current_exe()
        .unwrap_or_else(|| r"C:\Program Files\Turn Off Monitor\turnoffmonitor.exe".into());
    let entry = app::run_entry_display(&exe);
    line(
        entry.as_ref().is_ok_and(|s| s.starts_with("HKCU\\")),
        "registry-format",
        entry.unwrap_or_else(|_| "format error".into()),
    );
    // Dry run: snapshot now, and prove at the end that nothing was written.
    let run_before = run_entry_present();

    let mut live: Option<HWND> = None;
    unsafe {
        let name = wide(SELFTEST_MUTEX);
        let handle = CreateMutexW(null_mut(), 0, name.as_ptr());
        let ok = !handle.is_null() && GetLastError() != ERROR_ALREADY_EXISTS;
        line(ok, "mutex", SELFTEST_MUTEX.into());
        if !handle.is_null() {
            windows_sys::Win32::Foundation::CloseHandle(handle);
        }

        let hinst = GetModuleHandleW(null());
        match create_window(hinst) {
            Some(hwnd) => {
                let registered = register_hotkey(hwnd, &hk);
                let err = GetLastError();
                line(
                    registered,
                    "hotkey-register",
                    if registered {
                        format!("{} on a hidden window", app::format_hotkey(&hk))
                    } else {
                        format!("{} rejected, GetLastError={err}", app::format_hotkey(&hk))
                    },
                );
                live = Some(hwnd);
            }
            None => line(false, "hotkey-register", "window creation failed".into()),
        }

        let sent = post_monitor_off();
        line(
            sent,
            "broadcast",
            "PostMessage(HWND_BROADCAST, WM_SYSCOMMAND, SC_MONITORPOWER, 2) accepted".into(),
        );
    }

    // Idle window: sit in the same blocking `GetMessage` pump the tray app uses,
    // with the hotkey registered and the tray window alive, for 60 s. A timer
    // ends the sample. No polling, so the CPU delta is the real idle cost.
    let cpu_before = process_cpu_ticks();
    unsafe {
        match live {
            Some(hwnd) => {
                SetTimer(hwnd, IDLE_TIMER_ID, IDLE_WINDOW_MS, None);
                let mut msg: MSG = zeroed();
                while GetMessageW(&mut msg, hwnd, 0, 0) > 0 {
                    if msg.message == WM_TIMER && msg.wParam == IDLE_TIMER_ID {
                        KillTimer(hwnd, IDLE_TIMER_ID);
                        break;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            None => Sleep(IDLE_WINDOW_MS),
        }
    }
    let idle = process_memory();
    let cpu_ticks = match (cpu_before, process_cpu_ticks()) {
        (Some(before), Some(after)) => Some(after.saturating_sub(before)),
        _ => None,
    };
    let cpu_ms = cpu_ticks.map(|t| t / 10_000);
    print(match idle {
        Some(mem) => format!(
            "MEM idle-{}s: {} (cpu={} ms)\n",
            IDLE_WINDOW_MS / 1000,
            mem_text(mem),
            cpu_ms
                .map(|ms| format!("{ms}"))
                .unwrap_or_else(|| "-".to_string())
        ),
        None => "MEM idle: unavailable\n".to_string(),
    });

    line(
        idle.is_some_and(|(ws, _)| ws > 0),
        "mem-working-set",
        match idle {
            Some(mem) => format!("{} K reported", mem.0 / 1024),
            None => "K32GetProcessMemoryInfo failed".into(),
        },
    );
    line(
        idle.is_some_and(|(_, private)| private < PRIVATE_BUDGET_BYTES),
        "mem-private",
        match idle {
            Some((_, private)) => format!(
                "{} K private, budget {} K (< 3 MB)",
                private / 1024,
                PRIVATE_BUDGET_BYTES / 1024
            ),
            None => "unavailable".into(),
        },
    );
    line(
        cpu_ticks == Some(0),
        "cpu-idle",
        match cpu_ticks {
            Some(ticks) => format!(
                "{} ms CPU over {} s idle",
                ticks / 10_000,
                IDLE_WINDOW_MS / 1000
            ),
            None => "GetProcessTimes failed".into(),
        },
    );

    unsafe {
        if let Some(hwnd) = live {
            UnregisterHotKey(hwnd, HOTKEY_ID);
            DestroyWindow(hwnd);
        }
    }

    line(
        run_entry_present() == run_before,
        "registry-dryrun",
        if run_before {
            "Run entry untouched (was present)".to_string()
        } else {
            "Run entry untouched (was absent)".to_string()
        },
    );

    print(format!("{passed} passed, {failed} failed\n"));
    if failed > 0 {
        1
    } else {
        0
    }
}

// Keep the `BOOL`/`c_void` imports honest on old linkers.
const _: BOOL = 0;
const _: *const c_void = null();
