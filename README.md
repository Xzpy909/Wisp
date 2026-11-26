# Wisp

A light, keyboard-first clipboard manager for Windows. Wisp sits in the tray,
remembers what you copy (text, files, images) and opens a small search popup on
a global shortcut: type to filter, Enter to paste.

Built in Rust with a native [Slint](https://slint.dev) UI, no browser engine.

## Status

Milestones 1 to 3 and 5 of 7:

- Records every copy: text, files and folders, and images (PNG or bitmap),
  with the app it came from. Repeat copies move to the top instead of
  duplicating. Content that password managers mark as private is skipped.
- History in SQLite (`%APPDATA%\Wisp\wisp.db`), images and thumbnails as PNG
  files beside it. Old clips are removed after 30 days or beyond 5,000 clips
  (both adjustable); pinned clips are always kept.
- `Alt+V` opens the popup with the newest clips. Type to search: every word
  must appear, as a substring or as its letters in order ("inv pd" finds
  "Invoice paid"), best matches first. The first 4,096 characters of each clip
  are searched.
- Enter pastes the selected clip into the app you were in; the clip moves to
  the top of the history
- Settings inside the popup (gear button next to close): popup style, glass
  effect, shortcut, history limits, what to record, Start with Windows, clear
  history
- Tray icon with menu, including Pause recording
- Three popup styles: Beam at the centre of the screen, Beam at the text
  cursor, and Drawer by the tray
- Opaque popup by default (about 4 MB of memory); optional acrylic glass on
  Windows 11 (about 40 MB, as it needs the GPU renderer)
- Start with Windows (per-user, no admin), on by default
- Single instance: launching Wisp again opens the popup

Next: installer, update check and first release (milestone 7).

## Backup and restore

Settings → **Backup and restore…** (also in the tray menu) saves all clips,
pinned ones and images included, under a name you choose, for example one set
of snippets per project. Each backup is a single `.wispclips` file in
`%APPDATA%\Wisp\Backups`; copy it to another computer's folder to move it.

Restoring replaces all current clips with the backup. Wisp first saves the
clips as they were to `Backups\Safety snapshots` (the newest 5 are kept), so a
restore can be undone by restoring the snapshot.

A `.wispclips` file is a SQLite database: the `clips` and `clip_files` tables
as in `wisp.db`, `clip_images` with the PNG and thumbnail as blobs, and a
`meta` table with the backup's name, time and format version.

## Keys in the popup

| Key | Action |
| --- | --- |
| Type | Search |
| ↑ ↓ | Move the selection |
| Enter, double-click | Paste into the app you were in |
| Shift+Enter | Paste as plain text (files as their paths) |
| Shift+click, Ctrl+click, Shift+↑↓ | Select several clips; Enter pastes them together (text one per line, files as files) and Ctrl+P, Ctrl+Delete act on all of them |
| Right-click | Menu: Paste, Paste as plain text, Pin or Unpin, Show in File Explorer, Forget |
| Tab, Shift+Tab | Show only text, links, images, files, colours, code or pinned clips |
| Ctrl+P | Pin or unpin; pinned clips stay on top and are never removed |
| Ctrl+Delete | Forget the clip |
| Ctrl+O | Show a file or image clip in File Explorer |
| Esc | Clear the search, then the filter, then close |

## Build and run

Requires Rust (stable) on Windows.

```powershell
cargo run -p wisp-app            # debug build, with a console for logs
cargo build --release -p wisp-app
.\target\release\wisp.exe        # add --background to start hidden in the tray
```

Every push also builds `wisp.exe` in GitHub Actions; download it from the run's
artifacts.

### Memory

Read **Memory (active private working set)** for `wisp.exe` in Task Manager's
Details tab after opening and closing the popup once.

| Mode | Renderer | Idle memory (measured) |
| --- | --- | --- |
| Default | Slint software renderer | about 4 MB |
| Glass effect on | Slint femtovg (OpenGL) | about 40 MB |

Most of the difference is the GPU driver loaded into the process. The tray
menu's **Glass effect** item switches modes and restarts Wisp. `SLINT_BACKEND`
(for example `winit-software` or `winit-femtovg`) overrides the setting.

## Settings

Open the popup and click the gear next to the close button. Settings are
saved to `%APPDATA%\Wisp\settings.json`:

```json
{
  "popup-style": "beam-centre",
  "hotkey": "Alt+V",
  "start-with-windows": true,
  "glass-effect": false,
  "keep-days": 30,
  "max-clips": 5000,
  "capture-images": true,
  "capture-files": true
}
```

## Layout

```
crates/wisp-core   clip storage, search, backups, images, placement, settings; no Windows APIs, unit-tested anywhere
crates/wisp-win    Win32: clipboard listener and paste, single instance, autostart, monitors, caret, focus
crates/wisp-app    the executable: tray, hotkey, popup wiring
ui/                Slint UI
docs/design/       interactive prototypes of the four popup concepts
```
