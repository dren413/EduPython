//! Windows printing: the standard print dialog (printer, copies, properties),
//! then the pages are drawn with GDI, the way Word and Notepad print.

use eframe::egui::Color32;
use windows_sys::Win32::Foundation::{COLORREF, GlobalFree};
use windows_sys::Win32::Graphics::Gdi::{
    CLIP_DEFAULT_PRECIS, CreateFontW, CreatePen, DEFAULT_CHARSET, DEFAULT_QUALITY, DeleteDC, DeleteObject, FF_MODERN, FF_SWISS,
    FIXED_PITCH, FW_BOLD, FW_NORMAL, GetDeviceCaps, HDC, HFONT, HGDIOBJ, LOGPIXELSX, LOGPIXELSY, LineTo, MoveToEx, OUT_DEFAULT_PRECIS,
    PHYSICALHEIGHT, PHYSICALOFFSETX, PHYSICALOFFSETY, PHYSICALWIDTH, PS_SOLID, SelectObject, SetBkMode, SetTextAlign, SetTextColor,
    TA_BASELINE, TA_LEFT, TA_RIGHT, TRANSPARENT, TextOutW, VARIABLE_PITCH,
};
use windows_sys::Win32::Storage::Xps::{DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, PD_NOPAGENUMS, PD_NOSELECTION, PD_RETURNDC, PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGW, PrintDlgW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;

use super::{Canvas, Font, PrintFile, PrintInfo, draw_page, paginate};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn colorref(c: Color32) -> COLORREF {
    c.r() as u32 | (c.g() as u32) << 8 | (c.b() as u32) << 16
}

/// Draws on a printer device context. Points are converted to printer pixels.
struct GdiCanvas {
    hdc: HDC,
    /// Printer pixels per point, horizontally and vertically.
    sx: f32,
    sy: f32,
    /// The printable area starts this many pixels from the paper edge.
    off_x: f32,
    off_y: f32,
    fonts: [HFONT; 4],
}

impl GdiCanvas {
    fn px(&self, x: f32, y: f32) -> (i32, i32) {
        ((x * self.sx - self.off_x).round() as i32, (y * self.sy - self.off_y).round() as i32)
    }

    fn font(&self, f: Font) -> HFONT {
        self.fonts[match f {
            Font::Code => 0,
            Font::CodeItalic => 1,
            Font::HeadBold => 2,
            Font::Head => 3,
        }]
    }

    fn draw_text(&mut self, x: f32, y: f32, font: Font, color: Color32, s: &str, align: u32) {
        let (px, py) = self.px(x, y);
        let text: Vec<u16> = s.encode_utf16().collect();
        // SAFETY: valid DC and font handles owned by this canvas.
        unsafe {
            SelectObject(self.hdc, self.font(font) as HGDIOBJ);
            SetTextColor(self.hdc, colorref(color));
            SetTextAlign(self.hdc, TA_BASELINE | align);
            TextOutW(self.hdc, px, py, text.as_ptr(), text.len() as i32);
        }
    }
}

impl Canvas for GdiCanvas {
    fn text(&mut self, x: f32, y: f32, font: Font, color: Color32, s: &str) {
        self.draw_text(x, y, font, color, s, TA_LEFT);
    }

    fn text_right(&mut self, right: f32, y: f32, font: Font, color: Color32, s: &str) {
        self.draw_text(right, y, font, color, s, TA_RIGHT);
    }

    fn hline(&mut self, x1: f32, x2: f32, y: f32, color: Color32) {
        let (a, py) = self.px(x1, y);
        let (b, _) = self.px(x2, y);
        let width = (0.6 * self.sy).round().max(1.0) as i32;
        // SAFETY: valid DC; the pen is selected out again and deleted.
        unsafe {
            let pen = CreatePen(PS_SOLID, width, colorref(color));
            let old = SelectObject(self.hdc, pen as HGDIOBJ);
            MoveToEx(self.hdc, a, py, std::ptr::null_mut());
            LineTo(self.hdc, b, py);
            SelectObject(self.hdc, old);
            DeleteObject(pen as HGDIOBJ);
        }
    }
}

/// Create a font of `size_pt` points for a printer with `dpi_y` pixels per inch.
unsafe fn make_font(dpi_y: i32, size_pt: f32, face: &str, bold: bool, italic: bool, pitch: u8) -> HFONT {
    let height = -((size_pt * dpi_y as f32 / 72.0).round() as i32);
    let face = wide(face);
    // SAFETY: plain GDI call; the face name is NUL-terminated.
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            if bold { FW_BOLD } else { FW_NORMAL } as i32,
            italic as u32,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            DEFAULT_QUALITY as u32,
            pitch as u32,
            face.as_ptr(),
        )
    }
}

