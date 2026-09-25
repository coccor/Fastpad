//! The tree's icon bitmaps (icon sets spec §6): one top-down 32-bit DIB section per (Material
//! icon, pixel size) and per (mask icon, pixel size, colour), made on first draw, blended 1:1
//! with gdi32's `GdiAlphaBlend` (no msimg32 import).

use super::masks::{MaskIcon, MaskSet, coverage};
use super::material::{MaterialIcon, pixels};
use super::resample::{pick_size, resample};
use std::collections::HashMap;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiAlphaBlend,
    HBITMAP, HDC, SelectObject,
};

/// A mask bitmap's key: the set, the icon, its pixel size and the COLORREF it is tinted with.
type MaskKey = (MaskSet, MaskIcon, u32, u32);

pub(crate) struct IconImages {
    bitmaps: HashMap<(MaterialIcon, u32), HBITMAP>,
    /// Tinted masks. A theme switch adds a colour's bitmaps; at most 9 icons × 2 sets × the
    /// theme colours × the pixel sizes in use, so the cache is never trimmed.
    masks: HashMap<MaskKey, HBITMAP>,
    /// One memory DC for every blend, made on first use.
    memory: HDC,
}

impl IconImages {
    pub(crate) fn new() -> Self {
        Self {
            bitmaps: HashMap::new(),
            masks: HashMap::new(),
            memory: std::ptr::null_mut(),
        }
    }

    /// Blends `icon` at `px` square, centred in `rect`. `false` when a bitmap or the memory DC
    /// could not be made: the caller draws the Minimal icon instead, and the next paint tries
    /// again.
    pub(crate) fn draw(&mut self, dc: HDC, icon: MaterialIcon, rect: RECT, px: u32) -> bool {
        let Some(bitmap) = self.bitmap(icon, px) else {
            return false;
        };
        self.blend(dc, bitmap, rect, px)
    }

    /// Blends `icon` of `set` at `px` square in `color` (a COLORREF), centred in `rect`. `false`
    /// when a bitmap or the memory DC could not be made: the row then shows no icon, and the
    /// next paint tries again.
    pub(crate) fn draw_mask(
        &mut self,
        dc: HDC,
        set: MaskSet,
        icon: MaskIcon,
        color: u32,
        rect: RECT,
        px: u32,
    ) -> bool {
        let Some(bitmap) = self.mask(set, icon, color, px) else {
            return false;
        };
        self.blend(dc, bitmap, rect, px)
    }

    /// Blends the `px`-square `bitmap` centred in `rect`, drawing only the part inside `rect`:
    /// a deep row in a narrow panel clips its icon box rather than getting a smaller bitmap.
    fn blend(&mut self, dc: HDC, bitmap: HBITMAP, rect: RECT, px: u32) -> bool {
        if self.memory.is_null() {
            self.memory = unsafe { CreateCompatibleDC(dc) };
            if self.memory.is_null() {
                return false;
            }
        }
        let size = px as i32;
        let x = rect.left + (rect.right - rect.left - size) / 2;
        let y = rect.top + (rect.bottom - rect.top - size) / 2;
        let (left, top) = (x.max(rect.left), y.max(rect.top));
        let (right, bottom) = ((x + size).min(rect.right), (y + size).min(rect.bottom));
        if right <= left || bottom <= top {
            return true;
        }
        let (width, height) = (right - left, bottom - top);
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            let previous = SelectObject(self.memory, bitmap);
            let drawn = GdiAlphaBlend(
                dc,
                left,
                top,
                width,
                height,
                self.memory,
                left - x,
                top - y,
                width,
                height,
                blend,
            );
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
        let bitmap = upload(&scaled(source, stored, px), px)?;
        self.bitmaps.insert((icon, px), bitmap);
        Some(bitmap)
    }

    fn mask(&mut self, set: MaskSet, icon: MaskIcon, color: u32, px: u32) -> Option<HBITMAP> {
        let key = (set, icon, px, color);
        if let Some(&bitmap) = self.masks.get(&key) {
            return Some(bitmap);
        }
        let stored = pick_size(px);
        let source = tint(coverage(set, icon, stored)?, color);
        let bitmap = upload(&scaled(&source, stored, px), px)?;
        self.masks.insert(key, bitmap);
        Some(bitmap)
    }
}

/// `source` (premultiplied BGRA at `stored` px) at `px`.
fn scaled(source: &[u8], stored: u32, px: u32) -> std::borrow::Cow<'_, [u8]> {
    if stored == px {
        source.into()
    } else {
        resample(source, stored, px).into()
    }
}

/// `mask` coloured `color` (a COLORREF, 0x00BBGGRR): premultiplied BGRA.
fn tint(mask: &[u8], color: u32) -> Vec<u8> {
    let [red, green, blue, _] = color.to_le_bytes();
    let times =
        |channel: u8, alpha: u8| ((u32::from(channel) * u32::from(alpha) + 127) / 255) as u8;
    mask.iter()
        .flat_map(|&alpha| {
            [
                times(blue, alpha),
                times(green, alpha),
                times(red, alpha),
                alpha,
            ]
        })
        .collect()
}

