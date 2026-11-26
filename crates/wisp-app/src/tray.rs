//! Tray icon and its menu.

use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use wisp_core::{PopupStyle, Settings};

pub enum Action {
    Open,
    Backups,
    Style(PopupStyle),
    ToggleAutostart,
    ToggleGlass,
    ToggleUpdateCheck,
    GetUpdate,
    TogglePause,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    open: MenuItem,
    backups: MenuItem,
    styles: Vec<(PopupStyle, CheckMenuItem)>,
    autostart: CheckMenuItem,
    glass: CheckMenuItem,
    updates: CheckMenuItem,
    pause: CheckMenuItem,
    quit: MenuItem,
    menu: Menu,
    /// "Update to Wisp x.y.z…", added to the top of the menu once a newer release is found.
    get_update: MenuItem,
    update_shown: std::cell::Cell<bool>,
    tooltip: std::cell::RefCell<String>,
}

impl Tray {
    pub fn new(settings: &Settings) -> Result<Self, Box<dyn std::error::Error>> {
        let open = MenuItem::new(format!("Open Wisp\t{}", settings.hotkey), true, None);
        let styles: Vec<_> = PopupStyle::ALL
            .iter()
            .map(|&s| (s, CheckMenuItem::new(s.label(), true, s == settings.popup_style, None)))
            .collect();
        let style_menu = Submenu::new("Popup style", true);
        for (_, item) in &styles {
            style_menu.append(item)?;
        }
        let autostart = CheckMenuItem::new("Start with Windows", true, settings.start_with_windows, None);
        let glass = CheckMenuItem::new("Glass effect (uses more memory)", true, settings.glass_effect, None);
        let updates = CheckMenuItem::new("Check for updates", true, settings.check_for_updates, None);
        let pause = CheckMenuItem::new("Pause recording", true, false, None);
        let backups = MenuItem::new("Clip sets…", true, None);
        let quit = MenuItem::new("Quit Wisp", true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &open,
            &backups,
            &pause,
            &PredefinedMenuItem::separator(),
            &style_menu,
            &glass,
            &autostart,
            &updates,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;

        // The icon embedded in wisp.exe, at the tray's size for this display.
        let size = wisp_win::screen::small_icon_size();
        let icon = Icon::from_resource(1, Some((size, size)))?;
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip(format!("Wisp ({})", settings.hotkey))
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(false)
            .build()?;

        let get_update = MenuItem::new("Update Wisp…", true, None);
        Ok(Self {
            icon: tray,
            open,
            backups,
            styles,
            autostart,
            glass,
            updates,
            pause,
            quit,
            menu,
            get_update,
            update_shown: std::cell::Cell::new(false),
            tooltip: std::cell::RefCell::new(format!("Wisp ({})", settings.hotkey)),
        })
    }

    pub fn action_for(&self, id: &MenuId) -> Option<Action> {
        if id == self.open.id() {
            return Some(Action::Open);
        }
        if id == self.backups.id() {
            return Some(Action::Backups);
        }
        if id == self.autostart.id() {
            return Some(Action::ToggleAutostart);
        }
        if id == self.glass.id() {
            return Some(Action::ToggleGlass);
        }
        if id == self.updates.id() {
            return Some(Action::ToggleUpdateCheck);
        }
        if id == self.get_update.id() {
            return Some(Action::GetUpdate);
        }
        if id == self.pause.id() {
            return Some(Action::TogglePause);
        }
        if id == self.quit.id() {
            return Some(Action::Quit);
        }
        self.styles.iter().find(|(_, item)| id == item.id()).map(|(s, _)| Action::Style(*s))
    }

    /// Show the current shortcut in the menu and the tooltip.
    pub fn set_hotkey(&self, hotkey: &str) {
        self.open.set_text(format!("Open Wisp\t{hotkey}"));
        *self.tooltip.borrow_mut() = format!("Wisp ({hotkey})");
        self.refresh_tooltip();
    }

    /// Offer a newer release at the top of the menu.
    pub fn show_update(&self, version: &str) {
        self.get_update.set_text(format!("Update to Wisp {version}…"));
        if !self.update_shown.replace(true) {
            let _ = self.menu.insert_items(&[&self.get_update, &PredefinedMenuItem::separator()], 0);
        }
        self.refresh_tooltip();
    }

    fn refresh_tooltip(&self) {
        let mut tip = self.tooltip.borrow().clone();
        if self.update_shown.get() {
            tip.push_str(" · update available");
        }
        let _ = self.icon.set_tooltip(Some(tip));
    }

    /// Make the check marks match the settings (muda toggles them on click).
    pub fn sync(&self, settings: &Settings, paused: bool) {
        self.pause.set_checked(paused);
        for (s, item) in &self.styles {
            item.set_checked(*s == settings.popup_style);
        }
        self.autostart.set_checked(settings.start_with_windows);
        self.glass.set_checked(settings.glass_effect);
        self.updates.set_checked(settings.check_for_updates);
    }
}
