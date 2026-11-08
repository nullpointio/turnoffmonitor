//! Pure, platform-independent logic.
//!
//! Everything here runs — and is tested — on any host OS: argument parsing,
//! hotkey-spec parsing, `MOD_*`/`VK_*` mapping and the registry strings we
//! hand to Win32. No `windows-sys` types are allowed in this module.

// `alloc` is available in both flavours: the shipped exe is `no_std` + `alloc`,
// while `cargo test` builds against std (which re-exports `alloc`).
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Hotkey registered when the user gives no `-hotkey` override.
pub const DEFAULT_HOTKEY: &str = "ctrl+alt+m";

pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;
/// Not in the budget's feature list but free: stops auto-repeat firing repeatedly.
/// Only referenced by the Windows layer, so silence dead-code on test builds.
#[cfg_attr(not(windows), allow(dead_code))]
pub const MOD_NOREPEAT: u32 = 0x4000;

/// `HKCU\...\Run` — the per-user autorun key, no admin rights involved.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Value name inside the Run key.
pub const RUN_VALUE: &str = "TurnOffMonitor";
/// Our own settings key (also used for the `-toggle` display-state marker).
pub const APP_KEY: &str = r"Software\TurnOffMonitor";
pub const STATE_VALUE: &str = "DisplayOff";

/// A parsed Win32 hotkey: modifier mask + virtual-key code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub mods: u32,
    pub vk: u32,
}

impl Hotkey {
    pub const fn new(mods: u32, vk: u32) -> Self {
        Self { mods, vk }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Resident: tray icon + global hotkey.
    Run(Hotkey),
    Off,
    Lock,
    Toggle,
    Install,
    Uninstall,
    Version,
    Selftest,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgError {
    Unknown(String),
    MissingValue(&'static str),
    BadHotkey(String),
    Conflict(&'static str),
}

impl fmt::Display for ArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArgError::Unknown(a) => write!(f, "unknown argument: {a}"),
            ArgError::MissingValue(hint) => write!(f, "missing value, expected {hint}"),
            ArgError::BadHotkey(e) => write!(f, "bad hotkey: {e}"),
            ArgError::Conflict(what) => write!(f, "{what}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    Empty,
    NoKey,
    NoModifier,
    Unknown(String),
    TooManyKeys(String),
    Duplicate(String),
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HotkeyError::Empty => write!(f, "empty spec"),
            HotkeyError::NoKey => write!(f, "no key given (e.g. ctrl+alt+m)"),
            HotkeyError::NoModifier => {
                write!(f, "needs at least one modifier (ctrl/alt/shift/win)")
            }
            HotkeyError::Unknown(t) => write!(f, "unknown token '{t}'"),
            HotkeyError::TooManyKeys(t) => write!(f, "more than one key: '{t}'"),
            HotkeyError::Duplicate(t) => write!(f, "duplicate modifier '{t}'"),
        }
    }
}

pub fn default_hotkey() -> Hotkey {
    // Compile-time constant, covered by `default_hotkey_parses`.
    parse_hotkey(DEFAULT_HOTKEY).expect("DEFAULT_HOTKEY is valid")
}

/// Normalize a raw argv token: case-insensitive, `/off` accepted like `-off`.
fn normalize(arg: &str) -> String {
    let a = arg.trim().to_ascii_lowercase();
    match a.strip_prefix('/') {
        Some(rest) => format!("-{rest}"),
        None => a,
    }
}

/// Parse argv (already without program name).
///
/// `-hotkey <spec>` on its own selects resident mode with that combo;
/// combining it with another command is a usage error.
pub fn parse_args(args: &[String]) -> Result<Command, ArgError> {
    let mut action: Option<Command> = None;
    let mut hotkey: Option<Hotkey> = None;
    let mut i = 0;

    while i < args.len() {
        let arg = normalize(&args[i]);
        match arg.as_str() {
            "-off" | "-monitor-off" => set_action(&mut action, Command::Off)?,
            "-lock" | "-off-lock" => set_action(&mut action, Command::Lock)?,
            "-toggle" => set_action(&mut action, Command::Toggle)?,
            "-install" => set_action(&mut action, Command::Install)?,
            "-uninstall" => set_action(&mut action, Command::Uninstall)?,
            "-version" | "-v" => set_action(&mut action, Command::Version)?,
            "-selftest" => set_action(&mut action, Command::Selftest)?,
            "-help" | "-h" | "-?" | "--help" => set_action(&mut action, Command::Help)?,
            "-hotkey" => {
                let value = args
                    .get(i + 1)
                    .ok_or(ArgError::MissingValue("-hotkey <spec>"))?;
                let hk = parse_hotkey(value).map_err(|e| ArgError::BadHotkey(e.to_string()))?;
                if hotkey.replace(hk).is_some() {
                    return Err(ArgError::Conflict("-hotkey given more than once"));
                }
                i += 1;
            }
            "" => return Err(ArgError::Unknown(args[i].clone())),
            _ => return Err(ArgError::Unknown(args[i].clone())),
        }
        i += 1;
    }

    match (action, hotkey) {
        (None, Some(hk)) => Ok(Command::Run(hk)),
        (None, None) => Ok(Command::Run(default_hotkey())),
        (Some(cmd), None) => Ok(cmd),
        (Some(_), Some(_)) => Err(ArgError::Conflict(
            "-hotkey cannot be combined with another command",
        )),
    }
}

fn set_action(slot: &mut Option<Command>, cmd: Command) -> Result<(), ArgError> {
    if slot.is_some() {
        return Err(ArgError::Conflict("more than one command given"));
    }
    *slot = Some(cmd);
    Ok(())
}

/// `ctrl+alt+m` → `Hotkey { mods: MOD_CONTROL|MOD_ALT, vk: 0x4D }`.
///
/// Requires at least one modifier: registering a bare `m` globally would
/// swallow every keystroke in the session.
pub fn parse_hotkey(spec: &str) -> Result<Hotkey, HotkeyError> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(HotkeyError::Empty);
    }

