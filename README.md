<p align="center">
  <img src="docs/assets/icon.png" width="96" alt="Turn Off Monitor icon">
</p>

<h1 align="center">Turn Off Monitor — Windows hotkey to turn off your display</h1>

<p align="center"><b>Press <code>Ctrl+Alt+M</code> to turn off your monitor with a keyboard shortcut on Windows 10/11. 58 KB single exe, no installer, no runtime, no account.</b></p>

The tools that already do this are 877 KB with an installer and last updated in 2019, or 119 KB with no
hotkey at all. This is a 58 KB Win32 binary that sits in the tray and listens for `Ctrl+Alt+M`.

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

## Common questions

**Is there a keyboard shortcut to turn off my monitor in Windows?** Windows has no built-in hotkey. Turn Off Monitor adds one: `Ctrl+Alt+M` by default, or any combo you pass with `-hotkey`. It sends the same `SC_MONITORPOWER` message Windows uses for its power button.

**Can I turn off the monitor without putting the PC to sleep?** Yes. `-off` blanks the display and exits immediately, leaving Windows and your downloads running. Move the mouse or press a key to wake the display.

**Does it work with multiple monitors?** It broadcasts to the desktop, so Windows turns off all attached displays together; there is no per-monitor hotkey.

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
| **This tool** | **59,392** |
| NirCmd (`monitor off`, no hotkey) | 119,808 |
| Sordum Monitor Off 1.1 (installer-free, frozen 2019) | 877,720 |

(`59,392` is the stripped `x86_64-pc-windows-gnu` release build; CI prints the exact size of the MSVC
binary it ships and fails above 61,440 bytes.)

The binary is `no_std` Rust with a hand-declared entry point and no GUI framework — that is where the size
goes. It ships with a real `.ico` embedded as a PE resource (16/32/48/256 px), used for the window, taskbar
and tray.

## Footprint

Measured on real Windows 11 x64 (idle in the tray, hotkey registered, no menus open):

| | Working set | Private bytes | CPU over 60 s |
|---|---|---|---|
| `turnoffmonitor.exe` idle | 10,116 K | 1,072 K | 0 % |

That is close to the floor for a Win32 process: no runtime, no allocator warm-up, no timers except the
250 ms one-shot used when switching displays off.

**Method.** `turnoffmonitor -selftest` measures its own process and prints two lines, one before any test
runs and one after the idle window:

```
MEM startup: <working set> K working set, <private> K private
MEM idle-60s: 10116 K working set, 1072 K private (cpu=0 ms)
```

The second line is the measurement in the table above.

- Working set and private bytes come from `K32GetProcessMemoryInfo` (`WorkingSetSize` and
  `PagefileUsage`, the same "Commit size" Task Manager shows), sampled at startup and again after the
  idle window.
- CPU comes from `GetProcessTimes`: kernel + user time sampled before and after the idle window.
- The idle window is not a `Sleep`. The self-test creates the tray window, registers the hotkey and then
  blocks in the same `GetMessageW` pump the resident app uses for 60 s, ended by a one-shot `WM_TIMER`.
  That is the real idle path, so the CPU delta is the real idle cost.
- The run then asserts `mem-private` (under 3 MB), `cpu-idle` (exactly 0 ms), and `mem-working-set`
  (a positive number was reported). A failed assertion exits 1, so CI fails.

Wine numbers are indicative only: Wine adds its own runtime to the process, so `-selftest` under Wine
reports roughly 16,000 K working set / 3,000 K private at startup and fails `mem-private`. The gate is
enforced on Windows.

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
- Same run measures its own footprint: working set and private bytes at startup and after a 60 s idle
  window, asserting private bytes under 3 MB and a CPU delta of exactly 0 ms.
- CI size gate: 61,440 bytes.

**Not verified on physical hardware:** that the display actually goes dark, and how the tray icon renders —
Wine treats `SC_MONITORPOWER` as a no-op. If it fails on your machine, open an issue with your Windows build
and GPU driver.

## Archived copy

An independent, permanently addressable copy of this repository is kept by Software Heritage:

* snapshot `swh:1:snp:3d7d0a6f200e794e6c7e22256d5f1eca9fb0dd2d` — [https://archive.softwareheritage.org/swh:1:snp:3d7d0a6f200e794e6c7e22256d5f1eca9fb0dd2d;origin=https://github.com/nullpointio/turnoffmonitor](https://archive.softwareheritage.org/swh:1:snp:3d7d0a6f200e794e6c7e22256d5f1eca9fb0dd2d;origin=https://github.com/nullpointio/turnoffmonitor)

## License

MIT — see [LICENSE](LICENSE).
