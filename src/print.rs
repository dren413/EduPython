//! Print the code, or export it as a PDF (with syntax colors and line numbers).
//!
//! Every page has a header with the **full path of the file** and the login
//! name, so a teacher correcting a pile of printouts knows whose code it is.
//! Each file starts on a new page.
//!
//! The page is drawn once ([`draw_page`]) onto a [`Canvas`]:
//! - PDF export: standard PDF fonts (Courier, Helvetica), nothing embedded.
//! - Printing on Windows: the native print dialog, then GDI (like Word does).
//! - Printing on macOS: the native print dialog (PDFKit prints the PDF).
//! - Printing on Linux: the PDF opens in the desktop's PDF viewer.

use std::path::PathBuf;

use eframe::egui::Color32;
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str, TextStr};

use crate::highlight;

#[cfg(windows)]
#[path = "print_windows.rs"]
mod windows;

/// One file to print: its full path (shown in the header) and its text.
pub struct PrintFile {
    pub path: PathBuf,
    pub text: String,
}

/// Header details that are the same on every page.
pub struct PrintInfo {
    /// Login name (on school PCs: the pupil's IAM id).
    pub user: String,
    /// When it was printed, e.g. "19.11.2026 15:20".
    pub date: String,
}

impl PrintInfo {
    pub fn now() -> Self {
        let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
        Self { user, date: chrono::Local::now().format("%d.%m.%Y %H:%M").to_string() }
    }
}

// Layout in points (1/72 inch), measured from the top-left corner of the paper.
const A4_W: f32 = 595.0;
const A4_H: f32 = 842.0;
const MARGIN_X: f32 = 42.0;
const MARGIN_TOP: f32 = 40.0;
const MARGIN_BOTTOM: f32 = 40.0;
const HEADER_H: f32 = 40.0;
const CODE_SIZE: f32 = 9.0;
const LINE_H: f32 = 11.2;
/// Courier / Courier New: every character is 0.6 em wide.
const CHAR_W: f32 = CODE_SIZE * 0.6;
/// Columns for the line number + one space.
const NUM_COLS: usize = 5;
const TAB: &str = "    ";

/// Characters per printed line (same on A4 and Letter, which is wider).
fn cols() -> usize {
    ((A4_W - 2.0 * MARGIN_X) / CHAR_W) as usize - NUM_COLS
}

/// Code lines that fit on a page of height `page_h` points.
fn rows_per_page(page_h: f32) -> usize {
    (((page_h - MARGIN_TOP - HEADER_H - MARGIN_BOTTOM) / LINE_H) as usize).max(10)
}

/// The fonts used on a page; each has a fixed size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Font {
    /// Courier 9 pt.
    Code,
    /// Courier oblique 9 pt (comments).
    CodeItalic,
    /// Helvetica bold 10.5 pt (the file path).
    HeadBold,
    /// Helvetica 8.5 pt (name, date, page number).
    Head,
}

impl Font {
    fn size(self) -> f32 {
        match self {
            Font::Code | Font::CodeItalic => CODE_SIZE,
            Font::HeadBold => 10.5,
            Font::Head => 8.5,
        }
    }
}

/// Something a page can be drawn on. Coordinates are points from the
/// top-left of the paper; `y` is the text baseline.
trait Canvas {
    fn text(&mut self, x: f32, y: f32, font: Font, color: Color32, s: &str);
    fn text_right(&mut self, right: f32, y: f32, font: Font, color: Color32, s: &str);
    fn hline(&mut self, x1: f32, x2: f32, y: f32, color: Color32);
}

/// A colored run of text on one printed line.
#[derive(Clone, Debug, PartialEq)]
struct Run {
    text: String,
    color: Color32,
    italic: bool,
}

/// One printed line: the source line number (only on its first part) and runs.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    number: Option<usize>,
    runs: Vec<Run>,
}

/// One printed page: which file it belongs to and its rows.
struct Page {
    file: usize,
    rows: Vec<Row>,
}

