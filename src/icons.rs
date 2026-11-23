//! Hand-drawn vector icons, painted with egui shapes (no emoji / font glyphs),
//! so they look the same on every platform and scale crisply with zoom.

use std::f32::consts::{PI, TAU};

use eframe::egui::{
    self, Color32, Mesh, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, TextStyle, Ui, Vec2,
    Widget, WidgetInfo, WidgetText, pos2, vec2,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    New,
    Open,
    Save,
    Run,
    Stop,
    Sun,
    Moon,
    Plus,
    Dot,
    Trash,
}

/// Paint `icon` into the square `rect` using `color`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center(), Vec2::splat(s));
    // map unit coordinates (0..1) into the icon square
    let p = |x: f32, y: f32| pos2(r.left() + x * s, r.top() + y * s);
    let stroke = Stroke::new((s * 0.09).max(1.2), color);

    match icon {
        Icon::New => {
            painter.add(Shape::closed_line(
                vec![p(0.22, 0.08), p(0.60, 0.08), p(0.80, 0.28), p(0.80, 0.92), p(0.22, 0.92)],
                stroke,
            ));
            painter.add(Shape::line(vec![p(0.60, 0.08), p(0.60, 0.28), p(0.80, 0.28)], stroke));
            painter.line_segment([p(0.51, 0.46), p(0.51, 0.76)], stroke);
            painter.line_segment([p(0.36, 0.61), p(0.66, 0.61)], stroke);
        }
        Icon::Open => {
            painter.add(Shape::closed_line(
                vec![p(0.08, 0.20), p(0.38, 0.20), p(0.47, 0.31), p(0.92, 0.31), p(0.92, 0.84), p(0.08, 0.84)],
                stroke,
            ));
            painter.line_segment([p(0.08, 0.43), p(0.92, 0.43)], stroke);
        }
        Icon::Save => {
            painter.add(Shape::closed_line(
                vec![p(0.12, 0.12), p(0.72, 0.12), p(0.88, 0.28), p(0.88, 0.88), p(0.12, 0.88)],
                stroke,
            ));
            painter.rect_stroke(Rect::from_min_max(p(0.30, 0.12), p(0.64, 0.36)), 0.0, stroke, StrokeKind::Middle);
            painter.rect_filled(Rect::from_min_max(p(0.28, 0.56), p(0.72, 0.88)), 1.0, color);
        }
        Icon::Run => {
            painter.add(Shape::convex_polygon(
                vec![p(0.24, 0.12), p(0.86, 0.50), p(0.24, 0.88)],
                color,
                Stroke::new(1.0, color),
            ));
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_min_max(p(0.20, 0.20), p(0.80, 0.80)), s * 0.08, color);
        }
        Icon::Sun => {
            let c = p(0.5, 0.5);
            painter.circle_filled(c, s * 0.19, color);
            for i in 0..8 {
                let a = i as f32 * TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([c + d * s * 0.31, c + d * s * 0.45], stroke);
            }
        }
        Icon::Moon => paint_crescent(painter, p(0.48, 0.52), s * 0.38, p(0.68, 0.34), s * 0.30, color),
        Icon::Plus => {
            painter.line_segment([p(0.5, 0.16), p(0.5, 0.84)], stroke);
            painter.line_segment([p(0.16, 0.5), p(0.84, 0.5)], stroke);
        }
        Icon::Trash => {
            // lid with handle
            painter.line_segment([p(0.14, 0.24), p(0.86, 0.24)], stroke);
            painter.add(Shape::line(vec![p(0.38, 0.24), p(0.38, 0.12), p(0.62, 0.12), p(0.62, 0.24)], stroke));
            // can, slightly narrower at the bottom
            painter.add(Shape::line(vec![p(0.22, 0.32), p(0.28, 0.90), p(0.72, 0.90), p(0.78, 0.32)], stroke));
            // ribs
            painter.line_segment([p(0.42, 0.40), p(0.43, 0.80)], stroke);
            painter.line_segment([p(0.58, 0.40), p(0.57, 0.80)], stroke);
        }
        Icon::Dot => {
            painter.circle_filled(p(0.5, 0.5), s * 0.28, color);
        }
    }
}

