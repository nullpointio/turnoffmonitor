//! Turn Off Monitor — one tiny exe that switches the displays off with a hotkey.
//!
//! The shipped binary is `no_std` + `alloc`: `std` alone costs ~250 KB of
//! Unicode/format machinery we do not need, and the whole point of this tool is
//! to be smaller than anything else that does the job. Release builds are
//! GUI-subsystem, so no console window flashes when the Run entry starts it.
//! Build with `--features console` for the console subsystem used by CI.

#![cfg_attr(all(windows, not(test)), no_std)]
#![cfg_attr(all(windows, not(test)), no_main)]
#![cfg_attr(
    all(windows, not(test), not(feature = "console")),
    windows_subsystem = "windows"
)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
#[cfg(not(all(windows, not(test))))]
use alloc::vec::Vec;

// The pure module is exercised by `cargo test`; on non-Windows the Win32 layer
// that calls most of it is compiled out, so silence dead-code there.
#[cfg_attr(not(windows), allow(dead_code))]
mod app;

#[cfg(windows)]
mod win;

// ---------------------------------------------------------------------------
// no_std runtime glue (Windows only; tests use std)
// ---------------------------------------------------------------------------

#[cfg(all(windows, not(test)))]
mod rt {
    use core::alloc::{GlobalAlloc, Layout};
    use core::ffi::c_void;
    use core::panic::PanicInfo;
    use windows_sys::Win32::System::Memory::{GetProcessHeap, HeapAlloc, HeapFree};
    use windows_sys::Win32::System::Threading::ExitProcess;

    struct WinHeap;

    unsafe impl GlobalAlloc for WinHeap {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // GlobalAlloc must return non-null for zero-sized requests.
            HeapAlloc(GetProcessHeap(), 0, layout.size().max(1)) as *mut u8
        }

        unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
            if !ptr.is_null() {
                HeapFree(GetProcessHeap(), 0, ptr as *const c_void);
            }
        }
    }

    #[global_allocator]
    static ALLOC: WinHeap = WinHeap;

    #[panic_handler]
    fn panic(_info: &PanicInfo) -> ! {
        // Never unwind: `panic = "abort"` in release. Exiting beats hanging a
        // tray app with no window.
        unsafe { ExitProcess(101) }
    }

    /// panic = "abort" still emits an unwind table reference; provide the
    /// personality so the link does not need libunwind.
    #[no_mangle]
    pub extern "C" fn rust_eh_personality() {}

    #[no_mangle]
    pub unsafe extern "system" fn mainCRTStartup() -> ! {
        crate::start()
    }
}

#[cfg(all(windows, not(test)))]
fn start() -> ! {
    let line = win::command_line();
    let args = app::split_command_line(&line);
    let args: alloc::vec::Vec<String> = args.into_iter().skip(1).collect();
    let code = run(&args);
    unsafe { windows_sys::Win32::System::Threading::ExitProcess(code as u32) }
}

/// Console output shared by both flavours.
#[cfg(windows)]
fn out(s: &str) {
    win::print(s);
}

#[cfg(not(windows))]
fn out(s: &str) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(s.as_bytes());
    let _ = std::io::stdout().flush();
}

#[cfg(not(all(windows, not(test))))]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(&args));
}

fn run(args: &[String]) -> i32 {
    let cmd = match app::parse_args(args) {
        Ok(cmd) => cmd,
        Err(err) => {
            out(&format!("turnoffmonitor: {err}\n\n{}", app::usage()));
            return 2;
        }
    };

    match cmd {
        app::Command::Help => {
            out(&app::usage());
            return 0;
        }
        app::Command::Version => {
            out(&format!("turnoffmonitor {}\n", app::VERSION));
            return 0;
        }
        _ => {}
    }

    #[cfg(windows)]
    {
        win::dispatch(cmd)
    }

    #[cfg(not(windows))]
    {
        let _ = cmd;
        eprintln!("this build targets Windows only; run the tests with `cargo test`");
        1
    }
}

#[cfg(test)]
mod tests {
    use super::app::Command;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn exit_code_contract_for_bad_arguments_is_two() {
        assert!(super::app::parse_args(&args(&["-nope"])).is_err());
    }

    #[test]
    fn help_and_version_are_handled_before_platform_dispatch() {
        assert_eq!(super::app::parse_args(&args(&["-help"])), Ok(Command::Help));
        assert_eq!(
            super::app::parse_args(&args(&["-version"])),
            Ok(Command::Version)
        );
    }
}
