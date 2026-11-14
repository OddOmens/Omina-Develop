//! What Develop remembers between runs: the look, recent projects, window placement, the
//! panel layout, your own presets, export choices and a few preferences. Kept as JSON in
//! Develop's settings folder (`~/Library/Application Support/Develop` on macOS).
//! Settings that can't be read fall back to their defaults one by one, and an unreadable file
//! is kept aside as `settings.damaged.json` rather than lost.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use develop_core::image_io::ExportFormat;
use develop_core::presets::Preset;
use serde::{Deserialize, Serialize};

use crate::features::Feature;
use crate::theme::{Accent, ThemeName};

/// Where the window was last time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

/// A project opened lately, newest first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecentProject {
    pub path: PathBuf,
    /// When it was last opened or saved, in seconds since 1970.
    pub used: u64,
}

/// What shows around the photo in the viewer (Lightroom's background options).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backdrop {
    /// The panel's own color, which follows the theme.
    #[default]
    Theme,
    Black,
    DarkGray,
    MiddleGray,
    White,
}

impl Backdrop {
    pub const ALL: [Backdrop; 5] = [Self::Theme, Self::Black, Self::DarkGray, Self::MiddleGray, Self::White];

    pub fn name(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::Black => "Black",
            Self::DarkGray => "Dark Gray",
            Self::MiddleGray => "Middle Gray",
            Self::White => "White",
        }
    }

    /// The color to draw, or `None` for the theme's pasteboard color.
    pub fn color(self) -> Option<u32> {
        match self {
            Self::Theme => None,
            Self::Black => Some(0x000000),
            Self::DarkGray => Some(0x2b2b2b),
            Self::MiddleGray => Some(0x777777),
            Self::White => Some(0xffffff),
        }
    }
}

/// How Export last sized the photo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportSize {
    /// The full size of the cropped photo.
    #[default]
    Full,
    /// Shrunk so its longer side is at most this many pixels (never enlarged).
    LongEdge(u32),
}

/// The choices Export starts with: what was used last time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportChoices {
    pub format: ExportFormat,
    pub quality: u8,
    pub size: ExportSize,
}

impl Default for ExportChoices {
    fn default() -> Self {
        Self { format: ExportFormat::Jpeg, quality: 90, size: ExportSize::Full }
    }
}

/// What Develop shows when it starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StartWith {
    #[default]
    StartScreen,
    LastProject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub theme: ThemeName,
    pub accent: Accent,
    pub window: Option<WindowPlacement>,
    pub recent_projects: Vec<RecentProject>,
    /// The panel arrangement (gpui-kit's dock state), restored at launch.
    pub workspace_layout: Option<serde_json::Value>,
    pub backdrop: Backdrop,
    pub start_with: StartWith,
    /// How many steps undo keeps.
    pub history_states: usize,
    /// Presets saved with Create Preset, listed after the built-in ones.
    pub user_presets: Vec<Preset>,
    /// What Export used last.
    pub export: ExportChoices,
    /// The folder Open and Export start in.
    pub last_folder: Option<PathBuf>,
    /// The folder the last export was written to, where exports made without the sheet
    /// (Iris's "export as JPEG", "export all") go.
    pub export_folder: Option<PathBuf>,
    /// Panels taken off screen, and where each was.
    pub hidden_panels: Vec<crate::panels::dock::HiddenPanel>,
    /// Shows the rule-of-thirds grid over the crop while cropping.
    pub crop_grid: bool,
    /// What Iris was called here before her name was shared with the other apps (see
    /// `iris::settings`). Read once, to start the shared settings when no app has written them.
    pub assistant_name: String,
    /// Whether Iris is used at all: her rail button, badge, suggestions and chat. Like a feature
    /// switch in Studio's Settings ▸ Features.
    pub iris_on: bool,
    /// Which of Iris's rules run.
    pub assistant: develop_core::assistant::Options,
    /// Whether Iris's face sits in a corner of the photo with a count of what it noticed.
    pub assistant_badge: bool,
    /// Tools and features turned off in Settings ▸ Tools & Features (see `features`).
    #[serde(deserialize_with = "crate::features::read_features_off")]
    pub features_off: BTreeSet<Feature>,
    /// Whether the welcome setup has been through (or skipped), so it shows only on the first launch.
    pub onboarded: bool,
    /// How often unsaved work is copied aside in case Develop closes without saving it, in
    /// seconds (0: never). See `recovery`.
    pub recovery_seconds: u32,
    /// The keyboard shortcuts changed in Settings ▸ Keyboard Shortcuts.
    pub shortcuts: crate::keymap::Shortcuts,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemeName::default(),
            accent: Accent::default(),
            window: None,
            recent_projects: Vec::new(),
            workspace_layout: None,
            backdrop: Backdrop::default(),
            start_with: StartWith::default(),
            history_states: 100,
            user_presets: Vec::new(),
            export: ExportChoices::default(),
            last_folder: None,
            export_folder: None,
            hidden_panels: Vec::new(),
            crop_grid: true,
            assistant_name: String::new(),
            iris_on: true,
            assistant: Default::default(),
            assistant_badge: true,
            features_off: BTreeSet::new(),
            onboarded: false,
            recovery_seconds: 30,
            shortcuts: crate::keymap::Shortcuts::default(),
        }
    }
}

