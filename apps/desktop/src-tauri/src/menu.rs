//! The macOS menu bar (ADR 0074).
//!
//! Its own items (File's Save, Save as…, Print…, and the rest) run the UI's
//! registry commands. Each window's UI sends their titles in its language,
//! their shortcuts, and which of them it can run now; the menu bar shows the
//! window in front. Picking an item asks that window to run the command, as
//! its shortcut would. The system's items (Hide, Copy, Minimize, …) keep their
//! native actions; the UI only names them. Windows and Linux have no menu bar.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Deserialize;
use tauri::{AppHandle, Emitter as _, Manager, State, WebviewWindow};

use crate::error::CommandResult;
use crate::state::lock;

/// Asks a window's UI to run the registry command the menu chose.
pub const MENU_COMMAND: &str = "menu-command";

/// The registry commands the menu runs, in menu order (`commands/menu.ts`
/// lists the same; a test there checks).
pub const COMMANDS: [&str; 9] = [
    "app.settings",
    "window.new",
    "file.open",
    "file.close",
    "file.save",
    "file.saveAs",
    "file.print",
    "help.guide",
    "app.shortcuts",
];

/// Closes the document in front, or the window when it shows none.
const CLOSE: &str = "file.close";

/// One of the menu's own items, as the window's UI sees it now.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuItemText {
    /// A registry command id, one of [`COMMANDS`].
    pub id: String,
    pub title: String,
    pub enabled: bool,
    /// The command's shortcut ("CmdOrCtrl+Shift+S"), if it has one.
    pub accelerator: Option<String>,
}

/// The menu in a window's language and state.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MenuText {
    /// Titles of the submenus and of the system's items, by key (`menu.` in
    /// the UI's locales).
    pub titles: HashMap<String, String>,
    pub items: Vec<MenuItemText>,
}

impl MenuText {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn title<'a>(&'a self, key: &'a str) -> &'a str {
        self.titles.get(key).map_or(key, String::as_str)
    }
}

/// Where a picked menu item goes.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    /// Ask this window's UI to run the command.
    Run(String),
    /// Close this window: it has no UI to ask (a print preview).
    Close(String),
    Nowhere,
}

/// The menu each window sent, and which one is on screen.
#[derive(Debug, Default)]
struct Windows {
    /// Each UI window's menu, as it last sent it.
    texts: HashMap<String, MenuText>,
    /// The window whose menu is on screen; the menu's commands go to it.
    shown: Option<String>,
}

impl Windows {
    /// A window sent its menu. Returns it when the menu bar should show it:
    /// the window is in front, already shown, or the first to send one.
    fn remember(&mut self, window: &str, text: MenuText, in_front: bool) -> Option<MenuText> {
        let show = in_front || self.shown.as_deref().is_none_or(|shown| shown == window);
        self.texts.insert(window.to_string(), text.clone());
        if !show {
            return None;
        }
        self.shown = Some(window.to_string());
        Some(text)
    }

    /// A window came to the front: its menu, if it has a UI that sent one.
    fn focused(&mut self, window: &str) -> Option<MenuText> {
        let text = self.texts.get(window)?.clone();
        self.shown = Some(window.to_string());
        Some(text)
    }

    fn closed(&mut self, window: &str) {
        self.texts.remove(window);
        if self.shown.as_deref() == Some(window) {
            self.shown = None;
        }
    }

    /// Where `command` goes when the window in front is `in_front`.
    fn route(&self, command: &str, in_front: Option<&str>) -> Route {
        let with_ui = in_front.filter(|window| self.texts.contains_key(*window));
        if command == CLOSE {
            // Close acts on the window in front, whatever it is.
            return match (in_front, with_ui) {
                (_, Some(window)) => Route::Run(window.to_string()),
                (Some(window), None) => Route::Close(window.to_string()),
                (None, None) => Route::Nowhere,
            };
        }
        // Anything else goes to a UI: the one in front, else the one whose
        // menu is on screen (a print preview in front has none).
        with_ui.map(str::to_string).or_else(|| self.shown.clone()).map_or(Route::Nowhere, Route::Run)
    }
}

