//! Soft, antialiased shapes for the owner-drawn Settings dialog, its dropdown list and the About
//! box, and the look they share: the tones, the corner radius, the focus ring and the title
//! row's ×. A paint
//! builds a `Frame`: shapes first, then text. The shapes go through Direct2D, bound to the
//! off-screen memory DC that `side_panel::paint_buffered` hands over; the text is drawn on top
//! with GDI afterwards, so it keeps ClearType. Direct2D is never held across a GDI call on the
//! same DC.
//!
//! Direct2D is loaded when a dialog opens, never at startup, through `preview::dwrite`'s
//! loader so d2d1.dll stays out of FastPad.exe's import table. Without it, or when a frame
//! fails, the shapes fall back to square GDI fills: every control still shows, only the corners
//! are square.

use super::palette::Palette;
use super::panel::fill;
use crate::platform::{OwnedModule, wide_null};
use crate::preview::dwrite::{create_d2d_factory, load_system_library};
use crate::preview::render::color_f;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_ROUNDED_RECT, ID2D1DCRenderTarget, ID2D1Factory,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DrawTextW, HDC, HFONT,
    IntersectClipRect, RestoreDC, SaveDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};

/// The corner radius of cards, controls and buttons.
pub(crate) const RADIUS_AT_96_DPI: i32 = 4;
/// The focus ring's stroke, and its gap outside the control it rings.
pub(crate) const FOCUS_WIDTH_AT_96_DPI: i32 = 2;
pub(crate) const FOCUS_GAP_AT_96_DPI: i32 = 1;
/// A dialog's title row, and its ×, as wide as the main window's caption close button.
pub(crate) const TITLE_HEIGHT_AT_96_DPI: i32 = 44;
pub(crate) const TITLE_CLOSE_WIDTH_AT_96_DPI: i32 = 46;
pub(crate) const GLYPH_FONT: &str = "Segoe MDL2 Assets";

/// The fills of the soft controls, from the theme's palette. Hover and press shift a fill
/// toward the text colour: darker in light themes, lighter in dark ones. High contrast may
/// only use system colour pairs, so there it keeps each fill and outlines every card and
/// control instead.
pub(crate) struct Tones {
    /// A setting's card, and the rules under a title row and over a footer: a step off the
    /// panel (strip_background is too close to it).
    pub card: u32,
    pub card_hot: u32,
    /// Dropdowns, steppers and segment tracks: a step off the card.
    pub control: u32,
    pub control_hot: u32,
    pub control_down: u32,
    pub accent: u32,
    pub accent_hot: u32,
    pub accent_down: u32,
    /// Text and knobs on the accent.
    pub on_accent: u32,
    /// High contrast's outline.
    pub outline: Option<u32>,
}

impl Tones {
    pub(crate) fn new(colors: &Palette) -> Self {
        let shade = |color: u32, alpha: u32| {
            if colors.high_contrast {
                color
            } else {
                crate::catppuccin::blend(colors.editor_foreground, color, alpha)
            }
        };
        let (card, control) = if colors.high_contrast {
            (colors.strip_background, colors.strip_background)
        } else {
            (colors.hover_background, colors.pressed_background)
        };
        let accent = colors.selection_background;
        Self {
            card,
            card_hot: shade(card, 16),
            control,
            control_hot: shade(control, 28),
            control_down: shade(control, 56),
            accent,
            accent_hot: shade(accent, 28),
            accent_down: shade(accent, 56),
            on_accent: colors
                .selection_foreground
                .unwrap_or(colors.editor_foreground),
            outline: colors.high_contrast.then_some(colors.muted_foreground),
        }
    }

    /// A rounded fill, outlined in high contrast.
    pub(crate) fn soft(&self, frame: &mut Frame<'_>, rect: RECT, radius: i32, color: u32) {
        frame.shape(Shape::Round {
            rect,
            radius,
            color,
        });
        if let Some(outline) = self.outline {
            frame.shape(Shape::Ring {
                rect,
                radius,
                width: 1,
                color: outline,
            });
        }
    }
}

/// A title row's ×: muted at rest; when `hot`, it fills its full-height corner like the caption
/// close button.
pub(crate) fn title_close(
    frame: &mut Frame<'_>,
    colors: &Palette,
    glyph_font: HFONT,
    rect: RECT,
    hot: bool,
) {
    if hot {
        frame.shape(Shape::Fill {
            rect,
            color: colors.close_hover_background,
        });
    }
    frame.text(
        glyph_font,
        if hot {
            colors.close_hover_foreground
        } else {
            colors.muted_foreground
        },
        super::titlebar::GLYPH_CLOSE,
        rect,
        DT_CENTER,
    );
}

