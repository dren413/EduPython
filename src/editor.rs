//! Python code editor: highlighting, line numbers, smart indentation.

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Frame, Id, Key, Margin, Modifiers, Rect, ScrollArea, Sense,
    TextBuffer, TextEdit, Ui, pos2, text::CCursor, text_selection::CCursorRange, vec2,
};

use crate::highlight::highlight;

const INDENT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditKey {
    Tab,
    ShiftTab,
    Enter,
    Backspace,
    ToggleComment,
}

/// Pure text transformation for the smart-editing keys.
/// `start..end` is the selection in *chars*. Returns the new selection,
/// or `None` if the key should be handled by the default text edit.
pub fn apply_key(text: &mut String, start: usize, end: usize, key: EditKey) -> Option<(usize, usize)> {
    let mut chars: Vec<char> = text.chars().collect();
    let (start, end) = (start.min(end).min(chars.len()), start.max(end).min(chars.len()));
    let line_start = |chars: &[char], i: usize| chars[..i].iter().rposition(|&c| c == '\n').map_or(0, |p| p + 1);
    let line_end = |chars: &[char], i: usize| chars[i..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| i + p);

    // Lines touched by the selection (as start indices).
    let touched = |chars: &[char]| -> Vec<usize> {
        let mut last = end;
        if end > start && end > 0 && chars[end - 1] == '\n' {
            last = end - 1;
        }
        let mut v = vec![line_start(chars, start)];
        for (i, &c) in chars.iter().enumerate().take(last).skip(start) {
            if c == '\n' {
                v.push(i + 1);
            }
        }
        v
    };

    let multi_line = chars[start..end].contains(&'\n');
    let result = match key {
        EditKey::Tab if !multi_line => {
            let col = start - line_start(&chars, start);
            let n = INDENT - col % INDENT;
            chars.splice(start..end, std::iter::repeat_n(' ', n));
            Some((start + n, start + n))
        }
        EditKey::Tab | EditKey::ShiftTab => {
            let lines = touched(&chars);
            let mut shift = 0isize;
            let first = lines[0];
            for &ls in lines.iter().rev() {
                if key == EditKey::Tab {
                    if line_end(&chars, ls) > ls {
                        chars.splice(ls..ls, std::iter::repeat_n(' ', INDENT));
                        shift += INDENT as isize;
                    }
                } else {
                    let n = chars[ls..].iter().take(INDENT).take_while(|&&c| c == ' ').count();
                    chars.drain(ls..ls + n);
                    shift -= n as isize;
                }
            }
            let new_end = (end as isize + shift).max(first as isize) as usize;
            if multi_line { Some((first, new_end)) } else {
                let s = (start as isize + shift).max(first as isize) as usize;
                Some((s, s))
            }
        }
        EditKey::Enter => {
            let ls = line_start(&chars, start);
            let before: String = chars[ls..start].iter().collect();
            let mut indent = before.chars().take_while(|&c| c == ' ').count();
            let trimmed = before.trim();
            if trimmed.ends_with(':') {
                indent += INDENT;
            } else if ["return", "pass", "break", "continue", "raise"]
                .iter()
                .any(|kw| trimmed == *kw || trimmed.starts_with(&format!("{kw} ")))
            {
                indent = indent.saturating_sub(INDENT);
            }
            let mut ins = vec!['\n'];
            ins.extend(std::iter::repeat_n(' ', indent));
            let pos = start + ins.len();
            chars.splice(start..end, ins);
            Some((pos, pos))
        }
        EditKey::Backspace => {
            if start != end || start == 0 {
                return None;
            }
            let ls = line_start(&chars, start);
            let col = start - ls;
            if col == 0 || !chars[ls..start].iter().all(|&c| c == ' ') {
                return None;
            }
            let n = (col - 1) % INDENT + 1;
            chars.drain(start - n..start);
            Some((start - n, start - n))
        }
        EditKey::ToggleComment => {
            let lines = touched(&chars);
            let non_empty: Vec<usize> = lines
                .iter()
                .copied()
                .filter(|&ls| chars[ls..line_end(&chars, ls)].iter().any(|c| !c.is_whitespace()))
                .collect();
            if non_empty.is_empty() {
                return Some((start, end));
            }
            let indent_of = |chars: &[char], ls: usize| chars[ls..].iter().take_while(|&&c| c == ' ').count();
            let all_commented = non_empty.iter().all(|&ls| chars.get(ls + indent_of(&chars, ls)) == Some(&'#'));
            let min_indent = non_empty.iter().map(|&ls| indent_of(&chars, ls)).min().unwrap_or(0);
            let mut delta = 0isize;
            for &ls in non_empty.iter().rev() {
                if all_commented {
                    let p = ls + indent_of(&chars, ls);
                    let n = if chars.get(p + 1) == Some(&' ') { 2 } else { 1 };
                    chars.drain(p..p + n);
                    delta -= n as isize;
                } else {
                    chars.splice(ls + min_indent..ls + min_indent, ['#', ' ']);
                    delta += 2;
                }
            }
            let first = lines[0];
            let last_end = line_end(&chars, (*lines.last().unwrap() as isize + delta.min(0)).max(0) as usize);
            if start == end {
                let p = (start as isize + delta).max(first as isize) as usize;
                Some((p, p))
            } else {
                Some((first, last_end))
            }
        }
    };
    *text = chars.into_iter().collect();
    result
}

