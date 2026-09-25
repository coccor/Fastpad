//! The label that follows the pointer while a Notebook tree drag is under way (tree drag spec
//! §3.2): the dragged item's icon and name in a small layered popup, so the drag reads as one
//! everywhere, over the editor too. The caller paints the label once into a `LabelImage`; the
//! popup shows that bitmap and only moves after. It never takes the focus or a click.

use crate::platform::wide_null;
use crate::window::panel::scale;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetMonitorInfoW, HBITMAP, HDC,
    HGDIOBJ, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, ShowWindow, ULW_ALPHA,
    UpdateLayeredWindow, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const CLASS: &str = "FastPadDragLabel";
/// About 85% opaque: the rows under it still show through.
const OPACITY: u8 = 217;
// At 96 DPI: how far right of and below the pointer's hot spot the label sits, clear of the
// arrow; and the gap to the pointer when it flips to the left or above near a monitor's edge.
const OFFSET_X: i32 = 12;
const OFFSET_Y: i32 = 20;
const FLIP_GAP: i32 = 4;

/// A 32-bit bitmap in a memory DC that the label is painted into once, before it shows.
pub(crate) struct LabelImage {
    pub(crate) dc: HDC,
    pub(crate) size: SIZE,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}

impl LabelImage {
    /// A `width` × `height` image, or `None` if GDI can't make one.
    pub(crate) fn new(width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
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
        if bitmap.is_null() {
            return None;
        }
        let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        if dc.is_null() {
            unsafe { DeleteObject(bitmap) };
            return None;
        }
        let previous = unsafe { SelectObject(dc, bitmap) };
        Some(Self {
            dc,
            size: SIZE {
                cx: width,
                cy: height,
            },
            bitmap,
            previous,
        })
    }
}

impl Drop for LabelImage {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteDC(self.dc);
            DeleteObject(self.bitmap);
        }
    }
}

/// The label's popup while a drag is under way.
#[derive(Clone, Copy)]
pub(crate) struct DragLabel {
    hwnd: HWND,
    size: SIZE,
    dpi: u32,
}

impl DragLabel {
    /// Shows `image` next to screen point `pointer`, owned by `owner` (so it goes with the
    /// window). `name` is the popup's text, for screen readers and tests. `None` if the popup
    /// can't be made: the drag goes on without it. Call it with nothing of the App borrowed.
    pub(crate) fn show(
        owner: HWND,
        image: &LabelImage,
        name: &str,
        pointer: POINT,
        dpi: u32,
    ) -> Option<Self> {
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class = wide_null(CLASS);
        static REGISTERED: std::sync::Once = std::sync::Once::new();
        REGISTERED.call_once(|| {
            let window_class = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            unsafe { RegisterClassW(&window_class) };
        });
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                class.as_ptr(),
                wide_null(name).as_ptr(),
                WS_POPUP,
                0,
                0,
                image.size.cx,
                image.size.cy,
                owner,
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        let label = Self {
            hwnd,
            size: image.size,
            dpi,
        };
        let at = label.origin(pointer);
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: OPACITY,
            // No per-pixel alpha: GDI text leaves the alpha byte undefined.
            AlphaFormat: 0,
        };
        let shown = unsafe {
            UpdateLayeredWindow(
                hwnd,
                std::ptr::null_mut(),
                &at,
                &image.size,
                image.dc,
                &source,
                0,
                &blend,
                ULW_ALPHA,
            )
        };
        if shown == 0 {
            unsafe { DestroyWindow(hwnd) };
            return None;
        }
        unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        Some(label)
    }

    /// Moves the label next to screen point `pointer`. Call it with nothing of the App borrowed.
    pub(crate) fn move_to(&self, pointer: POINT) {
        let at = self.origin(pointer);
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                at.x,
                at.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// Where the label goes for `pointer`, on the work area of the pointer's monitor.
    fn origin(&self, pointer: POINT) -> POINT {
        place(pointer, self.size, work_area(pointer), self.dpi)
    }

    /// Destroys the popup. Call it with nothing of the App borrowed.
    pub(crate) fn destroy(self) {
        unsafe { DestroyWindow(self.hwnd) };
    }

    #[cfg(test)]
    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

/// The work area of the monitor nearest screen point `point`.
pub(crate) fn work_area(point: POINT) -> RECT {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        GetMonitorInfoW(monitor, &mut info);
    }
    info.rcWork
}

/// The label's top-left for a `size` label and the pointer at `pointer` (screen coordinates):
/// right of and below the pointer, flipped to its left or above where it would cross `area`'s
/// edge, and kept inside `area` when it fits neither way.
pub(crate) fn place(pointer: POINT, size: SIZE, area: RECT, dpi: u32) -> POINT {
    let along = |at: i32, offset: i32, extent: i32, low: i32, high: i32| {
        let mut start = at + scale(offset, dpi);
        if start + extent > high {
            start = at - scale(FLIP_GAP, dpi) - extent;
        }
        start.min(high - extent).max(low)
    };
    POINT {
        x: along(pointer.x, OFFSET_X, size.cx, area.left, area.right),
        y: along(pointer.y, OFFSET_Y, size.cy, area.top, area.bottom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: RECT = RECT {
        left: 0,
        top: 0,
        right: 1000,
        bottom: 800,
    };
    const SIZE_80_24: SIZE = SIZE { cx: 80, cy: 24 };

    #[test]
    fn the_label_sits_right_of_and_below_the_pointer() {
        let at = place(POINT { x: 100, y: 100 }, SIZE_80_24, AREA, 96);
        assert_eq!((at.x, at.y), (112, 120));
        let at = place(POINT { x: 100, y: 100 }, SIZE_80_24, AREA, 192);
        assert_eq!((at.x, at.y), (124, 140), "the offsets scale");
    }

    #[test]
    fn near_the_right_or_bottom_edge_the_label_flips() {
        let at = place(POINT { x: 950, y: 790 }, SIZE_80_24, AREA, 96);
        assert_eq!((at.x, at.y), (950 - 4 - 80, 790 - 4 - 24));
    }

    #[test]
    fn a_label_that_fits_neither_way_stays_on_the_monitor() {
        let narrow = RECT {
            left: 0,
            top: 0,
            right: 100,
            bottom: 30,
        };
        let at = place(POINT { x: 50, y: 10 }, SIZE_80_24, narrow, 96);
        assert_eq!((at.x, at.y), (0, 0));
        let at = place(
            POINT { x: -500, y: -500 },
            SIZE_80_24,
            RECT {
                left: -1000,
                top: -1000,
                right: 0,
                bottom: 0,
            },
            96,
        );
        assert_eq!((at.x, at.y), (-488, -480), "a monitor left of the primary");
    }
}
