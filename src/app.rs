//! The EduPython application window.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use eframe::egui::{
    self, Align, Color32, Context, Id, Key, KeyboardShortcut, Layout, Modifiers, RichText, Ui, ViewportCommand,
};

use crate::booklet::Booklet;
use crate::config::SchoolConfig;
use crate::console::{Console, ConsoleAction, Kind, parse_traceback_location};
use crate::editor::{self, EditorOptions};
use crate::icons::{Icon, IconButton};
use crate::project::{Project, Template, python_file_name};
use crate::python::{self, PythonInfo, RunEvent, Runner, Source};
use crate::settings::Settings;
use crate::uml::codegen;
use crate::uml::model::ClassInfo;
use crate::uml::parser::parse_project;
use crate::uml::svg;
use crate::uml::view::{UmlAction, UmlView};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The app icon (also used for the window / Dock icon in `main.rs`).
pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

enum Pending {
    Close,
    NewProject,
    OpenProject(Option<PathBuf>),
}

enum Dialog {
    NewProject { name: String, location: String, template: Template, error: String },
    NewFile { name: String, error: String },
    RenameFile { idx: usize, name: String, error: String },
    ConfirmDeleteFile { idx: usize },
    Python { input: String, error: String },
    About,
    Shortcuts,
    Unsaved(Pending),
    AddClass { name: String, base: String, own_file: bool, add_import: bool, error: String },
    AddAttribute { class: String, name: String, ty: String, as_param: bool, error: String },
    AddMethod { class: String, name: String, params: String, ret: String, constructor: bool, error: String },
    ConfirmDeleteMethod { class: String, line: usize, name: String },
}

#[derive(Clone, Copy, PartialEq)]
enum RunKind {
    Program,
    PipInstall,
}

pub struct EduPythonApp {
    settings: Settings,
    project: Option<Project>,
    active: usize,
    python: Option<PythonInfo>,
    python_rx: Option<Receiver<PythonInfo>>,
    runner: Option<Runner>,
    run_kind: RunKind,
    console: Console,
    stderr_buf: String,
    classes: Vec<ClassInfo>,
    uml_stale: bool,
    uml: UmlView,
    dialog: Option<Dialog>,
    error_marker: Option<(String, usize)>,
    goto: Option<(usize, usize)>,
    cursor_line: Option<usize>,
    allow_close: bool,
    window_title: String,
    status: String,
    /// Answer of the "unsaved changes" dialog (saved or discarded).
    dialog_result: Option<bool>,
    /// App icon shown on the welcome screen.
    logo: Option<egui::TextureHandle>,
    /// edupython.ini installed by the school (or defaults).
    school: SchoolConfig,
    booklet: Option<Booklet>,
}

fn shortcut(m: Modifiers, k: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(m, k)
}

impl EduPythonApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open_arg: Option<PathBuf>, config: Option<PathBuf>) -> Self {
        let settings = Settings::load();
        let ctx = &cc.egui_ctx;
        ctx.set_theme(if settings.dark { egui::Theme::Dark } else { egui::Theme::Light });
        ctx.set_zoom_factor(settings.ui_zoom.clamp(0.6, 2.5));
        // Zoom (booklet, UML) with Ctrl + mouse wheel on every platform, and also
        // Cmd + wheel on a Mac. Trackpad pinch works everywhere.
        ctx.options_mut(|o| {
            o.input_options.zoom_modifier = Modifiers { ctrl: true, command: true, mac_cmd: true, ..Default::default() };
        });
        ctx.global_style_mut(|s| {
            s.spacing.button_padding = egui::vec2(8.0, 4.0);
        });

        let logo = eframe::icon_data::from_png_bytes(ICON_PNG).ok().map(|icon| {
            let image = egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
            let options = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
            ctx.load_texture("edupython_logo", image, options)
        });

