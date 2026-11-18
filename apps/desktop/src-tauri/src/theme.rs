//! Light or dark for the native window.
//!
//! The page themes itself with CSS. This keeps the native parts in step: the
//! title bar (and the area behind the macOS traffic lights), and the window
//! and webview background that shows before the page has painted.
//!
//! The window starts hidden (`"visible": false` in `tauri.conf.json`) and the
//! front end shows it through [`set_window_theme`] once the page has its
//! colours, so it never flashes white. The choice is also kept in a small file
//! so the next launch can set the window up before the page loads.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use tauri::window::Color;
use tauri::{AppHandle, Manager, Runtime, State, Theme, WebviewWindow, Window, WindowEvent};

/// The page background (`--bg` in `styles.css`) of each theme.
const LIGHT_BACKGROUND: Color = Color(0xff, 0xf6, 0xf3, 0xff);
const DARK_BACKGROUND: Color = Color(0x1c, 0x12, 0x16, 0xff);

/// How long to wait for the front end before showing the window anyway.
const SHOW_DEADLINE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    fn parse(text: &str) -> Self {
        match text.trim() {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// `None` follows the system.
    fn window_theme(self) -> Option<Theme> {
        match self {
            Self::System => None,
            Self::Light => Some(Theme::Light),
            Self::Dark => Some(Theme::Dark),
        }
    }

    fn background(self, system: Theme) -> Color {
        let dark = match self {
            Self::System => system == Theme::Dark,
            Self::Light => false,
            Self::Dark => true,
        };
        if dark {
            DARK_BACKGROUND
        } else {
            LIGHT_BACKGROUND
        }
    }
}

/// The current choice, for following the system's changes.
#[derive(Debug, Default)]
pub struct WindowTheme(Mutex<ThemeChoice>);

impl WindowTheme {
    fn get(&self) -> ThemeChoice {
        self.0.lock().map(|choice| *choice).unwrap_or_default()
    }

    fn set(&self, choice: ThemeChoice) {
        if let Ok(mut current) = self.0.lock() {
            *current = choice;
        }
    }
}

fn choice_file<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("theme"))
}

fn load<R: Runtime>(app: &AppHandle<R>) -> ThemeChoice {
    choice_file(app)
        .and_then(|file| std::fs::read_to_string(file).ok())
        .map(|text| ThemeChoice::parse(&text))
        .unwrap_or_default()
}

fn store<R: Runtime>(app: &AppHandle<R>, choice: ThemeChoice) {
    let Some(file) = choice_file(app) else { return };
    if std::fs::read_to_string(&file).is_ok_and(|text| ThemeChoice::parse(&text) == choice) {
        return;
    }
    // Only a head start for the next launch; the page has its own copy.
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, choice.as_str());
}

fn apply<R: Runtime>(window: &WebviewWindow<R>, choice: ThemeChoice) {
    let _ = window.set_theme(choice.window_theme());
    let system = window.theme().unwrap_or(Theme::Light);
    let _ = window.set_background_color(Some(choice.background(system)));
}

/// Set the main window up in the remembered theme while it is still hidden,
/// and show it after a while should the front end never do so.
pub fn setup<R: Runtime>(app: &AppHandle<R>) {
    let choice = load(app);
    app.manage(WindowTheme(Mutex::new(choice)));
    if let Some(window) = app.get_webview_window("main") {
        apply(&window, choice);
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_DEADLINE);
        if let Some(window) = app.get_webview_window("main")
            && !window.is_visible().unwrap_or(true)
        {
            let _ = window.show();
        }
    });
}

/// Keep the background in step when the system theme changes while the
/// window follows it. (The page follows on its own through CSS.)
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    let WindowEvent::ThemeChanged(system) = event else {
        return;
    };
    let choice = window.state::<WindowTheme>().get();
    if choice != ThemeChoice::System {
        return;
    }
    if let Some(webview) = window.get_webview_window(window.label()) {
        let _ = webview.set_background_color(Some(choice.background(*system)));
    }
}

/// Give the window `theme`, remember it, and show the window if it is
/// still hidden.
#[tauri::command]
pub fn set_window_theme(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, WindowTheme>,
    theme: ThemeChoice,
) {
    state.set(theme);
    apply(&window, theme);
    if !window.is_visible().unwrap_or(true) {
        let _ = window.show();
    }
    store(&app, theme);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_choices_read_back() {
        for choice in [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark] {
            assert_eq!(ThemeChoice::parse(choice.as_str()), choice);
        }
        assert_eq!(ThemeChoice::parse("dark\n"), ThemeChoice::Dark);
        assert_eq!(ThemeChoice::parse("sepia"), ThemeChoice::System);
        assert_eq!(ThemeChoice::parse(""), ThemeChoice::System);
    }

    #[test]
    fn background_follows_the_choice() {
        assert_eq!(ThemeChoice::System.background(Theme::Dark), DARK_BACKGROUND);
        assert_eq!(
            ThemeChoice::System.background(Theme::Light),
            LIGHT_BACKGROUND
        );
        assert_eq!(ThemeChoice::Light.background(Theme::Dark), LIGHT_BACKGROUND);
        assert_eq!(ThemeChoice::Dark.background(Theme::Light), DARK_BACKGROUND);
    }
}