/// One shape, in client pixels. Colours are `COLORREF`s.
#[derive(Clone, Copy)]
pub(crate) enum Shape {
    /// A square fill.
    Fill { rect: RECT, color: u32 },
    /// A rounded fill. A radius of half the height makes a pill; of half a square's side, a
    /// circle.
    Round { rect: RECT, radius: i32, color: u32 },
    /// A rounded outline `width` wide, inside `rect`.
    Ring {
        rect: RECT,
        radius: i32,
        width: i32,
        color: u32,
    },
}

struct Text<'a> {
    font: HFONT,
    color: u32,
    text: Cow<'a, str>,
    rect: RECT,
    align: u32,
}

/// What one paint draws: every shape, then every text, each clipped to the clip current when it
/// was added.
#[derive(Default)]
pub(crate) struct Frame<'a> {
    shapes: Vec<(Shape, Option<RECT>)>,
    texts: Vec<(Text<'a>, Option<RECT>)>,
    clip: Option<RECT>,
}

impl<'a> Frame<'a> {
    /// Clips what is added from now on to `clip`, or nothing.
    pub(crate) fn clip(&mut self, clip: Option<RECT>) {
        self.clip = clip;
    }

    pub(crate) fn shape(&mut self, shape: Shape) {
        self.shapes.push((shape, self.clip));
    }

    /// A single line of `text` in `rect`, vertically centred, ellipsized; `align` is `DT_LEFT`,
    /// `DT_CENTER` or `DT_RIGHT`.
    pub(crate) fn text(
        &mut self,
        font: HFONT,
        color: u32,
        text: impl Into<Cow<'a, str>>,
        rect: RECT,
        align: u32,
    ) {
        let text = Text {
            font,
            color,
            text: text.into(),
            rect,
            align,
        };
        self.texts.push((text, self.clip));
    }

    /// Every text added so far: its string, its rect and the clip it was added under.
    #[cfg(test)]
    pub(crate) fn test_texts(&self) -> Vec<(String, RECT, Option<RECT>)> {
        self.texts
            .iter()
            .map(|(text, clip)| (text.text.to_string(), text.rect, *clip))
            .collect()
    }

    /// Draws the shapes through `canvas`, then the text with GDI.
    pub(crate) fn paint(&self, dc: HDC, client: RECT, canvas: &Canvas) {
        canvas.draw(dc, client, &self.shapes);
        unsafe {
            SetBkMode(dc, TRANSPARENT as i32);
            for (text, clip) in &self.texts {
                with_clip(dc, *clip, || draw_text(dc, text));
            }
        }
    }
}

