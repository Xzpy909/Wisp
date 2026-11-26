//! The running app: hidden popup, tray icon, global shortcut.

use std::cell::{Cell, RefCell};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{
    ComponentHandle, Image, PhysicalPosition, PhysicalSize, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode,
    VecModel,
};
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
use wisp_core::clip::{now_secs, relative_time};
use wisp_core::{images, popup_rect, Clip, Kind, PopupStyle, Settings, Store};
use wisp_win::clipboard::{self, CaptureOptions};
use wisp_win::{autostart, instance, screen, window, Hwnd};

/// Most clips the popup lists at once.
const LIST_LIMIT: usize = 200;

/// Retention limits shared with the clipboard thread.
struct Retention {
    keep_days: AtomicU32,
    max_clips: AtomicU32,
}

slint::include_modules!();

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Run `f` against the app state on the UI thread.
fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(app) = guard.as_mut() {
                f(app);
            }
        }
    });
}

/// Queue `f` onto the UI thread from any thread.
fn on_ui(f: impl FnOnce(&mut App) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || with_app(f));
}

struct App {
    popup: Popup,
    settings: Settings,
    settings_path: PathBuf,
    tray: crate::tray::Tray,
    hotkeys: GlobalHotKeyManager,
    hotkey: Option<HotKey>,
    hwnd: Option<Hwnd>,
    /// The window that was active before the popup opened; paste goes there.
    previous: Hwnd,
    focus_watch: Timer,
    store: Option<Store>,
    capture: Arc<CaptureOptions>,
    retention: Arc<Retention>,
    thumbs: HashMap<i64, Image>,
}

impl App {
    fn is_visible(&self) -> bool {
        self.popup.window().is_visible()
    }

    fn toggle(&mut self) {
        if self.is_visible() {
            self.hide(true);
        } else {
            self.show();
        }
    }

    fn show(&mut self) {
        if self.is_visible() {
            return;
        }
        self.previous = window::foreground_window();
        let style = self.settings.popup_style;

        let anchor = match style {
            PopupStyle::BeamCursor => screen::caret_pos().or_else(screen::cursor_pos),
            _ => None,
        };
        // The popup opens on the monitor you are working on: the caret's or
        // pointer's for Beam at cursor, otherwise the active window's.
        let monitor = anchor
            .and_then(screen::monitor_at)
            .or_else(|| screen::monitor_of_window(self.previous))
            .or_else(|| screen::cursor_pos().and_then(screen::monitor_at));
        let Some(mon) = monitor else { return };
        let r = popup_rect(style, mon.bounds, mon.work, anchor, mon.scale);

        self.popup.set_drawer(style == PopupStyle::Drawer);
        self.popup.set_query("".into());
        self.popup.set_page(0);
        self.popup.set_selected(0);
        self.popup.set_dark(window::apps_use_dark_theme());
        self.settings_to_ui();
        self.refresh();

        self.prewarm();
        let w = self.popup.window();
        w.set_position(PhysicalPosition::new(r.x, r.y));
        w.set_size(PhysicalSize::new(r.w as u32, r.h as u32));
        if self.popup.show().is_err() {
            return;
        }
        // Position again: moving between monitors with different DPI can
        // make the toolkit adjust the size on the way.
        w.set_position(PhysicalPosition::new(r.x, r.y));
        w.set_size(PhysicalSize::new(r.w as u32, r.h as u32));

        if let Some(h) = self.hwnd {
            window::prepare_popup(h, self.popup.get_dark());
            window::bring_to_front(h);
            self.watch_focus(h);
        }
        self.popup.invoke_focus_search();
    }