        let school = SchoolConfig::load(config.as_deref());
        let booklet = school.booklet.clone().map(|p| Booklet::new(p, school.booklet_title.clone(), settings.booklet_page));
        let custom = if school.allow_change { settings.python_command.clone() } else { None };
        let python_rx = Some(python::detect_async(school.clone(), custom, ctx.clone()));
        let mut app = Self {
            settings,
            logo,
            school,
            booklet,
            project: None,
            active: 0,
            python: None,
            python_rx,
            runner: None,
            run_kind: RunKind::Program,
            console: Console::default(),
            stderr_buf: String::new(),
            classes: Vec::new(),
            uml_stale: true,
            uml: UmlView::default(),
            dialog: None,
            error_marker: None,
            goto: None,
            cursor_line: None,
            allow_close: false,
            window_title: String::new(),
            status: String::new(),
            dialog_result: None,
        };
        // Always start on the welcome screen, unless a folder was given on the command line.
        if let Some(dir) = open_arg.filter(|d| d.is_dir()) {
            app.open_project(&dir);
        }
        app
    }

    // ── helpers ───────────────────────────────────────────────────────────

    fn is_running(&self) -> bool {
        self.runner.as_ref().is_some_and(Runner::is_running)
    }

    fn is_dirty(&self) -> bool {
        self.project.as_ref().is_some_and(Project::is_dirty)
    }

    fn info(&mut self, msg: impl Into<String>) {
        self.console.end_line();
        self.console.push(Kind::Info, &(msg.into() + "\n"));
    }

    fn error(&mut self, msg: impl Into<String>) {
        self.console.end_line();
        self.console.push(Kind::Err, &(msg.into() + "\n"));
    }

    fn editor_id(&self, idx: usize) -> Id {
        let p = self.project.as_ref().unwrap();
        Id::unique(("editor", p.dir.clone(), p.files[idx].name.clone()))
    }

    fn set_dark(&mut self, ctx: &Context, dark: bool) {
        self.settings.dark = dark;
        ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
        self.settings.save();
    }

    // ── project actions ───────────────────────────────────────────────────

    fn set_project(&mut self, project: Project) {
        self.close_project_quietly();
        self.settings.add_recent(&project.dir.to_string_lossy());
        self.settings.save();
        self.console.clear();
        if project.legacy {
            self.console.push(
                Kind::Hint,
                "This project was made with an older version that had a visual tkinter designer.\n\
                 gui.py is now shown as a normal file – the visual GUI designer no longer exists.\n",
            );
        }
        self.project = Some(project);
        self.active = 0;
        self.uml_stale = true;
        self.uml.fit_request = true;
        self.error_marker = None;
    }

    fn close_project_quietly(&mut self) {
        if let Some(r) = self.runner.as_mut() {
            r.kill();
        }
        self.runner = None;
        if let Some(p) = self.project.as_mut() {
            let _ = p.save_meta();
        }
    }

    fn open_project(&mut self, dir: &Path) {
        match Project::open(dir) {
            Ok(p) => self.set_project(p),
            Err(e) => {
                self.settings.recent_projects.retain(|r| Path::new(r) != dir);
                self.settings.save();
                self.error(e);
            }
        }
    }

    /// Ask about unsaved changes before doing `p`.
    fn request(&mut self, p: Pending) {
        if self.is_dirty() {
            self.dialog = Some(Dialog::Unsaved(p));
        } else {
            self.perform(p);
        }
    }

    fn perform(&mut self, p: Pending) {
        match p {
            Pending::Close => {
                self.allow_close = true;
            }
            Pending::NewProject => {
                // The school's folder (ini `[projects] folder`) wins, so every
                // new project starts in e.g. H:\Documents.
                let location = self
                    .school
                    .projects_folder
                    .as_ref()
                    .map(|d| d.to_string_lossy().into_owned())
                    .or_else(|| self.project.as_ref().and_then(|p| p.dir.parent().map(|d| d.to_string_lossy().into_owned())))
                    .or_else(|| dirs::document_dir().map(|d| d.to_string_lossy().into_owned()))
                    .unwrap_or_default();
                self.dialog = Some(Dialog::NewProject {
                    name: String::new(),
                    location,
                    template: Template::Script,
                    error: String::new(),
                });
            }
            Pending::OpenProject(dir) => {
                let dir = dir.or_else(|| {
                    let mut d = rfd::FileDialog::new().set_title("Open a project folder");
                    if let Some(f) = self.school.projects_folder.as_ref().filter(|f| f.is_dir()) {
                        d = d.set_directory(f);
                    }
                    d.pick_folder()
                });
                if let Some(dir) = dir {
                    self.open_project(&dir);
                }
            }
        }
    }

    fn save(&mut self) -> bool {
        let Some(p) = self.project.as_mut() else { return true };
        match p.save() {
            Ok(()) => {
                self.status = "Saved".into();
                true
            }
            Err(e) => {
                self.error(e);
                false
            }
        }
    }

    // ── running ───────────────────────────────────────────────────────────

    /// Look for Python again (in the background).
    fn redetect_python(&mut self, ctx: &Context) {
        let custom = if self.school.allow_change { self.settings.python_command.clone() } else { None };
        self.python = None;
        self.python_rx = Some(python::detect_async(self.school.clone(), custom, ctx.clone()));
    }

    fn python_ready(&mut self) -> Option<PythonInfo> {
        match &self.python {
            None => {
                self.info("Still looking for Python… try again in a moment.");
                None
            }
            Some(info) if info.source == Source::NotFound => {
                if self.school.allow_change {
                    self.error(
                        "No Python 3 interpreter was found. Install Python from python.org, \
                         or choose one in File > Python Interpreter…",
                    );
                } else {
                    self.error("No Python 3 interpreter was found. Please tell your teacher.");
                }
                None
            }
            Some(info) => Some(info.clone()),
        }
    }

    fn run(&mut self, ctx: &Context, current_file: bool) {
        if self.is_running() || self.project.is_none() {
            return;
        }
        if !self.save() {
            return;
        }
        let Some(info) = self.python_ready() else { return };
        let p = self.project.as_ref().unwrap();
        let script = if current_file { p.files[self.active].name.clone() } else { p.main_file.clone() };
        let dir = p.dir.clone();
        self.console.clear();
        self.stderr_buf.clear();
        self.error_marker = None;
        self.console.push(Kind::Info, &format!("Running {script}   ({})\n", info.version));
        match Runner::start(&info.exec, &dir, &[&script], ctx) {
            Ok(r) => {
                self.runner = Some(r);
                self.run_kind = RunKind::Program;
            }
            Err(e) => self.error(e),
        }
    }

    fn install_pygame(&mut self, ctx: &Context) {
        if self.is_running() {
            return;
        }
        let Some(info) = self.python_ready() else { return };
        let dir = self.project.as_ref().map(|p| p.dir.clone()).unwrap_or_else(std::env::temp_dir);
        self.console.clear();
        self.info(format!("Installing pygame for {} …", info.exec.display()));
        match Runner::start(&info.exec, &dir, &["-m", "pip", "install", "--upgrade", "pygame"], ctx) {
            Ok(r) => {
                self.runner = Some(r);
                self.run_kind = RunKind::PipInstall;
            }
            Err(e) => self.error(e),
        }
    }

    fn stop(&mut self) {
        if let Some(r) = self.runner.as_mut() {
            if r.is_running() {
                r.kill();
                self.console.end_line();
                self.console.push(Kind::Info, "Stopped\n");
            }
        }
    }

    fn poll_runner(&mut self, ctx: &Context) {
        let Some(r) = self.runner.as_mut() else { return };
        for ev in r.poll() {
            match ev {
                RunEvent::Stdout(s) => {
                    self.console.push(Kind::Out, &s);
                    if !s.ends_with('\n') {
                        // probably an input() prompt
                        self.console.focus_input = true;
                    }
                }
                RunEvent::Stderr(s) => {
                    self.stderr_buf.push_str(&s);
                    self.console.push(Kind::Err, &s);
                }
                RunEvent::Exit(code) => {
                    self.console.end_line();
                    self.after_exit(code, ctx);
                }
            }
        }
    }

    fn after_exit(&mut self, code: Option<i32>, ctx: &Context) {
        if self.run_kind == RunKind::PipInstall {
            self.info("Checking Python again…");
            self.redetect_python(ctx);
            return;
        }
        // Mark the last line of *our* code mentioned in the traceback.
        let mut marker = None;
        for line in self.stderr_buf.lines() {
            if let Some((path, l)) = parse_traceback_location(line) {
                if let Some(idx) = self.project_file_for(&path) {
                    marker = Some((self.project.as_ref().unwrap().files[idx].name.clone(), l));
                }
            }
        }
        if let Some((file, line)) = &marker {
            let msg = self.stderr_buf.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
            self.console.push(Kind::Hint, &format!("Problem in {file}, line {line}: {msg}\n"));
            if let Some(idx) = self.project.as_ref().unwrap().index_of(file) {
                self.active = idx;
                self.goto = Some((idx, *line));
            }
        }
        if let Some(module) = missing_module(&self.stderr_buf) {
            let tip = if module == "pygame" && self.school.allow_install {
                "Tip: pygame is not installed. Click the Python version in the toolbar to install it.\n".to_string()
            } else {
                format!("Tip: the package '{module}' is not installed on this computer. Ask your teacher.\n")
            };
            self.console.push(Kind::Hint, &tip);
        }
        self.error_marker = marker;
        match code {
            Some(0) => self.info("── Program finished ──"),
            Some(c) => self.info(format!("── Program finished with exit code {c} ──")),
            None => {}
        }
    }

    /// Map a path printed by Python to one of the project's files.
    fn project_file_for(&self, path: &str) -> Option<usize> {
        let p = self.project.as_ref()?;
        let path = Path::new(path);
        let name = path.file_name()?.to_str()?;
        let idx = p.index_of(name)?;
        let parent = path.parent().filter(|d| !d.as_os_str().is_empty());
        match parent {
            None => Some(idx),
            Some(d) => {
                let a = std::fs::canonicalize(d).ok()?;
                let b = std::fs::canonicalize(&p.dir).ok()?;
                (a == b).then_some(idx)
            }
        }
    }

    // ── UML → code ────────────────────────────────────────────────────────

    fn find_class(&self, name: &str) -> Option<ClassInfo> {
        self.classes.iter().find(|c| c.name == name).cloned()
    }

    /// Apply an edit to the file that defines `class`.
    fn edit_class(
        &mut self,
        class: &str,
        f: impl FnOnce(&str, usize) -> Option<(String, usize)>,
    ) -> Result<(), String> {
        let c = self.find_class(class).ok_or_else(|| format!("Class {class} not found."))?;
        let p = self.project.as_mut().ok_or("No project")?;
        let idx = p.index_of(&c.file).ok_or("File not found")?;
        let (text, line) = f(&p.files[idx].text, c.line).ok_or("Could not find the class in the code.")?;
        p.files[idx].text = text;
        self.active = idx;
        self.goto = Some((idx, line));
        self.uml_stale = true;
        Ok(())
    }

    fn add_class(&mut self, name: &str, base: &str, own_file: bool, add_import: bool) -> Result<(), String> {
        if !codegen::is_valid_identifier(name) {
            return Err("The class name must be a valid Python name (letters, digits, _).".into());
        }
        if self.find_class(name).is_some() {
            return Err(format!("A class called {name} already exists."));
        }
        let p = self.project.as_mut().ok_or("No project")?;
        let idx = if own_file {
            let file = python_file_name(&name.to_lowercase())?;
            match p.index_of(&file) {
                Some(i) => i,
                None => p.add_file(&file)?,
            }
        } else {
            self.active.min(p.files.len() - 1)
        };
        let base = base.trim();
        let (text, line) = codegen::add_class(&p.files[idx].text, name, Some(base).filter(|b| !b.is_empty()));
        // Base class from another project file needs an import.
        let mut text = text;
        if let Some(bc) = self.classes.iter().find(|c| c.name == base) {
            if bc.file != p.files[idx].name {
                let module = bc.file.trim_end_matches(".py");
                text = format!("from {module} import {base}\n\n\n{text}");
            }
        }
        let file_name = p.files[idx].name.clone();
        p.files[idx].text = text;
        let line = p.files[idx].text.lines().position(|l| l.starts_with(&format!("class {name}"))).map_or(line, |i| i + 1);
        if own_file && add_import && file_name != p.main_file {
            if let Some(mi) = p.index_of(&p.main_file.clone()) {
                let module = file_name.trim_end_matches(".py");
                let import = format!("from {module} import {name}");
                if !p.files[mi].text.contains(&import) {
                    p.files[mi].text = insert_import(&p.files[mi].text, &import);
                }
            }
        }
        self.active = idx;
        self.goto = Some((idx, line));
        self.uml_stale = true;
        Ok(())
    }

    fn handle_uml_action(&mut self, action: UmlAction) {
        match action {
            UmlAction::Goto { file, line } => {
                if let Some(idx) = self.project.as_ref().and_then(|p| p.index_of(&file)) {
                    self.active = idx;
                    self.goto = Some((idx, line));
                }
            }
            UmlAction::AddClass => {
                self.dialog = Some(Dialog::AddClass {
                    name: String::new(),
                    base: String::new(),
                    own_file: true,
                    add_import: true,
                    error: String::new(),
                })
            }
            UmlAction::AddAttribute { class } => {
                self.dialog = Some(Dialog::AddAttribute {
                    class,
                    name: String::new(),
                    ty: String::new(),
                    as_param: false,
                    error: String::new(),
                })
            }
            UmlAction::AddMethod { class } => {
                self.dialog = Some(Dialog::AddMethod {
                    class,
                    name: String::new(),
                    params: String::new(),
                    ret: String::new(),
                    constructor: false,
                    error: String::new(),
                })
            }
            UmlAction::AddConstructor { class } => {
                self.dialog = Some(Dialog::AddMethod {
                    class,
                    name: "__init__".into(),
                    params: String::new(),
                    ret: String::new(),
                    constructor: true,
                    error: String::new(),
                })
            }
            UmlAction::DeleteMethod { class, line, name } => {
                self.dialog = Some(Dialog::ConfirmDeleteMethod { class, line, name })
            }
            UmlAction::ExportSvg => self.export_svg(),
            UmlAction::LayoutChanged => {
                if let Some(p) = self.project.as_mut() {
                    p.mark_meta_dirty();
                    let _ = p.save_meta();
                }
            }
        }
    }

    fn export_svg(&mut self) {
        let Some(p) = self.project.as_ref() else { return };
        let svg = svg::export(&self.classes, &p.uml_positions);
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export UML diagram")
            .add_filter("SVG image", &["svg"])
            .set_directory(&p.dir)
            .set_file_name(format!("{}_uml.svg", p.name()))
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, svg) {
            Ok(()) => self.info(format!("UML diagram saved to {}", path.display())),
            Err(e) => self.error(format!("Could not save the diagram: {e}")),
        }
    }

    // ── UI: menu / toolbar ────────────────────────────────────────────────

    fn shortcuts(&mut self, ctx: &Context) {
        let cmd = Modifiers::COMMAND;
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if pressed(shortcut(cmd, Key::S)) {
            self.save();
        }
        if pressed(shortcut(cmd, Key::N)) {
            self.request(Pending::NewProject);
        }
        if pressed(shortcut(cmd, Key::O)) {
            self.request(Pending::OpenProject(None));
        }
        if self.booklet.is_some() && pressed(shortcut(cmd, Key::F)) {
            self.settings.show_booklet = true;
            if let Some(b) = self.booklet.as_mut() {
                b.focus_search();
            }
        }
        if self.project.is_some() && pressed(shortcut(cmd, Key::T)) {
            self.dialog = Some(Dialog::NewFile { name: String::new(), error: String::new() });
        }
        if pressed(shortcut(Modifiers::SHIFT, Key::F5)) || pressed(shortcut(cmd | Modifiers::SHIFT, Key::Period)) {
            self.stop();
        }
        if pressed(shortcut(cmd, Key::F5)) || pressed(shortcut(cmd | Modifiers::SHIFT, Key::R)) {
            self.run(ctx, true);
        }
        if pressed(shortcut(Modifiers::NONE, Key::F5)) || pressed(shortcut(cmd, Key::R)) {
            self.run(ctx, false);
        }
        if pressed(shortcut(cmd | Modifiers::ALT, Key::Equals)) || pressed(shortcut(cmd | Modifiers::ALT, Key::Plus)) {
            self.settings.editor_font_size = (self.settings.editor_font_size + 1.0).min(40.0);
        }
        if pressed(shortcut(cmd | Modifiers::ALT, Key::Minus)) {
            self.settings.editor_font_size = (self.settings.editor_font_size - 1.0).max(9.0);
        }
    }

    fn menu_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let has_project = self.project.is_some();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.add(egui::Button::new("New Project…").shortcut_text(ctx.format_shortcut(&shortcut(Modifiers::COMMAND, Key::N)))).clicked() {
                    self.request(Pending::NewProject);
                }
                if ui.add(egui::Button::new("Open Project…").shortcut_text(ctx.format_shortcut(&shortcut(Modifiers::COMMAND, Key::O)))).clicked() {
                    self.request(Pending::OpenProject(None));
                }
                let recent = self.settings.recent_projects.clone();
                ui.add_enabled_ui(!recent.is_empty(), |ui| {
                    ui.menu_button("Open Recent", |ui| {
                        for r in recent {
                            if ui.button(&r).clicked() {
                                self.request(Pending::OpenProject(Some(PathBuf::from(&r))));
                            }
                        }
                    });
                });
                if ui.add_enabled(has_project, egui::Button::new("Save").shortcut_text(ctx.format_shortcut(&shortcut(Modifiers::COMMAND, Key::S)))).clicked() {
                    self.save();
                }
                ui.separator();
                if ui.add_enabled(has_project, egui::Button::new("New File…").shortcut_text(ctx.format_shortcut(&shortcut(Modifiers::COMMAND, Key::T)))).clicked() {
                    self.dialog = Some(Dialog::NewFile { name: String::new(), error: String::new() });
                }
                if ui.add_enabled(has_project, egui::Button::new("Show Project Folder")).clicked() {
                    if let Some(p) = &self.project {
                        ctx.open_url(egui::OpenUrl::new_tab(format!("file://{}", p.dir.display())));
                    }
                }
                ui.separator();
                if ui.button(if self.school.allow_change { "Python Interpreter…" } else { "Python Interpreter" }).clicked() {
                    let input = self
                        .settings
                        .python_command
                        .clone()
                        .or_else(|| self.python.as_ref().map(|p| p.exec.command.clone()))
                        .unwrap_or_default();
                    self.dialog = Some(Dialog::Python { input, error: String::new() });
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                // No project open (start screen): nothing to undo. `editor_id`
                // needs a project, so only ask for it when there is one.
                let id = self.project.as_ref().and_then(|p| p.files.get(self.active)).map(|_| self.editor_id(self.active));
                let (can_undo, can_redo) = match (&self.project, id) {
                    (Some(p), Some(id)) => p.files.get(self.active).map_or((false, false), |f| editor::can_undo_redo(&ctx, id, &f.text)),
                    _ => (false, false),
                };
                let undo = ui.add_enabled(can_undo, egui::Button::new("Undo").shortcut_text(ctx.format_shortcut(&shortcut(Modifiers::COMMAND, Key::Z)))).clicked();
                let redo = ui
                    .add_enabled(can_redo, egui::Button::new("Redo").shortcut_text(ctx.format_shortcut(&shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))))
                    .clicked();
                if (undo || redo)
                    && let Some(id) = id
                    && let Some(f) = self.project.as_mut().and_then(|p| p.files.get_mut(self.active))
                    && editor::undo_redo(&ctx, id, &mut f.text, redo)
                {
                    self.uml_stale = true;
                }
            });
            ui.menu_button("View", |ui| {
                let mut dark = self.settings.dark;
                if ui.checkbox(&mut dark, "Dark mode").changed() {
                    self.set_dark(&ctx, dark);
                }
                if ui.checkbox(&mut self.settings.show_uml, "UML class diagram").changed()
                    || ui.add_enabled(self.settings.show_uml, egui::Checkbox::new(&mut self.settings.uml_grid, "UML grid")).changed()
                    || ui.add_enabled(self.booklet.is_some(), egui::Checkbox::new(&mut self.settings.show_booklet, "Booklet")).changed()
                {
                    self.settings.save();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Keyboard Shortcuts").clicked() {
                    self.dialog = Some(Dialog::Shortcuts);
                }
                if ui.button("About EduPython").clicked() {
                    self.dialog = Some(Dialog::About);
                }
            });
        });
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::new((logo.id(), egui::vec2(22.0, 22.0))));
            }
            ui.label(RichText::new("EduPython").strong().size(16.0));
            if let Some(p) = &self.project {
                ui.label(RichText::new(format!("— {}", p.name())).weak());
            }
            ui.separator();
            if ui.add(IconButton::new(Icon::New).text("New")).on_hover_text("New project").clicked() {
                self.request(Pending::NewProject);
            }
            if ui.add(IconButton::new(Icon::Open).text("Open")).on_hover_text("Open a project folder").clicked() {
                self.request(Pending::OpenProject(None));
            }
            let dirty = self.is_dirty();
            if ui.add_enabled(self.project.is_some(), IconButton::new(Icon::Save).text(if dirty { "Save •" } else { "Save" })).clicked() {
                self.save();
            }
            ui.separator();
            let running = self.is_running();
            let has = self.project.is_some();
            let main = self.project.as_ref().map(|p| p.main_file.clone()).unwrap_or_default();
            let run_btn = IconButton::new(Icon::Run).text(RichText::new("Run").strong()).color(Color32::WHITE).fill(Color32::from_rgb(46, 160, 67));
            if ui.add_enabled(has && !running, run_btn).on_hover_text(format!("Run {main} (F5)")).clicked() {
                self.run(&ctx, false);
            }
            if ui.add_enabled(has && !running, IconButton::new(Icon::RunFile).text("Run this file")).on_hover_text("Run the file that is open in the editor").clicked() {
                self.run(&ctx, true);
            }
            let stop_btn = IconButton::new(Icon::Stop)
                .text("Stop")
                .color(if running { Color32::WHITE } else { ui.visuals().weak_text_color() })
                .fill(if running { Color32::from_rgb(200, 60, 60) } else { ui.visuals().widgets.inactive.weak_bg_fill });
            if ui.add_enabled(running, stop_btn).on_hover_text("Stop the program (Shift+F5)").clicked() {
                self.stop();
            }
            if running {
                ui.spinner();
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let dark = self.settings.dark;
                if ui.add(IconButton::new(if dark { Icon::Sun } else { Icon::Moon })).on_hover_text("Light / dark mode").clicked() {
                    self.set_dark(&ctx, !dark);
                }
                let booklet_hint = match &self.booklet {
                    Some(b) => format!("Show the booklet ({})", b.title),
                    None => format!(
                        "No booklet installed.\nIT can put {} (or a edupython.ini) next to the EduPython program.",
                        crate::config::DEFAULT_BOOKLET
                    ),
                };
                let booklet_btn = ui.add_enabled(self.booklet.is_some(), egui::Button::selectable(self.settings.show_booklet && self.booklet.is_some(), "Booklet"));
                if booklet_btn.on_hover_text(booklet_hint).on_disabled_hover_text("No booklet installed").clicked() {
                    self.settings.show_booklet = !self.settings.show_booklet;
                    self.settings.save();
                }
                if ui.selectable_label(self.settings.show_uml, "UML").on_hover_text("Show the class diagram (left)").clicked() {
                    self.settings.show_uml = !self.settings.show_uml;
                    self.settings.save();
                }
                ui.separator();
                if ui.button("A+").on_hover_text("Bigger code font").clicked() {
                    self.settings.editor_font_size = (self.settings.editor_font_size + 1.0).min(40.0);
                }
                if ui.button("A−").on_hover_text("Smaller code font").clicked() {
                    self.settings.editor_font_size = (self.settings.editor_font_size - 1.0).max(9.0);
                }
                ui.separator();
                match &self.python {
                    None => {
                        ui.spinner();
                        ui.label(RichText::new("looking for Python…").weak());
                    }
                    Some(info) => {
                        let ok = info.source != Source::NotFound;
                        let dot = if ok { Color32::from_rgb(70, 180, 90) } else { Color32::from_rgb(220, 70, 70) };
                        let text = if ok { info.version.clone() } else { "No Python found".into() };
                        let missing = info.missing();
                        let dot = if ok && !missing.is_empty() { Color32::from_rgb(230, 160, 40) } else { dot };
                        let click = if self.school.allow_change { "Click to change" } else { "Click for details" };
                        let resp = ui
                            .add(IconButton::new(Icon::Dot).text(text).icon_color(dot).frame(false))
                            .on_hover_text(if self.school.show_paths {
                                format!("{}\n({})\n{}\n{click}", info.exec.display(), info.source.label(), info.packages_summary())
                            } else {
                                format!("Python {} ({})\n{}\n{click}", info.version, info.source.label(), info.packages_summary())
                            });
                        if resp.clicked() {
                            let input = self.settings.python_command.clone().unwrap_or_else(|| info.exec.command.clone());
                            self.dialog = Some(Dialog::Python { input, error: String::new() });
                        }
                    }
                }
            });
        });
    }

    /// One tab per project file. Tabs cannot be closed: a file disappears
    /// only when it is deleted (right-click > Delete).
    fn file_tabs(&mut self, ui: &mut Ui) {
        let Some(p) = self.project.as_mut() else { return };
        let mut set_main = None;
        let editor_bg = if self.settings.dark { Color32::from_rgb(40, 44, 52) } else { Color32::from_rgb(253, 253, 253) };
        let green = Color32::from_rgb(46, 160, 67);
        let accent = ui.visuals().selection.stroke.color;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
            for (i, f) in p.files.iter().enumerate() {
                let is_main = f.name == p.main_file;
                let active = self.active == i;
                let font = egui::TextStyle::Button.resolve(ui.style());
                let text_color = if active { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
                let galley = ui.painter().layout_no_wrap(f.name.clone(), font, text_color);
                let icon_w = if is_main { 14.0 } else { 0.0 };
                let dot_w = if f.is_dirty() { 12.0 } else { 0.0 };
                let size = egui::vec2(galley.size().x + icon_w + dot_w + 20.0, ui.spacing().interact_size.y + 4.0);
                let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                if ui.is_rect_visible(rect) {
                    let painter = ui.painter();
                    let radius = egui::CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 };
                    let fill = if active {
                        editor_bg
                    } else if resp.hovered() {
                        ui.visuals().widgets.hovered.weak_bg_fill
                    } else {
                        ui.visuals().widgets.inactive.weak_bg_fill
                    };
                    painter.rect_filled(rect, radius, fill);
                    if active {
                        let top = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 2.0));
                        painter.rect_filled(top, egui::CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 }, accent);
                    }
                    let mut x = rect.left() + 10.0;
                    if is_main {
                        let r = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 4.5), egui::Vec2::splat(9.0));
                        crate::icons::paint(painter, r, Icon::Run, green);
                        x += icon_w;
                    }
                    let pos = egui::pos2(x, rect.center().y - galley.size().y / 2.0);
                    let gw = galley.size().x;
                    painter.galley(pos, galley, text_color);
                    if f.is_dirty() {
                        painter.circle_filled(egui::pos2(pos.x + gw + 8.0, rect.center().y), 3.0, Color32::from_rgb(220, 160, 60));
                    }
                }
                let hover = match (is_main, f.is_dirty()) {
                    (true, true) => "Main file, started with Run (F5). Not saved yet.\nRight-click: rename, delete",
                    (true, false) => "Main file, started with Run (F5).\nRight-click: rename, delete",
                    (false, true) => "Not saved yet.\nRight-click: set as main file, rename, delete",
                    (false, false) => "Right-click: set as main file, rename, delete",
                };
                let resp = resp.on_hover_text(hover);
                if resp.clicked() {
                    self.active = i;
                }
                resp.context_menu(|ui| {
                    if ui.add_enabled(!is_main, egui::Button::new("Set as main file")).clicked() {
                        set_main = Some(i);
                    }
                    if ui.button("Rename…").clicked() {
                        self.dialog = Some(Dialog::RenameFile { idx: i, name: f.name.clone(), error: String::new() });
                    }
                    if ui.add_enabled(p.files.len() > 1, egui::Button::new("Delete…")).clicked() {
                        self.dialog = Some(Dialog::ConfirmDeleteFile { idx: i });
                    }
                });
            }
            ui.add_space(4.0);
            if ui.add(IconButton::new(Icon::Plus).small()).on_hover_text("New file (Ctrl+T)").clicked() {
                self.dialog = Some(Dialog::NewFile { name: String::new(), error: String::new() });
            }
        });
        if let Some(i) = set_main {
            p.set_main(i);
            let _ = p.save_meta();
        }
    }

    fn editor_area(&mut self, ui: &mut Ui) {
        if self.project.is_none() {
            self.welcome(ui);
            return;
        }
        self.active = self.active.min(self.project.as_ref().unwrap().files.len() - 1);
        ui.horizontal(|ui| {
            let line = self.cursor_line;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(l) = line {
                    ui.label(RichText::new(format!("Line {l}")).weak());
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| self.file_tabs(ui));
            });
        });
        let idx = self.active.min(self.project.as_ref().unwrap().files.len() - 1);
        self.active = idx;
        let id = self.editor_id(idx);
        let dark = self.settings.dark;
        let p = self.project.as_mut().unwrap();
        let file = &mut p.files[idx];

        let error_line = self.error_marker.as_ref().filter(|(f, _)| *f == file.name).map(|(_, l)| *l);
        let goto_line = self.goto.filter(|(i, _)| *i == idx).map(|(_, l)| l);
        if goto_line.is_some() {
            self.goto = None;
        }
        let bg = if dark { Color32::from_rgb(40, 44, 52) } else { Color32::from_rgb(253, 253, 253) };
        // Ctrl + mouse wheel or trackpad pinch over the code changes the code size.
        let area = ui.available_rect_before_wrap();
        let (zoom, pointer) = ui.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
        if pointer.is_some_and(|p| area.contains(p)) && (zoom - 1.0).abs() > 1e-4 {
            self.settings.editor_font_size = (self.settings.editor_font_size * zoom).clamp(9.0, 40.0);
        }
        let resp = editor::show(
            ui,
            id,
            &mut file.text,
            EditorOptions { font_size: self.settings.editor_font_size, dark, error_line, goto_line, bg },
        );
        if resp.cursor_line.is_some() {
            self.cursor_line = resp.cursor_line;
        }
        if resp.changed {
            self.uml_stale = true;
            if error_line.is_some() {
                self.error_marker = None;
            }
        }
    }

    fn welcome(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.12);
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::new((logo.id(), egui::vec2(128.0, 128.0))));
                ui.add_space(8.0);
            }
            ui.label(RichText::new("EduPython").size(34.0).strong());
            ui.label(RichText::new("Write Python, run it, and see your classes as UML.").size(15.0).weak());
            ui.add_space(24.0);
            if ui.add(IconButton::new(Icon::New).text(RichText::new("New Project…").size(16.0)).min_size(egui::vec2(240.0, 36.0))).clicked() {
                self.request(Pending::NewProject);
            }
            ui.add_space(6.0);
            if ui.add(IconButton::new(Icon::Open).text(RichText::new("Open Project Folder…").size(16.0)).min_size(egui::vec2(240.0, 36.0))).clicked() {
                self.request(Pending::OpenProject(None));
            }
            if !self.settings.recent_projects.is_empty() {
                ui.add_space(24.0);
                ui.label(RichText::new("Recent projects").strong());
                for r in self.settings.recent_projects.clone() {
                    let name = Path::new(&r).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(r.clone());
                    if ui.link(name).on_hover_text(&r).clicked() {
                        self.open_project(Path::new(&r));
                    }
                }
            }
        });
    }

    // ── dialogs ───────────────────────────────────────────────────────────

    fn dialogs(&mut self, ctx: &Context) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        let class_names: Vec<String> = self.classes.iter().map(|c| c.name.clone()).collect();

        let modal = egui::Modal::new(Id::unique("edupython_dialog")).show(ctx, |ui| {
            ui.set_min_width(380.0);
            match &mut dialog {
                Dialog::NewProject { name, location, template, error } => {
                    ui.heading("New Project");
                    ui.add_space(6.0);
                    ui.label("Project name:");
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text("e.g. Exercise1").desired_width(f32::INFINITY));
                    if ui.memory(|m| m.focused().is_none()) {
                        r.request_focus();
                    }
                    ui.label("Location:");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(location).desired_width(280.0));
                        if ui.button("Choose…").clicked() {
                            let mut picker = rfd::FileDialog::new().set_title("Where should the project be created?");
                            if Path::new(location.trim()).is_dir() {
                                picker = picker.set_directory(location.trim());
                            }
                            if let Some(d) = picker.pick_folder() {
                                *location = d.to_string_lossy().into_owned();
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.label("Start with:");
                    for t in Template::ALL {
                        ui.radio_value(template, t, t.label());
                    }
                    show_error(ui, error);
                    let (ok, cancel) = ok_cancel(ui, "Create", enter);
                    if ok {
                        match Project::create(Path::new(location.trim()), name, *template) {
                            Ok(p) => {
                                self.set_project(p);
                                keep = false;
                            }
                            Err(e) => *error = e,
                        }
                    }
                    keep &= !cancel;
                }
                Dialog::NewFile { name, error } => {
                    ui.heading("New File");
                    ui.label("File name:");
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text("e.g. player.py").desired_width(f32::INFINITY));
                    r.request_focus();
                    ui.label(RichText::new("Tip for 1CB: put each class in its own file, e.g. ball.py for class Ball.").weak().small());
                    show_error(ui, error);
                    let (ok, cancel) = ok_cancel(ui, "Create", enter);
                    if ok {
                        match self.project.as_mut().unwrap().add_file(name) {
                            Ok(i) => {
                                self.active = i;
                                keep = false;
                            }
                            Err(e) => *error = e,
                        }
                    }
                    keep &= !cancel;
                }
                Dialog::RenameFile { idx, name, error } => {
                    ui.heading("Rename File");
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                    r.request_focus();
                    ui.label(RichText::new("Remember to update import lines in other files.").weak().small());
                    show_error(ui, error);
                    let (ok, cancel) = ok_cancel(ui, "Rename", enter);
                    if ok {
                        let p = self.project.as_mut().unwrap();
                        match p.rename_file(*idx, name) {
                            Ok(()) => {
                                let _ = p.save_meta();
                                self.uml_stale = true;
                                keep = false;
                            }
                            Err(e) => *error = e,
                        }
                    }
                    keep &= !cancel;
                }
                Dialog::ConfirmDeleteFile { idx } => {
                    let name = self.project.as_ref().unwrap().files[*idx].name.clone();
                    ui.heading("Delete File");
                    ui.label(format!("Really delete {name}? This cannot be undone."));
                    let (ok, cancel) = ok_cancel(ui, "Delete", false);
                    if ok {
                        let p = self.project.as_mut().unwrap();
                        match p.delete_file(*idx) {
                            Ok(()) => {
                                let _ = p.save_meta();
                                self.active = 0;
                                self.uml_stale = true;
                            }
                            Err(e) => self.error(e),
                        }
                        keep = false;
                    }
                    keep &= !cancel;
                }
                Dialog::Python { input, error } => {
                    ui.heading("Python Interpreter");
                    let mut install = false;
                    if let Some(info) = &self.python {
                        let show_paths = self.school.show_paths;
                        egui::Grid::new("pyinfo").num_columns(2).show(ui, |ui| {
                            if show_paths {
                                ui.label("In use:");
                                ui.label(info.exec.display());
                                ui.end_row();
                            }
                            ui.label("Version:");
                            ui.label(format!("{} ({})", info.version, info.source.label()));
                            ui.end_row();
                            for (name, ok) in &info.packages {
                                ui.label(format!("{name}:"));
                                if *ok {
                                    ui.label("installed");
                                } else if name == "pygame" && self.school.allow_install {
                                    install = ui.add_enabled(!self.is_running(), egui::Button::new("Install pygame")).clicked();
                                } else {
                                    ui.label(RichText::new("not installed").color(Color32::from_rgb(220, 70, 70)));
                                }
                                ui.end_row();
                            }
                            if show_paths && !info.exec.extra_path.is_empty() {
                                ui.label("Package folders:");
                                ui.label(info.exec.extra_path.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
                                ui.end_row();
                            }
                            if let Some(ini) = self.school.source.as_ref().filter(|_| show_paths) {
                                ui.label("School settings:");
                                ui.label(ini.display().to_string());
                                ui.end_row();
                            }
                        });
                        ui.separator();
                    }
                    if install {
                        self.install_pygame(ctx);
                        keep = false;
                    }
                    if !self.school.allow_change {
                        ui.label(RichText::new("Python is set up by your school and cannot be changed here.").weak());
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.button("Check again").clicked() {
                                self.redetect_python(ctx);
                                keep = false;
                            }
                            if ui.button("Close").clicked() || enter {
                                keep = false;
                            }
                        });
                    } else {
                    ui.label("Use this interpreter:");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(input).hint_text("e.g. C:\\Python313\\python.exe or /usr/bin/python3").desired_width(300.0));
                        if ui.button("Browse…").clicked() {
                            if let Some(f) = rfd::FileDialog::new().set_title("Choose the Python program").pick_file() {
                                *input = f.to_string_lossy().into_owned();
                            }
                        }
                    });
                    show_error(ui, error);
                    ui.horizontal(|ui| {
                        if ui.button("Use this").clicked() {
                            match python::validate(input) {
                                Ok(_) => {
                                    self.settings.python_command = Some(input.trim().to_string());
                                    self.settings.save();
                                    self.redetect_python(ctx);
                                    keep = false;
                                }
                                Err(e) => *error = e,
                            }
                        }
                        if ui.button("Find automatically").clicked() {
                            self.settings.python_command = None;
                            self.settings.save();
                            self.redetect_python(ctx);
                            keep = false;
                        }
                        if ui.button("Close").clicked() {
                            keep = false;
                        }
                    });
                    }
                }
                Dialog::About => {
                    ui.vertical_centered(|ui| {
                        if let Some(logo) = &self.logo {
                            ui.add(egui::Image::new((logo.id(), egui::vec2(72.0, 72.0))));
                            ui.add_space(4.0);
                        }
                        ui.heading("EduPython");
                        ui.label(format!("Version {VERSION} (native)"));
                        ui.add_space(6.0);
                        ui.label("Dren Gashi");
                        ui.hyperlink_to("www.edupython.lu", "https://www.edupython.lu");
                        ui.hyperlink_to("github.com/dren413/EduPython", "https://github.com/dren413/EduPython");
                        ui.add_space(6.0);
                        ui.label(RichText::new("A Python editor for classes 2CB (code) and 1CB (classes, UML, pygame).").weak());
                        ui.add_space(8.0);
                        if ui.button("Close").clicked() || enter {
                            keep = false;
                        }
                    });
                }
                Dialog::Shortcuts => {
                    ui.heading("Keyboard Shortcuts");
                    egui::Grid::new("keys").num_columns(2).striped(true).show(ui, |ui| {
                        for (k, v) in [
                            ("F5  /  Ctrl+R", "Run main file"),
                            ("Ctrl+F5  /  Ctrl+Shift+R", "Run the open file"),
                            ("Shift+F5", "Stop"),
                            ("Ctrl+S", "Save"),
                            ("Ctrl+N / Ctrl+O", "New / open project"),
                            ("Ctrl+T", "New file"),
                            ("Ctrl+F", "Search the booklet"),
                            ("Ctrl + mouse wheel / pinch", "Zoom the code or the booklet (under the mouse)"),
                            ("Ctrl+/", "Comment / uncomment lines"),
                            ("Tab / Shift+Tab", "Indent / unindent (also several lines)"),
                            ("Ctrl+Z / Ctrl+Shift+Z", "Undo / redo"),
                            ("Ctrl+Alt + / −", "Code font size"),
                            ("Ctrl + / −", "Zoom whole window"),
                        ] {
                            ui.monospace(k);
                            ui.label(v);
                            ui.end_row();
                        }
                    });
                    ui.label(RichText::new("On a Mac, Ctrl and ⌘ both work.").weak());
                    if ui.button("Close").clicked() || enter {
                        keep = false;
                    }
                }
                Dialog::Unsaved(_) => {
                    ui.heading("Unsaved changes");
                    ui.label("Do you want to save your changes first?");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() || enter {
                            if self.save() {
                                keep = false;
                                self.dialog_result = Some(true);
                            }
                        }
                        if ui.button("Don't save").clicked() {
                            keep = false;
                            self.dialog_result = Some(false);
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::AddClass { name, base, own_file, add_import, error } => {
                    ui.heading("New Class");
                    ui.label("Class name:");
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text("e.g. Player").desired_width(f32::INFINITY));
                    r.request_focus();
                    ui.label("Inherits from (optional):");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(base).hint_text("e.g. Animal or pygame.sprite.Sprite").desired_width(260.0));
                        egui::ComboBox::from_id_salt("base_pick").selected_text("pick").width(70.0).show_ui(ui, |ui| {
                            ui.selectable_value(base, String::new(), "(none)");
                            for c in &class_names {
                                ui.selectable_value(base, c.clone(), c);
                            }
                            ui.selectable_value(base, "pygame.sprite.Sprite".to_string(), "pygame.sprite.Sprite");
                        });
                    });
                    ui.checkbox(own_file, format!("In its own file ({}.py)", if name.is_empty() { "name".into() } else { name.to_lowercase() }));
                    ui.add_enabled(*own_file, egui::Checkbox::new(add_import, "Add the import to the main file"));
                    show_error(ui, error);
                    let (ok, cancel) = ok_cancel(ui, "Add class", enter);
                    if ok {
                        let (n, b, o, a) = (name.trim().to_string(), base.clone(), *own_file, *add_import);
                        match self.add_class(&n, &b, o, a) {
                            Ok(()) => keep = false,
                            Err(e) => *error = e,
                        }
                    }
                    keep &= !cancel;
                }
                Dialog::AddAttribute { class, name, ty, as_param, error } => {
                    ui.heading(format!("New attribute in {class}"));
                    ui.label("Name:");
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text("e.g. score").desired_width(f32::INFINITY));
                    r.request_focus();
                    ui.label("Type:");
                    type_field(ui, "attr_ty", ty, &class_names);
                    ui.checkbox(as_param, "Set it from a constructor parameter (__init__)");
                    preview(ui, &if ty.trim().is_empty() { name.clone() } else { format!("{}: {}", name, ty.trim()) });
                    show_error(ui, error);
                    let (ok, cancel) = ok_cancel(ui, "Add attribute", enter);
                    if ok {
                        let n = name.trim().to_string();
                        let t = ty.trim().to_string();
                        let exists = self.find_class(class).is_some_and(|c| c.attributes.iter().any(|a| a.name == n));
                        if !codegen::is_valid_identifier(&n) {
                            *error = "The name must be a valid Python name (letters, digits, _).".into();
                        } else if exists {
                            *error = format!("{class} already has an attribute {n}.");
                        } else {
                            let as_param = *as_param;
                            match self.edit_class(class, |src, line| {
                                codegen::add_attribute(src, line, &n, Some(t.as_str()).filter(|t| !t.is_empty()), as_param)
                            }) {
                                Ok(()) => keep = false,
                                Err(e) => *error = e,
                            }
                        }
                    }
                    keep &= !cancel;
                }
                Dialog::AddMethod { class, name, params, ret, constructor, error } => {
                    ui.heading(if *constructor { format!("Constructor of {class}") } else { format!("New method in {class}") });
                    if !*constructor {
                        ui.label("Name:");
                        let r = ui.add(egui::TextEdit::singleline(name).hint_text("e.g. move").desired_width(f32::INFINITY));
                        r.request_focus();
                    }
                    ui.label("Parameters (without self):");
                    let r = ui.add(egui::TextEdit::singleline(params).hint_text("e.g. p:(int,int), speed:float").desired_width(f32::INFINITY));
                    if *constructor {
                        r.request_focus();
                        ui.label(RichText::new("Each parameter also becomes an attribute (self.x = x).").weak().small());
                    } else {
                        ui.label("Return type (optional):");
                        type_field(ui, "ret_ty", ret, &class_names);
                    }
                    let parsed = codegen::parse_param_list(params);
                    let shown: Vec<String> = parsed.iter().map(|p| p.display()).collect();
                    let mut sig = format!("{}({})", if *constructor { "__init__" } else { name.trim() }, shown.join(", "));
                    if !ret.trim().is_empty() && !*constructor {
                        sig.push_str(&format!(": {}", ret.trim()));
                    }
                    preview(ui, &sig);
                    show_error(ui, error);
                    let (ok, cancel) = ok_cancel(ui, if *constructor { "Add constructor" } else { "Add method" }, enter);
                    if ok {
                        let n = name.trim().to_string();
                        let exists = self.find_class(class).is_some_and(|c| c.methods.iter().any(|m| m.name == n));
                        if !codegen::is_valid_identifier(&n) {
                            *error = "The method name must be a valid Python name.".into();
                        } else if exists {
                            *error = format!("{class} already has a method {n}().");
                        } else if let Some(bad) = parsed.iter().find(|p| !codegen::is_valid_identifier(&p.name)) {
                            *error = format!("'{}' is not a valid parameter name.", bad.name);
                        } else {
                            let r = ret.trim().to_string();
                            let c = *constructor;
                            match self.edit_class(class, |src, line| {
                                codegen::add_method(src, line, &n, &parsed, Some(r.as_str()).filter(|r| !r.is_empty()), c)
                            }) {
                                Ok(()) => keep = false,
                                Err(e) => *error = e,
                            }
                        }
                    }
                    keep &= !cancel;
                }
                Dialog::ConfirmDeleteMethod { class, line, name } => {
                    ui.heading("Delete method");
                    ui.label(format!("Delete {name}() from {class}? Its code will be removed (Ctrl+Z in the editor undoes it)."));
                    let (ok, cancel) = ok_cancel(ui, "Delete", false);
                    if ok {
                        let l = *line;
                        if let Err(e) = self.edit_class(class, |src, _| codegen::delete_method(src, l)) {
                            self.error(e);
                        }
                        keep = false;
                    }
                    keep &= !cancel;
                }
            }
        });
        if modal.should_close() {
            keep = false;
        }

        if let Dialog::Unsaved(pending) = dialog {
            if self.dialog_result.take().is_some() {
                self.perform(pending);
            } else if keep {
                self.dialog = Some(Dialog::Unsaved(pending));
            }
            return;
        }
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }
}