/// Develop's settings folder.
pub fn folder() -> PathBuf {
    #[cfg(test)]
    {
        // Per test thread, so tests running side by side never share (or overwrite) settings.
        let thread = format!("{:?}", std::thread::current().id()).replace(|c: char| !c.is_ascii_alphanumeric(), "");
        std::env::temp_dir().join(format!("develop-test-settings-{}-{thread}", std::process::id()))
    }
    #[cfg(not(test))]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        if cfg!(target_os = "macos") {
            home.join("Library/Application Support/Develop")
        } else if cfg!(target_os = "windows") {
            std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or(home).join("Develop")
        } else {
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("develop")
        }
    }
}

fn settings_path() -> PathBuf {
    folder().join("settings.json")
}

/// Reads the settings, falling back to defaults for anything missing or unreadable. Your own
/// presets are repaired as a project file's edit is, since the file can be edited by hand.
pub fn load() -> AppSettings {
    let mut settings = read();
    for preset in &mut settings.user_presets {
        develop_core::photo::sanitize_preset(preset);
    }
    settings
}

/// The settings as saved, each unreadable one at its default.
fn read() -> AppSettings {
    let Ok(text) = std::fs::read_to_string(settings_path()) else { return AppSettings::default() };
    if let Ok(settings) = serde_json::from_str::<AppSettings>(&text) {
        return settings;
    }
    if let Err(error) = std::fs::write(folder().join("settings.damaged.json"), &text) {
        eprintln!("could not keep a copy of the unreadable settings: {error}");
    }
    // Keep whatever settings can still be read, one by one.
    let Ok(serde_json::Value::Object(saved)) = serde_json::from_str::<serde_json::Value>(&text) else { return AppSettings::default() };
    let mut settings = serde_json::to_value(AppSettings::default()).unwrap_or_default();
    if let serde_json::Value::Object(defaults) = &mut settings {
        for (key, value) in saved {
            if defaults.contains_key(&key) {
                let mut trial = defaults.clone();
                trial.insert(key.clone(), value.clone());
                if serde_json::from_value::<AppSettings>(serde_json::Value::Object(trial)).is_ok() {
                    defaults.insert(key, value);
                }
            }
        }
    }
    serde_json::from_value(settings).unwrap_or_default()
}

/// Writes the settings.
pub fn save(settings: &AppSettings) {
    let written = std::fs::create_dir_all(folder())
        .map_err(|error| error.to_string())
        .and_then(|()| serde_json::to_vec_pretty(settings).map_err(|error| error.to_string()))
        .and_then(|bytes| develop_core::image_io::write_atomically(&settings_path(), &bytes));
    if let Err(error) = written {
        eprintln!("could not save Develop's settings: {error}");
    }
}

/// Reads, changes and writes the settings.
pub fn update(change: impl FnOnce(&mut AppSettings)) {
    let mut settings = load();
    change(&mut settings);
    save(&settings);
}

pub fn now_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

/// Puts a project at the top of the recent list (at most 20 are kept).
pub fn note_project_used(path: &Path) {
    let path = path.to_path_buf();
    update(|settings| {
        settings.recent_projects.retain(|recent| recent.path != path);
        settings.recent_projects.insert(0, RecentProject { path, used: now_seconds() });
        settings.recent_projects.truncate(20);
    });
}

/// Takes a project off the recent list.
pub fn forget_recent(path: &Path) {
    update(|settings| settings.recent_projects.retain(|recent| recent.path != path));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_settings_survive_a_damaged_neighbor() {
        let mut settings = AppSettings { history_states: 20, ..AppSettings::default() };
        settings.crop_grid = false;
        let mut value = serde_json::to_value(&settings).unwrap();
        value["theme"] = serde_json::Value::String("NoSuchTheme".into());
        std::fs::create_dir_all(folder()).unwrap();
        std::fs::write(settings_path(), value.to_string()).unwrap();
        let loaded = load();
        assert_eq!(loaded.history_states, 20);
        assert!(!loaded.crop_grid);
        assert_eq!(loaded.theme, ThemeName::default());
        // Only its own file goes: other tests running alongside keep files in the same folder.
        let _ = std::fs::remove_file(settings_path());
    }
}