    /// Create the native window once, off screen, and style it (tool window,
    /// rounded corners, acrylic) before it is ever visible. Without this the
    /// very first open showed an opaque grey frame instead of the glass.
    fn prewarm(&mut self) {
        if self.hwnd.is_some() {
            return;
        }
        let software = match std::env::var("SLINT_BACKEND") {
            Ok(b) => b.contains("software"),
            Err(_) => !self.settings.glass_effect,
        };
        self.popup.set_translucent(!software);
        let w = self.popup.window();
        w.set_position(PhysicalPosition::new(-32000, -32000));
        w.set_size(PhysicalSize::new(64, 64));
        if self.popup.show().is_err() {
            return;
        }
        self.hwnd = native_handle(&self.popup);
        if let Some(h) = self.hwnd {
            window::prepare_popup(h, window::apps_use_dark_theme());
            let acrylic = !software && window::apply_acrylic(h);
            self.popup.set_translucent(acrylic);
        }
        let _ = self.popup.hide();
    }

    /// Hide when the popup loses focus, like a menu. Polls only while the
    /// popup is open, so Wisp stays at zero CPU when idle.
    fn watch_focus(&mut self, hwnd: Hwnd) {
        let mut seen_focus = false;
        self.focus_watch.start(TimerMode::Repeated, Duration::from_millis(120), move || {
            let fg = window::foreground_window();
            if fg == hwnd {
                seen_focus = true;
            } else if seen_focus {
                on_ui(|app| app.hide(false));
            }
        });
    }

    fn hide(&mut self, restore_focus: bool) {
        self.focus_watch.stop();
        let _ = self.popup.hide();
        if restore_focus {
            window::restore_focus(self.previous);
        }
    }

    fn on_menu(&mut self, id: &tray_icon::menu::MenuId) {
        use crate::tray::Action;
        match self.tray.action_for(id) {
            Some(Action::Open) => self.show(),
            Some(Action::Style(s)) => {
                self.settings.popup_style = s;
                self.save();
            }
            Some(Action::ToggleAutostart) => {
                self.settings.start_with_windows = !self.settings.start_with_windows;
                apply_autostart(&self.settings);
                self.save();
            }
            Some(Action::ToggleGlass) => {
                self.settings.glass_effect = !self.settings.glass_effect;
                self.save();
                self.restart();
            }
            Some(Action::TogglePause) => {
                let paused = !self.capture.paused.load(Ordering::Relaxed);
                self.capture.paused.store(paused, Ordering::Relaxed);
            }
            Some(Action::Quit) => self.quit(),
            None => {}
        }
        self.sync_tray();
    }

    fn sync_tray(&self) {
        self.tray.sync(&self.settings, self.capture.paused.load(Ordering::Relaxed));
    }

    /// The renderer is chosen once at startup, so switching glass restarts Wisp.
    fn restart(&mut self) {
        let spawned = std::env::current_exe()
            .and_then(|exe| std::process::Command::new(exe).args(["--background", "--restart"]).spawn());
        match spawned {
            Ok(_) => self.quit(),
            Err(e) => window::warn("Wisp", &format!("Could not restart Wisp:\n{e}\n\nQuit and start it again.")),
        }
    }

    /// Reload the clip list for the current search text.
    fn refresh(&mut self) {
        let Some(store) = self.store.as_ref() else { return };
        let query = self.popup.get_query().to_string();
        let clips = store.search(&query, LIST_LIMIT).unwrap_or_default();
        let total = store.count().unwrap_or(0);
        let now = now_secs();
        if self.thumbs.len() > 4 * LIST_LIMIT {
            self.thumbs.clear();
        }
        let rows: Vec<ClipRow> = clips
            .iter()
            .map(|c| clip_row(c, now, store, &mut self.thumbs))
            .collect();
        let shown = rows.len();
        self.popup.set_clips(Rc::new(VecModel::from(rows)).into());
        if self.popup.get_selected() as usize >= shown {
            self.popup.set_selected(0);
        }
        let label = match (query.trim().is_empty(), total) {
            (_, 0) => String::new(),
            (true, 1) => "1 clip".into(),
            (true, n) => format!("{n} clips"),
            (false, n) => format!("{shown} of {n}"),
        };
        self.popup.set_count_label(label.into());
    }

    /// A new clip was recorded on the clipboard thread.
    fn on_captured(&mut self) {
        if self.is_visible() && self.popup.get_page() == 0 {
            self.refresh();
        }
    }

