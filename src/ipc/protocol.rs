use crate::{FastPadError, Result};
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

pub const FRAME_MAGIC: [u8; 4] = *b"FPI1";
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
pub const HEADER_BYTES: usize = FRAME_MAGIC.len() + 1 + 4;

const COMMAND_OPEN: u8 = 1;
const COMMAND_NEW: u8 = 2;
const COMMAND_ACTIVATE: u8 = 3;
const COMMAND_OPEN_FOLDER: u8 = 4;
const COMMAND_CAPTURE: u8 = 5;
/// The text field's length when a capture has no text.
const NO_TEXT: u32 = u32::MAX;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IpcRequest {
    Open(PathBuf),
    New,
    Activate,
    OpenFolder(PathBuf),
    /// `--capture` (second brain phase 1 spec §10.1): with neither text nor attachments, the
    /// capture box; otherwise filed with no window. Paths are absolute.
    Capture {
        text: Option<String>,
        attachments: Vec<PathBuf>,
    },
}

pub fn encode_frame(request: &IpcRequest) -> Result<Vec<u8>> {
    let (command, payload) = match request {
        IpcRequest::Open(path) => (COMMAND_OPEN, encode_path(path)?),
        IpcRequest::New => (COMMAND_NEW, Vec::new()),
        IpcRequest::Activate => (COMMAND_ACTIVATE, Vec::new()),
        IpcRequest::OpenFolder(path) => (COMMAND_OPEN_FOLDER, encode_path(path)?),
        IpcRequest::Capture { text, attachments } => (
            COMMAND_CAPTURE,
            encode_capture(text.as_deref(), attachments)?,
        ),
    };
    if HEADER_BYTES + payload.len() > MAX_FRAME_BYTES {
        return Err(FastPadError::Ipc("frame exceeds 64 KiB"));
    }
    let mut frame = Vec::with_capacity(HEADER_BYTES + payload.len());
    frame.extend_from_slice(&FRAME_MAGIC);
    frame.push(command);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn decode_frame(bytes: &[u8]) -> Result<IpcRequest> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(FastPadError::Ipc("frame exceeds 64 KiB"));
    }
    if bytes.len() < HEADER_BYTES {
        return Err(FastPadError::Ipc("frame header is truncated"));
    }
    if bytes[..FRAME_MAGIC.len()] != FRAME_MAGIC {
        return Err(FastPadError::Ipc("frame magic is not FPI1"));
    }
    let command = bytes[FRAME_MAGIC.len()];
    let mut length = [0_u8; 4];
    length.copy_from_slice(&bytes[FRAME_MAGIC.len() + 1..HEADER_BYTES]);
    let payload = &bytes[HEADER_BYTES..];
    if payload.len() != u32::from_le_bytes(length) as usize {
        return Err(FastPadError::Ipc("frame length does not match its payload"));
    }
    match command {
        COMMAND_OPEN => decode_path(payload).map(IpcRequest::Open),
        COMMAND_NEW | COMMAND_ACTIVATE if !payload.is_empty() => {
            Err(FastPadError::Ipc("command does not accept a payload"))
        }
        COMMAND_NEW => Ok(IpcRequest::New),
        COMMAND_ACTIVATE => Ok(IpcRequest::Activate),
        COMMAND_OPEN_FOLDER => decode_path(payload).map(IpcRequest::OpenFolder),
        COMMAND_CAPTURE => decode_capture(payload),
        _ => Err(FastPadError::Ipc("unknown command")),
    }
}

/// A capture's payload: the text as a u32 unit count (`NO_TEXT` for none) and its UTF-16 units,
/// then a u32 count of attachments, each a u32 unit count and its path's units. All little-endian.
fn encode_capture(text: Option<&str>, attachments: &[PathBuf]) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    match text {
        Some(text) => {
            let units: Vec<u16> = text.encode_utf16().collect();
            payload.extend_from_slice(&(units.len() as u32).to_le_bytes());
            payload.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
        }
        None => payload.extend_from_slice(&NO_TEXT.to_le_bytes()),
    }
    payload.extend_from_slice(&(attachments.len() as u32).to_le_bytes());
    for path in attachments {
        let units = encode_path(path)?;
        payload.extend_from_slice(&((units.len() / 2) as u32).to_le_bytes());
        payload.extend_from_slice(&units);
    }
    Ok(payload)
}

