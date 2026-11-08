<p align="center">
  <img src="docs/assets/icon.png" width="96" alt="Turn Off Monitor icon">
</p>

<h1 align="center">Turn Off Monitor</h1>

<p align="center"><b>One hotkey, monitors off. 52 KB, single exe, no installer, no runtime.</b></p>

The tools that already do this are 877 KB with an installer and last updated in 2019, or 119 KB with no
hotkey at all. This is a 52 KB Win32 binary that sits in the tray and listens for `Ctrl+Alt+M`.

## Download

**[TurnOffMonitor.exe — latest release](https://github.com/nullpointio/turnoffmonitor/releases/latest)**

- Windows 10/11, x64. Portable: run it, it stays in the tray.
- Unsigned — SmartScreen will warn once (`More info → Run anyway`). SHA-256 is on the release page.

## Use

| Command | What it does |
|---|---|
| *(no arguments)* | tray icon + global hotkey `Ctrl+Alt+M` |
| `-off` | turn monitors off, exit immediately |
| `-lock` | lock the workstation, then turn monitors off |
| `-toggle` | flip the session state — handy for a Stream Deck or a script |
| `-hotkey Ctrl+Alt+J` | register a different combo for this run |
| `-install` / `-uninstall` | add/remove the `HKCU\…\Run` entry (start with Windows) |
| `-selftest` | run the built-in checks and exit |
| `-version` | print the version |

Tray menu: **Turn off monitors · Turn off and lock · Start with Windows · Quit**.

Because `-off` and `-lock` exit immediately, both work as the target of a desktop shortcut, a scheduled
task, or a Stream Deck button — no tray process needed.

## How it works

It sends the documented `SC_MONITORPOWER` message that Windows itself uses for its power button:
`PostMessage(HWND_BROADCAST, WM_SYSCOMMAND, SC_MONITORPOWER, 2)`. No drivers, no admin rights, no kernel
components.

Monitors wake on the next mouse move or keypress, as Windows intends. There is no "stay off" lock — holding
the screen off requires intercepting input, which is what gets other tools flagged by antivirus. If you want
that, you don't want this tool.

## Size, honestly

| | Bytes |
|---|---|
| **This tool** | **52,736** |
| NirCmd (`monitor off`, no hotkey) | 119,808 |
| Sordum Monitor Off 1.1 (installer-free, frozen 2019) | 877,720 |

The binary is `no_std` Rust with a hand-declared entry point and no GUI framework — that is where the size
goes. It ships with a real `.ico` embedded as a PE resource (16/32/48/256 px), used for the window, taskbar
and tray.

## Build

```bash
cargo test
cargo build --release                                  # on Windows (MSVC)
cargo build --release --target x86_64-pc-windows-gnu   # cross-compile from Linux
```

Release builds come from GitHub Actions on `windows-latest`, which also runs `-selftest` against the real
binary and fails the build above 60 KB.

## Verification

- 25 unit tests (argument parsing, hotkey grammar, registry value formatting) on Linux.
- `-selftest` on Windows CI: hotkey registration, single-instance mutex, the broadcast call, and a registry
  dry-run all must pass before a release is created.
- CI size gate: 60 KB.

**Not verified on physical hardware:** that the display actually goes dark, and how the tray icon renders —
Wine treats `SC_MONITORPOWER` as a no-op. If it fails on your machine, open an issue with your Windows build
and GPU driver.

## License

MIT — see [LICENSE](LICENSE).
