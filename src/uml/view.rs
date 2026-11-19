//! Interactive UML class diagram (drawn with the egui painter).

use std::collections::{BTreeMap, HashSet};

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontFamily, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
    pos2, vec2,
};

use super::layout::{self, BoxMetrics, ClassLines, Part, RelKind};
use super::model::ClassInfo;

/// What the user asked for in the diagram; handled by the app.
#[derive(Clone, Debug)]
pub enum UmlAction {
    Goto { file: String, line: usize },
    AddClass,
    AddAttribute { class: String },
    AddMethod { class: String },
    AddConstructor { class: String },
    DeleteMethod { class: String, line: usize, name: String },
    ExportSvg,
    /// Layout changed (box moved / auto placed): save quietly.
    LayoutChanged,
}

enum Drag {
    Box(String),
    Pan,
}

pub struct UmlView {
    pan: Vec2,
    zoom: f32,
    drag: Option<Drag>,
    menu_target: Option<(usize, Part)>,
    pub fit_request: bool,
}

impl Default for UmlView {
    fn default() -> Self {
        Self { pan: vec2(0.0, 0.0), zoom: 1.0, drag: None, menu_target: None, fit_request: true }
    }
}

pub struct Colors {
    pub bg: Color32,
    pub grid: Color32,
    pub fill: Color32,
    pub header: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub line: Color32,
    pub highlight: Color32,
}

pub fn colors(dark: bool) -> Colors {
    if dark {
        Colors {
            bg: Color32::from_rgb(30, 32, 38),
            grid: Color32::from_rgb(40, 43, 50),
            fill: Color32::from_rgb(44, 48, 58),
            header: Color32::from_rgb(52, 70, 102),
            border: Color32::from_rgb(130, 150, 190),
            text: Color32::from_rgb(225, 228, 235),
            muted: Color32::from_rgb(150, 158, 175),
            line: Color32::from_rgb(170, 180, 200),
            highlight: Color32::from_rgb(255, 200, 90),
        }
    } else {
        Colors {
            bg: Color32::from_rgb(248, 249, 252),
            grid: Color32::from_rgb(234, 237, 243),
            fill: Color32::from_rgb(255, 255, 255),
            header: Color32::from_rgb(220, 232, 250),
            border: Color32::from_rgb(70, 90, 130),
            text: Color32::from_rgb(25, 30, 40),
            muted: Color32::from_rgb(100, 108, 125),
            line: Color32::from_rgb(70, 80, 100),
            highlight: Color32::from_rgb(220, 140, 0),
        }
    }
}

impl UmlView {
    fn to_screen(&self, origin: Pos2, p: Pos2) -> Pos2 {
        origin + self.pan + p.to_vec2() * self.zoom
    }

    fn to_model(&self, origin: Pos2, p: Pos2) -> Pos2 {
        ((p - origin - self.pan) / self.zoom).to_pos2()
    }

    fn fit(&mut self, view: Rect, rects: &[Rect]) {
        let Some(first) = rects.first() else {
            self.pan = vec2(0.0, 0.0);
            self.zoom = 1.0;
            return;
        };
        let bbox = rects.iter().fold(*first, |a, b| a.union(*b)).expand(20.0);
        self.zoom = (view.width() / bbox.width()).min(view.height() / bbox.height()).clamp(0.3, 1.5);
        self.pan = view.center() - view.min - bbox.center().to_vec2() * self.zoom;
    }