/// The menu bar's state, managed by the app.
#[derive(Default)]
pub struct MenuState {
    windows: Mutex<Windows>,
    #[cfg(target_os = "macos")]
    native: Mutex<Option<native::NativeMenu>>,
}

impl MenuState {
    /// Puts `text` on the menu bar, building it the first time (the system's
    /// default menu stands in until a window's UI has sent its own).
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    fn show(&self, app: &AppHandle, text: &MenuText) {
        #[cfg(target_os = "macos")]
        {
            let mut native = lock(&self.native);
            if native.is_none() {
                match native::NativeMenu::install(app) {
                    Ok(menu) => *native = Some(menu),
                    Err(err) => {
                        eprintln!("mirsam: could not build the menu bar: {err}");
                        return;
                    }
                }
            }
            if let Some(menu) = native.as_mut() {
                menu.update(text);
            }
        }
    }
}

/// The calling window's menu: titles in its language, shortcuts, and which
/// commands it can run now. Sent again whenever any of that changes.
#[tauri::command]
pub fn set_menu(
    app: AppHandle,
    window: WebviewWindow,
    menus: State<'_, MenuState>,
    menu: MenuText,
) -> CommandResult<()> {
    let in_front = window.is_focused().unwrap_or(false);
    let shown = lock(&menus.windows).remember(window.label(), menu, in_front);
    if let Some(text) = shown {
        menus.show(&app, &text);
    }
    Ok(())
}

/// A window came to the front: the menu bar shows its state.
pub fn window_focused(app: &AppHandle, window: &str) {
    let menus = app.state::<MenuState>();
    let shown = lock(&menus.windows).focused(window);
    if let Some(text) = shown {
        menus.show(app, &text);
    }
}

/// A window closed: its menu goes with it.
pub fn window_closed(app: &AppHandle, window: &str) {
    lock(&app.state::<MenuState>().windows).closed(window);
}

/// A menu item was picked: the window it concerns runs the command.
pub fn picked(app: &AppHandle, id: &str) {
    let Some(command) = COMMANDS.into_iter().find(|command| *command == id) else {
        return;
    };
    let in_front = app
        .webview_windows()
        .into_iter()
        .find(|(_, window)| window.is_focused().unwrap_or(false))
        .map(|(label, _)| label);
    let route = lock(&app.state::<MenuState>().windows).route(command, in_front.as_deref());
    match route {
        Route::Run(window) => {
            if let Err(err) = app.emit_to(window.as_str(), MENU_COMMAND, command) {
                eprintln!("mirsam: could not run {command} from the menu: {err}");
            }
        }
        Route::Close(window) => {
            if let Some(window) = app.get_webview_window(&window)
                && let Err(err) = window.close()
            {
                eprintln!("mirsam: could not close the window: {err}");
            }
        }
        Route::Nowhere => {}
    }
}

#[cfg(target_os = "macos")]
mod native {
    use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
    use tauri::{AppHandle, Wry};

    use super::{COMMANDS, MenuText};

