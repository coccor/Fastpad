use std::ffi::OsString;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LaunchOptions {
    pub new_window: bool,
    pub diagnostic: bool,
    pub request: LaunchRequest,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum LaunchRequest {
    #[default]
    New,
    Open(OsString),
    /// `--capture` (second brain phase 1 spec §10.1): the capture box, or text and files filed
    /// with no window.
    Capture(CaptureRequest),
}

/// What `--capture` was given.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CaptureRequest {
    pub text: Option<CaptureText>,
    /// `--attach` paths, as given (made absolute before forwarding).
    pub attachments: Vec<OsString>,
}

impl CaptureRequest {
    /// Nothing to file: show the capture box.
    pub fn opens_box(&self) -> bool {
        self.text.is_none() && self.attachments.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureText {
    Text(String),
    /// `-`: read from standard input.
    Stdin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchError {
    UnknownFlag,
    MultiplePaths,
    /// `--attach` without `--capture`, or without a path after it.
    AttachNeedsCapture,
}

pub fn parse(
    args: impl IntoIterator<Item = OsString>,
) -> core::result::Result<LaunchOptions, LaunchError> {
    let mut options = LaunchOptions::default();
    let mut path = None;
    let mut capture: Option<CaptureRequest> = None;
    let mut attachments = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--new-window") => options.new_window = true,
            Some("--diagnostic") => options.diagnostic = true,
            Some("--capture") if capture.is_none() => capture = Some(CaptureRequest::default()),
            Some("--attach") => match args.next() {
                Some(path) => attachments.push(path),
                None => return Err(LaunchError::AttachNeedsCapture),
            },
            Some("-") if capture.as_ref().is_some_and(|c| c.text.is_none()) => {
                if let Some(capture) = capture.as_mut() {
                    capture.text = Some(CaptureText::Stdin);
                }
            }
            Some(value) if value.starts_with('-') => return Err(LaunchError::UnknownFlag),
            _ if capture.as_ref().is_some_and(|c| c.text.is_none()) && path.is_none() => {
                if let Some(capture) = capture.as_mut() {
                    capture.text = Some(CaptureText::Text(arg.to_string_lossy().into_owned()));
                }
            }
            _ if path.is_some() || capture.is_some() => return Err(LaunchError::MultiplePaths),
            _ => path = Some(arg),
        }
    }

    match capture {
        Some(mut capture) => {
            if path.is_some() {
                return Err(LaunchError::MultiplePaths);
            }
            capture.attachments = attachments;
            options.request = LaunchRequest::Capture(capture);
        }
        None if !attachments.is_empty() => return Err(LaunchError::AttachNeedsCapture),
        None => {
            if let Some(path) = path {
                options.request = LaunchRequest::Open(path);
            }
        }
    }

    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn os(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn capture(text: Option<CaptureText>, attachments: &[&str]) -> LaunchRequest {
        LaunchRequest::Capture(CaptureRequest {
            text,
            attachments: os(attachments),
        })
    }

    #[test]
    fn empty_launch_creates_new_document() {
        assert_eq!(parse(os(&[])), Ok(LaunchOptions::default()));
    }

    #[test]
    fn file_and_flags_are_order_independent() {
        assert_eq!(
            parse(os(&["notes.md", "--diagnostic", "--new-window"])),
            Ok(LaunchOptions {
                new_window: true,
                diagnostic: true,
                request: LaunchRequest::Open(OsString::from("notes.md")),
            })
        );
    }

    #[test]
    fn two_paths_are_rejected() {
        assert_eq!(
            parse(os(&["a.txt", "b.txt"])),
            Err(LaunchError::MultiplePaths)
        );
    }

    #[test]
    fn capture_takes_text_stdin_and_attachments() {
        let request = |args: &[&str]| parse(os(args)).map(|options| options.request);
        assert_eq!(request(&["--capture"]), Ok(capture(None, &[])));
        assert!(matches!(
            request(&["--capture"]),
            Ok(LaunchRequest::Capture(c)) if c.opens_box()
        ));
        assert_eq!(
            request(&["--capture", "call Dana"]),
            Ok(capture(Some(CaptureText::Text("call Dana".into())), &[]))
        );
        assert_eq!(
            request(&["--capture", "-"]),
            Ok(capture(Some(CaptureText::Stdin), &[]))
        );
        assert_eq!(
            request(&["--capture", "lag", "--attach", "a.png", "--attach", "b.log"]),
            Ok(capture(
                Some(CaptureText::Text("lag".into())),
                &["a.png", "b.log"]
            ))
        );
        assert_eq!(
            request(&["--attach", "a.png", "--capture"]),
            Ok(capture(None, &["a.png"]))
        );
    }

    #[test]
    fn capture_refuses_what_it_cannot_use() {
        assert_eq!(
            parse(os(&["--capture", "a", "b"])),
            Err(LaunchError::MultiplePaths)
        );
        assert_eq!(
            parse(os(&["--attach", "a.png"])),
            Err(LaunchError::AttachNeedsCapture)
        );
        assert_eq!(
            parse(os(&["--capture", "--attach"])),
            Err(LaunchError::AttachNeedsCapture)
        );
        assert_eq!(
            parse(os(&["--capture", "--bogus"])),
            Err(LaunchError::UnknownFlag)
        );
    }
}
