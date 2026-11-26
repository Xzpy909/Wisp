//! The running app: hidden popup, tray icon, global shortcut.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, PhysicalPosition, PhysicalSize, Timer, TimerMode};
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
use wisp_core::{popup_rect, PopupStyle, Settings};
use wisp_win::{autostart, instance, screen, window, Hwnd};

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
        self.popup.set_style_label(style.label().into());
        self.popup.set_query("".into());
        self.popup.set_dark(window::apps_use_dark_theme());

        let w = self.popup.window();
        w.set_position(PhysicalPosition::new(r.x, r.y));
        w.set_size(PhysicalSize::new(r.w as u32, r.h as u32));
        if self.popup.show().is_err() {
            return;
        }
        // The native window exists only after the first show.
        if self.hwnd.is_none() {
            self.hwnd = native_handle(&self.popup);
            if let Some(h) = self.hwnd {
                let software = std::env::var("SLINT_BACKEND").is_ok_and(|b| b.contains("software"));
                let acrylic = !software && window::apply_acrylic(h);
                self.popup.set_translucent(acrylic);
            }
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
            Some(Action::Quit) => {
                if let Some(hk) = self.hotkey.take() {
                    let _ = self.hotkeys.unregister(hk);
                }
                let _ = slint::quit_event_loop();
            }
            None => {}
        }
        self.tray.sync(&self.settings);
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

pub fn run() {
    let single = match instance::acquire() {
        instance::Instance::Primary(s) => s,
        instance::Instance::Secondary => return,
    };
    let background = std::env::args().any(|a| a == "--background");

    let settings_path = Settings::default_path();
    let (settings, first_run) = Settings::load(&settings_path);
    if first_run {
        let _ = settings.save(&settings_path);
    }
    apply_autostart(&settings);

    let popup = Popup::new().expect("create popup window");
    popup.set_hotkey_label(settings.hotkey.clone().into());
    popup.on_dismiss(|| with_app(|app| app.hide(true)));

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
        })
    });

    if !background {
        on_ui(|app| app.show());
    }

    slint::run_event_loop_until_quit().expect("event loop");
    APP.with(|cell| cell.borrow_mut().take());
    drop(single);
}
