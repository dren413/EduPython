//! Geometry of the UML diagram in "model" units (zoom 1.0), shared by the
//! on-screen view and the SVG export so both look the same.

use std::collections::{BTreeMap, HashMap, HashSet};

use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

use super::model::ClassInfo;

pub const FONT: f32 = 13.0;
/// Width of one monospace character at `FONT` size.
pub const CHAR_W: f32 = FONT * 0.6;
pub const LINE_H: f32 = 18.0;
pub const PAD_X: f32 = 10.0;
pub const PAD_Y: f32 = 6.0;
pub const MIN_W: f32 = 120.0;
const GAP_X: f32 = 50.0;
const GAP_Y: f32 = 70.0;

/// The text content of one class box.
pub struct ClassLines {
    pub title: String,
    /// External base classes (not in the project), e.g. `extends Sprite`.
    pub subtitle: Option<String>,
    /// (text, is_static)
    pub attrs: Vec<(String, bool)>,
    pub methods: Vec<(String, bool)>,
}

pub fn class_lines(c: &ClassInfo, project_classes: &HashSet<&str>) -> ClassLines {
    let external: Vec<String> = c
        .bases
        .iter()
        .filter(|b| !project_classes.contains(short(b)))
        .map(|b| short(b).to_string())
        .collect();
    ClassLines {
        title: c.name.clone(),
        subtitle: (!external.is_empty()).then(|| format!("extends {}", external.join(", "))),
        attrs: c.attributes.iter().map(|a| (a.display(), a.is_static)).collect(),
        methods: c.methods.iter().map(|m| (m.display(), m.is_static)).collect(),
    }
}

pub fn short(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// Heights of the three compartments.
pub struct BoxMetrics {
    pub size: Vec2,
    pub header_h: f32,
    pub attrs_h: f32,
}

fn compartment_h(n: usize) -> f32 {
    if n == 0 { PAD_Y * 2.0 } else { n as f32 * LINE_H + PAD_Y * 2.0 }
}

pub fn metrics(lines: &ClassLines) -> BoxMetrics {
    let longest = std::iter::once(lines.title.chars().count() + 2)
        .chain(lines.subtitle.iter().map(|s| s.chars().count()))
        .chain(lines.attrs.iter().map(|a| a.0.chars().count()))
        .chain(lines.methods.iter().map(|m| m.0.chars().count()))
        .max()
        .unwrap_or(0);
    let width = (longest as f32 * CHAR_W + 2.0 * PAD_X).max(MIN_W).ceil();
    let header_h = LINE_H * if lines.subtitle.is_some() { 2.0 } else { 1.0 } + PAD_Y * 2.0;
    let attrs_h = compartment_h(lines.attrs.len());
    let methods_h = compartment_h(lines.methods.len());
    BoxMetrics { size: vec2(width, header_h + attrs_h + methods_h), header_h, attrs_h }
}

/// What part of a class box a point is on.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Header,
    Attribute(usize),
    Method(usize),
    AttrArea,
    MethodArea,
}

pub fn hit_part(rect: Rect, m: &BoxMetrics, n_attrs: usize, n_methods: usize, p: Pos2) -> Part {
    let y = p.y - rect.top();
    if y < m.header_h {
        return Part::Header;
    }
    if y < m.header_h + m.attrs_h {
        let i = ((y - m.header_h - PAD_Y) / LINE_H).floor();
        return if i >= 0.0 && (i as usize) < n_attrs { Part::Attribute(i as usize) } else { Part::AttrArea };
    }
    let i = ((y - m.header_h - m.attrs_h - PAD_Y) / LINE_H).floor();
    if i >= 0.0 && (i as usize) < n_methods { Part::Method(i as usize) } else { Part::MethodArea }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RelKind {
    /// child → parent
    Inherits,
    /// owner → target; label = attribute names, many = collection
    Association { label: String, many: bool },
}

#[derive(Clone, Debug)]
pub struct Relation {
    pub from: usize,
    pub to: usize,
    pub kind: RelKind,
}

/// Project-internal relationships between classes.
pub fn relations(classes: &[ClassInfo]) -> Vec<Relation> {
    let index: HashMap<&str, usize> = classes.iter().enumerate().map(|(i, c)| (c.name.as_str(), i)).collect();
    let mut out = Vec::new();
    for (i, c) in classes.iter().enumerate() {
        for b in &c.bases {
            if let Some(&j) = index.get(short(b)) {
                if j != i {
                    out.push(Relation { from: i, to: j, kind: RelKind::Inherits });
                }
            }
        }
        // association per target class, labels merged
        let mut assoc: BTreeMap<usize, (Vec<String>, bool)> = BTreeMap::new();
        for a in &c.attributes {
            let Some(ty) = &a.ty else { continue };
            let many = ["list", "set", "dict", "tuple"].iter().any(|k| ty.starts_with(&format!("{k}[")));
            for word in ty.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')) {
                if let Some(&j) = index.get(word) {
                    if j != i {
                        let e = assoc.entry(j).or_default();
                        e.0.push(a.name.clone());
                        e.1 |= many;
                    }
                }
            }
        }
        for (j, (labels, many)) in assoc {
            out.push(Relation { from: i, to: j, kind: RelKind::Association { label: labels.join(", "), many } });
        }
    }
    out
}