pub struct EditorResponse {
    pub changed: bool,
    pub cursor_line: Option<usize>,
}

pub struct EditorOptions {
    pub font_size: f32,
    pub dark: bool,
    /// 1-based line to mark as the error line.
    pub error_line: Option<usize>,
    /// 1-based line to jump to this frame.
    pub goto_line: Option<usize>,
    pub bg: Color32,
}

fn char_index_of_line(text: &str, line: usize) -> usize {
    let mut current = 1;
    for (ci, c) in text.chars().enumerate() {
        if current >= line {
            return ci;
        }
        if c == '\n' {
            current += 1;
        }
    }
    text.chars().count()
}

fn line_of_char(text: &str, ci: usize) -> usize {
    text.chars().take(ci).filter(|&c| c == '\n').count() + 1
}

pub fn show(ui: &mut Ui, id: Id, text: &mut String, opts: EditorOptions) -> EditorResponse {
    let mut changed = false;

    // ── Smart keys (only when the editor has keyboard focus) ──
    if ui.memory(|m| m.has_focus(id)) {
        let key = ui.input_mut(|i| {
            if i.consume_key(Modifiers::SHIFT, Key::Tab) {
                Some(EditKey::ShiftTab)
            } else if i.consume_key(Modifiers::NONE, Key::Tab) {
                Some(EditKey::Tab)
            } else if i.consume_key(Modifiers::COMMAND, Key::Slash) {
                Some(EditKey::ToggleComment)
            } else if !i.modifiers.any() && i.key_pressed(Key::Enter) {
                i.consume_key(Modifiers::NONE, Key::Enter);
                Some(EditKey::Enter)
            } else if !i.modifiers.any() && i.key_pressed(Key::Backspace) {
                Some(EditKey::Backspace)
            } else {
                None
            }
        });
        if let Some(key) = key {
            if apply_smart_key(ui, id, text, key) {
                changed = true;
                if key == EditKey::Backspace {
                    ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace));
                }
            }
        }
    }
    let font = FontId::monospace(opts.font_size);
    let (row_h, char_w) = ui.fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, '0')));
    let n_lines = text.matches('\n').count() + 1;
    let digits = n_lines.to_string().len().max(2);
    let gutter_w = digits as f32 * char_w + 20.0;
    let margin_y = 6.0;
    let visible_rows = ((ui.available_height() - 2.0 * margin_y) / row_h).floor().max(1.0) as usize;
    let dark = opts.dark;
    let gutter_color = if dark { Color32::from_gray(110) } else { Color32::from_gray(150) };
    let gutter_active = if dark { Color32::from_gray(210) } else { Color32::from_gray(40) };
    let error_bg = if dark { Color32::from_rgba_unmultiplied(220, 60, 60, 60) } else { Color32::from_rgba_unmultiplied(230, 60, 60, 45) };

    if let Some(line) = opts.goto_line {
        let ci = char_index_of_line(text, line);
        let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(ci))));
        state.store(ui.ctx(), id);
        ui.memory_mut(|m| m.request_focus(id));
    }

    let mut cursor_line = None;
    Frame::NONE.fill(opts.bg).show(ui, |ui| {
        ScrollArea::both().id_salt(id.with("scroll")).auto_shrink(false).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let height = (n_lines.max(visible_rows)) as f32 * row_h + 2.0 * margin_y;
                let (gutter, _) = ui.allocate_exact_size(vec2(gutter_w, height), Sense::hover());
                let text_top = gutter.top() + margin_y;
                let full_width = Rect::from_min_max(
                    pos2(gutter.left(), 0.0),
                    pos2(ui.clip_rect().right().max(gutter.right() + 4000.0), 0.0),
                );

                if let Some(l) = opts.error_line {
                    let y = text_top + (l.saturating_sub(1)) as f32 * row_h;
                    ui.painter().rect_filled(
                        Rect::from_x_y_ranges(full_width.x_range(), y..=y + row_h),
                        0.0,
                        error_bg,
                    );
                }

                let font2 = font.clone();
                let mut layouter = move |ui: &Ui, buf: &dyn TextBuffer, _wrap: f32| {
                    let job = highlight(buf.as_str(), font2.clone(), dark);
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                let output = TextEdit::multiline(text)
                    .id(id)
                    .font(font.clone())
                    .code_editor()
                    .frame(Frame::NONE)
                    .margin(Margin::symmetric(8, margin_y as i8))
                    .desired_width(ui.available_width().max(200.0))
                    .desired_rows(n_lines.max(visible_rows))
                    .lock_focus(true)
                    .layouter(&mut layouter)
                    .show(ui);
                if output.response.changed() {
                    changed = true;
                }

                let active_line = output
                    .cursor_range
                    .map(|r| line_of_char(text, r.primary.index.0));
                cursor_line = active_line;

                // Line numbers for the visible rows only.
                let clip = ui.clip_rect();
                let first = (((clip.top() - text_top) / row_h).floor().max(0.0)) as usize;
                let last = ((((clip.bottom() - text_top) / row_h).ceil()) as usize + 1).min(n_lines);
                let painter = ui.painter();
                for i in first..last {
                    let n = i + 1;
                    let y = text_top + i as f32 * row_h;
                    let is_err = opts.error_line == Some(n);
                    let color = if is_err {
                        Color32::from_rgb(230, 80, 80)
                    } else if active_line == Some(n) {
                        gutter_active
                    } else {
                        gutter_color
                    };
                    painter.text(pos2(gutter.right() - 8.0, y), Align2::RIGHT_TOP, n.to_string(), font.clone(), color);
                }

                if let Some(line) = opts.goto_line {
                    let y = text_top + (line.saturating_sub(1)) as f32 * row_h;
                    ui.scroll_to_rect(
                        Rect::from_min_size(pos2(gutter.left(), y), vec2(gutter_w, row_h)),
                        Some(Align::Center),
                    );
                }
            });
        });
    });

    EditorResponse { changed, cursor_line }
}

