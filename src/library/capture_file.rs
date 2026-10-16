//! Filing a quick capture on disk (second brain phase 1 spec §10.3, §10.4): the attachments are
//! copied into the notebook first, then the note is appended to or written. All or nothing: a
//! failure removes the attachments already written. Works without a window, so a capture-only
//! FastPad files captures the same way the main window does.

use super::capture::{self, Attachment, ObsidianAttachments};
use super::dates::{self, Date, Time};
use super::notebook_ini::{CaptureTarget, NotebookSettings};
use super::template::{self, Filled};
use std::path::{Path, PathBuf};

/// A file this large asks before it is copied into the notebook, and `--attach` refuses it.
pub const LARGE_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// Something attached to a capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Source {
    /// A pasted or snipped image, already PNG.
    Png(Vec<u8>),
    /// A file to copy in, keeping its name.
    File(PathBuf),
}

/// One capture to file.
pub struct Request<'a> {
    pub root: &'a Path,
    pub settings: &'a NotebookSettings,
    pub target: CaptureTarget,
    pub date: Date,
    pub time: Time,
    pub text: &'a str,
    pub attachments: &'a [Source],
    /// The window in front when the capture began, when recorded.
    pub source: Option<&'a str>,
}

/// Where a capture went.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Filed {
    /// The note, relative to the notebook.
    pub note: PathBuf,
    /// The 0-based line the capture starts on.
    pub line: usize,
}

/// A note that is open in a tab of the running FastPad gets the capture in its buffer, as one
/// undo step, rather than on disk under the tab.
pub trait OpenTabs {
    /// The text of the tab showing `path` (absolute), if one does.
    fn text(&self, path: &Path) -> Option<String>;
    /// Adds `text` to the end of that tab's buffer.
    fn append(&mut self, path: &Path, text: &str) -> Result<(), String>;
}

/// No tabs: the capture-only process, and tests.
pub struct NoTabs;

impl OpenTabs for NoTabs {
    fn text(&self, _path: &Path) -> Option<String> {
        None
    }
    fn append(&mut self, _path: &Path, _text: &str) -> Result<(), String> {
        Err("no tab".to_owned())
    }
}

/// The notebook a capture goes to when no FastPad window has one open (spec §10.3): the most
/// recent notebook in `folders.ini` that still exists, else `fallback` (`Documents\FastPad`).
pub fn capture_root(data_dir: Option<&Path>, fallback: Option<PathBuf>) -> Option<PathBuf> {
    let recent = data_dir
        .map(|data| super::local::read_folders(&super::local::folders_file(data)))
        .unwrap_or_default();
    recent
        .folders
        .iter()
        .find(|folder| super::folder_exists(folder))
        .cloned()
        .or(fallback)
        .map(|folder| super::normalize_folder(&folder))
}

/// The conventions of the notebook at `root`, from its `notebook.ini` (defaults when it has none).
pub fn notebook_settings_of(root: &Path) -> NotebookSettings {
    std::fs::read(super::notebook_ini::notebook_file(root))
        .map(|bytes| super::notebook_ini::parse(&String::from_utf8_lossy(&bytes)).0)
        .unwrap_or_default()
}

/// What the capture box's title calls `target` in a notebook with `settings`.
pub fn target_name(settings: &NotebookSettings, target: CaptureTarget) -> String {
    match target {
        CaptureTarget::Inbox => settings
            .inbox
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        CaptureTarget::Daily => "today's note".to_owned(),
        CaptureTarget::Folder => format!("{}\\", settings.capture_folder.display()),
    }
}

/// Obsidian's attachment folder for the vault at `root`, from `.obsidian\app.json`.
pub fn obsidian_attachments_of(root: &Path) -> Option<ObsidianAttachments> {
    let text = std::fs::read_to_string(root.join(".obsidian").join("app.json")).ok()?;
    capture::obsidian_attachments(&text)
}

/// Today's daily note, relative to the notebook (spec §9.2).
pub fn daily_note(settings: &NotebookSettings, date: Date) -> PathBuf {
    settings.daily_folder.join(super::title::file_name(
        &dates::format(date, &settings.daily_format),
        "md",
    ))
}

/// A new daily note's text: the daily template filled in, or the date as a heading. The
/// second value is a problem to report (an unreadable or missing named template).
pub fn daily_start(
    root: &Path,
    settings: &NotebookSettings,
    date: Date,
    time: Time,
) -> (Filled, Option<String>) {
    let template = root.join(&settings.daily_template);
    let title = dates::format(date, &settings.daily_format);
    if template.is_file() {
        return match crate::file::loader::load(&template) {
            Ok(file) => (
                template::fill(
                    &file.text,
                    &template::Context {
                        date,
                        time,
                        daily_format: &settings.daily_format,
                        title: &title,
                    },
                ),
                None,
            ),
            Err(error) => (
                template::default_daily(date),
                Some(format!(
                    "FastPad could not read the template {}: {error}",
                    template.display()
                )),
            ),
        };
    }
    let named = settings.daily_template != NotebookSettings::default().daily_template;
    (
        template::default_daily(date),
        named.then(|| {
            format!(
                "The daily note template {} doesn't exist, so today's note starts with its date.",
                settings.daily_template.display()
            )
        }),
    )
}

