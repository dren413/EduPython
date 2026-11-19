//! Per-user settings, stored as JSON in the OS config folder.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub dark: bool,
    pub editor_font_size: f32,
    pub ui_zoom: f32,
    pub python_command: Option<String>,
    pub show_uml: bool,
    pub show_booklet: bool,
    /// Last page viewed in the booklet (0-based).
    pub booklet_page: usize,
    pub recent_projects: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: true,
            editor_font_size: 15.0,
            ui_zoom: 1.0,
            python_command: None,
            show_uml: true,
            show_booklet: false,
            booklet_page: 0,
            recent_projects: Vec::new(),
        }
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("EduPython").join("settings.json"))
}

/// Settings saved by the app under its old name (Pythonizer).
fn legacy_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Pythonizer").join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .or_else(|| legacy_path().and_then(|p| std::fs::read_to_string(p).ok()))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, s);
        }
    }

    pub fn add_recent(&mut self, dir: &str) {
        self.recent_projects.retain(|d| d != dir);
        self.recent_projects.insert(0, dir.to_string());
        self.recent_projects.truncate(10);
    }
}