    fn zoom_around(&mut self, origin: Pos2, anchor: Pos2, factor: f32) {
        let before = self.to_model(origin, anchor);
        self.zoom = (self.zoom * factor).clamp(0.25, 3.0);
        let after = self.to_screen(origin, before);
        self.pan += anchor - after;
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        classes: &[ClassInfo],
        positions: &mut BTreeMap<String, [f32; 2]>,
        dark: bool,
    ) -> Option<UmlAction> {
        let mut action = None;
        let col = colors(dark);

        // ── toolbar ──
        let mut fit_clicked = false;
        let mut arrange_clicked = false;
        let mut zoom_btn = 1.0;
        ui.horizontal(|ui| {
            ui.strong("UML");
            ui.separator();
            if ui.add(crate::icons::IconButton::new(crate::icons::Icon::Plus).text("Class")).on_hover_text("Add a new class").clicked() {
                action = Some(UmlAction::AddClass);
            }
            if ui.button("Fit").on_hover_text("Show the whole diagram").clicked() {
                fit_clicked = true;
            }
            if ui.button("Arrange").on_hover_text("Arrange all boxes automatically").clicked() {
                arrange_clicked = true;
            }
            if ui.button("−").clicked() {
                zoom_btn = 1.0 / 1.2;
            }
            ui.label(format!("{:.0}%", self.zoom * 100.0));
            if ui.button("+").clicked() {
                zoom_btn = 1.2;
            }
            if ui.button("Export SVG…").on_hover_text("Save the diagram as an image (SVG)").clicked() {
                action = Some(UmlAction::ExportSvg);
            }
        });

        // ── geometry ──
        let names: HashSet<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        let lines: Vec<ClassLines> = classes.iter().map(|c| layout::class_lines(c, &names)).collect();
        let metrics: Vec<BoxMetrics> = lines.iter().map(layout::metrics).collect();
        let sizes: Vec<Vec2> = metrics.iter().map(|m| m.size).collect();
        if arrange_clicked {
            *positions = layout::auto_layout(classes, &sizes);
            action = Some(UmlAction::LayoutChanged);
            self.fit_request = true;
        } else if layout::place_missing(classes, &sizes, positions) && action.is_none() {
            action = Some(UmlAction::LayoutChanged);
        }
        let model_rects: Vec<Rect> = classes
            .iter()
            .zip(&sizes)
            .map(|(c, s)| layout::rect_at(positions.get(&c.name).copied().unwrap_or([0.0, 0.0]), *s))
            .collect();

        let (canvas, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let origin = canvas.min;
        if fit_clicked || self.fit_request {
            self.fit(canvas, &model_rects);
            self.fit_request = false;
        }
        if zoom_btn != 1.0 {
            self.zoom_around(origin, canvas.center(), zoom_btn);
        }

        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, 0.0, col.bg);
        // dotted grid
        let step = 24.0 * self.zoom;
        if step > 8.0 {
            let start = origin + vec2(self.pan.x.rem_euclid(step), self.pan.y.rem_euclid(step));
            let mut x = start.x;
            while x < canvas.right() {
                let mut y = start.y;
                while y < canvas.bottom() {
                    painter.circle_filled(pos2(x, y), 1.0, col.grid);
                    y += step;
                }
                x += step;
            }
        }

        let screen_rects: Vec<Rect> = model_rects
            .iter()
            .map(|r| Rect::from_min_max(self.to_screen(origin, r.min), self.to_screen(origin, r.max)))
            .collect();

        let pointer = ui.input(|i| i.pointer.interact_pos());
        let hit_box = |p: Pos2| screen_rects.iter().rposition(|r| r.contains(p));

        // ── interaction ──
        if resp.hovered() {
            let (zoom_delta, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
            if zoom_delta != 1.0 {
                if let Some(p) = pointer {
                    self.zoom_around(origin, p, zoom_delta);
                }
            } else if scroll != Vec2::ZERO {
                self.pan += scroll;
            }
        }
        if resp.drag_started() {
            self.drag = match pointer.and_then(hit_box) {
                Some(i) => Some(Drag::Box(classes[i].name.clone())),
                None => Some(Drag::Pan),
            };
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            match &self.drag {
                Some(Drag::Box(name)) => {
                    if let Some(p) = positions.get_mut(name) {
                        p[0] += d.x / self.zoom;
                        p[1] += d.y / self.zoom;
                    }
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
                Some(Drag::Pan) => {
                    self.pan += d;
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
                None => {}
            }
        }
        if resp.drag_stopped() {
            if matches!(self.drag, Some(Drag::Box(_))) {
                action = Some(UmlAction::LayoutChanged);
            }
            self.drag = None;
        }

        let (pan, zoom) = (self.pan, self.zoom);
        let part_at = |i: usize, p: Pos2| -> Part {
            let model = ((p - origin - pan) / zoom).to_pos2();
            layout::hit_part(model_rects[i], &metrics[i], lines[i].attrs.len(), lines[i].methods.len(), model)
        };

        let hovered_box = if resp.hovered() { pointer.and_then(hit_box) } else { None };
        if hovered_box.is_some() && self.drag.is_none() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }

        if resp.double_clicked() {
            if let Some(i) = pointer.and_then(hit_box) {
                let c = &classes[i];
                let line = match part_at(i, pointer.unwrap()) {
                    Part::Method(m) => c.methods[m].line,
                    _ => c.line,
                };
                action = Some(UmlAction::Goto { file: c.file.clone(), line });
            }
        }
        if resp.secondary_clicked() {
            self.menu_target = pointer.and_then(|p| hit_box(p).map(|i| (i, part_at(i, p))));
        }

        // ── relations (below boxes) ──
        let stroke = Stroke::new(1.4 * self.zoom.max(0.6), col.line);
        let small = FontId::new((FONT_SMALL * self.zoom).max(6.0), FontFamily::Proportional);
        for rel in layout::relations(classes) {
            let (a, b) = (screen_rects[rel.from], screen_rects[rel.to]);
            let start = layout::border_point(a, b.center());
            let end = layout::border_point(b, a.center());
            let dir = (end - start).normalized();
            let normal = vec2(-dir.y, dir.x);
            let head = 13.0 * self.zoom;
            match &rel.kind {
                RelKind::Inherits => {
                    let base = end - dir * head;
                    painter.line_segment([start, base], stroke);
                    painter.add(Shape::convex_polygon(
                        vec![end, base + normal * head * 0.55, base - normal * head * 0.55],
                        col.bg,
                        stroke,
                    ));
                }
                RelKind::Association { label, many } => {
                    painter.line_segment([start, end], stroke);
                    painter.line_segment([end, end - dir * head + normal * head * 0.45], stroke);
                    painter.line_segment([end, end - dir * head - normal * head * 0.45], stroke);
                    let mid = start + (end - start) * 0.5 + normal * 10.0 * self.zoom;
                    painter.text(mid, Align2::CENTER_CENTER, label, small.clone(), col.muted);
                    if *many {
                        let p = end - dir * (head + 8.0 * self.zoom) - normal * 10.0 * self.zoom;
                        painter.text(p, Align2::CENTER_CENTER, "*", small.clone(), col.muted);
                    }
                }
            }
        }

        // ── boxes ──
        let mono = FontId::monospace(layout::FONT * self.zoom);
        let title_font = FontId::new(layout::FONT * 1.1 * self.zoom, FontFamily::Proportional);
        for (i, r) in screen_rects.iter().enumerate() {
            let m = &metrics[i];
            let l = &lines[i];
            let z = self.zoom;
            let hovered = hovered_box == Some(i);
            let border = Stroke::new(if hovered { 2.0 } else { 1.3 }, if hovered { col.highlight } else { col.border });
            // shadow
            painter.rect_filled(r.translate(vec2(3.0, 3.0) * z), 3.0 * z, Color32::from_black_alpha(if dark { 70 } else { 22 }));
            painter.rect_filled(*r, 3.0 * z, col.fill);
            let header = Rect::from_min_size(r.min, vec2(r.width(), m.header_h * z));
            painter.rect_filled(header, egui::CornerRadius { nw: (3.0 * z) as u8, ne: (3.0 * z) as u8, sw: 0, se: 0 }, col.header);
            painter.rect_stroke(*r, 3.0 * z, border, StrokeKind::Inside);

            let mut y = r.top() + layout::PAD_Y * z;
            painter.text(pos2(r.center().x, y), Align2::CENTER_TOP, &l.title, title_font.clone(), col.text);
            y += layout::LINE_H * z;
            if let Some(sub) = &l.subtitle {
                painter.text(pos2(r.center().x, y), Align2::CENTER_TOP, sub, small.clone(), col.muted);
            }

            let sep1 = r.top() + m.header_h * z;
            let sep2 = sep1 + m.attrs_h * z;
            painter.line_segment([pos2(r.left(), sep1), pos2(r.right(), sep1)], border);
            painter.line_segment([pos2(r.left(), sep2), pos2(r.right(), sep2)], border);

            let hovered_part = if hovered && self.drag.is_none() { pointer.map(|p| part_at(i, p)) } else { None };
            let draw_rows = |rows: &[(String, bool)], top: f32, is_method: bool| {
                for (k, (text, is_static)) in rows.iter().enumerate() {
                    let y = top + (layout::PAD_Y + k as f32 * layout::LINE_H) * z;
                    let pos = pos2(r.left() + layout::PAD_X * z, y);
                    let row_hover = match &hovered_part {
                        Some(Part::Attribute(a)) => !is_method && *a == k,
                        Some(Part::Method(mm)) => is_method && *mm == k,
                        _ => false,
                    };
                    let color = if row_hover { col.highlight } else { col.text };
                    let g = painter.text(pos, Align2::LEFT_TOP, text, mono.clone(), color);
                    if *is_static {
                        painter.line_segment(
                            [pos2(g.left(), g.bottom() - 1.0), pos2(g.right(), g.bottom() - 1.0)],
                            Stroke::new(1.0, color),
                        );
                    }
                }
            };
            draw_rows(&l.attrs, sep1, false);
            draw_rows(&l.methods, sep2, true);
        }

        if classes.is_empty() {
            painter.text(
                canvas.center(),
                Align2::CENTER_CENTER,
                "No classes yet.\nRight-click here (or press + Class) to add one,\nor write  class Name:  in your code.",
                FontId::proportional(14.0),
                col.muted,
            );
        }

        // ── context menu ──
        let target = self.menu_target.clone();
        resp.context_menu(|ui| {
            ui.set_min_width(200.0);
            match &target {
                Some((i, part)) => {
                    let c = &classes[*i];
                    ui.label(egui::RichText::new(&c.name).strong());
                    ui.separator();
                    if let Part::Method(m) = part {
                        let method = &c.methods[*m];
                        if ui.button(format!("Go to {}()", method.name)).clicked() {
                            action = Some(UmlAction::Goto { file: c.file.clone(), line: method.line });
                        }
                        if ui.button(format!("Delete {}()", method.name)).clicked() {
                            action = Some(UmlAction::DeleteMethod {
                                class: c.name.clone(),
                                line: method.line,
                                name: method.name.clone(),
                            });
                        }
                        ui.separator();
                    }
                    if ui.button("Add attribute…").clicked() {
                        action = Some(UmlAction::AddAttribute { class: c.name.clone() });
                    }
                    if ui.button("Add method…").clicked() {
                        action = Some(UmlAction::AddMethod { class: c.name.clone() });
                    }
                    match c.methods.iter().find(|m| m.name == "__init__") {
                        Some(init) => {
                            if ui.button("Go to constructor").clicked() {
                                action = Some(UmlAction::Goto { file: c.file.clone(), line: init.line });
                            }
                        }
                        None => {
                            if ui.button("Add constructor…").clicked() {
                                action = Some(UmlAction::AddConstructor { class: c.name.clone() });
                            }
                        }
                    }
                    ui.separator();
                    if ui.button("Go to class code").clicked() {
                        action = Some(UmlAction::Goto { file: c.file.clone(), line: c.line });
                    }
                }
                None => {
                    if ui.button("New class…").clicked() {
                        action = Some(UmlAction::AddClass);
                    }
                    if ui.button("Arrange automatically").clicked() {
                        *positions = layout::auto_layout(classes, &sizes);
                        self.fit_request = true;
                        action = Some(UmlAction::LayoutChanged);
                    }
                    if ui.button("Export SVG…").clicked() {
                        action = Some(UmlAction::ExportSvg);
                    }
                }
            }
        });

        if !classes.is_empty() {
            painter.text(
                canvas.left_bottom() + vec2(8.0, -6.0),
                Align2::LEFT_BOTTOM,
                "Drag boxes · double-click opens code · right-click to add members",
                FontId::proportional(11.0),
                col.muted,
            );
        }
        action
    }
}

const FONT_SMALL: f32 = 11.5;