fn show_error(ui: &mut Ui, error: &str) {
    if !error.is_empty() {
        ui.colored_label(Color32::from_rgb(230, 80, 80), error);
    }
}

fn preview(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("UML:").weak());
        ui.monospace(text);
    });
}

fn ok_cancel(ui: &mut Ui, ok_label: &str, enter: bool) -> (bool, bool) {
    ui.add_space(8.0);
    let mut ok = false;
    let mut cancel = false;
    ui.horizontal(|ui| {
        ok = ui.button(RichText::new(ok_label).strong()).clicked() || enter;
        cancel = ui.button("Cancel").clicked();
    });
    (ok, cancel)
}

/// Text field for a UML type with a drop-down of common choices.
fn type_field(ui: &mut Ui, id: &str, value: &mut String, classes: &[String]) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(value).hint_text("int, str, (int,int), list[Ball] …").desired_width(260.0));
        egui::ComboBox::from_id_salt(id).selected_text("pick").width(70.0).show_ui(ui, |ui| {
            for t in ["", "int", "float", "str", "bool", "list", "dict", "(int,int)", "(int,int,int)"] {
                ui.selectable_value(value, t.to_string(), if t.is_empty() { "(none)" } else { t });
            }
            for c in classes {
                ui.selectable_value(value, c.clone(), c);
                ui.selectable_value(value, format!("list[{c}]"), format!("list[{c}]"));
            }
        });
    });
}

