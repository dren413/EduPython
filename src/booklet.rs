//! The exam booklet: a read-only PDF viewer panel with search.
//!
//! Rendering (hayro, pure Rust) runs on a background thread so scrolling and
//! zooming never wait for a page: until a page is ready, its previous image
//! (or a blank sheet) is shown. The text of every page is extracted once, also
//! in the background, for searching.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use eframe::egui::{
    self, Color32, ColorImage, Key, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2,
};
use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps, InterpreterCache, InterpreterSettings,
    SoftMask, interpret_page,
};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{self, BezPath, Point, Shape};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

const GAP: f32 = 10.0;
const MARGIN: f32 = 8.0;
/// Render widths are rounded up to this many pixels so small panel
/// resizes reuse the existing textures.
const WIDTH_STEP: f32 = 128.0;
const MAX_WIDTH_PX: f32 = 4096.0;
const MIN_ZOOM: f32 = 0.4;
const MAX_ZOOM: f32 = 5.0;
/// Pages above/below the visible ones that are rendered in advance.
const PRELOAD: usize = 2;

// ─────────────────────────────────────────────────────────────────────────────
// Background worker
// ─────────────────────────────────────────────────────────────────────────────

/// Text of one page, in page points (origin top-left).
#[derive(Default)]
pub struct PageText {
    /// For searching: `chars[i]` (lower-case, no accents) sits in `boxes[i]`.
    chars: Vec<char>,
    boxes: Vec<kurbo::Rect>,
    /// For selecting and copying: the real characters (with case, accents,
    /// line breaks and code indentation) and where they are.
    copy: Vec<(char, kurbo::Rect)>,
}

enum Msg {
    Loaded(Vec<(f32, f32)>),
    Failed(String),
    Page { index: usize, width: u32, texture: Option<TextureHandle> },
    Text(Vec<PageText>),
    Links(Vec<Vec<Link>>),
}

/// A clickable area on a page (page points, origin top-left).
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    rect: kurbo::Rect,
    target: LinkTarget,
}

#[derive(Clone, Debug, PartialEq)]
enum LinkTarget {
    /// Another place in the booklet: page and (optionally) the height on it.
    Page(usize, Option<f64>),
    /// A web or mail address.
    Uri(String),
}

#[derive(Default)]
struct Queue {
    /// Pages to render, most important first. Replaced by the UI every frame.
    wanted: Vec<(usize, u32)>,
    busy: Option<(usize, u32)>,
    quit: bool,
}

type Shared = Arc<(Mutex<Queue>, Condvar)>;

fn spawn_worker(path: PathBuf, ctx: egui::Context) -> (Shared, Receiver<Msg>) {
    let shared: Shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
    let (tx, rx) = channel();
    let queue = Arc::clone(&shared);
    thread::Builder::new()
        .name("booklet".into())
        .spawn(move || worker(path, ctx, queue, tx))
        .expect("could not start the booklet thread");
    (shared, rx)
}

fn worker(path: PathBuf, ctx: egui::Context, queue: Shared, tx: Sender<Msg>) {
    let send = |m: Msg| {
        let _ = tx.send(m);
        ctx.request_repaint();
    };
    let pdf = match std::fs::read(&path) {
        Err(e) => return send(Msg::Failed(format!("Could not read {}: {e}", path.display()))),
        Ok(bytes) => match Pdf::new(bytes) {
            Err(e) => return send(Msg::Failed(format!("Could not open the PDF: {e:?}"))),
            Ok(pdf) => pdf,
        },
    };
    let sizes: Vec<(f32, f32)> = pdf.pages().iter().map(|p| p.render_dimensions()).collect();
    if sizes.is_empty() {
        return send(Msg::Failed("The PDF has no pages.".into()));
    }
    send(Msg::Loaded(sizes.clone()));
    let links = std::panic::catch_unwind(AssertUnwindSafe(|| extract_links(&pdf))).unwrap_or_default();
    send(Msg::Links(links));

    let settings = InterpreterSettings::default();
    let cache = RenderCache::new();
    let mut text_done = false;
    loop {
        let job = {
            let (lock, cvar) = &*queue;
            let mut q = lock.lock().unwrap();
            loop {
                if q.quit {
                    return;
                }
                if !q.wanted.is_empty() {
                    let job = q.wanted.remove(0);
                    q.busy = Some(job);
                    break Some(job);
                }
                if !text_done {
                    break None; // nothing to render: extract the text now
                }
                q = cvar.wait(q).unwrap();
            }
        };
        match job {
            Some((index, width)) => {
                let texture = render_page(&ctx, &pdf, &cache, &settings, index, sizes[index].0, width);
                send(Msg::Page { index, width, texture });
                queue.0.lock().unwrap().busy = None;
            }
            None => {
                let text = std::panic::catch_unwind(AssertUnwindSafe(|| extract_text(&pdf))).unwrap_or_default();
                send(Msg::Text(text));
                text_done = true;
            }
        }
    }
}

fn render_page<'a>(
    ctx: &egui::Context,
    pdf: &'a Pdf,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    index: usize,
    page_width_pt: f32,
    width_px: u32,
) -> Option<TextureHandle> {
    let scale = width_px as f32 / page_width_pt;
    // A broken PDF must never take the pupil's work down with it.
    let pixmap = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let page = &pdf.pages()[index];
        let pixmap_settings = PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE };
        hayro::render(page, cache, settings, &RenderSettings::default(), &pixmap_settings)
    }))
    .ok()?;
    let size = [pixmap.width() as usize, pixmap.height() as usize];
    if size[0] == 0 || size[1] == 0 {
        return None;
    }
    let image = ColorImage::from_rgba_premultiplied(size, pixmap.data_as_u8_slice());
    let options = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
    Some(ctx.load_texture(format!("booklet_page_{index}"), image, options))
}

// ─────────────────────────────────────────────────────────────────────────────
// Links (table of contents, cross references, web addresses)
// ─────────────────────────────────────────────────────────────────────────────