fn apply_smart_key(ui: &Ui, id: Id, text: &mut String, key: EditKey) -> bool {
    let Some(mut state) = TextEdit::load_state(ui.ctx(), id) else { return false };
    let Some(range) = state.cursor.char_range() else { return false };
    let (a, b) = (range.primary.index.0, range.secondary.index.0);
    let mut new_text = text.clone();
    let Some((s, e)) = apply_key(&mut new_text, a.min(b), a.max(b), key) else { return false };
    *text = new_text;
    let range = if s == e {
        CCursorRange::one(CCursor::new(s))
    } else {
        CCursorRange::two(CCursor::new(s), CCursor::new(e))
    };
    state.cursor.set_char_range(Some(range));
    state.store(ui.ctx(), id);
    true
}

/// Convenience used by the menu: toggle comment for the focused editor.
pub fn toggle_comment(ctx: &egui::Context, id: Id, text: &mut String) -> bool {
    let Some(mut state) = TextEdit::load_state(ctx, id) else { return false };
    let Some(range) = state.cursor.char_range() else { return false };
    let (a, b) = (range.primary.index.0, range.secondary.index.0);
    let Some((s, e)) = apply_key(text, a.min(b), a.max(b), EditKey::ToggleComment) else { return false };
    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(s), CCursor::new(e))));
    state.store(ctx, id);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, sel: (usize, usize), key: EditKey) -> (String, Option<(usize, usize)>) {
        let mut s = src.to_string();
        let r = apply_key(&mut s, sel.0, sel.1, key);
        (s, r)
    }

    #[test]
    fn enter_indents() {
        let src = "def f():";
        assert_eq!(run(src, (8, 8), EditKey::Enter), ("def f():\n    ".into(), Some((13, 13))));
        let src = "    x = 1";
        assert_eq!(run(src, (9, 9), EditKey::Enter).0, "    x = 1\n    ");
        let src = "        return x";
        assert_eq!(run(src, (16, 16), EditKey::Enter).0, "        return x\n    ");
    }

    #[test]
    fn tab_and_backspace() {
        assert_eq!(run("ab", (1, 1), EditKey::Tab), ("a   b".into(), Some((4, 4))));
        assert_eq!(run("a\nb\n", (0, 3), EditKey::Tab).0, "    a\n    b\n");
        assert_eq!(run("    a\n  b", (0, 9), EditKey::ShiftTab).0, "a\nb");
        assert_eq!(run("        x", (8, 8), EditKey::Backspace), ("    x".into(), Some((4, 4))));
        assert_eq!(run("  a", (3, 3), EditKey::Backspace).1, None);
    }

    #[test]
    fn comments() {
        let (s, _) = run("    a\n    b", (0, 11), EditKey::ToggleComment);
        assert_eq!(s, "    # a\n    # b");
        let (s, _) = run(&s, (0, 15), EditKey::ToggleComment);
        assert_eq!(s, "    a\n    b");
    }
}