/// Split a file into printed rows: tabs expanded, long lines wrapped at `cols`.
fn layout(text: &str, cols: usize) -> Vec<Row> {
    let spans = highlight::print_spans(text);
    let mut style = vec![(Color32::BLACK, false); text.len()];
    for (range, color, italic) in spans {
        for s in &mut style[range] {
            *s = (color, italic);
        }
    }
    let mut rows = Vec::new();
    let mut offset = 0;
    for (n, line) in text.split('\n').enumerate() {
        let line_start = offset;
        offset += line.len() + 1;
        let line = line.strip_suffix('\r').unwrap_or(line);
        if n > 0 && line.is_empty() && line_start >= text.len() {
            break; // the empty piece after a final newline
        }
        let mut row = Row { number: Some(n + 1), runs: Vec::new() };
        let mut width = 0;
        for (i, c) in line.char_indices() {
            let (color, italic) = style[line_start + i];
            let piece: &str = if c == '\t' { TAB } else { &line[i..i + c.len_utf8()] };
            for pc in piece.chars() {
                if width == cols {
                    rows.push(std::mem::replace(&mut row, Row { number: None, runs: Vec::new() }));
                    width = 0;
                }
                match row.runs.last_mut() {
                    Some(r) if r.color == color && r.italic == italic => r.text.push(pc),
                    _ => row.runs.push(Run { text: pc.to_string(), color, italic }),
                }
                width += 1;
            }
        }
        rows.push(row);
    }
    rows
}

/// Lay out all files into pages; each file starts on a new page.
fn paginate(files: &[PrintFile], page_h: f32) -> Vec<Page> {
    let per_page = rows_per_page(page_h);
    let mut pages = Vec::new();
    for (fi, f) in files.iter().enumerate() {
        let rows = layout(&f.text, cols());
        if rows.is_empty() {
            pages.push(Page { file: fi, rows: Vec::new() });
        }
        for chunk in rows.chunks(per_page) {
            pages.push(Page { file: fi, rows: chunk.to_vec() });
        }
    }
    pages
}

/// Shorten a path from the left so it fits `max` characters (keeps the file name).
fn fit_path(path: &str, max: usize) -> String {
    let n = path.chars().count();
    if n <= max {
        return path.to_string();
    }
    let tail: String = path.chars().skip(n - (max - 1)).collect();
    format!("…{tail}")
}

/// Draw one page: header (path, user, date, page number) and the code.
fn draw_page(c: &mut impl Canvas, page_w: f32, page: &Page, files: &[PrintFile], info: &PrintInfo, number: usize, total: usize) {
    let black = Color32::BLACK;
    let gray = Color32::from_gray(90);
    let light = Color32::from_gray(150);
    let right = page_w - MARGIN_X;
    let top = MARGIN_TOP;
    let path = files.get(page.file).map(|f| f.path.display().to_string()).unwrap_or_default();
    // Helvetica-Bold 10.5 pt: about 0.58 em per character on average.
    let max_chars = ((right - MARGIN_X) / (Font::HeadBold.size() * 0.58)) as usize;
    c.text(MARGIN_X, top + 10.5, Font::HeadBold, black, &fit_path(&path, max_chars));
    let meta = if info.user.is_empty() { info.date.clone() } else { format!("{}  ·  {}", info.user, info.date) };
    c.text(MARGIN_X, top + 24.0, Font::Head, gray, &meta);
    c.text_right(right, top + 24.0, Font::Head, gray, &format!("Page {number} / {total}"));
    c.hline(MARGIN_X, right, top + 30.0, light);

    let code_x = MARGIN_X + NUM_COLS as f32 * CHAR_W;
    let mut y = top + HEADER_H + CODE_SIZE;
    for row in &page.rows {
        if let Some(n) = row.number {
            c.text(MARGIN_X, y, Font::Code, light, &format!("{n:>width$}", width = NUM_COLS - 1));
        }
        let mut col = 0;
        for run in &row.runs {
            let font = if run.italic { Font::CodeItalic } else { Font::Code };
            c.text(code_x + col as f32 * CHAR_W, y, font, run.color, &run.text);
            col += run.text.chars().count();
        }
        y += LINE_H;
    }
}

// ── PDF ──────────────────────────────────────────────────────────────────

/// Encode text for the standard PDF fonts (WinAnsiEncoding). Characters
/// that are not in it (emoji, Greek, …) become `?`.
fn win_ansi(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c {
            ' '..='~' => c as u8,
            '\u{a0}'..='\u{ff}' => c as u32 as u8,
            '€' => 0x80,
            '‚' => 0x82,
            '„' => 0x84,
            '…' => 0x85,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            'Œ' => 0x8C,
            'œ' => 0x9C,
            _ => b'?',
        })
        .collect()
}

