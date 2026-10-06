// The Mac pill and mote land in phase 3; until then only Windows draws with the canvas.
#![cfg_attr(target_os = "macos", allow(dead_code))]

#[cfg(windows)]
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
#[cfg(windows)]
use windows::core::w;
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, GdiFlush, GetDC, GetTextExtentPoint32W,
    ReleaseDC, SelectObject, SetBkMode, SetTextColor, TextOutW, ANTIALIASED_QUALITY, AC_SRC_ALPHA, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, FW_NORMAL,
    HBITMAP, HDC, OUT_DEFAULT_PRECIS, TRANSPARENT,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

/// Premultiplied RGBA canvas; shapes are drawn with per-pixel coverage so edges are antialiased.
pub(crate) struct Canvas {
    w: i32,
    h: i32,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    pub(crate) fn new(w: i32, h: i32) -> Self {
        Canvas { w, h, px: vec![[0.0; 4]; (w * h) as usize] }
    }

    /// Composite a capsule (rounded rect, radius = half the short side) of 0xRRGGBB at `alpha` over the canvas.
    pub(crate) fn capsule(&mut self, x: f32, y: f32, cw: f32, ch: f32, rgb: u32, alpha: f32) {
        let r = cw.min(ch) / 2.0;
        let (cx, cy) = (x + cw / 2.0, y + ch / 2.0);
        let (bx, by) = (cw / 2.0 - r, ch / 2.0 - r);
        let col = [(rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF].map(|c| c as f32 / 255.0);
        for py in (y.floor() as i32).max(0)..((y + ch).ceil() as i32).min(self.h) {
            for pxl in (x.floor() as i32).max(0)..((x + cw).ceil() as i32).min(self.w) {
                // signed distance from the pixel centre to the capsule edge
                let qx = (pxl as f32 + 0.5 - cx).abs() - bx;
                let qy = (py as f32 + 0.5 - cy).abs() - by;
                let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
                let a = (0.5 - d).clamp(0.0, 1.0) * alpha;
                if a > 0.0 {
                    let dst = &mut self.px[(py * self.w + pxl) as usize];
                    for i in 0..3 {
                        dst[i] = col[i] * a + dst[i] * (1.0 - a);
                    }
                    dst[3] = a + dst[3] * (1.0 - a);
                }
            }
        }
    }

    /// A soft round light at (cx, cy) fading to nothing at `r`, laid only over what's already
    /// drawn, so it never spills past the shape.
    pub(crate) fn glow(&mut self, cx: f32, cy: f32, r: f32, rgb: u32, alpha: f32) {
        let col = [(rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF].map(|c| c as f32 / 255.0);
        for py in 0..self.h {
            for pxl in 0..self.w {
                let d = (pxl as f32 + 0.5 - cx).hypot(py as f32 + 0.5 - cy);
                if d >= r {
                    continue;
                }
                let dst = &mut self.px[(py * self.w + pxl) as usize];
                let a = alpha * (1.0 - d / r).powi(2) * dst[3];
                for i in 0..3 {
                    dst[i] = col[i] * a + dst[i] * (1.0 - a);
                }
                dst[3] = a + dst[3] * (1.0 - a);
            }
        }
    }

    /// A soft round light at (cx, cy) fading to nothing at `r`, over transparent canvas too.
    pub(crate) fn halo(&mut self, cx: f32, cy: f32, r: f32, rgb: u32, alpha: f32) {
        let col = [(rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF].map(|c| c as f32 / 255.0);
        for py in 0..self.h {
            for pxl in 0..self.w {
                let d = (pxl as f32 + 0.5 - cx).hypot(py as f32 + 0.5 - cy);
                if d >= r {
                    continue;
                }
                let a = alpha * (1.0 - d / r).powi(2);
                let dst = &mut self.px[(py * self.w + pxl) as usize];
                for i in 0..3 {
                    dst[i] = col[i] * a + dst[i] * (1.0 - a);
                }
                dst[3] = a + dst[3] * (1.0 - a);
            }
        }
    }

    /// Draw `spans` left to right with the top-left at (x, y), each in its own colour at `alpha`.
    /// GDI draws white on black with grayscale antialiasing, so any channel is the coverage.
    /// `size` is what `measure` gave for these spans at `px`, so callers that already have it
    /// don't pay for it twice.
    #[cfg(windows)]
    pub(crate) fn text(&mut self, x: f32, y: f32, spans: &[Span], px: i32, size: (i32, i32), alpha: f32) {
        let (tw, th) = size;
        if tw <= 0 || th <= 0 || alpha <= 0.0 {
            return;
        }
        let (cov, runs) = with_font(px, |dc| unsafe {
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: tw,
                    biHeight: -th,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else { return (Vec::new(), Vec::new()) };
            if bits.is_null() {
                let _ = DeleteObject(bmp.into());
                return (Vec::new(), Vec::new());
            }
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (tw * th) as usize);
            pixels.fill(0);
            let old = SelectObject(dc, bmp.into());
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x00FF_FFFF));
            let mut at = 0;
            let mut runs = Vec::new();
            for (s, rgb) in spans {
                let wide: Vec<u16> = s.encode_utf16().collect();
                let _ = TextOutW(dc, at, 0, &wide);
                let next = at + extent(dc, &wide).cx;
                runs.push((at, next, *rgb));
                at = next;
            }
            let _ = GdiFlush();
            let cov = pixels.iter().map(|p| ((p >> 8) & 0xFF) as f32 / 255.0).collect::<Vec<_>>();
            let _ = SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
            (cov, runs)
        });
        let (ox, oy) = (x.round() as i32, y.round() as i32);
        for row in 0..th {
            for col in 0..tw {
                let a = cov.get((row * tw + col) as usize).copied().unwrap_or(0.0) * alpha;
                let (dx, dy) = (ox + col, oy + row);
                if a <= 0.0 || dx < 0 || dy < 0 || dx >= self.w || dy >= self.h {
                    continue;
                }
                let rgb = runs.iter().find(|(s, e, _)| col >= *s && col < *e).or(runs.last()).map_or(0xFFFFFF, |r| r.2);
                let c = [(rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF].map(|v| v as f32 / 255.0);
                let dst = &mut self.px[(dy * self.w + dx) as usize];
                for i in 0..3 {
                    dst[i] = c[i] * a + dst[i] * (1.0 - a);
                }
                dst[3] = a + dst[3] * (1.0 - a);
            }
        }
    }

    // TODO(macos phase 3): CoreText, for the mote's messages.
    #[cfg(target_os = "macos")]
    pub(crate) fn text(&mut self, _x: f32, _y: f32, _spans: &[Span], _px: i32, _size: (i32, i32), _alpha: f32) {}

    /// Pack as premultiplied BGRA (0xAARRGGBB little-endian), what UpdateLayeredWindow expects.
    pub(crate) fn into_bgra(self) -> Vec<u32> {
        let q = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u32;
        self.px.into_iter().map(|[r, g, b, a]| (q(a) << 24) | (q(r) << 16) | (q(g) << 8) | q(b)).collect()
    }
}

/// A run of text in one colour (0xRRGGBB).
pub(crate) type Span = (String, u32);

/// Runs `f` with a memory DC holding the UI font, `px` pixels to the em.
#[cfg(windows)]
fn with_font<T>(px: i32, f: impl FnOnce(HDC) -> T) -> T {
    unsafe {
        let dc = CreateCompatibleDC(None);
        let font = CreateFontW(
            -px, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY, DEFAULT_PITCH.0 as u32, w!("Segoe UI Variable Text"),
        );
        let old = SelectObject(dc, font.into());
        let r = f(dc);
        let _ = SelectObject(dc, old);
        let _ = DeleteObject(font.into());
        let _ = DeleteDC(dc);
        r
    }
}

#[cfg(windows)]
fn extent(dc: HDC, wide: &[u16]) -> SIZE {
    let mut sz = SIZE::default();
    let _ = unsafe { GetTextExtentPoint32W(dc, wide, &mut sz) };
    sz
}

/// Width and height of `spans` drawn side by side at `px`; (0, 0) when there's no text.
#[cfg(windows)]
pub(crate) fn measure(spans: &[Span], px: i32) -> (i32, i32) {
    if spans.iter().all(|(s, _)| s.is_empty()) {
        return (0, 0);
    }
    with_font(px, |dc| {
        spans.iter().fold((0, 0), |(w, h), (s, _)| {
            let sz = extent(dc, &s.encode_utf16().collect::<Vec<u16>>());
            (w + sz.cx, h.max(sz.cy))
        })
    })
}

// TODO(macos phase 3): CoreText, for the mote's messages.
#[cfg(target_os = "macos")]
pub(crate) fn measure(_spans: &[Span], _px: i32) -> (i32, i32) {
    (0, 0)
}

/// Push premultiplied BGRA `pixels` (`w * h`) to the layered window `hwnd` at (x, y), through a
/// 32-bit DIB and UpdateLayeredWindow.
#[cfg(windows)]
pub(crate) fn push(hwnd: HWND, x: i32, y: i32, w: i32, h: i32, pixels: &[u32]) {
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp: HBITMAP = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
        if bmp.is_invalid() || bits.is_null() {
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            return;
        }
        let old = SelectObject(mem, bmp.into());

        std::slice::from_raw_parts_mut(bits as *mut u32, pixels.len()).copy_from_slice(pixels);

        let blend = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        let _ = UpdateLayeredWindow(
            hwnd,
            Some(screen),
            Some(&POINT { x, y }),
            Some(&SIZE { cx: w, cy: h }),
            Some(mem),
            Some(&POINT { x: 0, y: 0 }),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );
        let _ = SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glow_only_lights_what_is_drawn() {
        let mut c = Canvas::new(20, 10);
        c.capsule(0.0, 0.0, 10.0, 10.0, 0x202020, 1.0);
        c.glow(5.0, 5.0, 15.0, 0x60D060, 1.0);
        let px = c.into_bgra();
        assert!((px[5 * 20 + 5] >> 8) & 0xFF > 0x80, "lit inside the capsule");
        assert_eq!(px[5 * 20 + 15], 0, "nothing where nothing was drawn");
    }

    #[cfg(windows)] // GDI text
    #[test]
    fn text_is_drawn_in_each_spans_colour() {
        let spans: Vec<Span> = vec![("Hob".into(), 0xFF0000), ("HAWB".into(), 0x0000FF)];
        let (w, h) = measure(&spans, 16);
        assert!(w > 20 && h >= 16, "{w}x{h}");
        let mut c = Canvas::new(w, h);
        c.text(0.0, 0.0, &spans, 16, (w, h), 1.0);
        let px = c.into_bgra();
        let cols = |r: std::ops::Range<i32>| px.iter().enumerate().filter(|(i, _)| r.contains(&(*i as i32 % w))).map(|(_, p)| *p).collect::<Vec<_>>();
        assert!(cols(0..w / 3).iter().any(|p| (p >> 16) & 0xFF > 0x80 && p & 0xFF == 0), "red on the left");
        assert!(cols(2 * w / 3..w).iter().any(|p| p & 0xFF > 0x80 && (p >> 16) & 0xFF == 0), "blue on the right");
        for p in px {
            let a = p >> 24;
            assert!((p >> 16) & 0xFF <= a && (p >> 8) & 0xFF <= a && p & 0xFF <= a, "not premultiplied: {p:08X}");
        }
    }

    #[cfg(windows)] // GDI text
    #[test]
    fn measure_handles_wide_characters_and_nothing() {
        assert!(measure(&[("日本".into(), 0xFFFFFF)], 16).0 > 10);
        assert_eq!(measure(&[], 16), (0, 0));
    }

    #[cfg(windows)] // GDI text
    #[test]
    fn text_respects_alpha() {
        let spans: Vec<Span> = vec![("Hi".into(), 0xFFFFFF)];
        let (w, h) = measure(&spans, 16);
        let draw = |a: f32| {
            let mut c = Canvas::new(w, h);
            c.text(0.0, 0.0, &spans, 16, (w, h), a);
            c.into_bgra().iter().map(|p| p >> 24).max().unwrap()
        };
        assert!(draw(0.5) < draw(1.0));
        assert_eq!(draw(0.0), 0);
    }
}