    fn settings_to_ui(&self) {
        let s = &self.settings;
        let p = &self.popup;
        p.set_style_index(PopupStyle::ALL.iter().position(|x| *x == s.popup_style).unwrap_or(0) as i32);
        p.set_glass(s.glass_effect);
        p.set_autostart(s.start_with_windows);
        p.set_capture_images(s.capture_images);
        p.set_capture_files(s.capture_files);
        p.set_keep_days(s.keep_days as i32);
        p.set_max_clips(s.max_clips as i32);
        p.set_hotkey_text(s.hotkey.clone().into());
        p.set_hotkey_error("".into());
    }

    fn settings_from_ui(&mut self) {
        let p = &self.popup;
        let before = self.settings.clone();
        let s = &mut self.settings;
        s.popup_style = PopupStyle::ALL.get(p.get_style_index() as usize).copied().unwrap_or_default();
        s.start_with_windows = p.get_autostart();
        s.capture_images = p.get_capture_images();
        s.capture_files = p.get_capture_files();
        s.keep_days = p.get_keep_days().max(0) as u32;
        s.max_clips = p.get_max_clips().max(0) as u32;
        self.apply_capture_settings();
        if before.start_with_windows != self.settings.start_with_windows {
            apply_autostart(&self.settings);
        }
        if (before.keep_days, before.max_clips) != (self.settings.keep_days, self.settings.max_clips) {
            if let Some(store) = &self.store {
                let _ = store.apply_retention(self.settings.keep_days, self.settings.max_clips, now_secs());
            }
        }
        if before.popup_style != self.settings.popup_style && self.is_visible() {
            // Reopen in the new style so the change is visible at once.
            self.hide(false);
            self.show();
            self.popup.set_page(1);
        }
        self.save();
        self.sync_tray();
    }

    fn apply_capture_settings(&self) {
        self.capture.images.store(self.settings.capture_images, Ordering::Relaxed);
        self.capture.files.store(self.settings.capture_files, Ordering::Relaxed);
        self.retention.keep_days.store(self.settings.keep_days, Ordering::Relaxed);
        self.retention.max_clips.store(self.settings.max_clips, Ordering::Relaxed);
    }

    fn apply_hotkey(&mut self, text: &str) {
        let text = text.trim().to_string();
        let new: HotKey = match text.parse() {
            Ok(h) => h,
            Err(_) => {
                self.popup.set_hotkey_error("Not a shortcut Wisp understands. Try Alt+V or Ctrl+Shift+Space.".into());
                return;
            }
        };
        if self.hotkey.is_some_and(|h| h.id() == new.id()) {
            self.popup.set_hotkey_error("".into());
            return;
        }
        if let Some(old) = self.hotkey.take() {
            let _ = self.hotkeys.unregister(old);
            if self.hotkeys.register(new).is_err() {
                let _ = self.hotkeys.register(old);
                self.hotkey = Some(old);
                self.popup.set_hotkey_error(format!("Another app already uses {text}.").into());
                return;
            }
        } else if self.hotkeys.register(new).is_err() {
            self.popup.set_hotkey_error(format!("Another app already uses {text}.").into());
            return;
        }
        self.hotkey = Some(new);
        self.settings.hotkey = text.clone();
        self.save();
        self.tray.set_hotkey(&text);
        self.popup.set_hotkey_label(text.into());
        self.popup.set_hotkey_error("".into());
    }

    fn clear_history(&mut self) {
        if let Some(store) = &self.store {
            if let Err(e) = store.clear(true) {
                window::warn("Wisp", &format!("Could not clear history:\n{e}"));
            }
        }
        self.thumbs.clear();
        self.refresh();
    }

    fn quit(&mut self) {
        if let Some(hk) = self.hotkey.take() {
            let _ = self.hotkeys.unregister(hk);
        }
        let _ = slint::quit_event_loop();
    }

    fn save(&self) {
        if let Err(e) = self.settings.save(&self.settings_path) {
            window::warn("Wisp", &format!("Could not save settings:\n{e}"));
        }
    }
}

