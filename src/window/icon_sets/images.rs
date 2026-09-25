//! The tree's Material bitmaps (icon sets spec §6): one top-down 32-bit DIB section per (icon,
//! pixel size), made on first draw, blended 1:1 with gdi32's `GdiAlphaBlend` (no msimg32 import).

use super::material::{MaterialIcon, pixels};
use super::resample::{pick_size, resample};
use std::collections::HashMap;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiAlphaBlend,
    HBITMAP, HDC, SelectObject,
};

pub(crate) struct IconImages {
    bitmaps: HashMap<(MaterialIcon, u32), HBITMAP>,
    /// One memory DC for every blend, made on first use.
    memory: HDC,
}

impl IconImages {
    pub(crate) fn new() -> Self {
        Self {
            bitmaps: HashMap::new(),
            memory: std::ptr::null_mut(),
        }
    }

    /// Blends `icon` at `px` square, centred in `rect`. `false` when a bitmap or the memory DC
    /// could not be made: the caller draws the Minimal glyph instead, and the next paint tries
    /// again.
    pub(crate) fn draw(&mut self, dc: HDC, icon: MaterialIcon, rect: RECT, px: u32) -> bool {
        let Some(bitmap) = self.bitmap(icon, px) else {
            return false;
        };
        if self.memory.is_null() {
            self.memory = unsafe { CreateCompatibleDC(dc) };
            if self.memory.is_null() {
                return false;
            }
        }
        let size = px as i32;
        let x = rect.left + (rect.right - rect.left - size) / 2;
        let y = rect.top + (rect.bottom - rect.top - size) / 2;
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            let previous = SelectObject(self.memory, bitmap);
            let drawn = GdiAlphaBlend(dc, x, y, size, size, self.memory, 0, 0, size, size, blend);
            SelectObject(self.memory, previous);
            drawn != 0
        }
    }

    fn bitmap(&mut self, icon: MaterialIcon, px: u32) -> Option<HBITMAP> {
        if let Some(&bitmap) = self.bitmaps.get(&(icon, px)) {
            return Some(bitmap);
        }
        let stored = pick_size(px);
        let source = pixels(icon, stored)?;
        let scaled;
        let data = if stored == px {
            source
        } else {
            scaled = resample(source, stored, px);
            &scaled[..]
        };
        let (bitmap, bits) = dib_section(px as i32, px as i32);
        if bitmap.is_null() || bits.is_null() {
            return None;
        }
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), bits.cast::<u8>(), data.len()) };
        self.bitmaps.insert((icon, px), bitmap);
        Some(bitmap)
    }
}

#[cfg(test)]
impl IconImages {
    /// The pixel sizes of every bitmap cached so far, for tests that check a clipped icon box
    /// never asks for (and caches) an oddly, narrowly sized bitmap.
    pub(crate) fn cached_pixel_sizes(&self) -> Vec<u32> {
        self.bitmaps.keys().map(|&(_, px)| px).collect()
    }
}

impl Drop for IconImages {
    fn drop(&mut self) {
        for bitmap in self.bitmaps.values() {
            unsafe { DeleteObject(*bitmap) };
        }
        if !self.memory.is_null() {
            unsafe { DeleteDC(self.memory) };
        }
    }
}

/// A `width` × `height` top-down 32-bit DIB section and its bits (null on failure).
fn dib_section(width: i32, height: i32) -> (HBITMAP, *mut core::ffi::c_void) {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..unsafe { std::mem::zeroed() }
        },
        ..unsafe { std::mem::zeroed() }
    };
    let mut bits = std::ptr::null_mut();
    let bitmap = unsafe {
        CreateDIBSection(
            std::ptr::null_mut(),
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        )
    };
    if !bitmap.is_null() && bits.is_null() {
        // A bitmap handle with no bits is useless; free it rather than leaking it on every
        // caller that just checks `bits.is_null()` and returns `None`.
        unsafe { DeleteObject(bitmap) };
        return (std::ptr::null_mut(), std::ptr::null_mut());
    }
    (bitmap, bits)
}

/// A 32-bit DIB section selected into a memory DC, for pixel tests that read what was drawn.
#[cfg(test)]
pub(crate) struct TestTarget {
    pub(crate) dc: HDC,
    bitmap: HBITMAP,
    previous: windows_sys::Win32::Graphics::Gdi::HGDIOBJ,
    bits: *mut u32,
    width: i32,
    height: i32,
}

#[cfg(test)]
impl TestTarget {
    pub(crate) fn new(width: i32, height: i32) -> Self {
        let (bitmap, bits) = dib_section(width, height);
        assert!(!bitmap.is_null() && !bits.is_null());
        let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        assert!(!dc.is_null());
        let previous = unsafe { SelectObject(dc, bitmap) };
        Self {
            dc,
            bitmap,
            previous,
            bits: bits.cast(),
            width,
            height,
        }
    }

    /// Every pixel set to `color` (0x00RRGGBB, as the DIB stores it).
    pub(crate) fn fill(&mut self, color: u32) {
        self.pixels_mut().fill(color);
    }