fn extract_links(pdf: &Pdf) -> Vec<Vec<Link>> {
    use hayro::hayro_syntax::object::{Array, Dict, Name, String as PdfString};

    let pages = pdf.pages();
    let page_of: HashMap<_, usize> = pages.iter().enumerate().filter_map(|(i, p)| Some((p.raw().obj_id()?, i))).collect();
    let catalog = pdf.xref().get::<Dict>(pdf.xref().root_id());
    let to_page = |page: usize, x: f64, y: f64| -> Point {
        let t = pages[page].initial_transform(true).to_kurbo();
        t * Point::new(x, y)
    };

    // [page /XYZ left top zoom], [page /FitH top], [page /Fit], …
    let explicit = |arr: &Array| -> Option<LinkTarget> {
        let first = arr.raw_iter().next()?;
        let page = match first {
            hayro::hayro_syntax::object::MaybeRef::Ref(r) => *page_of.get(&r.into())?,
            hayro::hayro_syntax::object::MaybeRef::NotRef(hayro::hayro_syntax::object::Object::Number(n)) => n.as_f64() as usize,
            _ => return None,
        };
        let kind = match arr.raw_iter().nth(1) {
            Some(hayro::hayro_syntax::object::MaybeRef::NotRef(hayro::hayro_syntax::object::Object::Name(n))) => n.as_str().to_string(),
            _ => String::new(),
        };
        let nums: Vec<Option<f64>> = arr
            .raw_iter()
            .skip(2)
            .map(|o| match o {
                hayro::hayro_syntax::object::MaybeRef::NotRef(hayro::hayro_syntax::object::Object::Number(n)) => Some(n.as_f64()),
                _ => None,
            })
            .collect();
        let top = match kind.as_str() {
            "XYZ" => nums.get(1).copied().flatten(),
            "FitH" | "FitBH" => nums.first().copied().flatten(),
            _ => None,
        };
        let y = top.map(|t| to_page(page, 0.0, t).y);
        (page < pages.len()).then_some(LinkTarget::Page(page, y))
    };

    // named destinations: catalog /Dests (old style) and the /Names /Dests tree
    fn tree_lookup<'a>(node: &Dict<'a>, key: &[u8], depth: u32) -> Option<hayro::hayro_syntax::object::Object<'a>> {
        use hayro::hayro_syntax::object::{Array, Object, String as PdfString};
        if depth > 32 {
            return None;
        }
        if let Some(names) = node.get::<Array>(b"Names") {
            let mut it = names.iter::<Object>();
            while let (Some(k), Some(v)) = (it.next(), it.next()) {
                if let Object::String(k) = k {
                    if k.as_bytes() == key {
                        return Some(v);
                    }
                }
            }
        }
        if let Some(kids) = node.get::<Array>(b"Kids") {
            for kid in kids.iter::<Dict>() {
                if let Some(limits) = kid.get::<Array>(b"Limits") {
                    let l: Vec<PdfString> = limits.iter::<PdfString>().collect();
                    if l.len() == 2 && (key < l[0].as_bytes() || key > l[1].as_bytes()) {
                        continue;
                    }
                }
                if let Some(v) = tree_lookup(&kid, key, depth + 1) {
                    return Some(v);
                }
            }
        }
        None
    }
    let named = |key: &[u8]| -> Option<LinkTarget> {
        use hayro::hayro_syntax::object::Object;
        let cat = catalog.as_ref()?;
        let found = cat
            .get::<Dict>(b"Names")
            .and_then(|n| n.get::<Dict>(b"Dests"))
            .and_then(|tree| tree_lookup(&tree, key, 0))
            .or_else(|| cat.get::<Dict>(b"Dests").and_then(|d| d.get::<Object>(key)))?;
        match found {
            Object::Array(a) => explicit(&a),
            Object::Dict(d) => d.get::<Array>(b"D").and_then(|a| explicit(&a)),
            _ => None,
        }
    };
    let dest = |holder: &Dict, key: &[u8]| -> Option<LinkTarget> {
        if let Some(a) = holder.get::<Array>(key) {
            return explicit(&a);
        }
        if let Some(s) = holder.get::<PdfString>(key) {
            return named(s.as_bytes());
        }
        holder.get::<Name>(key).and_then(|n| named(n.as_ref()))
    };

    pages
        .iter()
        .enumerate()
        .map(|(i, page)| {
            let Some(annots) = page.raw().get::<Array>(b"Annots") else { return Vec::new() };
            annots
                .iter::<Dict>()
                .filter(|a| a.get::<Name>(b"Subtype").is_some_and(|n| n.as_str() == "Link"))
                .filter_map(|a| {
                    let r: Vec<f64> = a.get::<Array>(b"Rect")?.iter::<f64>().collect();
                    if r.len() != 4 {
                        return None;
                    }
                    let (p0, p1) = (to_page(i, r[0], r[1]), to_page(i, r[2], r[3]));
                    let rect = kurbo::Rect::from_points(p0, p1);
                    let target = match a.get::<Dict>(b"A") {
                        Some(action) => match action.get::<Name>(b"S").map(|n| n.as_str().to_string()).as_deref() {
                            Some("GoTo") => dest(&action, b"D"),
                            Some("URI") => action
                                .get::<PdfString>(b"URI")
                                .map(|u| LinkTarget::Uri(std::string::String::from_utf8_lossy(u.as_bytes()).into_owned())),
                            _ => None,
                        },
                        None => dest(&a, b"Dest"),
                    }?;
                    Some(Link { rect, target })
                })
                .collect()
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Text extraction (for search)
// ─────────────────────────────────────────────────────────────────────────────

/// One drawn glyph, in page points (origin top-left).
struct RawGlyph {
    x: f64,
    y: f64,
    left: f64,
    right: f64,
    size: f64,
    text: String,
}

/// A hayro device that only records where each character is drawn.
#[derive(Default)]
struct TextCollector {
    glyphs: Vec<RawGlyph>,
}

impl<'a> Device<'a> for TextCollector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        for g in run.glyphs() {
            let t = props.transform * g.transform();
            let origin = t * Point::ZERO;
            // glyph space has 1000 units per em
            let size = ((t * Point::new(0.0, 1000.0)) - origin).hypot();
            if size <= 0.01 {
                continue;
            }
            let (left, right) = match &**g {
                Glyph::Outline(o) => {
                    let b = t.transform_rect_bbox(o.outline().bounding_box());
                    if b.width() > 0.0 { (b.x0, b.x1) } else { (origin.x, origin.x + size * 0.3) }
                }
                Glyph::Type3(t3) => {
                    let w = t3.advance_width().unwrap_or(500.0) as f64 / 1000.0 * size;
                    (origin.x, origin.x + w)
                }
            };
            let text = match g.as_unicode() {
                Some(BfString::Char(c)) => c.to_string(),
                Some(BfString::String(s)) => s,
                None => continue,
            };
            self.glyphs.push(RawGlyph { x: origin.x, y: origin.y, left, right, size, text });
        }
    }
}