unsafe fn draw_text(dc: HDC, text: &Text<'_>) {
    let mut wide = wide_null(&text.text);
    let mut rect = text.rect;
    unsafe {
        let previous = SelectObject(dc, text.font as _);
        SetTextColor(dc, text.color);
        DrawTextW(
            dc,
            wide.as_mut_ptr(),
            -1,
            &mut rect,
            text.align | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        SelectObject(dc, previous);
    }
}

fn with_clip(dc: HDC, clip: Option<RECT>, draw: impl FnOnce()) {
    let Some(clip) = clip else {
        return draw();
    };
    unsafe {
        let saved = SaveDC(dc);
        IntersectClipRect(dc, clip.left, clip.top, clip.right, clip.bottom);
        draw();
        RestoreDC(dc, saved);
    }
}

/// Draws shapes: with Direct2D when it loaded, else with GDI. A clone shares the one Direct2D
/// stack, so a dropdown list reuses its dialog's instead of loading another on every open.
#[derive(Clone)]
pub(crate) struct Canvas {
    direct2d: Option<Rc<Direct2D>>,
}

/// Field order matters: the target drops before the factory, and both before the module that
/// implements them.
struct Direct2D {
    /// Recreated on the next paint after a frame fails (a lost device).
    target: RefCell<Option<Target>>,
    /// Paints in a row that failed to produce a target or a frame; at `MAX_FAILURES` the canvas
    /// gives up on Direct2D and uses GDI for the rest of its life.
    failures: Cell<u32>,
    factory: ID2D1Factory,
    _module: OwnedModule,
}

const MAX_FAILURES: u32 = 3;

struct Target {
    brush: ID2D1SolidColorBrush,
    target: ID2D1DCRenderTarget,
}

impl Canvas {
    /// Loads Direct2D and creates its DC render target; GDI only if either fails. Call when a
    /// window opens, never at startup.
    pub(crate) fn load() -> Self {
        Self {
            direct2d: Direct2D::load().map(Rc::new),
        }
    }

    /// Plain GDI fills, as when Direct2D can't load.
    #[cfg(test)]
    pub(crate) const fn gdi() -> Self {
        Self { direct2d: None }
    }

    #[cfg(test)]
    pub(crate) fn uses_direct2d(&self) -> bool {
        self.direct2d.is_some()
    }

    fn draw(&self, dc: HDC, client: RECT, shapes: &[(Shape, Option<RECT>)]) {
        if let Some(direct2d) = self.direct2d.as_deref()
            && direct2d.failures.get() < MAX_FAILURES
        {
            let mut slot = direct2d.target.borrow_mut();
            if slot.is_none() {
                *slot = Target::create(&direct2d.factory).ok();
            }
            if let Some(target) = slot.as_ref() {
                if unsafe { target.draw(dc, client, shapes) }.is_ok() {
                    direct2d.failures.set(0);
                    return;
                }
                // The frame may be half drawn: GDI paints over all of it.
                *slot = None;
            }
            direct2d.failures.set(direct2d.failures.get() + 1);
        }
        for (shape, clip) in shapes {
            with_clip(dc, *clip, || unsafe { draw_gdi(dc, *shape) });
        }
    }
}

impl Direct2D {
    fn load() -> Option<Self> {
        let module = load_system_library("d2d1.dll").ok()?;
        let factory = create_d2d_factory(&module, D2D1_FACTORY_TYPE_SINGLE_THREADED).ok()?;
        let target = Target::create(&factory).ok()?;
        Some(Self {
            target: RefCell::new(Some(target)),
            failures: Cell::new(0),
            factory,
            _module: module,
        })
    }
}

impl Target {
    fn create(factory: &ID2D1Factory) -> windows::core::Result<Self> {
        // 96 DPI: one Direct2D unit is one pixel, whatever the monitor's scale.
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            // Software: cheaper than a GPU device and readback for a small DC-bound target.
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        unsafe {
            let target = factory.CreateDCRenderTarget(&properties)?;
            let brush = target.CreateSolidColorBrush(&color_f(0), None)?;
            Ok(Self { brush, target })
        }
    }

    unsafe fn draw(
        &self,
        dc: HDC,
        client: RECT,
        shapes: &[(Shape, Option<RECT>)],
    ) -> windows::core::Result<()> {
        let bounds = windows::Win32::Foundation::RECT {
            left: client.left,
            top: client.top,
            right: client.right,
            bottom: client.bottom,
        };
        let target = &self.target;
        unsafe {
            target.BindDC(windows::Win32::Graphics::Gdi::HDC(dc), &bounds)?;
            target.BeginDraw();
            for (shape, clip) in shapes {
                if let Some(clip) = clip {
                    target.PushAxisAlignedClip(&rect_f(*clip, 0.0), D2D1_ANTIALIAS_MODE_ALIASED);
                }
                self.draw_shape(*shape);
                if clip.is_some() {
                    target.PopAxisAlignedClip();
                }
            }
            target.EndDraw(None, None)
        }
    }

    unsafe fn draw_shape(&self, shape: Shape) {
        let target = &self.target;
        let brush = &self.brush;
        unsafe {
            match shape {
                Shape::Fill { rect, color } => {
                    brush.SetColor(&color_f(color));
                    target.FillRectangle(&rect_f(rect, 0.0), brush);
                }
                Shape::Round {
                    rect,
                    radius,
                    color,
                } => {
                    brush.SetColor(&color_f(color));
                    target.FillRoundedRectangle(&rounded(rect, 0.0, radius as f32), brush);
                }
                Shape::Ring {
                    rect,
                    radius,
                    width,
                    color,
                } => {
                    // A stroke is centred on its path: half a width in keeps it inside `rect`.
                    let half = width as f32 / 2.0;
                    brush.SetColor(&color_f(color));
                    target.DrawRoundedRectangle(
                        &rounded(rect, half, radius as f32 - half),
                        brush,
                        width as f32,
                        None,
                    );
                }
            }
        }
    }
}

fn rect_f(rect: RECT, inset: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: rect.left as f32 + inset,
        top: rect.top as f32 + inset,
        right: rect.right as f32 - inset,
        bottom: rect.bottom as f32 - inset,
    }
}

fn rounded(rect: RECT, inset: f32, radius: f32) -> D2D1_ROUNDED_RECT {
    let rect = rect_f(rect, inset);
    let most = ((rect.right - rect.left).min(rect.bottom - rect.top) / 2.0).max(0.0);
    let radius = radius.clamp(0.0, most);
    D2D1_ROUNDED_RECT {
        rect,
        radiusX: radius,
        radiusY: radius,
    }
}

