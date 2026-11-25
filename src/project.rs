//! A project is simply a folder of `.py` files plus an optional
//! `edupython.json` that remembers the main file and the UML layout.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const META_FILE: &str = "edupython.json";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Template {
    Empty,
    Script,
    Classes,
    Pygame,
}

impl Template {
    pub const ALL: [Template; 4] = [Template::Script, Template::Classes, Template::Pygame, Template::Empty];

    pub fn label(self) -> &'static str {
        match self {
            Template::Script => "Script (2CB) – BMI with input, print and if",
            Template::Classes => "Classes (1CB) – a class in its own file",
            Template::Pygame => "Pygame (1CB) – Pong with Paddle and Ball classes",
            Template::Empty => "Empty – just main.py",
        }
    }

    fn files(self) -> Vec<(&'static str, &'static str)> {
        match self {
            Template::Empty => vec![("main.py", "")],
            Template::Script => vec![("main.py", TEMPLATE_SCRIPT)],
            Template::Classes => vec![("main.py", TEMPLATE_CLASSES_MAIN), ("point.py", TEMPLATE_CLASSES_POINT)],
            Template::Pygame => vec![("main.py", TEMPLATE_PYGAME_PONG)],
        }
    }
}

/// Exercise 3.9 (2CB): body mass index.
const TEMPLATE_SCRIPT: &str = include_str!("../assets/templates/bmi.py");

const TEMPLATE_CLASSES_MAIN: &str = r#"from point import Point

p = Point(3, 4)
print(p.x, p.y)

p.move((1, 2))
print(p.x, p.y)
print("Distance to origin:", p.distance(Point(0, 0)))
"#;

const TEMPLATE_CLASSES_POINT: &str = r#"class Point:
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def move(self, p: tuple[int, int]):
        self.x = self.x + p[0]
        self.y = self.y + p[1]

    def distance(self, other: "Point") -> float:
        return ((self.x - other.x) ** 2 + (self.y - other.y) ** 2) ** 0.5
"#;

/// Exercise 5.19 (1CB): Pong with a Paddle and a Ball class.
const TEMPLATE_PYGAME_PONG: &str = include_str!("../assets/templates/pong.py");

#[derive(Serialize, Deserialize, Default, Debug)]
#[serde(default)]
struct Meta {
    version: u32,
    main_file: String,
    uml_positions: BTreeMap<String, [f32; 2]>,
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub name: String,
    pub text: String,
    saved_text: String,
    /// Full path of a file opened by drag & drop: it stays where it is, and
    /// `name` is then only the (unique) tab name. `None`: in the project folder.
    pub external: Option<PathBuf>,
}

impl SourceFile {
    fn new(name: &str, text: String) -> Self {
        Self { name: name.to_string(), saved_text: text.clone(), text, external: None }
    }

    pub fn is_dirty(&self) -> bool {
        self.text != self.saved_text
    }
}

pub struct Project {
    pub dir: PathBuf,
    pub files: Vec<SourceFile>,
    pub main_file: String,
    pub uml_positions: BTreeMap<String, [f32; 2]>,
    meta_dirty: bool,
    /// Created by the old tkinter version (`project.json` + `gui.py`).
    pub legacy: bool,
    /// Only dropped files, no real project: nothing but the files is written.
    pub loose: bool,
}

/// Compare paths after resolving links, relative parts, …
fn same_path(a: &Path, b: &Path) -> bool {
    a == b || matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

pub fn is_python_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("py")) && path.is_file()
}

fn read_source(path: &Path) -> Result<String, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    Ok(text.trim_start_matches('\u{feff}').replace("\r\n", "\n"))
}

