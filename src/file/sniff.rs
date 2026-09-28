//! Image file signatures, checked only after a file failed to open as text (image preview
//! spec §4), so an extensionless or misnamed image still opens in an image tab.

use std::io::Read;
use std::path::Path;

const SIGNATURES: [&[u8]; 8] = [
    b"\x89PNG\r\n\x1a\n",
    b"\xFF\xD8\xFF",
    b"GIF87a",
    b"GIF89a",
    b"BM",
    b"II*\0",
    b"MM\0*",
    b"\0\0\x01\0",
];

pub fn looks_like_image(head: &[u8]) -> bool {
    SIGNATURES
        .iter()
        .any(|signature| head.starts_with(signature))
        || (head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP")
}

/// Reads at most the first 16 bytes of `path`.
pub fn file_looks_like_image(path: &Path) -> bool {
    let mut head = [0_u8; 16];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut filled = 0;
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) | Err(_) => break,
            Ok(read) => filled += read,
        }
    }
    looks_like_image(&head[..filled])
}

#[cfg(test)]
mod tests {
    use super::looks_like_image;

    #[test]
    fn known_signatures_match_and_text_or_short_input_does_not() {
        // Break caught: a PNG saved as .dat getting the "unsupported text encoding" notice, or a
        // four-byte RIFF file indexing past its end.
        assert!(looks_like_image(b"\x89PNG\r\n\x1a\n\0\0"));
        assert!(looks_like_image(b"\xFF\xD8\xFF\xE0"));
        assert!(looks_like_image(b"RIFF\x10\0\0\0WEBPVP8 "));
        assert!(looks_like_image(b"\0\0\x01\0\x01\0"));
        assert!(!looks_like_image(b"RIFF"));
        assert!(!looks_like_image(b"RIFF\x10\0\0\0WAVEfmt "));
        assert!(!looks_like_image(b"\x00\x01\x02\x03"));
        assert!(!looks_like_image(b""));
    }
}