/// The fallback: square fills, and an outline as four fills.
unsafe fn draw_gdi(dc: HDC, shape: Shape) {
    unsafe {
        match shape {
            Shape::Fill { rect, color } | Shape::Round { rect, color, .. } => fill(dc, rect, color),
            Shape::Ring {
                rect, width, color, ..
            } => {
                let RECT {
                    left,
                    top,
                    right,
                    bottom,
                } = rect;
                for side in [
                    RECT {
                        bottom: top + width,
                        ..rect
                    },
                    RECT {
                        top: bottom - width,
                        ..rect
                    },
                    RECT {
                        top: top + width,
                        right: left + width,
                        bottom: bottom - width,
                        left,
                    },
                    RECT {
                        left: right - width,
                        top: top + width,
                        bottom: bottom - width,
                        right,
                    },
                ] {
                    fill(dc, side, color);
                }
            }
        }
    }
}

/// An off-screen 32-bit bitmap for painting tests, like the one `paint_buffered` draws into.
#[cfg(test)]
pub(crate) struct TestSurface {
    pub dc: HDC,
    bitmap: windows_sys::Win32::Graphics::Gdi::HBITMAP,
    previous: windows_sys::Win32::Graphics::Gdi::HGDIOBJ,
}

#[cfg(test)]
impl TestSurface {
    pub(crate) fn new(width: i32, height: i32) -> Self {
        use windows_sys::Win32::Graphics::Gdi::{
            CreateCompatibleBitmap, CreateCompatibleDC, GetDC, ReleaseDC,
        };
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            ReleaseDC(std::ptr::null_mut(), screen);
            assert!(!dc.is_null() && !bitmap.is_null());
            let previous = SelectObject(dc, bitmap);
            Self {
                dc,
                bitmap,
                previous,
            }
        }
    }

    pub(crate) fn pixel(&self, x: i32, y: i32) -> u32 {
        unsafe { windows_sys::Win32::Graphics::Gdi::GetPixel(self.dc, x, y) }
    }
}

#[cfg(test)]
impl Drop for TestSurface {
    fn drop(&mut self) {
        use windows_sys::Win32::Graphics::Gdi::{DeleteDC, DeleteObject};
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::rgb;

    const RED: u32 = rgb(200, 30, 30);
    const BLUE: u32 = rgb(30, 30, 200);
    const WHITE: u32 = rgb(255, 255, 255);

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    fn frame() -> Frame<'static> {
        let mut frame = Frame::default();
        frame.shape(Shape::Fill {
            rect: rect(0, 0, 100, 60),
            color: WHITE,
        });
        frame.shape(Shape::Round {
            rect: rect(10, 10, 50, 30),
            radius: 10,
            color: RED,
        });
        frame.shape(Shape::Ring {
            rect: rect(60, 10, 90, 40),
            radius: 4,
            width: 2,
            color: BLUE,
        });
        // Clipped to its left half.
        frame.clip(Some(rect(0, 45, 20, 60)));
        frame.shape(Shape::Fill {
            rect: rect(0, 45, 40, 60),
            color: BLUE,
        });
        frame
    }

    fn check_solid_parts(surface: &TestSurface) {
        assert_eq!(surface.pixel(30, 20), RED, "the middle of a rounded fill");
        assert_eq!(surface.pixel(75, 10), BLUE, "the top edge of an outline");
        assert_eq!(surface.pixel(75, 25), WHITE, "an outline leaves its inside");
        assert_eq!(surface.pixel(10, 50), BLUE, "inside the clip");
        assert_eq!(surface.pixel(30, 50), WHITE, "outside the clip");
    }

    #[test]
    fn gdi_fallback_paints_every_shape_with_square_corners() {
        // Break caught: a dialog that shows nothing, or no toggle state, on a system where
        // Direct2D doesn't load.
        let surface = TestSurface::new(100, 60);
        frame().paint(surface.dc, rect(0, 0, 100, 60), &Canvas::gdi());
        check_solid_parts(&surface);
        assert_eq!(surface.pixel(10, 10), RED, "square corners");
    }

    #[test]
    fn direct2d_paints_rounded_corners_into_a_memory_dc() {
        // Break caught: the DC render target drawing nowhere (unbound, or in DIPs at the wrong
        // scale), or square corners where Direct2D loaded.
        let canvas = Canvas::load();
        assert!(
            canvas.uses_direct2d(),
            "Direct2D loads on Windows 10 and later"
        );
        let surface = TestSurface::new(100, 60);
        frame().paint(surface.dc, rect(0, 0, 100, 60), &canvas);
        check_solid_parts(&surface);
        assert_eq!(
            surface.pixel(10, 10),
            WHITE,
            "the rounded corner is cut off"
        );
        // A second frame into the same target still draws.
        let again = TestSurface::new(100, 60);
        frame().paint(again.dc, rect(0, 0, 100, 60), &canvas);
        check_solid_parts(&again);
    }
}