fn size_key(size: f64) -> i64 {
    (size * 10.0).round() as i64
}

impl TextCollector {
    /// Turn the glyphs into searchable text. PDFs rarely draw spaces, so they
    /// are inferred: a space needs a visible white gap between the letters'
    /// ink *and* a step clearly larger than a normal letter step of that font
    /// size on this page. The second test keeps narrow characters in code
    /// (`[:]`, `2]`) together, the first keeps wide letters (`m`) together.
    fn finish(self) -> PageText {
        let same_line = |a: &RawGlyph, b: &RawGlyph| (a.y - b.y).abs() <= a.size.max(b.size) * 0.5;

        // normal letter step per font size: a low percentile of all steps
        let mut steps: HashMap<i64, Vec<f64>> = HashMap::new();
        for w in self.glyphs.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let adv = b.x - a.x;
            if same_line(a, b) && size_key(a.size) == size_key(b.size) && adv > a.size * 0.15 && adv < a.size * 2.0 {
                steps.entry(size_key(a.size)).or_default().push(adv);
            }
        }
        let step: HashMap<i64, f64> = steps
            .into_iter()
            .filter(|(_, v)| v.len() >= 8)
            .map(|(k, mut v)| {
                v.sort_by(f64::total_cmp);
                (k, v[v.len() / 10])
            })
            .collect();

        // Left edge of the text per font size, to rebuild code indentation.
        let mut base_x: HashMap<i64, f64> = HashMap::new();
        for (i, g) in self.glyphs.iter().enumerate() {
            if i == 0 || !same_line(&self.glyphs[i - 1], g) {
                let e = base_x.entry(size_key(g.size)).or_insert(g.x);
                *e = e.min(g.x);
            }
        }

        let mut out = PageText::default();
        let push_copy = |out: &mut PageText, c: char, r: kurbo::Rect| {
            if c == ' ' && out.copy.last().is_none_or(|&(l, _)| l == ' ' || l == '\n') {
                return;
            }
            out.copy.push((c, r));
        };
        let push = |out: &mut PageText, c: char, r: kurbo::Rect| {
            if c == ' ' && out.chars.last().is_none_or(|&l| l == ' ') {
                return;
            }
            out.chars.push(c);
            out.boxes.push(r);
        };
        for (i, g) in self.glyphs.iter().enumerate() {
            let line_box = kurbo::Rect::new(g.left, g.y - g.size * 0.78, g.right, g.y + g.size * 0.24);
            if i > 0 {
                let p = &self.glyphs[i - 1];
                let s = p.size.max(g.size);
                let adv = g.x - p.x;
                let new_line = !same_line(p, g) || adv < -s * 3.0;
                let space = if new_line {
                    true
                } else {
                    let normal = step.get(&size_key(g.size)).filter(|_| size_key(p.size) == size_key(g.size));
                    g.left - p.right > s * 0.18 && normal.is_none_or(|n| adv > n * 1.6)
                };
                let gap = kurbo::Rect::new(p.right, line_box.y0, p.right, line_box.y1);
                if space {
                    push(&mut out, ' ', gap);
                }
                if new_line {
                    while out.copy.last().is_some_and(|&(c, _)| c == ' ') {
                        out.copy.pop();
                    }
                    out.copy.push(('\n', gap));
                    // indentation of code lines (monospace: known step per character)
                    let key = size_key(g.size);
                    if let (Some(step), Some(base)) = (step.get(&key), base_x.get(&key)) {
                        let n = ((g.x - base) / step).round() as usize;
                        let at = kurbo::Rect::new(g.left, line_box.y0, g.left, line_box.y1);
                        for _ in 0..n.min(40) {
                            out.copy.push((' ', at));
                        }
                    }
                } else if space {
                    push_copy(&mut out, ' ', gap);
                }
            }
            let originals: Vec<char> = g.text.chars().flat_map(copy_char).collect();
            let m = originals.len().max(1) as f64;
            let cw = line_box.width() / m;
            for (k, c) in originals.into_iter().enumerate() {
                let r = kurbo::Rect::new(line_box.x0 + cw * k as f64, line_box.y0, line_box.x0 + cw * (k as f64 + 1.0), line_box.y1);
                push_copy(&mut out, c, r);
            }
            let chars: Vec<char> = g.text.chars().flat_map(normalize).collect();
            let n = chars.len().max(1) as f64;
            let w = line_box.width() / n;
            for (k, c) in chars.into_iter().enumerate() {
                let r = kurbo::Rect::new(line_box.x0 + w * k as f64, line_box.y0, line_box.x0 + w * (k as f64 + 1.0), line_box.y1);
                push(&mut out, c, r);
            }
        }
        out
    }
}

fn extract_text(pdf: &Pdf) -> Vec<PageText> {
    let cache = InterpreterCache::new();
    pdf.pages()
        .iter()
        .map(|page| {
            let (w, h) = page.render_dimensions();
            let bbox = kurbo::Rect::new(0.0, 0.0, w as f64, h as f64);
            let mut ctx = Context::new(page.initial_transform(true).to_kurbo(), bbox, &cache, page.xref(), InterpreterSettings::default());
            let mut dev = TextCollector::default();
            interpret_page(page, &mut ctx, &mut dev);
            dev.finish()
        })
        .collect()
}

/// A character as it should be pasted into code: ligatures split up and
/// typographic variants replaced by what Python expects (− → -, ’ → ', ∗ → *).
/// Case and accents are kept.
fn copy_char(c: char) -> Vec<char> {
    let lig = match c {
        'ﬀ' => "ff",
        'ﬁ' => "fi",
        'ﬂ' => "fl",
        'ﬃ' => "ffi",
        'ﬄ' => "ffl",
        'ﬅ' | 'ﬆ' => "st",
        _ => "",
    };
    if !lig.is_empty() {
        return lig.chars().collect();
    }
    vec![match c {
        '’' | '‘' | '′' => '\'',
        '“' | '”' => '"',
        '−' | '–' | '—' | '‐' => '-',
        '∗' => '*',
        '\u{a0}' | '\u{2009}' | '\u{202f}' | '\t' => ' ',
        _ => c,
    }]
}