/// Insert an import line after the existing imports at the top of a file.
fn insert_import(src: &str, import: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let last_import = lines
        .iter()
        .take_while(|l| {
            let t = l.trim();
            t.is_empty() || t.starts_with('#') || t.starts_with("import ") || t.starts_with("from ")
        })
        .enumerate()
        .filter(|(_, l)| l.starts_with("import ") || l.starts_with("from "))
        .map(|(i, _)| i)
        .last();
    let mut out: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    match last_import {
        Some(i) => out.insert(i + 1, import.to_string()),
        None => {
            out.insert(0, import.to_string());
            if out.len() > 1 {
                out.insert(1, String::new());
            }
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

impl eframe::App for EduPythonApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // background work
        if let Some(rx) = &self.python_rx {
            if let Ok(info) = rx.try_recv() {
                self.python = Some(info);
                self.python_rx = None;
            }
        }
        self.poll_runner(&ctx);
        if self.uml_stale {
            if let Some(p) = &self.project {
                self.classes = parse_project(p.files.iter().map(|f| (f.name.as_str(), f.text.as_str())));
            } else {
                self.classes.clear();
            }
            self.uml_stale = false;
        }

        // close handling
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.is_dirty() && !self.allow_close {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                if self.dialog.is_none() {
                    self.dialog = Some(Dialog::Unsaved(Pending::Close));
                }
            } else {
                self.close_project_quietly();
                self.settings.ui_zoom = ctx.zoom_factor();
                if let Some(b) = &self.booklet {
                    self.settings.booklet_page = b.page;
                }
                self.settings.save();
            }
        }
        if self.allow_close {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }

        if self.dialog.is_none() {
            self.shortcuts(&ctx);
        }

        // window title
        let title = match &self.project {
            Some(p) => format!("{}{} — EduPython", p.name(), if p.is_dirty() { " •" } else { "" }),
            None => "EduPython".into(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(3.0);
            self.toolbar(ui);
            ui.add_space(3.0);
        });

        if self.settings.show_booklet {
            if let Some(b) = self.booklet.as_mut() {
                egui::Panel::right("booklet").resizable(true).default_size(520.0).min_size(240.0).show(ui, |ui| b.show(ui));
            }
        }

        if self.project.is_some() {
            let running = self.is_running();
            let font = self.settings.editor_font_size;
            let dark = self.settings.dark;
            egui::Panel::bottom("console").resizable(true).default_size(200.0).min_size(80.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Console");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Clear").clicked() {
                            self.console.clear();
                        }
                        if ui.small_button("Copy").clicked() {
                            ui.ctx().copy_text(self.console.all_text());
                        }
                    });
                });
                match self.console.show(ui, running, font, dark) {
                    ConsoleAction::None => {}
                    ConsoleAction::SendInput(line) => {
                        self.console.end_line();
                        self.console.push(Kind::Input, &format!("{line}\n"));
                        if let Some(r) = self.runner.as_mut() {
                            r.send_line(&line);
                        }
                    }
                    ConsoleAction::Goto(path, line) => match self.project_file_for(&path) {
                        Some(idx) => {
                            self.active = idx;
                            self.goto = Some((idx, line));
                        }
                        None => self.status = format!("{path} is not part of this project"),
                    },
                }
            });

            if self.settings.show_uml {
                let mut action = None;
                egui::Panel::left("uml").resizable(true).default_size(460.0).min_size(220.0).show(ui, |ui| {
                    let p = self.project.as_mut().unwrap();
                    self.uml.show_grid = self.settings.uml_grid;
                    action = self.uml.show(ui, &self.classes, &mut p.uml_positions, self.settings.dark);
                });
                if let Some(a) = action {
                    self.handle_uml_action(a);
                }
            }
        }

        egui::CentralPanel::default().show(ui, |ui| self.editor_area(ui));
        self.dialogs(&ctx);
    }

    /// On a Mac, Ctrl works like Cmd for every shortcut (Ctrl+S, Ctrl+Z,
    /// Ctrl+C/V/X, Ctrl+A, …), because pupils also use Windows at school.
    #[cfg(target_os = "macos")]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        ctrl_as_cmd(raw);
    }

    fn on_exit(&mut self) {
        self.close_project_quietly();
        if let Some(b) = &self.booklet {
            self.settings.booklet_page = b.page;
        }
        self.settings.save();
    }
}