    /// The pixel at (`x`, `y`) as 0x00RRGGBB, after GDI has finished drawing.
    pub(crate) fn pixel(&self, x: i32, y: i32) -> u32 {
        unsafe { windows_sys::Win32::Graphics::Gdi::GdiFlush() };
        assert!((0..self.width).contains(&x) && (0..self.height).contains(&y));
        self.pixels()[(y * self.width + x) as usize] & 0x00FF_FFFF
    }

    /// The pixels of `rect`, row by row.
    pub(crate) fn area(&self, rect: RECT) -> Vec<u32> {
        (rect.top..rect.bottom)
            .flat_map(|y| (rect.left..rect.right).map(move |x| (x, y)))
            .map(|(x, y)| self.pixel(x, y))
            .collect()
    }

    fn pixels(&self) -> &[u32] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.width * self.height) as usize) }
    }

    fn pixels_mut(&mut self) -> &mut [u32] {
        unsafe { windows_sys::Win32::Graphics::Gdi::GdiFlush() };
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize) }
    }
}

#[cfg(test)]
impl Drop for TestTarget {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteDC(self.dc);
            DeleteObject(self.bitmap);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_blends_centred_at_its_pixel_size_over_the_background() {
        // Break caught: straight alpha drawn as premultiplied (dark fringes), an icon pinned to the
        // box's corner, or a 28 px box drawn at 32 and clipped (spec §6).
        const WHITE: u32 = 0x00FF_FFFF;
        let mut target = TestTarget::new(64, 64);
        target.fill(WHITE);
        let mut images = IconImages::new();
        let rect = RECT {
            left: 0,
            top: 0,
            right: 64,
            bottom: 64,
        };
        assert!(images.draw(target.dc, MaterialIcon::Markdown, rect, 28));
        let inside = |v: i32| (18..46).contains(&v);
        let mut bluish = false;
        for y in 0..64 {
            for x in 0..64 {
                let pixel = target.pixel(x, y);
                if !(inside(x) && inside(y)) {
                    assert_eq!(
                        pixel, WHITE,
                        "({x}, {y}) is outside the centred 28 px square"
                    );
                } else if pixel != WHITE {
                    let (red, blue) = ((pixel >> 16) & 0xFF, pixel & 0xFF);
                    bluish |= blue > red;
                }
            }
        }
        assert!(
            bluish,
            "the markdown icon (#42a5f5) is drawn inside the square"
        );
        assert!(images.draw(target.dc, MaterialIcon::Markdown, rect, 28));
        assert_eq!(
            images.bitmaps.len(),
            1,
            "the second draw reuses the cached bitmap"
        );
    }

    #[test]
    #[ignore = "timing; run at the final review: cargo test --lib material_icons_paint -- --ignored"]
    fn material_icons_paint_no_slower_than_minimal_glyphs() {
        // Break caught: a per-draw DC or bitmap creation that makes the tree's paint slower than
        // the glyphs it replaces (icon sets spec §7).
        use crate::window::file_icons::file_icon;
        use crate::window::side_panel::draw_text;
        use crate::window::titlebar::create_ui_font;
        use windows_sys::Win32::Graphics::Gdi::{
            DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_NORMAL,
        };

        const BANDS: i32 = 1_000;
        const RUNS: usize = 20;

        let band = |i: i32| RECT {
            left: 0,
            top: i * 24,
            right: 24,
            bottom: (i + 1) * 24,
        };
        let target = TestTarget::new(24, BANDS * 24);

        let mut images = IconImages::new();
        // Warms the memory DC and all 12 cached bitmaps, so the measured runs pay only the blend.
        for i in 0..BANDS {
            images.draw(target.dc, MaterialIcon::ALL[(i % 12) as usize], band(i), 16);
        }
        let mut material_us: Vec<u64> = (0..RUNS)
            .map(|_| {
                let started = std::time::Instant::now();
                for i in 0..BANDS {
                    images.draw(target.dc, MaterialIcon::ALL[(i % 12) as usize], band(i), 16);
                }
                started.elapsed().as_micros() as u64
            })
            .collect();
        material_us.sort_unstable();
        let material_median = material_us[RUNS / 2];

        let font = create_ui_font(12, "Segoe MDL2 Assets", FW_NORMAL as i32, false);
        let text = file_icon(Some("md")).text;
        let flags = DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX;
        for i in 0..BANDS {
            unsafe { draw_text(target.dc, text, band(i), font, 0x00FF_8800, flags) };
        }
        let mut minimal_us: Vec<u64> = (0..RUNS)
            .map(|_| {
                let started = std::time::Instant::now();
                for i in 0..BANDS {
                    unsafe { draw_text(target.dc, text, band(i), font, 0x00FF_8800, flags) };
                }
                started.elapsed().as_micros() as u64
            })
            .collect();
        minimal_us.sort_unstable();
        let minimal_median = minimal_us[RUNS / 2];

        println!(
            "material paint median {material_median} us over {BANDS} draws, \
             minimal paint median {minimal_median} us over {BANDS} draws"
        );
        assert!(
            (material_median as f64) <= (minimal_median as f64) * 1.10,
            "material {material_median} us > minimal {minimal_median} us * 1.10"
        );
    }
}