fn inheritance_depth(classes: &[ClassInfo]) -> Vec<usize> {
    let index: HashMap<&str, usize> = classes.iter().enumerate().map(|(i, c)| (c.name.as_str(), i)).collect();
    (0..classes.len())
        .map(|start| {
            let mut depth = 0;
            let mut cur = start;
            let mut seen = HashSet::new();
            while seen.insert(cur) {
                match classes[cur].bases.iter().find_map(|b| index.get(short(b))) {
                    Some(&p) => {
                        depth += 1;
                        cur = p;
                    }
                    None => break,
                }
            }
            depth
        })
        .collect()
}

/// Arrange all classes in rows: base classes on top, subclasses below.
pub fn auto_layout(classes: &[ClassInfo], sizes: &[Vec2]) -> BTreeMap<String, [f32; 2]> {
    let depth = inheritance_depth(classes);
    let max_depth = depth.iter().copied().max().unwrap_or(0);
    let mut out = BTreeMap::new();
    let mut y = 20.0;
    for d in 0..=max_depth {
        let row: Vec<usize> = (0..classes.len()).filter(|&i| depth[i] == d).collect();
        let mut x = 20.0;
        let mut row_h: f32 = 0.0;
        for i in row {
            out.insert(classes[i].name.clone(), [x, y]);
            x += sizes[i].x + GAP_X;
            row_h = row_h.max(sizes[i].y);
        }
        y += row_h + GAP_Y;
    }
    out
}

/// Give a position to classes that don't have one yet. Returns true if any
/// position was added.
pub fn place_missing(classes: &[ClassInfo], sizes: &[Vec2], positions: &mut BTreeMap<String, [f32; 2]>) -> bool {
    let missing: Vec<usize> = (0..classes.len()).filter(|&i| !positions.contains_key(&classes[i].name)).collect();
    if missing.is_empty() {
        return false;
    }
    let placed_any = classes.iter().any(|c| positions.contains_key(&c.name));
    if !placed_any {
        *positions = auto_layout(classes, sizes);
        return true;
    }
    let bottom = classes
        .iter()
        .zip(sizes)
        .filter_map(|(c, s)| positions.get(&c.name).map(|p| p[1] + s.y))
        .fold(0.0f32, f32::max);
    let mut x = 20.0;
    for i in missing {
        positions.insert(classes[i].name.clone(), [x, bottom + GAP_Y]);
        x += sizes[i].x + GAP_X;
    }
    true
}

/// Where the line from the centre of `r` towards `toward` leaves the rect.
pub fn border_point(r: Rect, toward: Pos2) -> Pos2 {
    let c = r.center();
    let d = toward - c;
    if d.length_sq() < 1e-6 {
        return c;
    }
    let sx = if d.x.abs() > 1e-6 { r.width() / 2.0 / d.x.abs() } else { f32::INFINITY };
    let sy = if d.y.abs() > 1e-6 { r.height() / 2.0 / d.y.abs() } else { f32::INFINITY };
    c + d * sx.min(sy)
}

pub fn rect_at(pos: [f32; 2], size: Vec2) -> Rect {
    Rect::from_min_size(pos2(pos[0], pos[1]), size)
}