/// The module name from "ModuleNotFoundError: No module named 'x'" (top-level package).
fn missing_module(stderr: &str) -> Option<String> {
    let rest = stderr.split("No module named '").nth(1)?;
    let name = rest.split('\'').next()?;
    Some(name.split('.').next().unwrap_or(name).to_string())
}

/// Rewrite Ctrl (without Cmd) into Cmd in all input of this frame, and turn
/// Ctrl+C / Ctrl+X / Ctrl+V into copy / cut / paste like egui does for Cmd.
#[cfg(target_os = "macos")]
fn ctrl_as_cmd(raw: &mut egui::RawInput) {
    fn fix(m: &mut Modifiers) -> bool {
        if m.ctrl && !m.mac_cmd {
            m.ctrl = false;
            m.mac_cmd = true;
            m.command = true;
            true
        } else {
            false
        }
    }
    let mut events = Vec::with_capacity(raw.events.len());
    for mut ev in std::mem::take(&mut raw.events) {
        match &mut ev {
            egui::Event::Key { key, pressed, modifiers, .. } => {
                let was_ctrl = fix(modifiers);
                let plain = !modifiers.shift && !modifiers.alt;
                if was_ctrl && *pressed && plain {
                    match key {
                        Key::C => {
                            events.push(egui::Event::Copy);
                            continue;
                        }
                        Key::X => {
                            events.push(egui::Event::Cut);
                            continue;
                        }
                        Key::V => {
                            let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
                            if !text.is_empty() {
                                events.push(egui::Event::Paste(text.replace("\r\n", "\n")));
                            }
                            continue;
                        }
                        _ => {}
                    }
                }
            }
            egui::Event::PointerButton { modifiers, .. }
            | egui::Event::MouseWheel { modifiers, .. }
            | egui::Event::ModifiersChanged(modifiers) => {
                fix(modifiers);
            }
            _ => {}
        }
        events.push(ev);
    }
    raw.events = events;
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn ctrl_works_like_cmd_on_mac() {
        let ctrl = Modifiers { ctrl: true, ..Default::default() };
        let key = |k: Key| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: ctrl };
        let mut raw = egui::RawInput { events: vec![key(Key::Z), key(Key::C), egui::Event::ModifiersChanged(ctrl)], ..Default::default() };
        ctrl_as_cmd(&mut raw);
        match &raw.events[0] {
            egui::Event::Key { modifiers, .. } => assert!(modifiers.command && modifiers.mac_cmd && !modifiers.ctrl),
            e => panic!("{e:?}"),
        }
        assert!(matches!(raw.events[1], egui::Event::Copy));
        assert!(matches!(raw.events[2], egui::Event::ModifiersChanged(m) if m.command));
    }
}
