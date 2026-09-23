//! Stable identifiers, the content fingerprint hash, and the escaping used in library files.

use std::io::Read;
use std::path::Path;

macro_rules! library_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub u128);

        impl $name {
            pub fn to_hex(self) -> String {
                format!("{:032x}", self.0)
            }

            pub fn parse_hex(text: &str) -> Option<Self> {
                if text.len() != 32 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return None;
                }
                u128::from_str_radix(text, 16).ok().map(Self)
            }
        }
    };
}

library_id!(NoteId);
library_id!(NotebookId);
library_id!(TagId);

/// Hands out IDs in the same shape as `RecoveryId`: process start, PID, then a counter.
#[derive(Debug)]
pub struct IdSource {
    process_start: u64,
    pid: u32,
    counter: u64,
}

impl IdSource {
    pub const fn new(process_start: u64, pid: u32) -> Self {
        Self {
            process_start,
            pid,
            counter: 0,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u128 {
        self.counter += 1;
        crate::document::RecoveryId::compose(self.process_start, self.pid, self.counter).0
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 64-bit FNV-1a, the content fingerprint and folder-key hash.
#[derive(Clone, Copy, Debug)]
pub struct Fnv1a(u64);

impl Fnv1a {
    pub const fn new() -> Self {
        Self(FNV_OFFSET)
    }

    pub fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }

    pub const fn finish(self) -> u64 {
        self.0
    }
}

impl Default for Fnv1a {
    fn default() -> Self {
        Self::new()
    }
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = Fnv1a::new();
    hash.update(bytes);
    hash.finish()
}

/// The fingerprint of a file's bytes, or `None` when it cannot be read or is larger than `limit`.
pub fn hash_file(path: &Path, limit: u64) -> Option<u64> {
    let mut file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > limit {
        return None;
    }
    let mut hash = Fnv1a::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            return Some(hash.finish());
        }
        hash.update(&buffer[..read]);
    }
}

/// Escapes `%`, `|`, CR and LF so a name fits in one `|`-separated field.
pub fn escape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '%' => output.push_str("%25"),
            '|' => output.push_str("%7C"),
            '\r' => output.push_str("%0D"),
            '\n' => output.push_str("%0A"),
            _ => output.push(character),
        }
    }
    output
}

/// Reverses `escape`. Any other `%` sequence is kept literally.
pub fn unescape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('%') {
        output.push_str(&rest[..index]);
        let code = rest.get(index + 1..index + 3);
        let decoded = match code.map(str::to_ascii_uppercase).as_deref() {
            Some("25") => Some('%'),
            Some("7C") => Some('|'),
            Some("0D") => Some('\r'),
            Some("0A") => Some('\n'),
            _ => None,
        };
        match decoded {
            Some(character) => {
                output.push(character);
                rest = &rest[index + 3..];
            }
            None => {
                output.push('%');
                rest = &rest[index + 1..];
            }
        }
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_as_32_hex_digits_and_reject_anything_else() {
        // Break caught: an ID written in one width and read back in another, so every record
        // loses its identity on the next launch.
        let id = NoteId(0xabc);
        assert_eq!(id.to_hex(), "00000000000000000000000000000abc");
        assert_eq!(NoteId::parse_hex(&id.to_hex()), Some(id));
        assert_eq!(NoteId::parse_hex("abc"), None);
        assert_eq!(NoteId::parse_hex("zz000000000000000000000000000abc"), None);
        assert_eq!(NotebookId::parse_hex(&"f".repeat(32)), Some(NotebookId(u128::MAX)));
    }

    #[test]
    fn id_sources_never_repeat_within_a_process() {
        let mut ids = IdSource::new(7, 42);
        let first = ids.next();
        let second = ids.next();
        assert_ne!(first, second);
        assert_eq!(first >> 64, 7);
    }

    #[test]
    fn fnv1a_matches_the_published_64_bit_vectors() {
        // Break caught: a wrong prime or offset basis, so another FastPad build (or another PC)
        // computes different fingerprints and sync matching silently stops working.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
        let mut streamed = Fnv1a::new();
        streamed.update(b"foo");
        streamed.update(b"bar");
        assert_eq!(streamed.finish(), fnv1a(b"foobar"));
    }

    #[test]
    fn hash_file_streams_the_bytes_and_refuses_files_over_the_limit() {
        let dir = std::env::temp_dir().join(format!("fastpad-ids-hash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.md");
        std::fs::write(&path, b"foobar").unwrap();
        assert_eq!(hash_file(&path, 6), Some(fnv1a(b"foobar")));
        assert_eq!(hash_file(&path, 5), None);
        assert_eq!(hash_file(&dir.join("missing.md"), 100), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn escaping_protects_separators_and_line_breaks_and_round_trips() {
        // Break caught: a notebook called "a|b" or a name with a newline splitting one record
        // into two lines, or a literal "%41" being decoded into something else.
        let name = "50% a|b\r\nc %41";
        let escaped = escape(name);
        assert_eq!(escaped, "50%25 a%7Cb%0D%0Ac %2541");
        assert!(!escaped.contains(['|', '\r', '\n']));
        assert_eq!(unescape(&escaped), name);
        assert_eq!(unescape("100%"), "100%");
        assert_eq!(unescape("%zz"), "%zz");
    }
}
