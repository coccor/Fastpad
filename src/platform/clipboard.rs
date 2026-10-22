//! Text onto the clipboard: plain, for the Keyboard Shortcuts page's Copy command ID, and plain
//! with HTML beside it, for the preview's selection.

use crate::Result;
use crate::platform::{last_error, wide_null};
use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

/// Replaces the clipboard's contents with `text`. `owner` may be null.
pub(crate) fn set_text(owner: HWND, text: &str) -> Result<()> {
    let wide = wide_null(text);
    if unsafe { OpenClipboard(owner) } == 0 {
        return Err(last_error());
    }
    let result = (|| unsafe {
        EmptyClipboard();
        let memory = GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of_val(wide.as_slice()));
        if memory.is_null() {
            return Err(last_error());
        }
        let target = GlobalLock(memory).cast::<u16>();
        if target.is_null() {
            let error = last_error();
            GlobalFree(memory);
            return Err(error);
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
        GlobalUnlock(memory);
        // The clipboard owns the memory once SetClipboardData succeeds.
        if SetClipboardData(u32::from(CF_UNICODETEXT), memory).is_null() {
            let error = last_error();
            GlobalFree(memory);
            return Err(error);
        }
        Ok(())
    })();
    unsafe { CloseClipboard() };
    result
}

/// Replaces the clipboard's contents with `text` and, in the registered `HTML Format`, `html`:
/// a complete CF_HTML document (`preview::selection::cf_html`). `owner` may be null.
pub(crate) fn set_text_and_html(owner: HWND, text: &str, html: &str) -> Result<()> {
    if !open(owner) {
        return Err(last_error());
    }
    let wide = wide_null(text);
    let mut utf8 = html.as_bytes().to_vec();
    utf8.push(0);
    let result = (|| unsafe {
        EmptyClipboard();
        put(
            u32::from(CF_UNICODETEXT),
            wide.as_ptr().cast(),
            std::mem::size_of_val(wide.as_slice()),
        )?;
        put(html_format(), utf8.as_ptr(), utf8.len())
    })();
    unsafe { CloseClipboard() };
    result
}

/// Copies `len` bytes at `data` into clipboard memory and hands it to the clipboard as `format`.
/// The clipboard must be open and owned.
unsafe fn put(format: u32, data: *const u8, len: usize) -> Result<()> {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, len);
        if memory.is_null() {
            return Err(last_error());
        }
        let target = GlobalLock(memory).cast::<u8>();
        if target.is_null() {
            let error = last_error();
            GlobalFree(memory);
            return Err(error);
        }
        std::ptr::copy_nonoverlapping(data, target, len);
        GlobalUnlock(memory);
        // The clipboard owns the memory once SetClipboardData succeeds.
        if SetClipboardData(format, memory).is_null() {
            let error = last_error();
            GlobalFree(memory);
            return Err(error);
        }
        Ok(())
    }
}

/// The registered `HTML Format` Word, Outlook and browsers read rich text from.
fn html_format() -> u32 {
    let name = wide_null("HTML Format");
    unsafe { windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW(name.as_ptr()) }
}

/// Opens the clipboard, waiting briefly while another program holds it.
fn open(owner: HWND) -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(owner) } != 0 {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

/// The registered `PNG` format browsers and Snipping Tool put on the clipboard.
fn png_format() -> u32 {
    let name = wide_null("PNG");
    unsafe { windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW(name.as_ptr()) }
}

fn available(format: u32) -> bool {
    format != 0
        && unsafe { windows_sys::Win32::System::DataExchange::IsClipboardFormatAvailable(format) }
            != 0
}

/// Whether the clipboard holds text.
pub(crate) fn has_text() -> bool {
    available(u32::from(CF_UNICODETEXT))
}

/// Whether the clipboard holds an image a capture can attach.
pub(crate) fn has_image() -> bool {
    use windows_sys::Win32::System::Ole::{CF_DIB, CF_DIBV5};
    available(png_format()) || available(u32::from(CF_DIBV5)) || available(u32::from(CF_DIB))
}

/// Whether the clipboard holds files copied in Explorer.
pub(crate) fn has_files() -> bool {
    available(u32::from(windows_sys::Win32::System::Ole::CF_HDROP))
}

/// A copy of the clipboard's `format` block; the clipboard must be open.
fn read_global(format: u32) -> Option<Vec<u8>> {
    use windows_sys::Win32::System::DataExchange::GetClipboardData;
    use windows_sys::Win32::System::Memory::GlobalSize;
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return None;
    }
    let size = unsafe { GlobalSize(handle) };
    let data = unsafe { GlobalLock(handle) }.cast::<u8>();
    if data.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(data, size) }.to_vec();
    unsafe { GlobalUnlock(handle) };
    Some(bytes)
}

/// The files copied in Explorer (`CF_HDROP`), or none.
pub(crate) fn files(owner: HWND) -> Vec<std::path::PathBuf> {
    use windows_sys::Win32::System::DataExchange::GetClipboardData;
    if !has_files() || !open(owner) {
        return Vec::new();
    }
    let handle = unsafe { GetClipboardData(u32::from(windows_sys::Win32::System::Ole::CF_HDROP)) };
    let paths = if handle.is_null() {
        Vec::new()
    } else {
        crate::platform::win32::dropped_paths(handle)
    };
    unsafe { CloseClipboard() };
    paths
}