pub fn print(files: &[PrintFile], info: &PrintInfo, title: &str) -> Result<bool, String> {
    // SAFETY: standard Win32 print dialog + GDI printing; every handle that
    // is created here is released before returning.
    unsafe {
        let mut pd: PRINTDLGW = std::mem::zeroed();
        pd.lStructSize = std::mem::size_of::<PRINTDLGW>() as u32;
        pd.hwndOwner = GetActiveWindow();
        pd.Flags = PD_RETURNDC | PD_NOSELECTION | PD_NOPAGENUMS | PD_USEDEVMODECOPIESANDCOLLATE;
        pd.nCopies = 1;
        let ok = PrintDlgW(&mut pd);
        let (hdc, dev_mode, dev_names, copies) = (pd.hDC, pd.hDevMode, pd.hDevNames, pd.nCopies);
        let free = || {
            if !dev_mode.is_null() {
                GlobalFree(dev_mode);
            }
            if !dev_names.is_null() {
                GlobalFree(dev_names);
            }
        };
        if ok == 0 {
            free();
            let err = CommDlgExtendedError();
            return if err == 0 { Ok(false) } else { Err(format!("The print dialog could not be opened (error {err}).")) };
        }
        if hdc.is_null() {
            free();
            return Err("No printer was chosen.".into());
        }

        let dpi_x = GetDeviceCaps(hdc, LOGPIXELSX as i32).max(1);
        let dpi_y = GetDeviceCaps(hdc, LOGPIXELSY as i32).max(1);
        let sx = dpi_x as f32 / 72.0;
        let sy = dpi_y as f32 / 72.0;
        let page_w = GetDeviceCaps(hdc, PHYSICALWIDTH as i32) as f32 / sx;
        let page_h = GetDeviceCaps(hdc, PHYSICALHEIGHT as i32) as f32 / sy;
        let fonts = [
            make_font(dpi_y, Font::Code.size(), "Courier New", false, false, FIXED_PITCH | FF_MODERN),
            make_font(dpi_y, Font::CodeItalic.size(), "Courier New", false, true, FIXED_PITCH | FF_MODERN),
            make_font(dpi_y, Font::HeadBold.size(), "Arial", true, false, VARIABLE_PITCH | FF_SWISS),
            make_font(dpi_y, Font::Head.size(), "Arial", false, false, VARIABLE_PITCH | FF_SWISS),
        ];
        let mut canvas = GdiCanvas {
            hdc,
            sx,
            sy,
            off_x: GetDeviceCaps(hdc, PHYSICALOFFSETX as i32) as f32,
            off_y: GetDeviceCaps(hdc, PHYSICALOFFSETY as i32) as f32,
            fonts,
        };
        SetBkMode(hdc, TRANSPARENT as i32);

        let pages = paginate(files, if page_h > 100.0 { page_h } else { super::A4_H });
        let doc_name = wide(title);
        let doc = DOCINFOW { cbSize: std::mem::size_of::<DOCINFOW>() as i32, lpszDocName: doc_name.as_ptr(), ..Default::default() };
        let mut result = Ok(true);
        if StartDocW(hdc, &doc) <= 0 {
            result = Err("The printer did not accept the document.".into());
        } else {
            // If the driver cannot make copies itself, nCopies tells us how many to print.
            for _ in 0..copies.max(1) {
                for (i, page) in pages.iter().enumerate() {
                    if StartPage(hdc) <= 0 {
                        result = Err("Printing stopped.".into());
                        break;
                    }
                    SetBkMode(hdc, TRANSPARENT as i32);
                    draw_page(&mut canvas, if page_w > 100.0 { page_w } else { super::A4_W }, page, files, info, i + 1, pages.len());
                    EndPage(hdc);
                }
            }
            EndDoc(hdc);
        }
        for f in fonts {
            DeleteObject(f as HGDIOBJ);
        }
        DeleteDC(hdc);
        free();
        result
    }
}
