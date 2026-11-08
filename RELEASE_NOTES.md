# Turn Off Monitor v1.0.1 — version info in the exe

**One hotkey, monitors off. 56 KB, single exe, no installer, no runtime.**

Existing tools that do this are 880 KB with an installer (Sordum), 120 KB without a hotkey (NirCmd), or
last updated in 2019. This is a 56 KB Win32 binary that sits in the tray and listens for `Ctrl+Alt+M`.

## What changed in 1.0.1

- The exe now carries a PE `VERSIONINFO` resource, so Task Manager's **Description** column and
  Explorer's Details tab show a product name and version instead of `TurnOffMonitor.ex…`.
- Fields: `CompanyName=nullpointio`, `ProductName=Turn Off Monitor`, `FileDescription=Turn off all
  monitors with one hotkey`, `FileVersion`/`ProductVersion=1.0.1.0`, `OriginalFilename=TurnOffMonitor.exe`,
  `LegalCopyright=MIT License — nullpointio`.
- `Cargo.toml` was still at `0.1.0` while the tag said `v1.0.0`; it is now `1.0.1`, so `-version`
  and the resource agree. No behaviour change.

## Downloads

- `TurnOffMonitor.exe` — 57,344 bytes, Windows 10/11 x64. Portable: run it, and it stays in the tray.
- `TurnOffMonitor-v1.0.1-win-x64.zip` — the same file, zipped.
- `SHA256SUMS.txt` — `powershell -c "(Get-FileHash .\TurnOffMonitor.exe -Algorithm SHA256).Hash"`

## Use

| Command | What it does |
|---|---|
| run with no arguments | tray icon + global hotkey `Ctrl+Alt+M` |
| `-off` | turn monitors off, exit |
| `-lock` | lock the workstation, then turn monitors off |
| `-toggle` | flip the session state (useful for a Stream Deck / script) |
| `-hotkey Ctrl+Alt+J` | register a different combo for this run |
| `-install` / `-uninstall` | add/remove the HKCU Run entry (start with Windows) |
| `-selftest` | run the built-in checks and exit |
| `-version` | print the version |

Tray menu: Turn off monitors · Turn off and lock · Start with Windows · Quit.

## Notes

- No admin rights, no drivers, no keyboard hooks. It sends the standard `SC_MONITORPOWER` message, the same
  one Windows uses for its own power button.
- Monitors wake on the next mouse move or keypress, as Windows intends. There is no "stay off" lock — that
  requires intercepting input, which is exactly what makes other tools get flagged by antivirus.
- Multi-monitor: the broadcast turns off all displays together.
- Unsigned: SmartScreen will warn once (`More info → Run anyway`).

## Verification

- 25 unit tests on Linux (argument parsing, hotkey grammar, registry value formatting).
- On `windows-latest`: build, then `-selftest` runs the real binary — hotkey registration, mutex, the
  broadcast call, and a dry-run of the registry path all have to pass before a release is created.
- Size gate in CI: the build fails above 60 KB.

**Not verified on physical hardware:** that the display actually goes dark, and that the tray icon renders —
Wine treats `SC_MONITORPOWER` as a no-op. The API call is the one Windows documents for this; if it fails on
your machine, open an issue with your GPU/driver and Windows build.

MIT licensed.