    let mut mods = 0u32;
    let mut key: Option<(String, u32)> = None;

    for raw in spec.split('+') {
        let token = raw.trim().to_ascii_lowercase();
        if token.is_empty() {
            return Err(HotkeyError::Unknown(raw.to_string()));
        }
        let bit = match token.as_str() {
            "ctrl" | "control" | "ctl" => Some(MOD_CONTROL),
            "alt" => Some(MOD_ALT),
            "shift" => Some(MOD_SHIFT),
            "win" | "windows" | "meta" | "super" => Some(MOD_WIN),
            _ => None,
        };
        if let Some(bit) = bit {
            if mods & bit != 0 {
                return Err(HotkeyError::Duplicate(token));
            }
            mods |= bit;
            continue;
        }
        if let Some(vk) = key_code(&token) {
            match &key {
                Some((prev, _)) => return Err(HotkeyError::TooManyKeys(format!("{prev}+{token}"))),
                None => key = Some((token, vk)),
            }
            continue;
        }
        return Err(HotkeyError::Unknown(raw.to_string()));
    }

    let (_, vk) = key.ok_or(HotkeyError::NoKey)?;
    if mods == 0 {
        return Err(HotkeyError::NoModifier);
    }
    Ok(Hotkey::new(mods, vk))
}

