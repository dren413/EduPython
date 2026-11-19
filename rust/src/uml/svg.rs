//! Export the class diagram as an SVG file (for worksheets / slides).

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;

use eframe::egui::{Rect, vec2};

use super::layout::{self, RelKind};
use super::model::ClassInfo;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn export(classes: &[ClassInfo], positions: &BTreeMap<String, [f32; 2]>) -> String {
    let names: HashSet<&str> = classes.iter().map(|c| c.name.as_str()).collect();
    let lines: Vec<_> = classes.iter().map(|c| layout::class_lines(c, &names)).collect();
    let metrics: Vec<_> = lines.iter().map(layout::metrics).collect();
    let rects: Vec<Rect> = classes
        .iter()
        .zip(&metrics)
        .map(|(c, m)| layout::rect_at(positions.get(&c.name).copied().unwrap_or([0.0, 0.0]), m.size))
        .collect();
    let bbox = rects
        .iter()
        .copied()
        .reduce(|a, b| a.union(b))
        .unwrap_or(Rect::from_min_size(Default::default(), vec2(200.0, 100.0)))
        .expand(30.0);

    let mut s = String::new();
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0}" height="{h:.0}" viewBox="{x:.0} {y:.0} {w:.0} {h:.0}">"#,
        x = bbox.left(),
        y = bbox.top(),
        w = bbox.width(),
        h = bbox.height()
    );
    let _ = writeln!(
        s,
        r##"<defs><marker id="inherit" viewBox="0 0 14 14" refX="13" refY="7" markerWidth="14" markerHeight="14" orient="auto-start-reverse" markerUnits="userSpaceOnUse"><path d="M0,0 L14,7 L0,14 Z" fill="white" stroke="#333" stroke-width="1.3"/></marker><marker id="assoc" viewBox="0 0 14 14" refX="13" refY="7" markerWidth="14" markerHeight="14" orient="auto-start-reverse" markerUnits="userSpaceOnUse"><path d="M0,1 L13,7 L0,13" fill="none" stroke="#333" stroke-width="1.3"/></marker></defs>"##
    );
    let _ = writeln!(s, r#"<rect x="{:.0}" y="{:.0}" width="{:.0}" height="{:.0}" fill="white"/>"#, bbox.left(), bbox.top(), bbox.width(), bbox.height());
    let font = format!(r#"font-family="Menlo, Consolas, 'DejaVu Sans Mono', monospace" font-size="{}""#, layout::FONT);

    for rel in layout::relations(classes) {
        let (a, b) = (rects[rel.from], rects[rel.to]);
        let p1 = layout::border_point(a, b.center());
        let p2 = layout::border_point(b, a.center());
        let marker = if rel.kind == RelKind::Inherits { "inherit" } else { "assoc" };
        let _ = writeln!(
            s,
            r##"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="#333" stroke-width="1.3" marker-end="url(#{marker})"/>"##,
            p1.x, p1.y, p2.x, p2.y
        );
        if let RelKind::Association { label, many } = &rel.kind {
            let dir = (p2 - p1).normalized();
            let normal = vec2(-dir.y, dir.x);
            let mid = p1 + (p2 - p1) * 0.5 + normal * 10.0;
            let _ = writeln!(s, r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-family="sans-serif" font-size="11" fill="#555">{}</text>"##, mid.x, mid.y, esc(label));
            if *many {
                let p = p2 - dir * 21.0 - normal * 10.0;
                let _ = writeln!(s, r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-family="sans-serif" font-size="12" fill="#555">*</text>"##, p.x, p.y);
            }
        }
    }

    for ((r, m), l) in rects.iter().zip(&metrics).zip(&lines) {
        let _ = writeln!(s, r##"<g><rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="3" fill="#ffffff" stroke="#333" stroke-width="1.3"/>"##, r.left(), r.top(), r.width(), r.height());
        let _ = writeln!(s, r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="3" fill="#dce8fa" stroke="#333" stroke-width="1.3"/>"##, r.left(), r.top(), r.width(), m.header_h);
        let baseline = |top: f32, k: usize| top + layout::PAD_Y + k as f32 * layout::LINE_H + layout::FONT;
        let _ = writeln!(s, r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-family="sans-serif" font-size="14" font-weight="bold" fill="#111">{}</text>"##, r.center().x, baseline(r.top(), 0), esc(&l.title));
        if let Some(sub) = &l.subtitle {
            let _ = writeln!(s, r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-family="sans-serif" font-size="11" fill="#555">{}</text>"##, r.center().x, baseline(r.top(), 1), esc(sub));
        }
        let sep1 = r.top() + m.header_h;
        let sep2 = sep1 + m.attrs_h;
        let _ = writeln!(s, r##"<line x1="{:.1}" y1="{sep2:.1}" x2="{:.1}" y2="{sep2:.1}" stroke="#333" stroke-width="1.3"/>"##, r.left(), r.right());
        for (top, rows) in [(sep1, &l.attrs), (sep2, &l.methods)] {
            for (k, (text, is_static)) in rows.iter().enumerate() {
                let deco = if *is_static { r#" text-decoration="underline""# } else { "" };
                let _ = writeln!(s, r##"<text x="{:.1}" y="{:.1}" {font} fill="#111"{deco}>{}</text>"##, r.left() + layout::PAD_X, baseline(top, k), esc(text));
            }
        }
        let _ = writeln!(s, "</g>");
    }
    s.push_str("</svg>\n");
    s
}