/// The clipboard's image as PNG bytes (second brain phase 1 spec §10.4): the `PNG` format as it
/// is, else a `CF_DIBV5` or `CF_DIB` bitmap encoded with WIC. `None` when it holds no image.
pub(crate) fn image_png(owner: HWND) -> Option<std::result::Result<Vec<u8>, String>> {
    use windows_sys::Win32::System::Ole::{CF_DIB, CF_DIBV5};
    if !has_image() || !open(owner) {
        return None;
    }
    let png = read_global(png_format()).filter(|bytes| bytes.starts_with(b"\x89PNG"));
    let dib = png
        .is_none()
        .then(|| read_global(u32::from(CF_DIBV5)).or_else(|| read_global(u32::from(CF_DIB))));
    unsafe { CloseClipboard() };
    if let Some(png) = png {
        return Some(Ok(png));
    }
    let pixels = dib
        .flatten()
        .and_then(|dib| crate::platform::png::dib_to_pixels(&dib));
    Some(match pixels {
        Some(pixels) => crate::platform::png::encode_png(&pixels),
        None => Err("FastPad can't read this kind of image from the clipboard.".to_owned()),
    })
}

/// Saving and restoring the user's clipboard around tests that write to it, and reading back what
/// they wrote.
#[cfg(test)]
pub(crate) mod test_support {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, OpenClipboard,
        SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
    };

    /// Opens the clipboard, waiting out another program (a clipboard manager) holding it.
    pub(crate) fn open() {
        for _ in 0..50 {
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the clipboard stayed open in another program");
    }

    /// The user's clipboard, put back on drop: every format held in global memory. GDI-handle
    /// formats (bitmaps, metafiles, palettes) are left out; Windows synthesizes the bitmap from
    /// the DIB that is kept.
    pub(crate) struct Backup(Vec<(u32, Vec<u8>)>);

    impl Backup {
        pub(crate) fn take() -> Self {
            const GDI_HANDLES: [u32; 7] = [2, 3, 9, 14, 0x80, 0x82, 0x8e];
            let mut formats = Vec::new();
            open();
            unsafe {
                let mut format = EnumClipboardFormats(0);
                while format != 0 {
                    let private_gdi = (0x300..=0x3ff).contains(&format);
                    if !GDI_HANDLES.contains(&format) && !private_gdi {
                        let data = GetClipboardData(format);
                        let size = if data.is_null() { 0 } else { GlobalSize(data) };
                        let bytes = if size == 0 {
                            std::ptr::null()
                        } else {
                            GlobalLock(data) as *const u8
                        };
                        if !bytes.is_null() {
                            formats
                                .push((format, std::slice::from_raw_parts(bytes, size).to_vec()));
                            GlobalUnlock(data);
                        }
                    }
                    format = EnumClipboardFormats(format);
                }
                CloseClipboard();
            }
            Self(formats)
        }
    }

    impl Drop for Backup {
        fn drop(&mut self) {
            open();
            unsafe {
                EmptyClipboard();
                for (format, bytes) in &self.0 {
                    let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
                    if memory.is_null() {
                        continue;
                    }
                    let target = GlobalLock(memory) as *mut u8;
                    if !target.is_null() {
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
                        GlobalUnlock(memory);
                        SetClipboardData(*format, memory);
                    }
                }
                CloseClipboard();
            }
        }
    }

    /// The bytes the clipboard holds in `format`, up to its first NUL unit for text.
    pub(crate) fn read(format: u32) -> Vec<u8> {
        open();
        unsafe {
            let data = GetClipboardData(format);
            let size = if data.is_null() { 0 } else { GlobalSize(data) };
            let pointer = if size == 0 {
                std::ptr::null()
            } else {
                GlobalLock(data) as *const u8
            };
            let bytes = if pointer.is_null() {
                Vec::new()
            } else {
                let bytes = std::slice::from_raw_parts(pointer, size).to_vec();
                GlobalUnlock(data);
                bytes
            };
            CloseClipboard();
            bytes
        }
    }

    /// The clipboard's Unicode text.
    pub(crate) fn read_text() -> String {
        let bytes = read(u32::from(windows_sys::Win32::System::Ole::CF_UNICODETEXT));
        let units = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .take_while(|unit| *unit != 0)
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&units)
    }

    /// The clipboard's `HTML Format` document.
    pub(crate) fn read_html() -> String {
        let bytes = read(super::html_format());
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{Backup, open, read_html, read_text};
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, GetClipboardData};
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    #[test]
    fn text_and_html_land_together() {
        // Break caught: the HTML replacing the plain text (EmptyClipboard between them), or
        // its bytes losing their NUL so Word reads past the end.
        let _backup = Backup::take();
        let html = crate::preview::selection::cf_html("<p><strong>é</strong></p>");
        super::set_text_and_html(std::ptr::null_mut(), "é", &html).unwrap();
        assert_eq!(read_text(), "é");
        assert_eq!(read_html(), html);
    }

    #[test]
    fn text_round_trips_through_the_clipboard() {
        // Break caught: Copy command ID putting nothing, or ANSI bytes, on the clipboard.
        use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
        let _backup = Backup::take();
        super::set_text(std::ptr::null_mut(), "file.saveAs").unwrap();
        open();
        let read = unsafe {
            let data = GetClipboardData(u32::from(CF_UNICODETEXT));
            let text = GlobalLock(data as _) as *const u16;
            let read = if text.is_null() {
                String::new()
            } else {
                let mut length = 0;
                while *text.add(length) != 0 {
                    length += 1;
                }
                let read = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
                GlobalUnlock(data as _);
                read
            };
            CloseClipboard();
            read
        };
        assert_eq!(read, "file.saveAs");
    }
}