/// Single key token → virtual-key code, or `None` if unknown.
fn key_code(token: &str) -> Option<u32> {
    let mut chars = token.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_lowercase() {
            return Some(c.to_ascii_uppercase() as u32);
        }
        if c.is_ascii_digit() {
            return Some(c as u32);
        }
    }
    if let Some(n) = token.strip_prefix('f') {
        if let Ok(num) = n.parse::<u32>() {
            if (1..=24).contains(&num) {
                return Some(0x6F + num); // VK_F1 = 0x70
            }
        }
    }
    Some(match token {
        "space" => 0x20,
        "tab" => 0x09,
        "enter" | "return" => 0x0D,
        "esc" | "escape" => 0x1B,
        "backspace" => 0x08,
        "insert" | "ins" => 0x2D,
        "delete" | "del" => 0x2E,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "pause" => 0x13,
        "printscreen" | "prtsc" => 0x2C,
        "scrolllock" | "scroll" => 0x91,
        "numlock" => 0x90,
        "capslock" => 0x14,
        "minus" => 0xBD,
        "equals" => 0xBB,
        "comma" => 0xBC,
        "period" => 0xBE,
        "slash" => 0xBF,
        "semicolon" => 0xBA,
        _ => return None,
    })
}

fn vk_name(vk: u32) -> String {
    match vk {
        0x20 => "Space".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x1B => "Esc".into(),
        0x08 => "Backspace".into(),
        0x2D => "Insert".into(),
        0x2E => "Delete".into(),
        0x24 => "Home".into(),
        0x23 => "End".into(),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x13 => "Pause".into(),
        0x2C => "PrintScreen".into(),
        0x91 => "ScrollLock".into(),
        0x90 => "NumLock".into(),
        0x14 => "CapsLock".into(),
        0xBD => "Minus".into(),
        0xBB => "Equals".into(),
        0xBC => "Comma".into(),
        0xBE => "Period".into(),
        0xBF => "Slash".into(),
        0xBA => "Semicolon".into(),
        0x41..=0x5A => char::from_u32(vk).unwrap_or('?').to_string(),
        0x30..=0x39 => char::from_u32(vk).unwrap_or('?').to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => format!("VK_{vk:02X}"),
    }
}

/// Canonical display form, e.g. `Ctrl+Alt+M`.
pub fn format_hotkey(hk: &Hotkey) -> String {
    let mut out = String::new();
    if hk.mods & MOD_CONTROL != 0 {
        out.push_str("Ctrl+");
    }
    if hk.mods & MOD_ALT != 0 {
        out.push_str("Alt+");
    }
    if hk.mods & MOD_SHIFT != 0 {
        out.push_str("Shift+");
    }
    if hk.mods & MOD_WIN != 0 {
        out.push_str("Win+");
    }
    out.push_str(&vk_name(hk.vk));
    out
}

/// Command line stored in the Run key: quoted, because `C:\Program Files\...`
/// contains spaces. Returns an error instead of writing something bogus.
pub fn run_value_data(exe: &str) -> Result<String, String> {
    let exe = exe.trim();
    if exe.is_empty() {
        return Err("empty executable path".into());
    }
    if exe.contains('"') {
        return Err("executable path contains a quote".into());
    }
    Ok(format!("\"{exe}\""))
}

/// Human-readable form of the Run entry, used by `-selftest` (never written).
pub fn run_entry_display(exe: &str) -> Result<String, String> {
    let data = run_value_data(exe)?;
    Ok(format!("HKCU\\{RUN_KEY}\\{RUN_VALUE} = {data}"))
}

pub const USAGE: &str = "\
turnoffmonitor {version} — switch the displays off, with one hotkey.

USAGE:
  turnoffmonitor                  tray icon + global hotkey (Ctrl+Alt+M)
  turnoffmonitor -hotkey <spec>   tray icon with a custom hotkey for this session
  turnoffmonitor -off             turn the monitors off, then exit
  turnoffmonitor -lock            lock the workstation, then turn the monitors off
  turnoffmonitor -toggle          toggle monitors off/on (best effort)
  turnoffmonitor -install         start with Windows (HKCU Run entry)
  turnoffmonitor -uninstall       remove the start-with-Windows entry
  turnoffmonitor -selftest        run the self test, print PASS/FAIL lines
  turnoffmonitor -version         print the version

