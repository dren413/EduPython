//! Per-user settings, stored as JSON in the OS config folder.

use std::path::{Path, PathBuf};

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
    /// Dotted grid behind the UML diagram.
    pub uml_grid: bool,
    /// Last page viewed in the booklet (0-based).
    pub booklet_page: usize,
    pub recent_projects: Vec<String>,
    /// A booklet PDF the pupil chose (used when the school installed none).
    pub booklet_file: Option<String>,
    /// Folder the user chose for new projects (`None`: the one from the ini).
    pub projects_folder: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: false,
            editor_font_size: 15.0,
            ui_zoom: 1.0,
            python_command: None,
            show_uml: true,
            show_booklet: true,
            uml_grid: true,
            booklet_page: 0,
            recent_projects: Vec::new(),
            booklet_file: None,
            projects_folder: None,
        }
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("EduPython").join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
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

    /// Remember where a project was created. Only a folder the user changed
    /// counts (not `offered`), and going back to the ini's folder forgets it,
    /// so a later change of the ini still reaches everybody else.
    pub fn remember_projects_folder(&mut self, chosen: &str, offered: &str, ini: Option<&Path>) {
        let same = |a: &str, b: &Path| Path::new(a).components().eq(b.components());
        if ini.is_some_and(|d| same(chosen, d)) {
            self.projects_folder = None;
        } else if !same(chosen, Path::new(offered)) {
            let path = std::path::absolute(chosen).unwrap_or_else(|_| chosen.into());
            self.projects_folder = Some(path.to_string_lossy().into_owned());
        }
    }

    pub fn add_recent(&mut self, dir: &str) {
        self.recent_projects.retain(|d| d != dir);
        self.recent_projects.insert(0, dir.to_string());
        self.recent_projects.truncate(10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_a_changed_projects_folder() {
        let dir = std::env::temp_dir();
        let path = |n: &str| dir.join(n).to_string_lossy().into_owned();
        let (ini, other) = (path("ini"), path("other"));
        let mut s = Settings::default();
        s.remember_projects_folder(&ini, &ini, Some(Path::new(&ini)));
        assert_eq!(s.projects_folder, None, "the offered folder is not a choice");
        s.remember_projects_folder(&other, &ini, Some(Path::new(&ini)));
        assert_eq!(s.projects_folder.as_ref(), Some(&other));
        // Survives saving and loading; old settings files still load.
        let json = serde_json::to_string(&s).unwrap();
        let mut s: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s.projects_folder.as_ref(), Some(&other));
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap().projects_folder, None);
        s.remember_projects_folder(&ini, &other, Some(Path::new(&ini)));
        assert_eq!(s.projects_folder, None, "back to the ini's folder");
    }
}