/// For searching: like [`copy_char`], plus lower-case and without accents.
fn normalize(c: char) -> Vec<char> {
    let mut out = Vec::new();
    for c in copy_char(c) {
        let folded = match c {
            'œ' | 'Œ' => "oe",
            'æ' | 'Æ' => "ae",
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => "a",
            'ç' | 'Ç' => "c",
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => "e",
            'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => "i",
            'ñ' | 'Ñ' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => "o",
            'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => "u",
            'ý' | 'ÿ' | 'Ý' => "y",
            '`' => "'",
            '«' | '»' => "\"",
            '×' => "*",
            '\n' | '\r' => " ",
            _ => "",
        };
        if folded.is_empty() {
            out.extend(c.to_lowercase());
        } else {
            out.extend(folded.chars());
        }
    }
    out
}

fn normalize_query(q: &str) -> Vec<char> {
    let mut out: Vec<char> = Vec::new();
    for c in q.chars().flat_map(normalize) {
        if c == ' ' && out.last().is_none_or(|&l| l == ' ') {
            continue;
        }
        out.push(c);
    }
    while out.last() == Some(&' ') {
        out.pop();
    }
    out
}

/// One search hit: its page and highlight boxes (one per line it spans).
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub page: usize,
    pub rects: Vec<kurbo::Rect>,
}

pub fn search(pages: &[PageText], query: &str) -> Vec<Hit> {
    let q = normalize_query(query);
    let mut hits = Vec::new();
    if q.is_empty() {
        return hits;
    }
    for (page, text) in pages.iter().enumerate() {
        if text.chars.len() < q.len() {
            continue;
        }
        let mut i = 0;
        while i + q.len() <= text.chars.len() {
            if text.chars[i..i + q.len()] == q[..] {
                let mut rects: Vec<kurbo::Rect> = Vec::new();
                for b in &text.boxes[i..i + q.len()] {
                    match rects.last_mut() {
                        Some(r) if (r.y0 - b.y0).abs() < 2.0 && b.x0 >= r.x0 - 1.0 => *r = r.union(*b),
                        _ if b.width() > 0.0 => rects.push(*b),
                        _ => {}
                    }
                }
                hits.push(Hit { page, rects });
                i += q.len();
            } else {
                i += 1;
            }
        }
    }
    hits
}

/// Caret position (0..=len) in `t.copy` closest to `pt` (page points).
fn caret_in(t: &PageText, pt: Point) -> usize {
    let mut best: Option<(f64, f64, usize)> = None; // (vertical, horizontal distance, index)
    for (i, &(c, r)) in t.copy.iter().enumerate() {
        if c == '\n' {
            continue;
        }
        let dy = if pt.y < r.y0 { r.y0 - pt.y } else if pt.y > r.y1 { pt.y - r.y1 } else { 0.0 };
        let dx = if pt.x < r.x0 { r.x0 - pt.x } else if pt.x > r.x1 { pt.x - r.x1 } else { 0.0 };
        if best.is_none_or(|(by, bx, _)| (dy, dx) < (by, bx)) {
            best = Some((dy, dx, i));
        }
    }
    match best {
        Some((_, _, i)) => {
            let r = t.copy[i].1;
            if pt.x > (r.x0 + r.x1) / 2.0 { i + 1 } else { i }
        }
        None => 0,
    }
}

/// The word (letters, digits, `_`) around caret `c`.
fn word_around(t: &PageText, c: usize) -> (usize, usize) {
    let is_word = |i: usize| t.copy.get(i).is_some_and(|(ch, _)| ch.is_alphanumeric() || *ch == '_');
    let mut a = c.min(t.copy.len());
    if !is_word(a) && a > 0 && is_word(a - 1) {
        a -= 1;
    }
    if !is_word(a) {
        return (a, (a + 1).min(t.copy.len()));
    }
    let mut b = a;
    while a > 0 && is_word(a - 1) {
        a -= 1;
    }
    while is_word(b) {
        b += 1;
    }
    (a, b)
}

/// The text between two carets (any order, may span pages).
fn selected_text(pages: &[PageText], a: (usize, usize), b: (usize, usize)) -> String {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let mut out = String::new();
    for page in lo.0..=hi.0.min(pages.len().saturating_sub(1)) {
        let t = &pages[page];
        let from = if page == lo.0 { lo.1 } else { 0 }.min(t.copy.len());
        let to = if page == hi.0 { hi.1 } else { t.copy.len() }.min(t.copy.len());
        if page > lo.0 {
            out.push('\n');
        }
        out.extend(t.copy[from.min(to)..to].iter().map(|(c, _)| *c));
    }
    // no trailing spaces on lines
    out.lines().map(str::trim_end).collect::<Vec<_>>().join("\n")
}

/// Highlight boxes for a run of characters: one rectangle per line.
fn line_rects(chars: &[(char, kurbo::Rect)]) -> Vec<kurbo::Rect> {
    let mut rects: Vec<kurbo::Rect> = Vec::new();
    for &(c, r) in chars {
        if c == '\n' {
            continue;
        }
        match rects.last_mut() {
            Some(last) if (last.y0 - r.y0).abs() < 2.0 => *last = last.union(r),
            _ if r.width() > 0.0 || c == ' ' => rects.push(r),
            _ => {}
        }
    }
    rects.retain(|r| r.width() > 0.0);
    rects
}

// ─────────────────────────────────────────────────────────────────────────────
// The panel
// ─────────────────────────────────────────────────────────────────────────────

enum State {
    NotStarted,
    Loading,
    Ready(Vec<(f32, f32)>),
    Failed(String),
}

pub struct Booklet {
    pub path: PathBuf,
    pub title: String,
    /// Page at the top of the view (0-based); saved in the settings.
    pub page: usize,
    state: State,
    worker: Option<(Shared, Receiver<Msg>)>,
    /// page index → (render width in px, texture). Old textures are kept
    /// (scaled) until the sharper one arrives.
    textures: HashMap<usize, (u32, TextureHandle)>,
    text: Option<Vec<PageText>>,
    links: Vec<Vec<Link>>,
    /// Jump to a page and height (page points) in the next frame.
    goto_point: Option<(usize, f64)>,
    zoom: f32,
    /// Scroll target for the next frame.
    scroll_to: Option<Vec2>,
    goto_page: Option<usize>,
    /// Scroll to the current search hit in the next frame.
    pending_hit: bool,
    /// Last scroll offset (to keep the point under the mouse while zooming).
    offset: Vec2,
    page_input: String,
    query: String,
    hits: Vec<Hit>,
    current_hit: usize,
    focus_search: bool,
    /// Text selection: (anchor, head), each (page, caret index into `PageText::copy`).
    sel: Option<((usize, usize), (usize, usize))>,
    /// Id of the page area (it takes the keyboard focus for copying).
    area_id: Option<egui::Id>,
}