fn decode_capture(payload: &[u8]) -> Result<IpcRequest> {
    let mut rest = payload;
    let mut take = |count: usize| -> Result<&[u8]> {
        if rest.len() < count {
            return Err(FastPadError::Ipc("capture payload is truncated"));
        }
        let (head, tail) = rest.split_at(count);
        rest = tail;
        Ok(head)
    };
    let read_u32 = |bytes: &[u8]| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let text_units = read_u32(take(4)?);
    let text = if text_units == NO_TEXT {
        None
    } else {
        let bytes = take(text_units as usize * 2)?;
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        Some(
            String::from_utf16(&units)
                .map_err(|_| FastPadError::Ipc("capture text is not valid UTF-16"))?,
        )
    };
    let count = read_u32(take(4)?);
    let mut attachments = Vec::new();
    for _ in 0..count {
        let units = read_u32(take(4)?) as usize;
        attachments.push(decode_path(take(units * 2)?)?);
    }
    if !rest.is_empty() {
        return Err(FastPadError::Ipc("capture payload has trailing bytes"));
    }
    Ok(IpcRequest::Capture { text, attachments })
}

fn encode_path(path: &std::path::Path) -> Result<Vec<u8>> {
    let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    validate_path_units(&units)?;
    Ok(units.iter().flat_map(|unit| unit.to_le_bytes()).collect())
}

fn decode_path(payload: &[u8]) -> Result<PathBuf> {
    if !payload.len().is_multiple_of(2) {
        return Err(FastPadError::Ipc("path payload has an odd byte length"));
    }
    let units = payload
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect::<Vec<_>>();
    validate_path_units(&units)?;
    Ok(PathBuf::from(OsString::from_wide(&units)))
}