HOTKEY SPEC:
  modifiers: ctrl, alt, shift, win   key: a-z, 0-9, f1-f24, space, tab,
  enter, esc, insert, delete, home, end, pgup, pgdn, arrows, ...
  example: -hotkey ctrl+shift+f9

No installer, no admin rights, no network. Exit codes: 0 ok, 1 failure, 2 usage.
";

pub fn usage() -> String {
    USAGE.replace("{version}", env!("CARGO_PKG_VERSION"))
}

/// Split a Windows command line (`GetCommandLineW`) into arguments.
///
/// Handles double quotes, which is what matters for `"C:\Program Files\..."`.
/// Backslash escapes follow the common subset of the `CommandLineToArgvW`
/// rules; the full 2n-backslash rule is not implemented because no argument we
/// accept can contain one.
pub fn split_command_line(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut started = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                started = true;
            }
            '\\' if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
                started = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if started {
                    args.push(core::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(cur);
    }
    args
}

// ---------------------------------------------------------------------------
// Selftest tables: the same cases the unit tests assert on.
// ---------------------------------------------------------------------------

pub struct TableCheck {
    pub cases: usize,
    pub failures: Vec<String>,
}

fn expect<T: PartialEq + fmt::Debug>(
    cases: &mut usize,
    fails: &mut Vec<String>,
    name: &str,
    got: T,
    want: T,
) {
    *cases += 1;
    if got != want {
        fails.push(format!("{name}: got {got:?}, want {want:?}"));
    }
}

pub fn check_parse_table() -> TableCheck {
    let mut cases = 0;
    let mut failures = Vec::new();
    let arg = |s: &str| vec![s.to_string()];
    let dflt = default_hotkey();

    expect(
        &mut cases,
        &mut failures,
        "no args",
        parse_args(&[]),
        Ok(Command::Run(dflt)),
    );
    expect(
        &mut cases,
        &mut failures,
        "-off",
        parse_args(&arg("-off")),
        Ok(Command::Off),
    );
    expect(
        &mut cases,
        &mut failures,
        "-lock",
        parse_args(&arg("-lock")),
        Ok(Command::Lock),
    );
    expect(
        &mut cases,
        &mut failures,
        "-toggle",
        parse_args(&arg("-toggle")),
        Ok(Command::Toggle),
    );
    expect(
        &mut cases,
        &mut failures,
        "-install",
        parse_args(&arg("-install")),
        Ok(Command::Install),
    );
    expect(
        &mut cases,
        &mut failures,
        "-uninstall",
        parse_args(&arg("-uninstall")),
        Ok(Command::Uninstall),
    );
    expect(
        &mut cases,
        &mut failures,
        "-version",
        parse_args(&arg("-version")),
        Ok(Command::Version),
    );
    expect(
        &mut cases,
        &mut failures,
        "-selftest",
        parse_args(&arg("-selftest")),
        Ok(Command::Selftest),
    );
    expect(
        &mut cases,
        &mut failures,
        "/OFF",
        parse_args(&arg("/OFF")),
        Ok(Command::Off),
    );
    expect(
        &mut cases,
        &mut failures,
        "-hotkey ctrl+shift+f9",
        parse_args(&["-hotkey".into(), "ctrl+shift+f9".into()]),
        Ok(Command::Run(Hotkey::new(MOD_CONTROL | MOD_SHIFT, 0x78))),
    );
    expect(
        &mut cases,
        &mut failures,
        "-hotkey missing",
        parse_args(&arg("-hotkey")),
        Err(ArgError::MissingValue("-hotkey <spec>")),
    );
    expect(
        &mut cases,
        &mut failures,
        "-hotkey bad",
        parse_args(&["-hotkey".into(), "ctrl+nope".into()]),
        Err(ArgError::BadHotkey("unknown token 'nope'".into())),
    );
    expect(
        &mut cases,
        &mut failures,
        "-hotkey + -off",
        parse_args(&["-hotkey".into(), "ctrl+alt+m".into(), "-off".into()]),
        Err(ArgError::Conflict(
            "-hotkey cannot be combined with another command",
        )),
    );
    expect(
        &mut cases,
        &mut failures,
        "two commands",
        parse_args(&["-off".into(), "-lock".into()]),
        Err(ArgError::Conflict("more than one command given")),
    );
    expect(
        &mut cases,
        &mut failures,
        "unknown",
        parse_args(&arg("-frobnicate")),
        Err(ArgError::Unknown("-frobnicate".into())),
    );

    TableCheck { cases, failures }
}