fn rgb(c: Color32) -> (f32, f32, f32) {
    (c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0)
}

fn pdf_font_name(f: Font) -> Name<'static> {
    match f {
        Font::Code => Name(b"F1"),
        Font::CodeItalic => Name(b"F2"),
        Font::HeadBold => Name(b"F3"),
        Font::Head => Name(b"F4"),
    }
}

struct PdfCanvas {
    content: Content,
    page_h: f32,
}

impl Canvas for PdfCanvas {
    fn text(&mut self, x: f32, y: f32, font: Font, color: Color32, s: &str) {
        let (r, g, b) = rgb(color);
        self.content.set_fill_rgb(r, g, b);
        self.content.begin_text().set_font(pdf_font_name(font), font.size()).next_line(x, self.page_h - y);
        self.content.show(Str(&win_ansi(s)));
        self.content.end_text();
    }

    fn text_right(&mut self, right: f32, y: f32, font: Font, color: Color32, s: &str) {
        // Helvetica digits and most letters are about 0.53 em wide.
        let w = s.chars().count() as f32 * font.size() * 0.53;
        self.text(right - w, y, font, color, s);
    }

    fn hline(&mut self, x1: f32, x2: f32, y: f32, color: Color32) {
        let (r, g, b) = rgb(color);
        self.content.set_stroke_rgb(r, g, b).set_line_width(0.6);
        self.content.move_to(x1, self.page_h - y).line_to(x2, self.page_h - y).stroke();
    }
}

/// Build an A4 PDF for `files` (each starts on a new page).
pub fn make_pdf(files: &[PrintFile], info: &PrintInfo, title: &str) -> Vec<u8> {
    let mut pages = paginate(files, A4_H);
    if pages.is_empty() {
        pages.push(Page { file: usize::MAX, rows: Vec::new() });
    }

    let mut pdf = Pdf::new();
    let catalog = Ref::new(1);
    let tree = Ref::new(2);
    let fonts = [Ref::new(3), Ref::new(4), Ref::new(5), Ref::new(6)];
    let info_id = Ref::new(7);
    let page_ids: Vec<Ref> = (0..pages.len()).map(|i| Ref::new(8 + 2 * i as i32)).collect();

    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids(page_ids.iter().copied()).count(pages.len() as i32);
    for (id, base) in fonts.iter().zip([&b"Courier"[..], b"Courier-Oblique", b"Helvetica-Bold", b"Helvetica"]) {
        pdf.type1_font(*id).base_font(Name(base)).encoding_predefined(Name(b"WinAnsiEncoding"));
    }
    pdf.document_info(info_id).title(TextStr(title)).author(TextStr(&info.user)).creator(TextStr("EduPython"));

    let total = pages.len();
    for (pi, page) in pages.iter().enumerate() {
        let page_id = page_ids[pi];
        let content_id = Ref::new(page_id.get() + 1);
        let mut p = pdf.page(page_id);
        p.media_box(Rect::new(0.0, 0.0, A4_W, A4_H));
        p.parent(tree);
        p.contents(content_id);
        {
            let mut resources = p.resources();
            let mut font_dict = resources.fonts();
            for (f, id) in [Font::Code, Font::CodeItalic, Font::HeadBold, Font::Head].into_iter().zip(fonts) {
                font_dict.pair(pdf_font_name(f), id);
            }
        }
        p.finish();

        let mut canvas = PdfCanvas { content: Content::new(), page_h: A4_H };
        draw_page(&mut canvas, A4_W, page, files, info, pi + 1, total);
        pdf.stream(content_id, &canvas.content.finish());
    }
    pdf.finish()
}

// ── printing ─────────────────────────────────────────────────────────────

/// Show the system's print dialog (choose printer, copies, …) and print.
/// Returns `Ok(false)` if the pupil cancelled.
pub fn print(files: &[PrintFile], info: &PrintInfo, title: &str) -> Result<bool, String> {
    #[cfg(windows)]
    {
        windows::print(files, info, title)
    }
    #[cfg(target_os = "macos")]
    {
        print_macos(&make_pdf(files, info, title), title)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        print_linux(&make_pdf(files, info, title), title)
    }
}