    /// The installed menu bar, kept to retitle and enable its items.
    pub struct NativeMenu {
        submenus: Vec<(&'static str, Submenu<Wry>)>,
        system: Vec<(&'static str, PredefinedMenuItem<Wry>)>,
        items: Vec<MenuItem<Wry>>,
        /// What the menu shows now, so an unchanged update does nothing.
        shown: Option<MenuText>,
    }

    impl NativeMenu {
        /// Builds the menu bar and makes it the app's. Titles come later, from
        /// [`NativeMenu::update`]: none are written here.
        pub fn install(app: &AppHandle) -> tauri::Result<Self> {
            let package = app.package_info();
            let about = AboutMetadata {
                name: Some(package.name.clone()),
                version: Some(package.version.to_string()),
                copyright: app.config().bundle.copyright.clone(),
                ..Default::default()
            };
            let about = PredefinedMenuItem::about(app, None, Some(about))?;
            let services = PredefinedMenuItem::services(app, None)?;
            let hide = PredefinedMenuItem::hide(app, None)?;
            let hide_others = PredefinedMenuItem::hide_others(app, None)?;
            let show_all = PredefinedMenuItem::show_all(app, None)?;
            let quit = PredefinedMenuItem::quit(app, None)?;
            let undo = PredefinedMenuItem::undo(app, None)?;
            let redo = PredefinedMenuItem::redo(app, None)?;
            let cut = PredefinedMenuItem::cut(app, None)?;
            let copy = PredefinedMenuItem::copy(app, None)?;
            let paste = PredefinedMenuItem::paste(app, None)?;
            let select_all = PredefinedMenuItem::select_all(app, None)?;
            let full_screen = PredefinedMenuItem::fullscreen(app, None)?;
            let minimize = PredefinedMenuItem::minimize(app, None)?;
            let zoom = PredefinedMenuItem::maximize(app, None)?;
            let command = |id: &str| MenuItem::with_id(app, id, id, false, None::<&str>);
            let [settings, new_window, open, close, save, save_as, print, guide, shortcuts] =
                COMMANDS.map(command);
            let (settings, new_window, open, close, save, save_as, print, guide, shortcuts) =
                (settings?, new_window?, open?, close?, save?, save_as?, print?, guide?, shortcuts?);
            let sep = || PredefinedMenuItem::separator(app);

            // The app menu's title is always the app's name; macOS shows its own.
            let app_menu = Submenu::with_items(
                app,
                package.name.clone(),
                true,
                &[
                    &about,
                    &sep()?,
                    &settings,
                    &sep()?,
                    &services,
                    &sep()?,
                    &hide,
                    &hide_others,
                    &show_all,
                    &sep()?,
                    &quit,
                ],
            )?;
            let file = Submenu::with_items(
                app,
                "file",
                true,
                &[&new_window, &open, &sep()?, &close, &sep()?, &save, &save_as, &sep()?, &print],
            )?;
            let edit = Submenu::with_items(
                app,
                "edit",
                true,
                &[&undo, &redo, &sep()?, &cut, &copy, &paste, &select_all],
            )?;
            let view = Submenu::with_items(app, "view", true, &[&full_screen])?;
            let window = Submenu::with_items(app, "window", true, &[&minimize, &zoom])?;
            let help = Submenu::with_items(app, "help", true, &[&guide, &shortcuts])?;
            let menu = Menu::with_items(app, &[&app_menu, &file, &edit, &view, &window, &help])?;
            app.set_menu(menu)?;
            // macOS lists the open windows in this menu, and adds search to Help.
            window.set_as_windows_menu_for_nsapp()?;
            help.set_as_help_menu_for_nsapp()?;
            Ok(Self {
                submenus: vec![
                    ("file", file),
                    ("edit", edit),
                    ("view", view),
                    ("window", window),
                    ("help", help),
                ],
                system: vec![
                    ("about", about),
                    ("services", services),
                    ("hide", hide),
                    ("hideOthers", hide_others),
                    ("showAll", show_all),
                    ("quit", quit),
                    ("undo", undo),
                    ("redo", redo),
                    ("cut", cut),
                    ("copy", copy),
                    ("paste", paste),
                    ("selectAll", select_all),
                    ("fullScreen", full_screen),
                    ("minimize", minimize),
                    ("zoom", zoom),
                ],
                items: vec![settings, new_window, open, close, save, save_as, print, guide, shortcuts],
                shown: None,
            })
        }

        /// Shows `text`: titles, shortcuts, and which items are enabled. A
        /// failure leaves that one item as it was.
        pub fn update(&mut self, text: &MenuText) {
            if self.shown.as_ref() == Some(text) {
                return;
            }
            let report = |result: tauri::Result<()>| {
                if let Err(err) = result {
                    eprintln!("mirsam: could not update the menu bar: {err}");
                }
            };
            for (key, submenu) in &self.submenus {
                report(submenu.set_text(text.title(key)));
            }
            for (key, item) in &self.system {
                report(item.set_text(text.title(key)));
            }
            for item in &self.items {
                // An item the UI did not send stays as it was: disabled at first.
                let Some(state) = text.items.iter().find(|state| item.id().as_ref() == state.id) else {
                    continue;
                };
                report(item.set_text(&state.title));
                report(item.set_enabled(state.enabled));
                report(item.set_accelerator(state.accelerator.as_deref()));
            }
            self.shown = Some(text.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(title: &str) -> MenuText {
        MenuText {
            titles: HashMap::from([("file".to_string(), title.to_string())]),
            items: vec![MenuItemText {
                id: "file.save".to_string(),
                title: "Save".to_string(),
                enabled: true,
                accelerator: Some("CmdOrCtrl+S".to_string()),
            }],
        }
    }

    #[test]
    fn the_first_window_to_send_its_menu_is_shown_then_the_one_in_front() {
        let mut windows = Windows::default();
        assert_eq!(windows.remember("main", text("File"), false), Some(text("File")));
        // Another window, behind: kept for when it comes to the front.
        assert_eq!(windows.remember("window-2", text("ملف"), false), None);
        assert_eq!(windows.shown.as_deref(), Some("main"));
        // The shown window's changes show at once.
        assert_eq!(windows.remember("main", text("Fichier"), false), Some(text("Fichier")));
        assert_eq!(windows.focused("window-2"), Some(text("ملف")));
        assert_eq!(windows.shown.as_deref(), Some("window-2"));
        // A window in front shows its menu even before it has the focus event.
        assert_eq!(windows.remember("window-3", text("File"), true), Some(text("File")));
        assert_eq!(windows.shown.as_deref(), Some("window-3"));
    }

    #[test]
    fn a_window_without_a_ui_leaves_the_menu_as_it_was() {
        let mut windows = Windows::default();
        windows.remember("main", text("File"), true);
        assert_eq!(windows.focused("print-7"), None);
        assert_eq!(windows.shown.as_deref(), Some("main"));
    }

    #[test]
    fn a_closed_window_takes_its_menu_with_it() {
        let mut windows = Windows::default();
        windows.remember("main", text("File"), true);
        windows.remember("window-2", text("File"), false);
        windows.closed("main");
        assert_eq!(windows.shown, None);
        assert_eq!(windows.route("file.save", None), Route::Nowhere);
        assert_eq!(windows.focused("window-2"), Some(text("File")));
        assert_eq!(windows.focused("main"), None);
    }

    #[test]
    fn commands_go_to_the_window_in_front() {
        let mut windows = Windows::default();
        windows.remember("main", text("File"), true);
        windows.remember("window-2", text("File"), false);
        assert_eq!(windows.route("file.save", Some("window-2")), Route::Run("window-2".into()));
        assert_eq!(windows.route("file.close", Some("main")), Route::Run("main".into()));
        // No window in front (all minimized): the one whose menu is shown.
        assert_eq!(windows.route("file.open", None), Route::Run("main".into()));
    }

    #[test]
    fn close_closes_a_print_preview_and_other_commands_skip_it() {
        let mut windows = Windows::default();
        windows.remember("main", text("File"), true);
        assert_eq!(windows.route("file.close", Some("print-7")), Route::Close("print-7".into()));
        assert_eq!(windows.route("file.print", Some("print-7")), Route::Run("main".into()));
        assert_eq!(windows.route("file.close", None), Route::Nowhere);
    }
}