/// Filled crescent: the disc (c1, r1) minus the disc (c2, r2).
fn paint_crescent(painter: &Painter, c1: Pos2, r1: f32, c2: Pos2, r2: f32, color: Color32) {
    let dv = c2 - c1;
    let d = dv.length();
    let phi = dv.y.atan2(dv.x);
    // distance from c1 to the chord through both intersection points, and half chord length
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = (r1 * r1 - a * a).max(0.0).sqrt();
    let alpha = h.atan2(a); // half-angle of the cut on the outer circle
    let beta = h.atan2(d - a); // half-angle of the bite on the inner circle

    const N: usize = 32;
    let outer: Vec<Pos2> = (0..=N)
        .map(|i| {
            let t = phi + alpha + (TAU - 2.0 * alpha) * i as f32 / N as f32;
            c1 + vec2(t.cos(), t.sin()) * r1
        })
        .collect();
    let inner: Vec<Pos2> = (0..=N)
        .map(|i| {
            let t = phi + PI - beta + 2.0 * beta * i as f32 / N as f32;
            c2 + vec2(t.cos(), t.sin()) * r2
        })
        .collect();

    let mut mesh = Mesh::default();
    for (o, n) in outer.iter().zip(&inner) {
        mesh.colored_vertex(*o, color);
        mesh.colored_vertex(*n, color);
    }
    for i in 0..N as u32 {
        let k = i * 2;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 3, k + 2);
    }
    painter.add(Shape::mesh(mesh));
    // thin outline for anti-aliased edges
    let mut outline = outer;
    outline.extend(inner.into_iter().rev());
    painter.add(Shape::closed_line(outline, Stroke::new(1.0, color)));
}

/// A button with a painted icon and optional text, styled like an egui button.
pub struct IconButton {
    icon: Icon,
    text: Option<WidgetText>,
    fill: Option<Color32>,
    color: Option<Color32>,
    icon_color: Option<Color32>,
    frame: bool,
    small: bool,
    min_size: Vec2,
}

impl IconButton {
    pub fn new(icon: Icon) -> Self {
        Self { icon, text: None, fill: None, color: None, icon_color: None, frame: true, small: false, min_size: Vec2::ZERO }
    }

    pub fn text(mut self, text: impl Into<WidgetText>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn fill(mut self, fill: Color32) -> Self {
        self.fill = Some(fill);
        self
    }

    /// Colour for both icon and text.
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    /// Colour for the icon only (overrides `color`).
    pub fn icon_color(mut self, color: Color32) -> Self {
        self.icon_color = Some(color);
        self
    }

    pub fn frame(mut self, frame: bool) -> Self {
        self.frame = frame;
        self
    }

    pub fn min_size(mut self, size: Vec2) -> Self {
        self.min_size = size;
        self
    }

    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }
}

impl Widget for IconButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let style = if self.small { TextStyle::Small } else { TextStyle::Button };
        let text_h = ui.text_style_height(&style);

        let pad = if self.small { vec2(ui.spacing().button_padding.x * 0.5, 1.0) } else { ui.spacing().button_padding };
        let gap = ui.spacing().icon_spacing.max(5.0);

        let galley = self.text.as_ref().map(|t| t.clone().into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, style.clone()));
        let icon_size = (galley.as_ref().map_or(text_h, |g| g.size().y.min(text_h * 1.6)) * 0.95).round();
        let text_w = galley.as_ref().map_or(0.0, |g| g.size().x + gap);
        let content_h = galley.as_ref().map_or(icon_size, |g| g.size().y.max(icon_size));
        let mut size = vec2(pad.x * 2.0 + icon_size + text_w, pad.y * 2.0 + content_h);
        if !self.small {
            size.y = size.y.max(ui.spacing().interact_size.y);
        }
        size = size.max(self.min_size);
        // centre icon + text horizontally when the button is wider than its content
        let extra = (size.x - (pad.x * 2.0 + icon_size + text_w)).max(0.0) / 2.0;

        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let label = self.text.as_ref().map(|t| t.text().to_owned()).unwrap_or_default();
        resp.widget_info(|| WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label));

        if ui.is_rect_visible(rect) {
            let v = ui.style().interact(&resp);
            if self.frame {
                let frame_rect = rect.expand(v.expansion);
                let fill = self.fill.unwrap_or(v.weak_bg_fill);
                ui.painter().rect(frame_rect, v.corner_radius, fill, v.bg_stroke, StrokeKind::Inside);
            }
            let text_color = self.color.unwrap_or_else(|| v.text_color());
            let icon_color = self.icon_color.unwrap_or(text_color);
            let left = rect.left() + pad.x + extra;
            let icon_rect = Rect::from_min_size(pos2(left, rect.center().y - icon_size / 2.0), Vec2::splat(icon_size));
            paint(ui.painter(), icon_rect, self.icon, icon_color);
            if let Some(g) = galley {
                let pos = pos2(icon_rect.right() + gap, rect.center().y - g.size().y / 2.0);
                ui.painter().galley(pos, g, text_color);
            }
        }
        resp
    }
}
