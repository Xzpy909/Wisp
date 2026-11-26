# Wisp

A light, keyboard-first clipboard manager for Windows. Wisp sits in the tray,
remembers what you copy (text, files, images) and opens a small search popup on
a global shortcut: type to filter, Enter to paste.

Built in Rust with a native [Slint](https://slint.dev) UI, no browser engine.

## Status

Milestone 1 of 7, the skeleton:

- Tray icon with menu: open, popup style, Start with Windows, quit
- `Alt+V` opens and closes the popup; Esc closes it and returns focus
- Three popup styles: Beam at the centre of the screen, Beam at the text
  cursor, and Drawer by the tray (switch in the tray menu)
- Acrylic backdrop on Windows 11, solid panel on Windows 10
- Start with Windows (per-user, no admin), on by default
- Single instance: launching Wisp again opens the popup

Clipboard capture, search and paste arrive in milestones 2 and 3.

## Build and run

Requires Rust (stable) on Windows.

```powershell
cargo run -p wisp-app            # debug build, with a console for logs
cargo build --release -p wisp-app
.\target\release\wisp.exe        # add --background to start hidden in the tray
```

Every push also builds `wisp.exe` in GitHub Actions; download it from the run's
artifacts.

### Measuring memory

The milestone 1 goal is to measure idle memory. Start Wisp, open and close the
popup once, wait a few seconds, then read **Memory (active private working
set)** for `wisp.exe` in Task Manager's Details tab. Compare the two renderers:

```powershell
.\wisp.exe                                   # GPU renderer (femtovg), acrylic
$env:SLINT_BACKEND="winit-software"; .\wisp.exe   # software renderer, opaque
```

## Settings

`%APPDATA%\Wisp\settings.json`:

```json
{
  "popup-style": "beam-centre",
  "hotkey": "Alt+V",
  "start-with-windows": true
}
```

`popup-style` is `beam-centre`, `beam-cursor` or `drawer`. `hotkey` accepts
combinations like `Ctrl+Shift+Space`. Restart Wisp after editing the file; a
settings window comes in milestone 6.

## Layout

```
crates/wisp-core   placement and settings, no Windows APIs, unit-tested anywhere
crates/wisp-win    Win32 helpers: single instance, autostart, monitors, caret, focus
crates/wisp-app    the executable: tray, hotkey, popup wiring
ui/                Slint UI
docs/design/       interactive prototypes of the four popup concepts
```