impl Drop for Booklet {
    fn drop(&mut self) {
        if let Some((shared, _)) = &self.worker {
            shared.0.lock().unwrap().quit = true;
            shared.1.notify_all();
        }
    }
}

impl Booklet {
    pub fn new(path: PathBuf, title: String, page: usize) -> Self {
        Self {
            path,
            title,
            page,
            state: State::NotStarted,
            worker: None,
            textures: HashMap::new(),
            text: None,
            links: Vec::new(),
            goto_point: None,
            zoom: 1.0,
            scroll_to: None,
            goto_page: Some(page),
            pending_hit: false,
            offset: Vec2::ZERO,
            page_input: String::new(),
            query: String::new(),
            hits: Vec::new(),
            current_hit: 0,
            focus_search: false,
            sel: None,
            area_id: None,
        }
    }

    /// Put the keyboard focus in the search box (Ctrl/Cmd+F).
    pub fn focus_search(&mut self) {
        self.focus_search = true;
    }

    fn page_count(&self) -> usize {
        match &self.state {
            State::Ready(sizes) => sizes.len(),
            _ => 0,
        }
    }

    fn poll_worker(&mut self) {
        let Some((_, rx)) = &self.worker else { return };
        let msgs: Vec<Msg> = rx.try_iter().collect();
        for msg in msgs {
            match msg {
                Msg::Loaded(sizes) => {
                    self.page = self.page.min(sizes.len() - 1);
                    self.state = State::Ready(sizes);
                }
                Msg::Failed(e) => self.state = State::Failed(e),
                Msg::Page { index, width, texture: Some(t) } => {
                    self.textures.insert(index, (width, t));
                }
                Msg::Page { texture: None, .. } => {}
                Msg::Links(l) => self.links = l,
                Msg::Text(t) => {
                    self.text = Some(t);
                    if !self.query.is_empty() {
                        self.run_search(false);
                    }
                }
            }
        }
    }

    fn run_search(&mut self, jump: bool) {
        self.hits = self.text.as_deref().map(|t| search(t, &self.query)).unwrap_or_default();
        // start at the first hit at or after the current page
        self.current_hit = self.hits.iter().position(|h| h.page >= self.page).unwrap_or(0);
        if jump && !self.hits.is_empty() {
            self.goto_hit();
        }
    }

    fn goto_hit(&mut self) {
        // resolved to a scroll offset in `show`, where the layout is known
        self.goto_page = None;
        self.scroll_to = None;
        self.pending_hit = true;
    }

