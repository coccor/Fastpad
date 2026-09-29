//! Plain text onto the clipboard, for the Keyboard Shortcuts page's Copy command ID.

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

#[cfg(test)]
mod tests {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, OpenClipboard,
        SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
    };

    /// Opens the clipboard, waiting out another program (a clipboard manager) holding it.
    fn open() {
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
    struct Backup(Vec<(u32, Vec<u8>)>);

    impl Backup {
        fn take() -> Self {
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