fn validate_path_units(units: &[u16]) -> Result<()> {
    if units.is_empty() {
        return Err(FastPadError::Ipc("path payload is empty"));
    }
    if units.contains(&0) {
        return Err(FastPadError::Ipc("path payload contains NUL"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn request_round_trips_utf16_path() {
        let request = IpcRequest::Open(PathBuf::from(r"C:\notes\zăpadă.md"));
        assert_eq!(
            decode_frame(&encode_frame(&request).unwrap()).unwrap(),
            request
        );
    }

    #[test]
    fn a_capture_round_trips_its_text_and_attachments() {
        for request in [
            IpcRequest::Capture {
                text: None,
                attachments: Vec::new(),
            },
            IpcRequest::Capture {
                text: Some(String::new()),
                attachments: Vec::new(),
            },
            IpcRequest::Capture {
                text: Some("lag spikes 📈\nsecond line".to_owned()),
                attachments: vec![
                    PathBuf::from(r"C:\shots\a b.png"),
                    PathBuf::from(r"C:\logs\ledger.log"),
                ],
            },
        ] {
            assert_eq!(
                decode_frame(&encode_frame(&request).unwrap()).unwrap(),
                request
            );
        }
    }

    #[test]
    fn a_damaged_capture_is_rejected() {
        let mut payload = 2_u32.to_le_bytes().to_vec();
        payload.extend_from_slice(&[b'a', 0]);
        assert!(decode_frame(&frame(5, payload.len() as u32, &payload)).is_err());
        let mut payload = u32::MAX.to_le_bytes().to_vec();
        payload.extend_from_slice(&0_u32.to_le_bytes());
        payload.push(7);
        assert!(decode_frame(&frame(5, payload.len() as u32, &payload)).is_err());
        let too_long = IpcRequest::Capture {
            text: Some("x".repeat(40_000)),
            attachments: Vec::new(),
        };
        assert!(
            encode_frame(&too_long).is_err(),
            "text past 64 KiB is refused"
        );
    }

    #[test]
    fn oversized_and_unknown_frames_are_rejected() {
        assert!(decode_frame(&vec![0; 65_537]).is_err());
        assert!(decode_frame(b"FPI1\xFF\0\0\0").is_err());
    }

    fn frame(command: u8, declared: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"FPI1".to_vec();
        bytes.push(command);
        bytes.extend_from_slice(&declared.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn rejection(bytes: &[u8]) -> &'static str {
        match decode_frame(bytes) {
            Err(FastPadError::Ipc(reason)) => reason,
            other => panic!("expected an IPC rejection, got {other:?}"),
        }
    }

    #[test]
    fn frame_layout_is_magic_command_little_endian_length_and_utf16le_path() {
        // Break caught: a big-endian length or UTF-8 path silently breaks older/newer secondaries.
        let bytes = encode_frame(&IpcRequest::Open(PathBuf::from("a\u{0103}"))).unwrap();
        assert_eq!(bytes, frame(1, 4, &[b'a', 0, 0x03, 0x01]));
        assert_eq!(encode_frame(&IpcRequest::New).unwrap(), frame(2, 0, &[]));
        assert_eq!(
            encode_frame(&IpcRequest::Activate).unwrap(),
            frame(3, 0, &[])
        );
    }

    #[test]
    fn new_and_activate_round_trip_without_payload() {
        for request in [IpcRequest::New, IpcRequest::Activate] {
            assert_eq!(
                decode_frame(&encode_frame(&request).unwrap()).unwrap(),
                request
            );
        }
    }

    #[test]
    fn strict_decoder_rejects_each_malformed_shape_for_its_own_reason() {
        // Break caught: a lenient decoder that truncates, ignores trailing bytes, or maps unknown
        // commands onto a default request mutates the primary from a malformed client.
        assert_eq!(rejection(&frame(0xFF, 0, &[])), "unknown command");
        assert_eq!(
            rejection(&frame(1, 3, &[b'a', 0, b'b'])),
            "path payload has an odd byte length"
        );
        assert_eq!(
            rejection(&frame(1, 4, &[b'a', 0, 0, 0])),
            "path payload contains NUL"
        );
        assert_eq!(rejection(&frame(1, 0, &[])), "path payload is empty");
        assert_eq!(
            rejection(&frame(1, 2, &[b'a', 0, b'b', 0])),
            "frame length does not match its payload"
        );
        assert_eq!(
            rejection(&frame(1, 4, &[b'a', 0])),
            "frame length does not match its payload"
        );
        assert_eq!(
            rejection(&frame(2, 2, &[b'a', 0])),
            "command does not accept a payload"
        );
        assert_eq!(rejection(b"FPI2\x02\0\0\0\0"), "frame magic is not FPI1");
        assert_eq!(rejection(b"FPI1\x02\0\0\0"), "frame header is truncated");
        assert_eq!(
            rejection(&vec![0; MAX_FRAME_BYTES + 1]),
            "frame exceeds 64 KiB"
        );
    }

    #[test]
    fn largest_path_that_fits_is_accepted_and_one_more_unit_is_refused() {
        // Break caught: an off-by-one limit either rejects valid long paths or emits frames the
        // primary must refuse.
        let units = (MAX_FRAME_BYTES - HEADER_BYTES) / 2;
        let fits = PathBuf::from("a".repeat(units));
        let frame = encode_frame(&IpcRequest::Open(fits.clone())).unwrap();
        assert!(frame.len() <= MAX_FRAME_BYTES);
        assert_eq!(decode_frame(&frame).unwrap(), IpcRequest::Open(fits));
        assert!(encode_frame(&IpcRequest::Open(PathBuf::from("a".repeat(units + 1)))).is_err());
    }

    #[test]
    fn open_folder_frames_carry_the_path_like_open() {
        let request = IpcRequest::OpenFolder(PathBuf::from(r"D:\Notes"));
        let frame = encode_frame(&request).unwrap();
        assert_eq!(frame[FRAME_MAGIC.len()], 4);
        assert_eq!(decode_frame(&frame).unwrap(), request);
        assert!(encode_frame(&IpcRequest::OpenFolder(PathBuf::new())).is_err());
    }

    #[test]
    fn encoder_refuses_paths_the_decoder_would_reject() {
        assert!(encode_frame(&IpcRequest::Open(PathBuf::new())).is_err());
        assert!(encode_frame(&IpcRequest::Open(PathBuf::from("a\0b"))).is_err());
    }
}