/// The line ending most of `text`'s lines use; `\n` for text without any.
pub fn dominant_newline(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    if crlf > lf { "\r\n" } else { "\n" }
}

/// The note the capture goes to, relative to the notebook. A one-file capture gets a name no
/// file has yet.
pub fn note_for(request: &Request<'_>) -> PathBuf {
    match request.target {
        CaptureTarget::Inbox => request.settings.inbox.clone(),
        CaptureTarget::Daily => daily_note(request.settings, request.date),
        CaptureTarget::Folder => {
            let folder = &request.settings.capture_folder;
            let stem = capture::capture_stem(request.date, request.time, request.text);
            let name = capture::unique_name(&stem, "md", |name| {
                request.root.join(folder).join(name).exists()
            });
            folder.join(name)
        }
    }
}

/// Removes attachments written for a capture that then failed.
pub fn remove_attachments(root: &Path, attachments: &[Attachment]) {
    for attachment in attachments {
        let _ = std::fs::remove_file(root.join(&attachment.path));
    }
}

/// Whether `path`'s extension names an image FastPad links with `![](…)`.
pub fn is_image(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        ["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg"]
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
    })
}

/// Copies or writes the attachments into the attachments folder for `note`, never over a file.
/// A failure removes the ones already written.
pub fn save_attachments(request: &Request<'_>, note: &Path) -> Result<Vec<Attachment>, String> {
    let obsidian = obsidian_attachments_of(request.root);
    let folder = capture::attachments_folder(
        request.settings.attachments_folder.as_deref(),
        obsidian.as_ref(),
        note,
    );
    let directory = request.root.join(&folder);
    let mut written: Vec<Attachment> = Vec::new();
    let image_stem = capture::capture_stem(request.date, request.time, request.text);
    for source in request.attachments {
        let (stem, extension, bytes) = match source {
            Source::Png(bytes) => (image_stem.clone(), "png".to_owned(), Ok(bytes.clone())),
            Source::File(path) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let (stem, extension) = match name.rsplit_once('.') {
                    Some((stem, extension)) if !stem.is_empty() => {
                        (stem.to_owned(), extension.to_owned())
                    }
                    _ => (name.clone().into_owned(), String::new()),
                };
                (
                    super::title::sanitize_stem(&stem),
                    extension,
                    std::fs::read(path).map_err(|error| {
                        format!("FastPad could not read {}: {error}", path.display())
                    }),
                )
            }
        };
        let saved = bytes.and_then(|bytes| {
            std::fs::create_dir_all(&directory).map_err(|error| {
                format!(
                    "FastPad could not create the folder {}: {error}",
                    directory.display()
                )
            })?;
            loop {
                let name =
                    capture::unique_name(&stem, &extension, |name| directory.join(name).exists());
                match crate::file::saver::save_atomic_new(&directory.join(&name), &bytes) {
                    Ok(()) => return Ok(name),
                    // Something took the name between the check and the write: try the next.
                    Err(error) if crate::file::saver::is_already_exists(&error) => {}
                    Err(error) => {
                        return Err(format!("FastPad could not save {name}: {error}"));
                    }
                }
            }
        });
        match saved {
            Ok(name) => written.push(Attachment {
                image: matches!(source, Source::Png(_)) || is_image(Path::new(&name)),
                path: folder.join(name),
            }),
            Err(error) => {
                remove_attachments(request.root, &written);
                return Err(error);
            }
        }
    }
    Ok(written)
}

/// Files the capture: attachments first, then the note. `tabs` takes the append when the note
/// is open in one. The error is for the capture box (or a notice, or stderr).
pub fn file_capture(request: &Request<'_>, tabs: &mut dyn OpenTabs) -> Result<Filed, String> {
    let note = note_for(request);
    let attachments = save_attachments(request, &note)?;
    let links = capture::link_lines(&note, &attachments);
    let filed = match request.target {
        CaptureTarget::Folder => write_capture_note(request, &note, &links),
        CaptureTarget::Inbox | CaptureTarget::Daily => append_capture(request, &note, &links, tabs),
    };
    if filed.is_err() {
        remove_attachments(request.root, &attachments);
    }
    filed
}