pub fn check_hotkey_table() -> TableCheck {
    let mut cases = 0;
    let mut failures = Vec::new();

    let ok = |cases: &mut usize, fails: &mut Vec<String>, spec: &str, want: Hotkey| {
        expect(cases, fails, spec, parse_hotkey(spec), Ok(want));
    };
    let err = |cases: &mut usize, fails: &mut Vec<String>, spec: &str, want: HotkeyError| {
        expect(cases, fails, spec, parse_hotkey(spec), Err(want));
    };

    ok(
        &mut cases,
        &mut failures,
        "ctrl+alt+m",
        Hotkey::new(MOD_CONTROL | MOD_ALT, 0x4D),
    );
    ok(
        &mut cases,
        &mut failures,
        "CTRL+ALT+M",
        Hotkey::new(MOD_CONTROL | MOD_ALT, 0x4D),
    );
    ok(
        &mut cases,
        &mut failures,
        "win+shift+9",
        Hotkey::new(MOD_WIN | MOD_SHIFT, 0x39),
    );
    ok(
        &mut cases,
        &mut failures,
        "alt+f12",
        Hotkey::new(MOD_ALT, 0x7B),
    );
    ok(
        &mut cases,
        &mut failures,
        "ctrl+pagedown",
        Hotkey::new(MOD_CONTROL, 0x22),
    );
    ok(
        &mut cases,
        &mut failures,
        "ctrl + alt + space",
        Hotkey::new(MOD_CONTROL | MOD_ALT, 0x20),
    );
    err(&mut cases, &mut failures, "", HotkeyError::Empty);
    err(&mut cases, &mut failures, "ctrl+alt", HotkeyError::NoKey);
    err(&mut cases, &mut failures, "m", HotkeyError::NoModifier);
    err(
        &mut cases,
        &mut failures,
        "ctrl+alt+nope",
        HotkeyError::Unknown("nope".into()),
    );
    err(
        &mut cases,
        &mut failures,
        "ctrl+a+b",
        HotkeyError::TooManyKeys("a+b".into()),
    );
    err(
        &mut cases,
        &mut failures,
        "ctrl+ctrl+a",
        HotkeyError::Duplicate("ctrl".into()),
    );
    err(
        &mut cases,
        &mut failures,
        "ctrl+f25",
        HotkeyError::Unknown("f25".into()),
    );

    TableCheck { cases, failures }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dflt() -> Hotkey {
        Hotkey::new(MOD_CONTROL | MOD_ALT, 0x4D)
    }

    #[test]
    fn default_hotkey_parses() {
        assert_eq!(default_hotkey(), dflt());
        assert_eq!(format_hotkey(&default_hotkey()), "Ctrl+Alt+M");
    }

    #[test]
    fn no_args_means_resident_with_default_hotkey() {
        assert_eq!(parse_args(&[]), Ok(Command::Run(dflt())));
    }

    #[test]
    fn single_commands_parse() {
        let cases = [
            ("-off", Command::Off),
            ("-lock", Command::Lock),
            ("-toggle", Command::Toggle),
            ("-install", Command::Install),
            ("-uninstall", Command::Uninstall),
            ("-version", Command::Version),
            ("-selftest", Command::Selftest),
            ("-help", Command::Help),
        ];
        for (arg, want) in cases {
            assert_eq!(parse_args(&[arg.to_string()]), Ok(want), "arg {arg}");
        }
    }

    #[test]
    fn slash_and_case_are_accepted() {
        assert_eq!(parse_args(&["/OFF".into()]), Ok(Command::Off));
        assert_eq!(parse_args(&["-VERSION".into()]), Ok(Command::Version));
    }

    #[test]
    fn unknown_argument_is_rejected_with_the_original_text() {
        assert_eq!(
            parse_args(&["--wat".into()]),
            Err(ArgError::Unknown("--wat".into()))
        );
    }

    #[test]
    fn hotkey_alone_selects_resident_mode() {
        assert_eq!(
            parse_args(&["-hotkey".into(), "win+shift+9".into()]),
            Ok(Command::Run(Hotkey::new(MOD_WIN | MOD_SHIFT, 0x39)))
        );
    }

    #[test]
    fn hotkey_without_value_is_a_usage_error() {
        assert_eq!(
            parse_args(&["-hotkey".into()]),
            Err(ArgError::MissingValue("-hotkey <spec>"))
        );
    }

    #[test]
    fn hotkey_and_command_conflict() {
        assert_eq!(
            parse_args(&["-off".into(), "-hotkey".into(), "ctrl+alt+m".into()]),
            Err(ArgError::Conflict(
                "-hotkey cannot be combined with another command"
            ))
        );
    }

    #[test]
    fn two_commands_conflict() {
        assert_eq!(
            parse_args(&["-off".into(), "-lock".into()]),
            Err(ArgError::Conflict("more than one command given"))
        );
    }

    #[test]
    fn repeated_hotkey_flag_conflicts() {
        assert_eq!(
            parse_args(&[
                "-hotkey".into(),
                "ctrl+alt+m".into(),
                "-hotkey".into(),
                "ctrl+alt+b".into()
            ]),
            Err(ArgError::Conflict("-hotkey given more than once"))
        );
    }

    #[test]
    fn letter_digit_and_function_keys_map_to_vk_codes() {
        assert_eq!(parse_hotkey("ctrl+alt+m"), Ok(dflt()));
        assert_eq!(parse_hotkey("shift+7"), Ok(Hotkey::new(MOD_SHIFT, 0x37)));
        assert_eq!(parse_hotkey("alt+f1"), Ok(Hotkey::new(MOD_ALT, 0x70)));
        assert_eq!(parse_hotkey("alt+f24"), Ok(Hotkey::new(MOD_ALT, 0x87)));
    }

    #[test]
    fn named_keys_map_to_vk_codes() {
        assert_eq!(
            parse_hotkey("ctrl+pageup"),
            Ok(Hotkey::new(MOD_CONTROL, 0x21))
        );
        assert_eq!(
            parse_hotkey("ctrl+escape"),
            Ok(Hotkey::new(MOD_CONTROL, 0x1B))
        );
        assert_eq!(parse_hotkey("win+space"), Ok(Hotkey::new(MOD_WIN, 0x20)));
    }

    #[test]
    fn modifier_aliases_are_equivalent() {
        assert_eq!(parse_hotkey("control+alt+m"), Ok(dflt()));
        assert_eq!(
            parse_hotkey("windows+alt+m"),
            Ok(Hotkey::new(MOD_WIN | MOD_ALT, 0x4D))
        );
    }

    #[test]
    fn hotkey_spec_errors_are_specific() {
        assert_eq!(parse_hotkey(""), Err(HotkeyError::Empty));
        assert_eq!(parse_hotkey("   "), Err(HotkeyError::Empty));
        assert_eq!(parse_hotkey("ctrl+alt"), Err(HotkeyError::NoKey));
        assert_eq!(parse_hotkey("m"), Err(HotkeyError::NoModifier));
        assert_eq!(
            parse_hotkey("ctrl+nope"),
            Err(HotkeyError::Unknown("nope".into()))
        );
        assert_eq!(
            parse_hotkey("ctrl+a+b"),
            Err(HotkeyError::TooManyKeys("a+b".into()))
        );
        assert_eq!(
            parse_hotkey("ctrl+ctrl+a"),
            Err(HotkeyError::Duplicate("ctrl".into()))
        );
        assert_eq!(
            parse_hotkey("ctrl+f25"),
            Err(HotkeyError::Unknown("f25".into()))
        );
        assert_eq!(parse_hotkey("ctrl+"), Err(HotkeyError::Unknown("".into())));
    }

    #[test]
    fn format_hotkey_uses_canonical_order() {
        let hk = Hotkey::new(MOD_WIN | MOD_SHIFT | MOD_ALT | MOD_CONTROL, 0x70);
        assert_eq!(format_hotkey(&hk), "Ctrl+Alt+Shift+Win+F1");
        assert_eq!(format_hotkey(&Hotkey::new(MOD_ALT, 0x2E)), "Alt+Delete");
    }

    #[test]
    fn format_hotkey_falls_back_to_vk_hex() {
        assert_eq!(format_hotkey(&Hotkey::new(MOD_ALT, 0xE5)), "Alt+VK_E5");
    }

    #[test]
    fn format_roundtrips_through_parse() {
        for spec in ["ctrl+alt+m", "win+shift+f9", "ctrl+pgdn", "alt+space"] {
            let hk = parse_hotkey(spec).unwrap();
            let shown = format_hotkey(&hk);
            assert_eq!(parse_hotkey(&shown), Ok(hk), "spec {spec} -> {shown}");
        }
    }

    #[test]
    fn run_value_is_quoted_and_preserves_spaces() {
        assert_eq!(
            run_value_data(r"C:\Program Files\Turn Off Monitor\turnoffmonitor.exe").unwrap(),
            r#""C:\Program Files\Turn Off Monitor\turnoffmonitor.exe""#
        );
    }

    #[test]
    fn run_value_rejects_empty_and_quoted_paths() {
        assert!(run_value_data("").is_err());
        assert!(run_value_data("  ").is_err());
        assert!(run_value_data("C:\\bad\"path.exe").is_err());
    }

    #[test]
    fn run_entry_display_matches_the_registry_location() {
        let shown = run_entry_display(r"C:\tools\turnoffmonitor.exe").unwrap();
        assert_eq!(
            shown,
            r#"HKCU\Software\Microsoft\Windows\CurrentVersion\Run\TurnOffMonitor = "C:\tools\turnoffmonitor.exe""#
        );
        assert!(APP_KEY.starts_with("Software\\"));
        assert_eq!(STATE_VALUE, "DisplayOff");
    }

    #[test]
    fn usage_lists_every_documented_flag() {
        let text = usage();
        for flag in [
            "-off",
            "-lock",
            "-toggle",
            "-hotkey",
            "-install",
            "-uninstall",
            "-version",
            "-selftest",
        ] {
            assert!(text.contains(flag), "usage misses {flag}");
        }
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn split_command_line_basics() {
        assert_eq!(split_command_line("a.exe -off"), vec!["a.exe", "-off"]);
        assert_eq!(split_command_line(""), Vec::<String>::new());
        assert_eq!(split_command_line("   "), Vec::<String>::new());
        assert_eq!(
            split_command_line(r#""C:\Program Files\a.exe" -off"#),
            vec![r"C:\Program Files\a.exe", "-off"]
        );
        assert_eq!(split_command_line("a\tb  c"), vec!["a", "b", "c"]);
        assert_eq!(split_command_line(r#"a\b"#), vec![r"a\b"]);
    }

    #[test]
    fn selftest_tables_pass() {
        let a = check_parse_table();
        assert!(a.cases >= 10, "only {} arg cases", a.cases);
        assert!(a.failures.is_empty(), "{:?}", a.failures);
        let h = check_hotkey_table();
        assert!(h.cases >= 10, "only {} hotkey cases", h.cases);
        assert!(h.failures.is_empty(), "{:?}", h.failures);
    }
}