/// Linux: there is no print dialog without GTK, so open the PDF in the
/// desktop's PDF viewer (Evince, Okular, …), which prints with Ctrl+P.
#[cfg(not(any(windows, target_os = "macos")))]
fn print_linux(pdf: &[u8], title: &str) -> Result<bool, String> {
    let dir = std::env::temp_dir().join("EduPython");
    // A new file each time: the viewer may still have the last one open.
    let path = dir.join(format!("{title}_{}.pdf", chrono::Local::now().format("%H%M%S")));
    std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, pdf)).map_err(|e| format!("Could not create the PDF: {e}"))?;
    std::process::Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map(|_| true)
        .map_err(|e| format!("Could not open the PDF viewer ({e}). Use File > Export as PDF instead."))
}

/// macOS: let PDFKit print our PDF with the standard print panel.
#[cfg(target_os = "macos")]
fn print_macos(pdf: &[u8], title: &str) -> Result<bool, String> {
    use objc2::AnyThread;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSPrintInfo;
    use objc2_foundation::{NSData, NSString};
    use objc2_pdf_kit::{PDFDocument, PDFPrintScalingMode};

    let mtm = MainThreadMarker::new().ok_or("Printing must start from the main window.")?;
    let data = NSData::with_bytes(pdf);
    // SAFETY: `data` is a valid PDF that we just created.
    let doc = unsafe { PDFDocument::initWithData(PDFDocument::alloc(), &data) }.ok_or("Could not prepare the pages.")?;
    let print_info = NSPrintInfo::sharedPrintInfo();
    // SAFETY: called on the main thread with a valid print info.
    let op = unsafe { doc.printOperationForPrintInfo_scalingMode_autoRotate(Some(&print_info), PDFPrintScalingMode::PageScaleDownToFit, true, mtm) }
        .ok_or("Could not start printing.")?;
    op.setJobTitle(Some(&NSString::from_str(title)));
    op.setShowsPrintPanel(true);
    op.setShowsProgressPanel(true);
    Ok(op.runOperation())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(rows: &[Row]) -> Vec<(Option<usize>, String)> {
        rows.iter().map(|r| (r.number, r.runs.iter().map(|x| x.text.as_str()).collect())).collect()
    }

    #[test]
    fn wraps_and_numbers_lines() {
        let rows = layout("def f():\n\treturn 12345678\n\n", 10);
        assert_eq!(
            plain(&rows),
            [
                (Some(1), "def f():".into()),
                (Some(2), "    return".into()),
                (None, " 12345678".into()),
                (Some(3), "".into()),
            ]
        );
        assert!(rows[0].runs.len() > 1, "keyword and name have their own colors");
    }

    #[test]
    fn encodes_accents() {
        assert_eq!(win_ansi("é€😀"), [0xE9, 0x80, b'?']);
        assert_eq!(fit_path("H:\\Documents\\Pong\\main.py", 12), "…ong\\main.py");
    }

    #[test]
    fn shorter_paper_gets_more_pages() {
        let text: String = (0..130).map(|i| format!("x = {i}\n")).collect();
        let files = [PrintFile { path: "a.py".into(), text }];
        assert_eq!(paginate(&files, A4_H).len(), 3);
        assert_eq!(paginate(&files, 792.0).len(), 3); // US Letter
        assert!(rows_per_page(792.0) < rows_per_page(A4_H));
    }

    #[test]
    fn pdf_has_header_on_every_page() {
        let long: String = (0..150).map(|i| format!("x = {i}\n")).collect();
        let files = [
            PrintFile { path: PathBuf::from("H:\\Documents\\Pong\\main.py"), text: long },
            PrintFile { path: PathBuf::from("H:\\Documents\\Pong\\ball.py"), text: "class Ball:\n    pass\n".into() },
        ];
        let info = PrintInfo { user: "iam123".into(), date: "19.11.2026 15:20".into() };
        let pdf = make_pdf(&files, &info, "Pong");
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.starts_with("%PDF-"));
        assert_eq!(s.matches("(H:\\\\Documents\\\\Pong\\\\main.py)").count(), 3, "header on each of the 3 pages of main.py");
        assert_eq!(s.matches("(H:\\\\Documents\\\\Pong\\\\ball.py)").count(), 1);
        assert!(s.contains("Page 4 / 4"));
        assert!(s.contains("iam123"));
    }
}
