//! Output console with clickable traceback lines and an input field for
//! `input()`.

use eframe::egui::{self, Color32, FontId, RichText, ScrollArea, Sense, TextWrapMode, Ui};

const MAX_LINES: usize = 5000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Out,
    Err,
    Input,
    Info,
    Hint,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
    /// (file path as printed by Python, 1-based line)
    pub link: Option<(String, usize)>,
}

#[derive(Default)]
pub struct Console {
    pub lines: Vec<Line>,
    /// The last line has no newline yet (e.g. an `input()` prompt).
    open: bool,
    pub input: String,
    history: Vec<String>,
    history_pos: Option<usize>,
    pub focus_input: bool,
}

/// `  File "C:\x\main.py", line 12, in <module>` → ("C:\x\main.py", 12)
pub fn parse_traceback_location(line: &str) -> Option<(String, usize)> {
    let rest = line.trim_start().strip_prefix("File \"")?;
    let q = rest.find('"')?;
    let path = &rest[..q];
    let after = rest[q + 1..].trim_start().strip_prefix(',')?.trim_start().strip_prefix("line ")?;
    let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    Some((path.to_string(), num.parse().ok()?))
}

pub enum ConsoleAction {
    None,
    Goto(String, usize),
    SendInput(String),
}

impl Console {
    pub fn clear(&mut self) {
        self.lines.clear();
        self.open = false;
    }

    pub fn push(&mut self, kind: Kind, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "");
        if text.is_empty() {
            return;
        }
        let segments: Vec<&str> = text.split('\n').collect();
        for (i, seg) in segments.iter().enumerate() {
            if i == segments.len() - 1 && seg.is_empty() {
                break; // text ended with '\n'
            }
            let continue_open = i == 0 && self.open && self.lines.last().is_some_and(|l| l.kind == kind);
            if continue_open {
                let last = self.lines.last_mut().unwrap();
                last.text.push_str(seg);
                last.link = parse_traceback_location(&last.text);
            } else {
                self.lines.push(Line { kind, text: seg.to_string(), link: parse_traceback_location(seg) });
            }
        }
        self.open = !text.ends_with('\n');
        if self.lines.len() > MAX_LINES {
            let excess = self.lines.len() - MAX_LINES;
            self.lines.drain(..excess);
        }
    }

    /// Ensure following output starts on a new line.
    pub fn end_line(&mut self) {
        self.open = false;
    }

    #[cfg(test)]
    pub fn last_line_open(&self) -> bool {
        self.open
    }

    pub fn all_text(&self) -> String {
        self.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    pub fn show(&mut self, ui: &mut Ui, running: bool, font_size: f32, dark: bool) -> ConsoleAction {
        let mut action = ConsoleAction::None;
        let font = FontId::monospace((font_size - 1.0).max(10.0));
        let colors = |k: Kind| -> Color32 {
            match (k, dark) {
                (Kind::Out, true) => Color32::from_gray(220),
                (Kind::Out, false) => Color32::from_gray(25),
                (Kind::Err, true) => Color32::from_rgb(255, 120, 110),
                (Kind::Err, false) => Color32::from_rgb(190, 30, 30),
                (Kind::Input, true) => Color32::from_rgb(120, 200, 255),
                (Kind::Input, false) => Color32::from_rgb(0, 90, 180),
                (Kind::Info, true) => Color32::from_gray(140),
                (Kind::Info, false) => Color32::from_gray(110),
                (Kind::Hint, true) => Color32::from_rgb(240, 200, 90),
                (Kind::Hint, false) => Color32::from_rgb(150, 100, 0),
            }
        };

        // Input row at the bottom.
        egui::Panel::bottom("console_input").resizable(false).show_separator_line(false).show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("›").font(font.clone()));
                let hint = if running { "Type input for your program here and press Enter" } else { "Input (available while the program runs)" };
                let resp = ui.add_enabled(
                    running,
                    egui::TextEdit::singleline(&mut self.input)
                        .id_salt("console_input_field")
                        .hint_text(hint)
                        .font(font.clone())
                        .desired_width(f32::INFINITY),
                );
                if self.focus_input && running {
                    resp.request_focus();
                    self.focus_input = false;
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && running {
                    let line = std::mem::take(&mut self.input);
                    self.history.push(line.clone());
                    self.history_pos = None;
                    action = ConsoleAction::SendInput(line);
                    resp.request_focus();
                }
                if resp.has_focus() && !self.history.is_empty() {
                    let (up, down) = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown)));
                    if up || down {
                        let len = self.history.len();
                        let pos = match (self.history_pos, up) {
                            (None, true) => Some(len - 1),
                            (Some(p), true) => Some(p.saturating_sub(1)),
                            (Some(p), false) if p + 1 < len => Some(p + 1),
                            _ => None,
                        };
                        self.history_pos = pos;
                        self.input = pos.map(|p| self.history[p].clone()).unwrap_or_default();
                    }
                }
            });
            ui.add_space(2.0);
        });

        let row_h = ui.fonts_mut(|f| f.row_height(&font)) + 1.0;
        ScrollArea::both()
            .id_salt("console_scroll")
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show_rows(ui, row_h, self.lines.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for line in &self.lines[range] {
                    let mut rt = RichText::new(if line.text.is_empty() { " " } else { &line.text })
                        .font(font.clone())
                        .color(colors(line.kind));
                    if line.kind == Kind::Input {
                        rt = rt.strong();
                    }
                    if let Some((file, l)) = &line.link {
                        let resp = ui
                            .add(egui::Label::new(rt.underline()).wrap_mode(TextWrapMode::Extend).sense(Sense::click()))
                            .on_hover_text("Click to show this line in the editor");
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() {
                            action = ConsoleAction::Goto(file.clone(), *l);
                        }
                    } else {
                        ui.add(egui::Label::new(rt).wrap_mode(TextWrapMode::Extend));
                    }
                }
            });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_lines() {
        let mut c = Console::default();
        c.push(Kind::Out, "Name? ");
        assert!(c.last_line_open());
        c.push(Kind::Input, "Bob\n");
        c.push(Kind::Out, "Hello\nWor");
        c.push(Kind::Out, "ld\n");
        let texts: Vec<&str> = c.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["Name? ", "Bob", "Hello", "World"]);
        c.push(Kind::Err, "  File \"/a/main.py\", line 3, in <module>\n");
        assert_eq!(c.lines.last().unwrap().link, Some(("/a/main.py".into(), 3)));
    }
}
