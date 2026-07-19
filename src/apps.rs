//! Default macOS application catalog.
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AppMeta {
    pub id: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    pub color: &'static str,
    pub dock: bool,
    pub desktop: bool,
    pub width: u32,
    pub height: u32,
    pub resizable: bool,
}

pub fn all_apps() -> Vec<AppMeta> {
    vec![
        app("finder", "Finder", "finder", "#1E90FF", true, true, 780, 520, true),
        app("safari", "Safari", "safari", "#0A84FF", true, false, 960, 640, true),
        app("messages", "Messages", "messages", "#34C759", true, false, 720, 500, true),
        app("mail", "Mail", "mail", "#1E88E5", true, false, 860, 560, true),
        app("maps", "Maps", "maps", "#34C759", true, false, 880, 580, true),
        app("photos", "Photos", "photos", "#FF9500", true, false, 820, 560, true),
        app("facetime", "FaceTime", "facetime", "#30D158", true, false, 640, 480, true),
        app("calendar", "Calendar", "calendar", "#FF3B30", true, false, 820, 560, true),
        app("notes", "Notes", "notes", "#FFCC00", true, true, 720, 500, true),
        app("reminders", "Reminders", "reminders", "#FF9500", true, false, 680, 500, true),
        app("music", "Music", "music", "#FF2D55", true, false, 780, 520, true),
        app("tv", "TV", "tv", "#000000", true, false, 900, 560, true),
        app("podcasts", "Podcasts", "podcasts", "#9933FF", false, false, 720, 500, true),
        app("appstore", "App Store", "appstore", "#0A84FF", true, false, 900, 600, true),
        app("systemsettings", "System Settings", "settings", "#8E8E93", true, false, 760, 540, true),
        app("calculator", "Calculator", "calculator", "#1C1C1E", false, true, 280, 400, false),
        app("terminal", "Terminal", "terminal", "#1C1C1E", false, true, 720, 460, true),
        app("textedit", "TextEdit", "textedit", "#FFFFFF", false, true, 640, 480, true),
        app("preview", "Preview", "preview", "#0A84FF", false, false, 700, 500, true),
        app("clock", "Clock", "clock", "#1C1C1E", false, false, 360, 420, false),
        app("weather", "Weather", "weather", "#5AC8FA", false, false, 400, 520, false),
        app("contacts", "Contacts", "contacts", "#8E8E93", false, false, 700, 500, true),
        app("books", "Books", "books", "#FF9500", false, false, 800, 560, true),
        app("launchpad", "Launchpad", "launchpad", "#1C1C1E", true, false, 0, 0, false),
        app("trash", "Trash", "trash", "#8E8E93", true, false, 600, 400, true),
    ]
}

fn app(id: &'static str, name: &'static str, icon: &'static str, color: &'static str, dock: bool, desktop: bool, w: u32, h: u32, r: bool) -> AppMeta {
    AppMeta { id, name, icon, color, dock, desktop, width: w, height: h, resizable: r }
}

pub fn dock_apps() -> Vec<AppMeta> { all_apps().into_iter().filter(|a| a.dock).collect() }
pub fn desktop_icons() -> Vec<AppMeta> { all_apps().into_iter().filter(|a| a.desktop).collect() }