    pub fn show(&mut self, ui: &mut Ui) {
        if matches!(self.state, State::NotStarted) {
            self.worker = Some(spawn_worker(self.path.clone(), ui.ctx().clone()));
            self.state = State::Loading;
        }
        self.poll_worker();
        let count = self.page_count();

        self.toolbar(ui, count);
        ui.separator();

        let sizes = match &self.state {
            State::Ready(sizes) => sizes.clone(),
            State::Failed(e) => {
                ui.colored_label(Color32::from_rgb(220, 70, 70), e.as_str());
                return;
            }
            _ => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Opening the booklet…");
                });
                return;
            }
        };

        let area_rect = ui.available_rect_before_wrap();
        let (zoom_delta, pointer) = ui.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
        let hovered = pointer.is_some_and(|p| area_rect.contains(p));

        // ── zoom (Ctrl + mouse wheel, trackpad pinch), anchored at the mouse ──
        let old_page_w = self.page_width(area_rect.width());
        if hovered && (zoom_delta - 1.0).abs() > 1e-4 {
            let new_zoom = (self.zoom * zoom_delta).clamp(MIN_ZOOM, MAX_ZOOM);
            if new_zoom != self.zoom {
                self.zoom = new_zoom;
                let f = self.page_width(area_rect.width()) / old_page_w;
                let anchor = pointer.unwrap() - area_rect.min;
                self.scroll_to = Some((self.offset + anchor) * f - anchor);
            }
        }

        // ── keyboard ──
        let area_id = self.area_id;
        let free_keys = ui.ctx().memory(|m| m.focused().is_none_or(|f| Some(f) == area_id));
        if hovered && free_keys {
            let (up, down, home, end) = ui.input(|i| {
                (i.key_pressed(Key::PageUp), i.key_pressed(Key::PageDown), i.key_pressed(Key::Home), i.key_pressed(Key::End))
            });
            if up && self.page > 0 {
                self.goto_page = Some(self.page - 1);
            } else if down && self.page + 1 < count {
                self.goto_page = Some(self.page + 1);
            } else if home {
                self.goto_page = Some(0);
            } else if end {
                self.goto_page = Some(count - 1);
            }
        }

        // ── layout: every page is scaled to the same width ──
        let page_w = self.page_width(area_rect.width());
        let content_w = page_w + 2.0 * MARGIN;
        let mut tops = Vec::with_capacity(count);
        let mut y = MARGIN;
        for (w, h) in &sizes {
            tops.push(y);
            y += page_w * h / w + GAP;
        }
        let total = vec2(content_w.max(area_rect.width()), y + MARGIN - GAP);
        let x0 = ((total.x - page_w) / 2.0).max(MARGIN);
        let page_rect_of = |i: usize| Rect::from_min_size(pos2(x0, tops[i]), vec2(page_w, page_w * sizes[i].1 / sizes[i].0));
        let pt_scale = |i: usize| page_w / sizes[i].0;

        if let Some(p) = self.goto_page.take() {
            self.scroll_to = Some(vec2(self.offset.x, tops[p.min(count - 1)] - MARGIN));
        }
        if let Some((p, y)) = self.goto_point.take() {
            let p = p.min(count - 1);
            self.scroll_to = Some(vec2(self.offset.x, (tops[p] + y as f32 * pt_scale(p) - 12.0).max(0.0)));
        }
        if std::mem::take(&mut self.pending_hit) {
            if let Some(hit) = self.hits.get(self.current_hit) {
                let s = pt_scale(hit.page);
                let r = hit.rects.first().copied().unwrap_or_default();
                let target = page_rect_of(hit.page).min + vec2(r.x0 as f32 * s, r.y0 as f32 * s);
                let x = if target.x > self.offset.x + area_rect.width() - 40.0 || target.x < self.offset.x {
                    (target.x - area_rect.width() * 0.3).max(0.0)
                } else {
                    self.offset.x
                };
                self.scroll_to = Some(vec2(x, (target.y - area_rect.height() * 0.3).max(0.0)));
            }
        }

        let mut area = egui::ScrollArea::both().id_salt("booklet_scroll").auto_shrink(false);
        if let Some(off) = self.scroll_to.take() {
            area = area.scroll_offset(off.max(Vec2::ZERO));
        }
        let ppp = ui.ctx().pixels_per_point();
        let width_px = ((page_w * ppp / WIDTH_STEP).ceil() * WIDTH_STEP).min(MAX_WIDTH_PX) as u32;
        let dark = ui.visuals().dark_mode;
        let hits = &self.hits;
        let current_hit = self.current_hit;
        let textures = &self.textures;
        let text = self.text.as_deref();
        let links = &self.links;
        let mut follow: Option<LinkTarget> = None;
        let sel = &mut self.sel;
        let sel_fill = ui.visuals().selection.bg_fill.gamma_multiply(0.55);
        let mut copy_now: Option<String> = None;
        let mut area_id = None;

        let mut visible: Vec<usize> = Vec::new();
        let mut page_now = self.page;
        let out = area.show_viewport(ui, |ui, viewport| {
            let (rect, resp) = ui.allocate_exact_size(total, Sense::click_and_drag());
            let origin = rect.min.to_vec2();
            area_id = Some(resp.id);

            // ── links: a click (also Ctrl/Cmd + click) follows them ──
            let link_at = |pos: egui::Pos2| -> Option<&Link> {
                let p = pos - origin;
                let i = (0..count).find(|&i| page_rect_of(i).contains(p))?;
                let pr = page_rect_of(i);
                let s = pt_scale(i);
                let pt = Point::new(((p.x - pr.min.x) / s) as f64, ((p.y - pr.min.y) / s) as f64);
                links.get(i)?.iter().find(|l| l.rect.inflate(1.0, 1.0).contains(pt))
            };
            let hovered_link = resp.hover_pos().and_then(link_at);
            if let Some(l) = hovered_link {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                let tip = match &l.target {
                    LinkTarget::Page(p, _) => format!("Go to page {}", p + 1),
                    LinkTarget::Uri(u) => u.clone(),
                };
                egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), resp.id.with("link_tip"), egui::PopupAnchor::Pointer)
                    .gap(12.0)
                    .show(|ui| ui.label(tip));
            }
            let clicked_link = if resp.clicked() { resp.interact_pointer_pos().and_then(link_at).cloned() } else { None };
            if let Some(l) = clicked_link {
                follow = Some(l.target);
            }

            // ── text selection: drag to select, double-click for a word ──
            if let Some(text) = text {
                // page + caret under a screen position (nearest page if between pages)
                let caret_at = |pos: egui::Pos2| -> (usize, usize) {
                    let p = pos - origin;
                    let i = (0..count)
                        .min_by(|&a, &b| {
                            let da = page_rect_of(a).distance_to_pos(p);
                            let db = page_rect_of(b).distance_to_pos(p);
                            da.total_cmp(&db)
                        })
                        .unwrap_or(0);
                    let pr = page_rect_of(i);
                    let s = pt_scale(i);
                    let pt = Point::new(((p.x - pr.min.x) / s) as f64, ((p.y - pr.min.y) / s) as f64);
                    (i, text.get(i).map_or(0, |t| caret_in(t, pt)))
                };
                if resp.hovered() && hovered_link.is_none() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                }
                if resp.drag_started() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let c = caret_at(pos);
                        *sel = Some((c, c));
                    }
                    resp.request_focus();
                } else if resp.dragged() {
                    if let (Some(pos), Some((_, head))) = (resp.interact_pointer_pos(), sel.as_mut()) {
                        *head = caret_at(pos);
                    }
                }
                if resp.double_clicked() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let (page, c) = caret_at(pos);
                        if let Some(t) = text.get(page) {
                            let (a, b) = word_around(t, c);
                            *sel = Some(((page, a), (page, b)));
                        }
                    }
                    resp.request_focus();
                } else if resp.clicked() && follow.is_none() {
                    *sel = None;
                    resp.request_focus();
                }
                let has_sel = sel.is_some_and(|(a, b)| a != b);
                if resp.has_focus() {
                    let (copy, esc) = ui.input(|i| (i.events.iter().any(|e| matches!(e, egui::Event::Copy)), i.key_pressed(Key::Escape)));
                    if copy && has_sel {
                        copy_now = sel.map(|(a, b)| selected_text(text, a, b));
                    }
                    if esc {
                        *sel = None;
                    }
                }
                resp.context_menu(|ui| {
                    if ui.add_enabled(has_sel, egui::Button::new("Copy").shortcut_text(ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, Key::C)))).clicked() {
                        copy_now = sel.map(|(a, b)| selected_text(text, a, b));
                        ui.close();
                    }
                    if ui.button("Select whole page").clicked() {
                        let page = page_now.min(count - 1);
                        *sel = Some(((page, 0), (page, text.get(page).map_or(0, |t| t.copy.len()))));
                        ui.close();
                    }
                });
            }

            let probe = viewport.min.y + viewport.height() * 0.3;
            page_now = tops.iter().rposition(|&t| t <= probe).unwrap_or(0);
            let painter = ui.painter();
            for i in 0..count {
                let pr = page_rect_of(i);
                if pr.max.y < viewport.min.y || pr.min.y > viewport.max.y {
                    continue;
                }
                visible.push(i);
                let pr = pr.translate(origin);
                let shadow = if dark { Color32::from_black_alpha(160) } else { Color32::from_black_alpha(40) };
                painter.rect_filled(pr.translate(vec2(1.5, 2.0)), 2.0, shadow);
                painter.rect_filled(pr, 0.0, Color32::WHITE);
                if let Some((_, tex)) = textures.get(&i) {
                    painter.image(tex.id(), pr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                painter.rect_stroke(pr, 0.0, Stroke::new(0.5, Color32::from_gray(170)), egui::StrokeKind::Outside);
                let s = page_w / sizes[i].0;
                for (n, hit) in hits.iter().enumerate().filter(|(_, h)| h.page == i) {
                    let fill = if n == current_hit {
                        Color32::from_rgba_unmultiplied(255, 140, 0, 110)
                    } else {
                        Color32::from_rgba_unmultiplied(255, 220, 0, 90)
                    };
                    for r in &hit.rects {
                        let hr = Rect::from_min_max(
                            pr.min + vec2(r.x0 as f32 * s, r.y0 as f32 * s),
                            pr.min + vec2(r.x1 as f32 * s, r.y1 as f32 * s),
                        )
                        .expand(1.0);
                        painter.rect_filled(hr, 2.0, fill);
                    }
                }
                // selected text on this page
                if let (Some((a, b)), Some(t)) = (*sel, text.and_then(|t| t.get(i))) {
                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                    if lo.0 <= i && i <= hi.0 {
                        let from = if lo.0 == i { lo.1 } else { 0 };
                        let to = if hi.0 == i { hi.1 } else { t.copy.len() };
                        for r in line_rects(&t.copy[from.min(to)..to.min(t.copy.len())]) {
                            let sr = Rect::from_min_max(
                                pr.min + vec2(r.x0 as f32 * s, r.y0 as f32 * s),
                                pr.min + vec2(r.x1 as f32 * s, r.y1 as f32 * s),
                            );
                            painter.rect_filled(sr, 1.0, sel_fill);
                        }
                    }
                }
            }
        });
        self.area_id = area_id;
        match follow {
            Some(LinkTarget::Page(p, y)) => self.goto_point = Some((p, y.unwrap_or(0.0))),
            Some(LinkTarget::Uri(u)) => ui.ctx().open_url(egui::OpenUrl::new_tab(u)),
            None => {}
        }
        if let Some(t) = copy_now {
            ui.ctx().copy_text(t);
        }
        self.offset = out.state.clamped_offset();
        self.page = page_now;

        // ── ask the worker for missing pages: visible first, then neighbours ──
        if let (Some((shared, _)), Some(&first), Some(&last)) = (&self.worker, visible.first(), visible.last()) {
            let lo = first.saturating_sub(PRELOAD);
            let hi = (last + PRELOAD).min(count - 1);
            let mut order: Vec<usize> = visible.clone();
            order.extend((last + 1..=hi).chain((lo..first).rev()));
            let (lock, cvar) = &**shared;
            let mut q = lock.lock().unwrap();
            q.wanted = order
                .into_iter()
                .filter(|i| self.textures.get(i).is_none_or(|(w, _)| *w != width_px))
                .map(|i| (i, width_px))
                .filter(|job| q.busy != Some(*job))
                .collect();
            if !q.wanted.is_empty() {
                cvar.notify_one();
            }
            drop(q);
            // free memory of pages far away
            self.textures.retain(|&i, _| i + 2 * PRELOAD >= first && i <= last + 2 * PRELOAD);
        }
    }

    fn page_width(&self, avail: f32) -> f32 {
        ((avail - 2.0 * MARGIN) * self.zoom).max(80.0)
    }

    fn toolbar(&mut self, ui: &mut Ui, count: usize) {
        ui.horizontal_wrapped(|ui| {
            ui.strong(&self.title);
            if count == 0 {
                return;
            }
            ui.separator();
            if ui.add_enabled(self.page > 0, egui::Button::new("<")).on_hover_text("Previous page (Page Up)").clicked() {
                self.goto_page = Some(self.page - 1);
            }
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.page_input)
                    .desired_width(36.0)
                    .align(egui::Align2::CENTER_CENTER)
                    .hint_text((self.page + 1).to_string()),
            );
            if resp.lost_focus() {
                if let Ok(n) = self.page_input.trim().parse::<usize>() {
                    self.goto_page = Some(n.clamp(1, count) - 1);
                }
                self.page_input.clear();
            }
            ui.label(format!("/ {count}"));
            if ui.add_enabled(self.page + 1 < count, egui::Button::new(">")).on_hover_text("Next page (Page Down)").clicked() {
                self.goto_page = Some(self.page + 1);
            }
            ui.separator();
            if ui.button("−").on_hover_text("Zoom out (Ctrl + mouse wheel, or pinch)").clicked() {
                self.zoom = (self.zoom / 1.2).max(MIN_ZOOM);
                self.goto_page = Some(self.page);
            }
            if ui.button(format!("{:.0}%", self.zoom * 100.0)).on_hover_text("Fit to width").clicked() {
                self.zoom = 1.0;
                self.goto_page = Some(self.page);
            }
            if ui.button("+").on_hover_text("Zoom in (Ctrl + mouse wheel, or pinch)").clicked() {
                self.zoom = (self.zoom * 1.2).min(MAX_ZOOM);
                self.goto_page = Some(self.page);
            }
        });
        if count == 0 {
            return;
        }
        // ── search ──
        ui.horizontal_wrapped(|ui| {
            let id = ui.scope_id().with("booklet_search");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id(id)
                    .hint_text("Search the booklet (Ctrl+F)")
                    .desired_width((ui.available_width() - 150.0).clamp(80.0, 260.0)),
            );
            if std::mem::take(&mut self.focus_search) {
                resp.request_focus();
            }
            if resp.changed() {
                self.run_search(true);
            }
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let shift = ui.input(|i| i.modifiers.shift);
            let n = self.hits.len();
            let mut step: i32 = 0;
            if enter && n > 0 {
                step = if shift { -1 } else { 1 };
                resp.request_focus(); // keep typing / pressing Enter
            }
            if !self.query.trim().is_empty() {
                if self.text.is_none() {
                    ui.spinner();
                } else if n == 0 {
                    ui.label(RichText::new("not found").color(Color32::from_rgb(220, 90, 70)));
                } else {
                    ui.label(format!("{} / {n}", self.current_hit + 1));
                }
                if ui.add_enabled(n > 0, egui::Button::new("Prev")).on_hover_text("Previous match (Shift+Enter)").clicked() {
                    step = -1;
                }
                if ui.add_enabled(n > 0, egui::Button::new("Next")).on_hover_text("Next match (Enter)").clicked() {
                    step = 1;
                }
                if ui.add(crate::icons::IconButton::new(crate::icons::Icon::Trash).small()).on_hover_text("Clear search").clicked() {
                    self.query.clear();
                    self.hits.clear();
                }
            }
            if step != 0 && n > 0 {
                self.current_hit = (self.current_hit as i32 + step).rem_euclid(n as i32) as usize;
                self.goto_hit();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_for_search() {
        assert_eq!(normalize_query("  Création  de   LISTES "), "creation de listes".chars().collect::<Vec<_>>());
        assert_eq!(normalize_query("range(8, −4)"), "range(8, -4)".chars().collect::<Vec<_>>());
    }

    #[test]
    fn finds_hits_across_boxes() {
        let mk = |s: &str, y: f64| -> Vec<(char, kurbo::Rect)> {
            s.chars().enumerate().map(|(i, c)| (c, kurbo::Rect::new(i as f64 * 5.0, y, i as f64 * 5.0 + 4.0, y + 10.0))).collect()
        };
        let mut page = PageText::default();
        for (c, r) in mk("print(x) ", 0.0).into_iter().chain(mk("print", 20.0)) {
            page.chars.push(c);
            page.boxes.push(r);
        }
        let hits = search(&[PageText::default(), page], "PRINT");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].page, 1);
        assert_eq!(hits[0].rects, [kurbo::Rect::new(0.0, 0.0, 24.0, 10.0)]);
        assert!(search(&[], "x").is_empty());
    }

    /// Renders and searches the real booklet if it is available on this machine.
    #[test]
    fn real_booklet_if_present() {
        let Some(path) = std::env::var_os("EDUPYTHON_TEST_BOOKLET").map(PathBuf::from) else { return };
        let pdf = Pdf::new(std::fs::read(path).unwrap()).unwrap();
        let ctx = egui::Context::default();
        let settings = InterpreterSettings::default();
        let cache = RenderCache::new();
        for i in 0..pdf.pages().len() {
            let w = pdf.pages()[i].render_dimensions().0;
            let tex = render_page(&ctx, &pdf, &cache, &settings, i, w, 1024).unwrap();
            assert_eq!(tex.size()[0], 1024);
        }
        let text = extract_text(&pdf);
        assert_eq!(text.len(), pdf.pages().len());
        let deepcopy = search(&text, "deepcopy");
        assert!(deepcopy.iter().any(|h| h.page == 13), "{deepcopy:?}");
        assert!(!search(&text, "création automatisée").is_empty());
        assert!(!search(&text, "MOUSEBUTTONDOWN").is_empty());
        assert!(!search(&text, "copy_list = my_list[:]").is_empty(), "spaces inside code");
        assert!(!search(&text, "[[1, 2], [3, 4]]").is_empty(), "no fake spaces in code");
        assert!(!search(&text, "first-level copy").is_empty(), "ligatures and minus");
        assert!(!search(&text, "pour réagir aussi longtemps").is_empty(), "prose");
        assert!(!search(&text, "if event.type == MOUSEBUTTONUP").is_empty(), "start of code lines");
        assert!(!search(&text, "x ** 2 for x in").is_empty(), "math asterisk");

        // table of contents links jump into the booklet
        let links = extract_links(&pdf);
        let toc: Vec<&Link> = links[2].iter().collect();
        assert!(toc.len() > 5, "{toc:?}");
        assert!(toc.iter().all(|l| matches!(l.target, LinkTarget::Page(p, Some(_)) if p > 2)), "{toc:?}");
        assert!(links[1].iter().any(|l| l.target == LinkTarget::Uri("mailto:secretariat@script.lu".into())));

        // copying keeps case, accents, line breaks and code indentation
        let page20: String = text[20].copy.iter().map(|(c, _)| *c).collect();
        assert!(page20.contains("if event.type == MOUSEBUTTONDOWN and event.button == 1:\n    <instruction(s)>"), "{page20}");
        assert!(page20.contains("Pour réagir aussi longtemps"), "{page20}");
        let page13: String = text[13].copy.iter().map(|(c, _)| *c).collect();
        assert!(page13.contains("my_list = list(range(8, -4, -3))"), "{page13}");
        assert!(page13.contains("first-level copy"), "{page13}");
        // select a word, and copy across a range
        let i = page13.find("deepcopy(my_list)").unwrap();
        let ci = page13[..i].chars().count() + 2;
        assert_eq!(word_around(&text[13], ci), (ci - 2, ci + 6));
        let copied = selected_text(&text, (13, ci - 2), (13, ci + 15));
        assert_eq!(copied, "deepcopy(my_list)");
    }

    /// Drives the real panel through egui frames: background loading,
    /// rendering, search and pinch zoom must all work without a window.
    #[test]
    fn panel_end_to_end_if_present() {
        let Some(path) = std::env::var_os("EDUPYTHON_TEST_BOOKLET").map(PathBuf::from) else { return };
        let ctx = egui::Context::default();
        let mut b = Booklet::new(path, "Booklet".into(), 13);
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 800.0));
        let mut frame = |b: &mut Booklet, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| b.show(ui));
            out.textures_delta.clear(); // no GPU in tests
        };
        let wait = |b: &mut Booklet, frame: &mut dyn FnMut(&mut Booklet, Vec<egui::Event>), done: &dyn Fn(&Booklet) -> bool| {
            for _ in 0..500 {
                frame(b, vec![]);
                if done(b) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("timed out");
        };
        wait(&mut b, &mut frame, &|b| b.page_count() == 26 && b.text.is_some() && !b.textures.is_empty());
        // restored page from the settings
        assert_eq!(b.page, 13, "should open at the saved page");
        let at = |b: &Booklet| b.textures.keys().copied().collect::<Vec<_>>();
        assert!(at(&b).contains(&13), "{:?}", at(&b));

        // search jumps to the hit
        b.query = "MOUSEBUTTONDOWN".into();
        b.run_search(true);
        assert!(!b.hits.is_empty());
        let target = b.hits[b.current_hit].page;
        frame(&mut b, vec![]);
        frame(&mut b, vec![]);
        assert_eq!(b.page, target, "view should move to the page of the hit");

        // pinch zoom while the pointer is over the pages
        let z0 = b.zoom;
        frame(&mut b, vec![egui::Event::PointerMoved(pos2(300.0, 400.0))]);
        frame(&mut b, vec![egui::Event::PointerMoved(pos2(300.0, 400.0)), egui::Event::Zoom(1.5)]);
        assert!((b.zoom - z0 * 1.5).abs() < 0.01, "zoom {} -> {}", z0, b.zoom);
        let page_after_zoom = b.page;
        assert!(page_after_zoom.abs_diff(target) <= 1, "zoom keeps the place: {target} -> {page_after_zoom}");
        // sharper pages arrive in the background
        let w = |b: &Booklet| b.textures.get(&b.page).map(|t| t.0).unwrap_or(0);
        let before = w(&b);
        wait(&mut b, &mut frame, &|b| w(b) > before);
    }
}