fn write_capture_note(
    request: &Request<'_>,
    note: &Path,
    links: &[String],
) -> Result<Filed, String> {
    let path = request.root.join(note);
    let text = capture::capture_note(
        request.date,
        request.time,
        request.text,
        links,
        request.source,
    );
    let made = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .map_err(crate::FastPadError::Io)
        .and_then(|()| crate::file::saver::save_atomic_new(&path, text.as_bytes()));
    made.map_err(|error| format!("FastPad could not save {}: {error}", path.display()))?;
    let line = text
        .lines()
        .position(|line| line.is_empty())
        .map_or(0, |blank| blank + 1);
    Ok(Filed {
        note: note.to_path_buf(),
        line,
    })
}

/// The line `entry` starts on once added to the end of `existing`: after its leading blank
/// lines.
fn entry_line(existing: &str, entry: &str) -> usize {
    let combined = format!("{existing}{entry}");
    let start = existing.len() + (entry.len() - entry.trim_start_matches(['\r', '\n']).len());
    combined[..start].matches('\n').count()
}

fn append_capture(
    request: &Request<'_>,
    note: &Path,
    links: &[String],
    tabs: &mut dyn OpenTabs,
) -> Result<Filed, String> {
    let path = request.root.join(note);
    let entry_for = |existing: &str| {
        let newline = if existing.is_empty() {
            "\n"
        } else {
            dominant_newline(existing)
        };
        capture::append_entry(
            existing,
            request.time,
            request.text,
            links,
            request.source,
            newline,
        )
    };
    if let Some(existing) = tabs.text(&path) {
        let entry = entry_for(&existing);
        tabs.append(&path, &entry)?;
        return Ok(Filed {
            note: note.to_path_buf(),
            line: entry_line(&existing, &entry),
        });
    }
    let fail = |error: &dyn std::fmt::Display| {
        format!(
            "FastPad could not add the capture to {}: {error}",
            path.display()
        )
    };
    for _ in 0..3 {
        let before = super::disk_stamp(&path);
        let (existing, encoding) = if path.exists() {
            let file = crate::file::loader::load(&path).map_err(|error| fail(&error))?;
            (file.text, file.encoding)
        } else {
            let start = match request.target {
                CaptureTarget::Daily => {
                    daily_start(request.root, request.settings, request.date, request.time)
                        .0
                        .text
                }
                _ => capture::INBOX_HEADING.to_owned(),
            };
            (start, crate::file::encoding::Encoding::default())
        };
        let entry = entry_for(&existing);
        let text = format!("{existing}{entry}");
        let bytes = crate::file::encoding::encode(&text, encoding);
        // Someone else wrote the file while it was read: read it again.
        if before.is_some() && super::disk_stamp(&path) != before {
            continue;
        }
        let saved = if before.is_some() {
            crate::file::saver::save_atomic(&path, &bytes)
        } else {
            path.parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .map_err(crate::FastPadError::Io)
                .and_then(|()| crate::file::saver::save_atomic_new(&path, &bytes))
        };
        match saved {
            Ok(()) => {
                return Ok(Filed {
                    note: note.to_path_buf(),
                    line: entry_line(&existing, &entry),
                });
            }
            // Another capture made the file first: append to it instead.
            Err(error) if crate::file::saver::is_already_exists(&error) => {}
            Err(error) => return Err(fail(&error)),
        }
    }
    Err(fail(&"it kept changing while FastPad wrote to it"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("fastpad-capture-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn read(&self, relative: &str) -> String {
            std::fs::read_to_string(self.0.join(relative)).unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn request<'a>(
        root: &'a Path,
        settings: &'a NotebookSettings,
        target: CaptureTarget,
        text: &'a str,
        attachments: &'a [Source],
    ) -> Request<'a> {
        Request {
            root,
            settings,
            target,
            date: Date::new(2026, 10, 19).unwrap(),
            time: Time {
                hour: 10,
                minute: 30,
            },
            text,
            attachments,
            source: None,
        }
    }

    #[test]
    fn a_missing_inbox_is_made_with_its_heading() {
        let scratch = Scratch::new("inbox-new");
        let settings = NotebookSettings::default();
        let filed = file_capture(
            &request(
                &scratch.0,
                &settings,
                CaptureTarget::Inbox,
                "Call Dana\nabout it",
                &[],
            ),
            &mut NoTabs,
        )
        .unwrap();
        assert_eq!(
            scratch.read("inbox.md"),
            "# Inbox\n\n- 10:30 Call Dana\n  about it\n"
        );
        assert_eq!(
            filed,
            Filed {
                note: PathBuf::from("inbox.md"),
                line: 2
            }
        );
    }

    #[test]
    fn an_append_keeps_the_files_encoding_and_line_endings() {
        let scratch = Scratch::new("inbox-crlf");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"# Inbox\r\n\r\n- 09:00 x\r\n");
        std::fs::write(scratch.0.join("inbox.md"), &bytes).unwrap();
        let settings = NotebookSettings::default();
        file_capture(
            &request(&scratch.0, &settings, CaptureTarget::Inbox, "y", &[]),
            &mut NoTabs,
        )
        .unwrap();
        let written = std::fs::read(scratch.0.join("inbox.md")).unwrap();
        assert_eq!(&written[..3], &[0xEF, 0xBB, 0xBF], "the BOM is kept");
        assert_eq!(
            String::from_utf8_lossy(&written[3..]),
            "# Inbox\r\n\r\n- 09:00 x\r\n- 10:30 y\r\n"
        );
    }

    #[test]
    fn a_daily_capture_makes_todays_note_first() {
        let scratch = Scratch::new("daily");
        let settings = NotebookSettings::default();
        let filed = file_capture(
            &request(&scratch.0, &settings, CaptureTarget::Daily, "idea", &[]),
            &mut NoTabs,
        )
        .unwrap();
        assert_eq!(
            scratch.read(r"journal\2026-10-19.md"),
            "# Monday, 19 October 2026\n\n- 10:30 idea\n"
        );
        assert_eq!(filed.line, 2);
    }

    #[test]
    fn one_file_per_capture_with_attachments_linked_from_it() {
        let scratch = Scratch::new("folder");
        let log = scratch.0.join("outside.log");
        std::fs::write(&log, "log").unwrap();
        let settings = NotebookSettings::default();
        let sources = [Source::Png(b"png".to_vec()), Source::File(log.clone())];
        let filed = file_capture(
            &request(
                &scratch.0,
                &settings,
                CaptureTarget::Folder,
                "lag spikes",
                &sources,
            ),
            &mut NoTabs,
        )
        .unwrap();
        assert_eq!(
            filed.note,
            PathBuf::from(r"inbox\2026-10-19 1030 lag spikes.md")
        );
        assert_eq!(
            scratch.read(r"inbox\2026-10-19 1030 lag spikes.md"),
            "---\ncaptured: 2026-10-19T10:30\n---\n\nlag spikes\n\n\
             ![](../attachments/2026-10-19%201030%20lag%20spikes.png)\n\
             [outside.log](../attachments/outside.log)\n"
        );
        assert_eq!(filed.line, 4);
        assert_eq!(scratch.read(r"attachments\outside.log"), "log");
        assert!(log.exists(), "the original is copied, not moved");

        let again = file_capture(
            &request(
                &scratch.0,
                &settings,
                CaptureTarget::Folder,
                "lag spikes",
                &sources,
            ),
            &mut NoTabs,
        )
        .unwrap();
        assert_eq!(
            again.note,
            PathBuf::from(r"inbox\2026-10-19 1030 lag spikes 2.md")
        );
        assert!(scratch.0.join(r"attachments\outside 2.log").exists());
    }

    #[test]
    fn a_failed_attachment_leaves_nothing_behind() {
        let scratch = Scratch::new("rollback");
        let settings = NotebookSettings::default();
        let sources = [
            Source::Png(b"png".to_vec()),
            Source::File(scratch.0.join("missing.pdf")),
        ];
        let error = file_capture(
            &request(&scratch.0, &settings, CaptureTarget::Inbox, "x", &sources),
            &mut NoTabs,
        )
        .unwrap_err();
        assert!(error.contains("missing.pdf"), "{error}");
        let left: Vec<_> = std::fs::read_dir(scratch.0.join("attachments"))
            .unwrap()
            .collect();
        assert!(left.is_empty(), "the image written first is removed");
        assert!(!scratch.0.join("inbox.md").exists(), "no note is written");
    }

    #[test]
    fn an_open_tab_takes_the_append() {
        struct Tab(String);
        impl OpenTabs for Tab {
            fn text(&self, path: &Path) -> Option<String> {
                path.ends_with("inbox.md").then(|| self.0.clone())
            }
            fn append(&mut self, _path: &Path, text: &str) -> Result<(), String> {
                self.0.push_str(text);
                Ok(())
            }
        }
        let scratch = Scratch::new("tab");
        let settings = NotebookSettings::default();
        let mut tab = Tab("# Inbox\n\nprose".to_owned());
        let filed = file_capture(
            &request(&scratch.0, &settings, CaptureTarget::Inbox, "x", &[]),
            &mut tab,
        )
        .unwrap();
        assert_eq!(tab.0, "# Inbox\n\nprose\n\n- 10:30 x\n");
        assert_eq!(filed.line, 4);
        assert!(
            !scratch.0.join("inbox.md").exists(),
            "the disk is left to the tab"
        );
    }

    #[test]
    fn the_most_common_line_ending_wins() {
        assert_eq!(dominant_newline("a\r\nb\r\nc\n"), "\r\n");
        assert_eq!(dominant_newline("a\nb\r\nc\n"), "\n");
        assert_eq!(dominant_newline("abc"), "\n");
    }
}