/// A `px`-square DIB section holding `data` (premultiplied BGRA, top-down).
fn upload(data: &[u8], px: u32) -> Option<HBITMAP> {
    let (bitmap, bits) = dib_section(px as i32, px as i32);
    if bitmap.is_null() || bits.is_null() {
        return None;
    }
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), bits.cast::<u8>(), data.len()) };
    Some(bitmap)
}

#[cfg(test)]
impl IconImages {
    /// The pixel sizes of every bitmap cached so far, for tests that check a clipped icon box
    /// never asks for (and caches) an oddly, narrowly sized bitmap.
    pub(crate) fn cached_pixel_sizes(&self) -> Vec<u32> {
        let masks = self.masks.keys().map(|&(_, _, px, _)| px);
        self.bitmaps
            .keys()
            .map(|&(_, px)| px)
            .chain(masks)
            .collect()
    }
}

impl Drop for IconImages {
    fn drop(&mut self) {
        for bitmap in self.bitmaps.values().chain(self.masks.values()) {
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
    fn a_mask_blends_in_its_colour_and_a_clipped_box_draws_only_its_part() {
        // Break caught: a mask tinted in the wrong channel order (a blue icon drawn red), one
        // blended as straight alpha (dark fringes), a colour change reusing the old colour's
        // bitmap, or a clipped box painting past its edge (spec §6).
        const WHITE: u32 = 0x00FF_FFFF;
        const BLUE: u32 = 0x00FF_0000; // a COLORREF: 0x00BBGGRR
        let mut target = TestTarget::new(64, 64);
        target.fill(WHITE);
        let mut images = IconImages::new();
        let rect = RECT {
            left: 0,
            top: 0,
            right: 64,
            bottom: 64,
        };
        let (set, icon) = (MaskSet::Solid, MaskIcon::Folder);
        assert!(images.draw_mask(target.dc, set, icon, BLUE, rect, 32));
        let area = target.area(rect);
        assert!(
            area.contains(&0x0000_00FF),
            "fully covered pixels are pure blue"
        );
        assert!(area.iter().all(|&pixel| {
            let (red, green, blue) = (pixel >> 16, (pixel >> 8) & 0xFF, pixel & 0xFF);
            blue == 0xFF && red == green
        }));
        assert!(images.draw_mask(target.dc, set, icon, 0x0000_00FF, rect, 32));
        assert!(images.draw_mask(target.dc, set, icon, BLUE, rect, 32));
        assert_eq!(images.masks.len(), 2, "one bitmap per colour, then reused");

        target.fill(WHITE);
        let narrow = RECT {
            left: 20,
            top: 0,
            right: 26,
            bottom: 64,
        };
        assert!(images.draw_mask(target.dc, set, icon, BLUE, narrow, 32));
        for y in 0..64 {
            for x in (0..20).chain(26..64) {
                assert_eq!(target.pixel(x, y), WHITE, "({x}, {y}) is outside the box");
            }
        }
        assert!(target.area(narrow).iter().any(|&pixel| pixel != WHITE));
        assert_eq!(images.cached_pixel_sizes(), vec![32, 32]);
    }

    #[test]
    #[ignore = "timing; run at the final review: cargo test --lib mask_icons_paint -- --ignored"]
    fn mask_icons_paint_no_slower_than_material_bitmaps() {
        // Break caught: a per-draw tint, DC or bitmap creation that makes the Minimal or Solid
        // tree paint slower than Material's (icon sets spec §7).
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
        let material = |images: &mut IconImages, i: i32| {
            images.draw(target.dc, MaterialIcon::ALL[(i % 12) as usize], band(i), 16);
        };
        let mask = |images: &mut IconImages, i: i32| {
            let icon = MaskIcon::ALL[(i % 9) as usize];
            images.draw_mask(target.dc, MaskSet::Minimal, icon, 0x00FF_8800, band(i), 16);
        };
        let median = |images: &mut IconImages, draw: &dyn Fn(&mut IconImages, i32)| {
            // The first pass warms the memory DC and the cached bitmaps.
            for i in 0..BANDS {
                draw(images, i);
            }
            let mut runs: Vec<u64> = (0..RUNS)
                .map(|_| {
                    let started = std::time::Instant::now();
                    for i in 0..BANDS {
                        draw(images, i);
                    }
                    started.elapsed().as_micros() as u64
                })
                .collect();
            runs.sort_unstable();
            runs[RUNS / 2]
        };
        let material_median = median(&mut images, &material);
        let mask_median = median(&mut images, &mask);
        println!(
            "material paint median {material_median} us, mask paint median {mask_median} us,              over {BANDS} draws"
        );
        assert!(
            (mask_median as f64) <= (material_median as f64) * 1.10,
            "mask {mask_median} us > material {material_median} us * 1.10"
        );
    }
}
