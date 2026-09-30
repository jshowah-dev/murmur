use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, AC_SRC_ALPHA,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP,
};
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

    /// Pack as premultiplied BGRA (0xAARRGGBB little-endian), what UpdateLayeredWindow expects.
    pub(crate) fn into_bgra(self) -> Vec<u32> {
        let q = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u32;
        self.px.into_iter().map(|[r, g, b, a]| (q(a) << 24) | (q(r) << 16) | (q(g) << 8) | q(b)).collect()
    }
}

/// Push premultiplied BGRA `pixels` (`w * h`) to the layered window `hwnd` at (x, y), through a
/// 32-bit DIB and UpdateLayeredWindow.
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
}