pub fn sanitize_name(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Normalise a new file name (`player` → `player.py`) and validate it.
pub fn python_file_name(name: &str) -> Result<String, String> {
    let mut n = sanitize_name(name);
    if n.is_empty() {
        return Err("Please enter a file name.".into());
    }
    if !n.ends_with(".py") {
        n.push_str(".py");
    }
    let stem = &n[..n.len() - 3];
    let valid = stem.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && stem.chars().all(|c| c.is_alphanumeric() || c == '_');
    if !valid {
        return Err("Use only letters, digits and _ (so the file can be imported).".into());
    }
    Ok(n)
}

impl Project {
    pub fn name(&self) -> String {
        self.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn create(parent: &Path, name: &str, template: Template) -> Result<Project, String> {
        let name = sanitize_name(name);
        if name.is_empty() {
            return Err("Please enter a project name.".into());
        }
        let dir = parent.join(&name);
        if dir.exists() {
            return Err(format!("A folder named '{name}' already exists there."));
        }
        fs::create_dir_all(&dir).map_err(|e| format!("Could not create the folder: {e}"))?;
        for (file, content) in template.files() {
            fs::write(dir.join(file), content).map_err(|e| format!("Could not write {file}: {e}"))?;
        }
        let mut p = Project::open(&dir)?;
        p.meta_dirty = true;
        p.save()?;
        Ok(p)
    }

    pub fn open(dir: &Path) -> Result<Project, String> {
        if !dir.is_dir() {
            return Err(format!("'{}' is not a folder.", dir.display()));
        }
        let meta: Meta = fs::read_to_string(dir.join(META_FILE))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let mut names: Vec<String> = fs::read_dir(dir)
            .map_err(|e| format!("Could not read the folder: {e}"))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".py"))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());

        let mut files = Vec::new();
        for n in &names {
            let text = fs::read_to_string(dir.join(n)).map_err(|e| format!("Could not read {n}: {e}"))?;
            // Editor works with \n only; we keep it that way on save too.
            files.push(SourceFile::new(n, text.replace("\r\n", "\n")));
        }
        let legacy = dir.join("project.json").exists() && dir.join("gui.py").exists();
        if files.is_empty() {
            files.push(SourceFile::new("main.py", String::new()));
            files[0].saved_text = "\u{0}".into(); // mark as unsaved so it gets written
        }
        let main_file = if files.iter().any(|f| f.name == meta.main_file) {
            meta.main_file.clone()
        } else if files.iter().any(|f| f.name == "main.py") {
            "main.py".into()
        } else {
            files[0].name.clone()
        };
        // main file first, then alphabetical
        files.sort_by_key(|f| (f.name != main_file, f.name.to_lowercase()));
        Ok(Project { dir: dir.to_path_buf(), files, main_file, uml_positions: meta.uml_positions, meta_dirty: false, legacy, loose: false })
    }

    /// A single file without a project (dropped on the welcome screen).
    pub fn open_single(path: &Path) -> Result<Project, String> {
        let dir = path.parent().ok_or("Not a file.")?.to_path_buf();
        let mut p = Project {
            dir,
            files: Vec::new(),
            main_file: String::new(),
            uml_positions: BTreeMap::new(),
            meta_dirty: false,
            legacy: false,
            loose: true,
        };
        p.open_file(path)?;
        p.main_file = p.files[0].name.clone();
        Ok(p)
    }

    /// Where the file of a tab is on disk.
    pub fn path_of(&self, idx: usize) -> PathBuf {
        let f = &self.files[idx];
        f.external.clone().unwrap_or_else(|| self.dir.join(&f.name))
    }

    pub fn index_of_path(&self, path: &Path) -> Option<usize> {
        (0..self.files.len()).find(|&i| same_path(&self.path_of(i), path))
    }

    /// Open an existing file in a tab and return the tab's index. The file is
    /// edited where it is; if it is open already, that tab is returned.
    pub fn open_file(&mut self, path: &Path) -> Result<usize, String> {
        if let Some(i) = self.index_of_path(path) {
            return Ok(i);
        }
        let text = read_source(path)?;
        let file_name = path.file_name().ok_or("Not a file.")?.to_string_lossy().into_owned();
        let in_project = !self.loose && path.parent().is_some_and(|d| same_path(d, &self.dir));
        let mut name = file_name.clone();
        let mut n = 2;
        while self.index_of(&name).is_some() {
            name = format!("{file_name} ({n})");
            n += 1;
        }
        let mut file = SourceFile::new(&name, text);
        if !in_project || name != file_name {
            file.external = Some(path.to_path_buf());
        }
        self.files.push(file);
        Ok(self.files.len() - 1)
    }

    /// Close the tab of a dropped file (the file itself is kept).
    pub fn close_file(&mut self, idx: usize) {
        if self.files.len() < 2 || self.files[idx].external.is_none() {
            return;
        }
        let f = self.files.remove(idx);
        if self.main_file == f.name {
            self.main_file = self.files[0].name.clone();
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.files.iter().any(SourceFile::is_dirty)
    }

    pub fn mark_meta_dirty(&mut self) {
        self.meta_dirty = true;
    }

    pub fn save(&mut self) -> Result<(), String> {
        for i in 0..self.files.len() {
            let path = self.path_of(i);
            let f = &mut self.files[i];
            if f.is_dirty() {
                fs::write(path, &f.text).map_err(|e| format!("Could not save {}: {e}", f.name))?;
                f.saved_text = f.text.clone();
            }
        }
        self.save_meta()
    }

    /// Saves only the UML layout / main file (never asks the user).
    pub fn save_meta(&mut self) -> Result<(), String> {
        if !self.meta_dirty || self.loose {
            return Ok(());
        }
        let meta = Meta { version: 2, main_file: self.main_file.clone(), uml_positions: self.uml_positions.clone() };
        let json = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
        fs::write(self.dir.join(META_FILE), json).map_err(|e| format!("Could not save {META_FILE}: {e}"))?;
        self.meta_dirty = false;
        Ok(())
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.files.iter().position(|f| f.name == name)
    }

    pub fn add_file(&mut self, name: &str) -> Result<usize, String> {
        let name = python_file_name(name)?;
        if self.index_of(&name).is_some() || self.dir.join(&name).exists() {
            return Err(format!("'{name}' already exists."));
        }
        fs::write(self.dir.join(&name), "").map_err(|e| format!("Could not create {name}: {e}"))?;
        self.files.push(SourceFile::new(&name, String::new()));
        Ok(self.files.len() - 1)
    }

    pub fn rename_file(&mut self, idx: usize, new_name: &str) -> Result<(), String> {
        let new_name = python_file_name(new_name)?;
        let old = self.files[idx].name.clone();
        if new_name == old {
            return Ok(());
        }
        let old_path = self.path_of(idx);
        let new_path = old_path.with_file_name(&new_name);
        if self.index_of(&new_name).is_some() || new_path.exists() {
            return Err(format!("'{new_name}' already exists."));
        }
        if old_path.exists() {
            fs::rename(&old_path, &new_path).map_err(|e| format!("Could not rename: {e}"))?;
        }
        if self.files[idx].external.is_some() {
            self.files[idx].external = Some(new_path);
        }
        self.files[idx].name = new_name.clone();
        if self.main_file == old {
            self.main_file = new_name;
            self.meta_dirty = true;
        }
        Ok(())
    }

    pub fn delete_file(&mut self, idx: usize) -> Result<(), String> {
        let name = self.files[idx].name.clone();
        let path = self.path_of(idx);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("Could not delete {name}: {e}"))?;
        }
        self.files.remove(idx);
        if self.files.is_empty() {
            self.files.push(SourceFile::new("main.py", String::new()));
            self.files[0].saved_text = "\u{0}".into();
        }
        if self.main_file == name {
            self.main_file = self.files[0].name.clone();
            self.meta_dirty = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_open_save() {
        let tmp = std::env::temp_dir().join(format!("edupython-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let mut p = Project::create(&tmp, "Demo", Template::Classes).unwrap();
        assert_eq!(p.files[0].name, "main.py");
        assert_eq!(p.files.len(), 2);
        let i = p.add_file("player").unwrap();
        p.files[i].text = "class Player:\n    pass\n".into();
        assert!(p.is_dirty());
        p.save().unwrap();
        p.uml_positions.insert("Player".into(), [10.0, 20.0]);
        p.mark_meta_dirty();
        p.save_meta().unwrap();
        let p2 = Project::open(&tmp.join("Demo")).unwrap();
        assert_eq!(p2.files.len(), 3);
        assert_eq!(p2.uml_positions["Player"], [10.0, 20.0]);
        assert!(python_file_name("1bad").is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dropped_files_are_edited_in_place() {
        let tmp = std::env::temp_dir().join(format!("edupython-drop-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let other = tmp.join("other");
        fs::create_dir_all(&other).unwrap();
        let ext = other.join("main.py");
        fs::write(&ext, "print(1)\r\n").unwrap();

        let mut p = Project::create(&tmp, "Demo", Template::Script).unwrap();
        let project_main = fs::read_to_string(tmp.join("Demo/main.py")).unwrap();
        let i = p.open_file(&ext).unwrap();
        assert_eq!((i, p.files[i].name.as_str(), p.files[i].text.as_str()), (1, "main.py (2)", "print(1)\n"));
        assert_eq!(p.open_file(&ext).unwrap(), i, "no second tab for the same file");
        p.files[i].text = "print(2)\n".into();
        p.save().unwrap();
        assert_eq!(fs::read_to_string(&ext).unwrap(), "print(2)\n");
        assert_eq!(fs::read_to_string(tmp.join("Demo/main.py")).unwrap(), project_main);
        p.rename_file(i, "hello").unwrap();
        assert!(other.join("hello.py").exists());
        p.close_file(i);
        assert_eq!(p.files.len(), 1);
        assert!(other.join("hello.py").exists());
        p.close_file(0);
        assert_eq!(p.files.len(), 1, "project files cannot be closed");

        // A new file in the project folder becomes a normal project file.
        fs::write(tmp.join("Demo/new.py"), "").unwrap();
        let i = p.open_file(&tmp.join("Demo/new.py")).unwrap();
        assert!(p.files[i].external.is_none());

        // Without a project: only that file, and no edupython.json next to it.
        let mut p = Project::open_single(&other.join("hello.py")).unwrap();
        p.mark_meta_dirty();
        p.save().unwrap();
        assert_eq!(p.files.len(), 1);
        assert!(!other.join(META_FILE).exists());
        assert!(Project::open_single(&other.join("missing.py")).is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn pong_template_shows_paddle_and_ball() {
        let files = Template::Pygame.files();
        assert_eq!(files.len(), 1);
        let classes = crate::uml::parser::parse_project(files.iter().map(|(n, t)| (*n, *t)));
        let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Paddle", "Ball"]);
        assert!(classes.iter().all(|c| c.bases.is_empty()));
        let ball = &classes[1];
        for a in ["x", "y", "r", "v", "score", "active", "vx", "vy", "gy"] {
            assert!(ball.attributes.iter().any(|x| x.name == a), "Ball.{a} missing");
        }
    }
}
