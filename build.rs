//! Build glue for the Windows binary:
//!
//! * selects the right linker entry point for the toolchain (the crate is
//!   `no_std` + `no_main`, so neither MSVC nor MinGW may look for `main`), and
//! * compiles `assets/icon.ico` into a PE resource so Explorer, the taskbar and
//!   the tray all get the same icon.
//!
//! The resource compiler is invoked directly: MinGW uses `windres`, MSVC uses
//! `rc.exe` located through the Windows SDK / Visual Studio install (neither is
//! on PATH in a plain CI shell).

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    // The binary's entry point, per toolchain. `rustc-link-arg-bins` keeps the
    // test harness (which does have a CRT `main`) unaffected.
    //
    // A custom entry point also removes the linker's ability to infer the
    // subsystem, so it must be stated explicitly: MSVC fails with LNK1221 and
    // MinGW guesses wrong otherwise. The `console` feature exists so -selftest
    // can print on Windows CI, and it needs the console subsystem.
    let console = env::var("CARGO_FEATURE_CONSOLE").is_ok();
    match target_env.as_str() {
        "msvc" => {
            println!("cargo:rustc-link-arg-bins=/ENTRY:mainCRTStartup");
            if console {
                println!("cargo:rustc-link-arg-bins=/SUBSYSTEM:CONSOLE");
            } else {
                println!("cargo:rustc-link-arg-bins=/SUBSYSTEM:WINDOWS");
            }
        }
        _ => {
            println!("cargo:rustc-link-arg-bins=-nostartfiles");
            println!("cargo:rustc-link-arg-bins=-Wl,-e,mainCRTStartup");
            if console {
                println!("cargo:rustc-link-arg-bins=-Wl,--subsystem,console");
            } else {
                println!("cargo:rustc-link-arg-bins=-Wl,--subsystem,windows");
            }
        }
    }

    match compile_resource(&out_dir, &target_env) {
        Some(obj) => println!("cargo:rustc-link-arg-bins={}", obj.display()),
        None => panic!("could not compile assets/icon.ico into a PE resource"),
    }
}

/// Write a minimal `.rc` (no `windows.h` needed for a numeric icon ID) and
/// compile it to a COFF object (MinGW) or `.res` (MSVC).
fn compile_resource(out_dir: &Path, target_env: &str) -> Option<PathBuf> {
    let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon.ico");
    if !icon.is_file() {
        println!("cargo:warning=missing {}", icon.display());
        return None;
    }

    let rc_path = out_dir.join("app.rc");
    let icon_path = icon.to_string_lossy().replace('\\', "\\\\");
    std::fs::write(&rc_path, format!("1 ICON \"{icon_path}\"\n")).ok()?;
    println!("cargo:rerun-if-changed={}", rc_path.display());

    if target_env == "msvc" {
        let rc = find_msvc_rc()?;
        let res = out_dir.join("app.res");
        let ok = run(
            Command::new(&rc)
                .arg("/nologo")
                .arg("/fo")
                .arg(&res)
                .arg(&rc_path),
            &rc,
        );
        ok.then_some(res)
    } else {
        let windres = find_windres()?;
        let obj = out_dir.join("app-resource.o");
        let ok = run(
            Command::new(&windres)
                .arg(&rc_path)
                .args(["-O", "coff", "-o"])
                .arg(&obj),
            &windres,
        );
        ok.then_some(obj)
    }
}

fn run(cmd: &mut Command, program: &Path) -> bool {
    match cmd.status() {
        Ok(s) if s.success() => true,
        Ok(s) => {
            println!("cargo:warning={} failed with {s}", program.display());
            false
        }
        Err(e) => {
            println!("cargo:warning=could not run {}: {e}", program.display());
            false
        }
    }
}

fn find_windres() -> Option<PathBuf> {
    let target = env::var("TARGET").unwrap_or_default();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    // Cargo's gnu triple is `x86_64-pc-windows-gnu`, the binutils prefix is
    // `x86_64-w64-mingw32`.
    let mut names = vec![format!("{target}-windres")];
    if target.ends_with("-pc-windows-gnu") {
        names.push(format!("{arch}-w64-mingw32-windres"));
    }
    names.push("windres".to_string());
    for name in names {
        if let Some(path) = search_path(&name) {
            return Some(path);
        }
    }
    println!("cargo:warning=no windres found for {target}");
    None
}

/// `rc.exe` is shipped with the Windows SDK but is not on PATH outside a
/// developer prompt, so probe the usual install roots (newest first).
fn find_msvc_rc() -> Option<PathBuf> {
    if let Ok(rc) = env::var("RC") {
        let p = PathBuf::from(rc);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(p) = search_path("rc.exe") {
        return Some(p);
    }

    let host_arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86") => "x86",
        _ => "x64",
    };

    // Windows SDK: <root>\Windows Kits\10\bin\<version>\<arch>\rc.exe
    let mut sdk_roots = Vec::new();
    if let Ok(dir) = env::var("WindowsSdkDir") {
        sdk_roots.push(PathBuf::from(dir));
    }
    if let Ok(pf) = env::var("ProgramFiles(x86)") {
        sdk_roots.push(PathBuf::from(pf).join("Windows Kits").join("10"));
    }
    if let Ok(pf) = env::var("ProgramFiles") {
        sdk_roots.push(PathBuf::from(pf).join("Windows Kits").join("10"));
    }
    let mut versions: Vec<PathBuf> = Vec::new();
    for root in &sdk_roots {
        if let Ok(entries) = std::fs::read_dir(root.join("bin")) {
            versions.extend(entries.flatten().map(|e| e.path()));
        }
    }
    versions.sort();
    for dir in versions.iter().rev() {
        let rc = dir.join(host_arch).join("rc.exe");
        if rc.is_file() {
            return Some(rc);
        }
    }

    // Visual Studio: VC\Tools\MSVC\<ver>\bin\Host<x64>\<arch>\rc.exe
    if let Ok(pf) = env::var("ProgramFiles(x86)") {
        let vswhere = PathBuf::from(pf)
            .join("Microsoft Visual Studio")
            .join("Installer")
            .join("vswhere.exe");
        if vswhere.is_file() {
            let output = Command::new(vswhere)
                .args(["-latest", "-products", "*", "-property", "installationPath"])
                .output()
                .ok()?;
            let root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
            let mut tools: Vec<PathBuf> =
                std::fs::read_dir(root.join("VC").join("Tools").join("MSVC"))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .collect();
            tools.sort();
            for dir in tools.iter().rev() {
                let rc = dir
                    .join("bin")
                    .join(format!("Host{host_arch}"))
                    .join(host_arch)
                    .join("rc.exe");
                if rc.is_file() {
                    return Some(rc);
                }
            }
        }
    }

    None
}

fn search_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}
