//! Painted bottom bar: no child HWND. It appears once deferred chrome is built and then stays, so
//! the first paint keeps the editor at full height. The left side shows the caret position, or the
//! oldest pending notification while there is one; the right side names the active document's
//! language and encoding.

use crate::document::Language;
use crate::editor::CaretStatus;
use crate::file::encoding::Encoding;
use crate::platform::theme::SystemTheme;
use crate::window::notification::NotificationCenter;

const STATUS_HEIGHT_AT_96_DPI: i64 = 22;

pub fn status_height(dpi: u32) -> i32 {
    ((STATUS_HEIGHT_AT_96_DPI * i64::from(dpi.max(96)) + 48) / 96) as i32
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusModel {
    pub theme: SystemTheme,
}

impl StatusModel {
    pub fn new(theme: SystemTheme) -> Self {
        Self { theme }
    }
}

/// The two text runs the bar paints: `left` is left-aligned, `right` right-aligned.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StatusBarText {
    pub left: String,
    pub right: String,
}

/// What the active tab contributes to the bar; `None` while no tab is open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActiveDocumentStatus {
    pub caret: CaretStatus,
    pub language: Language,
    pub encoding: Encoding,
}

pub fn status_bar_text(
    notifications: &NotificationCenter,
    active: Option<ActiveDocumentStatus>,
) -> StatusBarText {
    let left = status_text(notifications)
        .or_else(|| active.map(|active| caret_text(active.caret)))
        .unwrap_or_default();
    let right = active
        .map(|active| {
            format!(
                "{}    {}",
                language_name(active.language),
                encoding_name(active.encoding)
            )
        })
        .unwrap_or_default();
    StatusBarText { left, right }
}

/// The status bar of an image tab: notices on the left; size, format, file size and zoom on the
/// right (image preview spec §5).
pub(crate) fn image_status_bar_text(
    notifications: &NotificationCenter,
    image: &crate::window::image_host::ImageStatus,
) -> StatusBarText {
    let mut parts = Vec::new();
    if let Some((width, height)) = image.size {
        parts.push(format!("{width} \u{d7} {height}"));
    }
    if let Some(format) = image.format {
        parts.push(format.to_owned());
    }
    if let Some(bytes) = image.bytes {
        parts.push(file_size_text(bytes));
    }
    if let Some(zoom) = image.zoom_percent {
        parts.push(format!("{zoom}%"));
    }
    StatusBarText {
        left: status_text(notifications).unwrap_or_default(),
        right: parts.join(" \u{b7} "),
    }
}

pub fn file_size_text(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} bytes"),
        1024..1_048_576 => format!("{} KB", (bytes + 512) / 1024),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

pub fn status_text(notifications: &NotificationCenter) -> Option<String> {
    let (first, rest) = notifications.pending().split_first()?;
    Some(if rest.is_empty() {
        format!("{} (click to dismiss)", first.message)
    } else {
        format!("{} (+{} more, click to dismiss)", first.message, rest.len())
    })
}

pub fn caret_text(caret: CaretStatus) -> String {
    if caret.selected_characters == 0 {
        format!("Ln {}, Col {}", caret.line, caret.column)
    } else {
        format!(
            "Ln {}, Col {} ({} selected)",
            caret.line, caret.column, caret.selected_characters
        )
    }
}

pub fn language_name(language: Language) -> &'static str {
    crate::languages::display_name(language)
}

pub fn encoding_name(encoding: Encoding) -> &'static str {
    match encoding {
        Encoding::Utf8 => "UTF-8",
        Encoding::Utf8Bom => "UTF-8 with BOM",
        Encoding::Utf16Le => "UTF-16 LE",
        Encoding::Utf16Be => "UTF-16 BE",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActiveDocumentStatus, StatusBarText, file_size_text, image_status_bar_text,
        status_bar_text, status_text,
    };
    use crate::document::Language;
    use crate::editor::CaretStatus;
    use crate::file::encoding::Encoding;
    use crate::window::notification::NotificationCenter;

    #[test]
    fn an_image_status_lists_size_format_file_size_and_zoom() {
        // Break caught: an image tab showing a caret position and "Plain Text UTF-8".
        let image = crate::window::image_host::ImageStatus {
            size: Some((1920, 1080)),
            format: Some("PNG"),
            bytes: Some(250_880),
            zoom_percent: Some(50),
        };
        let text = image_status_bar_text(&NotificationCenter::new(), &image);
        assert_eq!(
            text.right,
            "1920 \u{d7} 1080 \u{b7} PNG \u{b7} 245 KB \u{b7} 50%"
        );
        assert_eq!(file_size_text(900), "900 bytes");
        assert_eq!(file_size_text(3_250_000), "3.1 MB");
    }

    #[test]
    fn status_text_is_absent_without_notifications_and_counts_extras() {
        // Break caught: a stale notice left on the bar after dismissal, or dropping the extra
        // count, which hides every warning after the first.
        let mut center = NotificationCenter::new();
        assert_eq!(status_text(&center), None);
        center.push("a");
        assert_eq!(
            status_text(&center).as_deref(),
            Some("a (click to dismiss)")
        );
        center.push("b");
        center.push("c");
        assert_eq!(
            status_text(&center).as_deref(),
            Some("a (+2 more, click to dismiss)")
        );
    }

    #[test]
    fn bar_shows_caret_language_and_encoding_until_a_notice_takes_the_left_side() {
        // Break caught: a bar that reports a zero-based caret, hides the selection size, loses the
        // document details while a notice is up, or keeps describing a tab that was closed.
        let mut center = NotificationCenter::new();
        let active = ActiveDocumentStatus {
            caret: CaretStatus {
                line: 12,
                column: 5,
                selected_characters: 0,
            },
            language: Language::Markdown,
            encoding: Encoding::Utf8Bom,
        };
        assert_eq!(
            status_bar_text(&center, Some(active)),
            StatusBarText {
                left: "Ln 12, Col 5".to_owned(),
                right: "Markdown    UTF-8 with BOM".to_owned(),
            }
        );

        let selecting = ActiveDocumentStatus {
            caret: CaretStatus {
                selected_characters: 8,
                ..active.caret
            },
            ..active
        };
        assert_eq!(
            status_bar_text(&center, Some(selecting)).left,
            "Ln 12, Col 5 (8 selected)"
        );

        center.push("disk full");
        let noticed = status_bar_text(&center, Some(active));
        assert_eq!(noticed.left, "disk full (click to dismiss)");
        assert_eq!(noticed.right, "Markdown    UTF-8 with BOM");

        center.dismiss_all();
        assert_eq!(status_bar_text(&center, None), StatusBarText::default());
    }
}