fn native_handle(popup: &Popup) -> Option<Hwnd> {
    let handle = popup.window().window_handle();
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

fn clip_row(c: &Clip, now: i64, store: &Store, thumbs: &mut HashMap<i64, Image>) -> ClipRow {
    let swatch = (c.kind == Kind::Color).then(|| parse_hex_color(&c.text)).flatten();
    let thumb = if c.kind == Kind::Image {
        if let Entry::Vacant(slot) = thumbs.entry(c.id) {
            let img = store
                .thumb_path(c)
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|b| images::decode_png(&b))
                .map(|r| Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&r.pixels, r.width, r.height)));
            if let Some(img) = img {
                slot.insert(img);
            }
        }
        thumbs.get(&c.id).cloned()
    } else {
        None
    };
    let mut meta = String::new();
    if c.pinned {
        meta.push_str("Pinned · ");
    }
    if let Some(app) = &c.source_app {
        meta.push_str(app);
        meta.push_str(" · ");
    }
    meta.push_str(&relative_time(c.created_at, now));
    ClipRow {
        kind: c.kind.as_str().into(),
        title: c.title().into(),
        meta: meta.into(),
        pinned: c.pinned,
        swatch: swatch.unwrap_or_default(),
        has_swatch: swatch.is_some(),
        has_thumb: thumb.is_some(),
        thumb: thumb.unwrap_or_default(),
    }
}

/// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
fn parse_hex_color(text: &str) -> Option<slint::Color> {
    let hex = text.trim().strip_prefix('#')?;
    let digits: Vec<u8> = hex.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
    let (r, g, b, a) = match digits.len() {
        3 | 4 => {
            let d = |i: usize| digits.get(i).map(|v| v * 17);
            (d(0)?, d(1)?, d(2)?, d(3).unwrap_or(255))
        }
        6 | 8 => {
            let d = |i: usize| digits.get(i * 2).zip(digits.get(i * 2 + 1)).map(|(h, l)| h * 16 + l);
            (d(0)?, d(1)?, d(2)?, d(3).unwrap_or(255))
        }
        _ => return None,
    };
    Some(slint::Color::from_argb_u8(a, r, g, b))
}

fn apply_autostart(settings: &Settings) {
    let result = if settings.start_with_windows {
        autostart::command_for_current_exe().and_then(|cmd| {
            // Rewrite only when it changed, e.g. after the exe moved.
            if autostart::current().as_deref() == Some(cmd.as_str()) {
                Ok(())
            } else {
                autostart::set(Some(&cmd))
            }
        })
    } else {
        autostart::set(None)
    };
    if let Err(e) = result {
        window::warn("Wisp", &format!("Could not change Start with Windows:\n{e}"));
    }
}

/// The software renderer keeps Wisp at a few MB; the GPU renderer is needed
/// only for the acrylic glass effect. `SLINT_BACKEND` still overrides both.
fn select_renderer(settings: &Settings) {
    if std::env::var_os("SLINT_BACKEND").is_some() {
        return;
    }
    let renderer = if settings.glass_effect { "femtovg" } else { "software" };
    let _ = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(renderer.into())
        .select();
}

fn register_hotkey(manager: &GlobalHotKeyManager, text: &str) -> Option<HotKey> {
    let hotkey: HotKey = match text.parse() {
        Ok(h) => h,
        Err(e) => {
            window::warn("Wisp", &format!("The shortcut \"{text}\" in settings.json is not valid ({e})."));
            return None;
        }
    };
    if let Err(e) = manager.register(hotkey) {
        window::warn(
            "Wisp",
            &format!(
                "Wisp could not register {text}, probably because another app uses it ({e}).\n\n\
                 Change \"hotkey\" in {} and restart Wisp. You can still open Wisp from the tray icon.",
                Settings::default_path().display()
            ),
        );
        return None;
    }
    Some(hotkey)
}

/// Record clipboard changes on a background thread with its own database
/// connection, then tell the UI thread.
fn start_capture(options: Arc<CaptureOptions>, retention: Arc<Retention>, data_dir: PathBuf) {
    let writer: RefCell<Option<Store>> = RefCell::new(None);
    let inserts = Cell::new(0u32);
    clipboard::start(options, move |cap| {
        let mut w = writer.borrow_mut();
        if w.is_none() {
            *w = Store::open(&data_dir).ok();
        }
        let Some(store) = w.as_ref() else { return };
        let now = now_secs();
        if store.insert(&cap, now).is_err() {
            return;
        }
        inserts.set(inserts.get() + 1);
        if inserts.get().is_multiple_of(25) {
            let _ = store.apply_retention(
                retention.keep_days.load(Ordering::Relaxed),
                retention.max_clips.load(Ordering::Relaxed),
                now,
            );
        }
        on_ui(|app| app.on_captured());
    });
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let single = match instance::acquire(has("--restart")) {
        instance::Instance::Primary(s) => s,
        instance::Instance::Secondary => return,
    };
    let background = has("--background");

    let settings_path = Settings::default_path();
    let (settings, first_run) = Settings::load(&settings_path);
    if first_run {
        let _ = settings.save(&settings_path);
    }
    apply_autostart(&settings);
    select_renderer(&settings);

    let data_dir = Settings::dir();
    let store = match Store::open(&data_dir) {
        Ok(s) => {
            let _ = s.apply_retention(settings.keep_days, settings.max_clips, now_secs());
            Some(s)
        }
        Err(e) => {
            window::warn("Wisp", &format!("Could not open clip history in {}:\n{e}", data_dir.display()));
            None
        }
    };

    let capture = Arc::new(CaptureOptions::default());
    let retention = Arc::new(Retention { keep_days: AtomicU32::new(0), max_clips: AtomicU32::new(0) });
    start_capture(capture.clone(), retention.clone(), data_dir);

    let popup = Popup::new().expect("create popup window");
    popup.set_hotkey_label(settings.hotkey.clone().into());
    popup.set_version(env!("CARGO_PKG_VERSION").into());
    popup.on_dismiss(|| with_app(|app| app.hide(true)));
    popup.on_query_edited(|_| with_app(|app| app.refresh()));
    popup.on_settings_changed(|| with_app(|app| app.settings_from_ui()));
    popup.on_glass_toggled(|| {
        with_app(|app| {
            app.settings.glass_effect = app.popup.get_glass();
            app.save();
            app.restart();
        })
    });
    popup.on_apply_hotkey(|t| with_app(|app| app.apply_hotkey(&t)));
    popup.on_clear_history(|| with_app(|app| app.clear_history()));
    popup.on_open_data_folder(|| {
        let _ = std::process::Command::new("explorer").arg(Settings::dir()).spawn();
    });

    let hotkeys = GlobalHotKeyManager::new().expect("hotkey manager");
    let hotkey = register_hotkey(&hotkeys, &settings.hotkey);
    GlobalHotKeyEvent::set_event_handler(Some(|e: GlobalHotKeyEvent| {
        if e.state == HotKeyState::Pressed {
            on_ui(|app| app.toggle());
        }
    }));

    let tray = crate::tray::Tray::new(&settings).expect("create tray icon");
    TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            on_ui(|app| app.show());
        }
    }));
    MenuEvent::set_event_handler(Some(Arc::new(|e: MenuEvent| {
        let id = e.id.clone();
        on_ui(move |app| app.on_menu(&id));
    })));

    single.on_activate(|| on_ui(|app| app.show()));

    APP.with(|cell| {
        *cell.borrow_mut() = Some(App {
            popup,
            settings,
            settings_path,
            tray,
            hotkeys,
            hotkey,
            hwnd: None,
            previous: 0,
            focus_watch: Timer::default(),
            store,
            capture,
            retention,
            thumbs: HashMap::new(),
        })
    });
    with_app(|app| app.apply_capture_settings());
    // Build the window ahead of time so the first open is styled and fast.
    on_ui(|app| app.prewarm());

    if !background {
        on_ui(|app| app.show());
    }

    slint::run_event_loop_until_quit().expect("event loop");
    APP.with(|cell| cell.borrow_mut().take());
    drop(single);
}
