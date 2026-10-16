//! Quick capture's text (second brain phase 1 spec §10.3, §10.4): the entry appended to an inbox
//! or daily note, the note written for one-file-per-capture, the names of captured files and the
//! links to them. Nothing here touches the disk.

use super::dates::{self, Date, Time};
use std::path::{Path, PathBuf};

/// What a capture is filed as.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attachment {
    /// Where the copy is, relative to the notebook.
    pub path: PathBuf,
    /// An image is linked as `![](…)`, anything else as `[name](…)`.
    pub image: bool,
}

/// The first up to five words of `text`'s first non-blank line, cleaned for a file name; `None`
/// when nothing is left.
pub fn first_words(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let words: Vec<&str> = line.split_whitespace().take(5).collect();
    let cleaned = super::title::sanitize_stem(&words.join(" "));
    (cleaned != "Untitled" || words.join(" ").eq_ignore_ascii_case("untitled")).then_some(cleaned)
}

/// `YYYY-MM-DD HHmm <first words>`, or with `capture` when the text has no words.
pub fn capture_stem(date: Date, time: Time, text: &str) -> String {
    format!(
        "{} {:02}{:02} {}",
        dates::format(date, "YYYY-MM-DD"),
        time.hour,
        time.minute,
        first_words(text).unwrap_or_else(|| "capture".to_owned())
    )
}

/// `stem.extension`, or `stem 2.extension`, `stem 3.extension`… when `taken` says the name is
/// used.
pub fn unique_name(stem: &str, extension: &str, taken: impl Fn(&str) -> bool) -> String {
    let name = |n: usize| {
        let stem = if n == 1 {
            stem.to_owned()
        } else {
            format!("{stem} {n}")
        };
        if extension.is_empty() {
            stem
        } else {
            format!("{stem}.{extension}")
        }
    };
    (1..)
        .map(name)
        .find(|candidate| !taken(candidate))
        .expect("an unused name")
}

/// One link line per attachment, relative to the note at `note` (both relative to the notebook).
pub fn link_lines(note: &Path, attachments: &[Attachment]) -> Vec<String> {
    attachments
        .iter()
        .map(|attachment| {
            let target = super::links::relative_target(note, &attachment.path, "");
            if attachment.image {
                format!("![]({target})")
            } else {
                let name = attachment
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .replace('[', "\\[")
                    .replace(']', "\\]");
                format!("[{name}]({target})")
            }
        })
        .collect()
}

/// The text to add to the end of `existing` (an inbox or daily note) for a capture of `text` with
/// `links` at `time`, in `newline` line endings: a list item stamped with the time, its other
/// lines and links indented under it, after a blank line unless the file already ends with one
/// or with a list item. `source` follows the time when recorded.
pub fn append_entry(
    existing: &str,
    time: Time,
    text: &str,
    links: &[String],
    source: Option<&str>,
    newline: &str,
) -> String {
    let mut lines: Vec<String> = text
        .trim_end()
        .lines()
        .map(|line| line.trim_end().to_owned())
        .skip_while(String::is_empty)
        .collect();
    lines.extend(links.iter().cloned());
    let stamp = match source {
        Some(source) => format!("{:02}:{:02} ({source})", time.hour, time.minute),
        None => format!("{:02}:{:02}", time.hour, time.minute),
    };
    let mut entry = String::new();
    let mut lines = lines.into_iter();
    match lines.next() {
        Some(first) => entry.push_str(&format!("- {stamp} {first}")),
        None => entry.push_str(&format!("- {stamp}")),
    }
    for line in lines {
        entry.push_str(newline);
        if !line.is_empty() {
            entry.push_str("  ");
            entry.push_str(&line);
        }
    }
    entry.push_str(newline);

    let mut prefix = String::new();
    if !existing.is_empty() && !existing.ends_with('\n') {
        prefix.push_str(newline);
    }
    let last = existing.trim_end_matches(['\r', '\n']).lines().last();
    let ends_blank = existing.is_empty()
        || existing.ends_with("\n\n")
        || existing.ends_with("\r\n\r\n")
        || existing.trim().is_empty();
    let ends_in_list = last.is_some_and(|line| {
        let trimmed = line.trim_start();
        line.starts_with("  ")
            || ["- ", "* ", "+ "]
                .iter()
                .any(|bullet| trimmed.starts_with(bullet))
    });
    if !ends_blank && !ends_in_list {
        prefix.push_str(newline);
    }
    prefix + &entry
}

/// The note for one capture in the capture folder (`folder` target).
pub fn capture_note(
    date: Date,
    time: Time,
    text: &str,
    links: &[String],
    source: Option<&str>,
) -> String {
    let mut note = format!(
        "---\ncaptured: {}T{:02}:{:02}\n",
        dates::format(date, "YYYY-MM-DD"),
        time.hour,
        time.minute
    );
    if let Some(source) = source {
        note.push_str(&format!(
            "source: \"{}\"\n",
            source.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    note.push_str("---\n\n");
    let text = text.trim();
    if !text.is_empty() {
        note.push_str(text);
        note.push('\n');
    }
    if !links.is_empty() {
        if !text.is_empty() {
            note.push('\n');
        }
        for link in links {
            note.push_str(link);
            note.push('\n');
        }
    }
    note
}

/// A new inbox's first text.
pub const INBOX_HEADING: &str = "# Inbox\n";

/// Where Obsidian keeps attachments, from `.obsidian\app.json`'s `attachmentFolderPath`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObsidianAttachments {
    /// `/`: the notebook root.
    Root,
    /// A folder inside the notebook.
    Folder(PathBuf),
    /// `./`: next to the note.
    NextToNote,
    /// `./<folder>`: in that folder under the note's folder.
    UnderNote(PathBuf),
}

/// Reads `attachmentFolderPath` from the text of `.obsidian\app.json`. A path that would leave
/// the notebook gives `None`, as does a file without the key.
pub fn obsidian_attachments(json: &str) -> Option<ObsidianAttachments> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let path = value.get("attachmentFolderPath")?.as_str()?.trim();
    let inside = |rest: &str| -> Option<PathBuf> {
        let mut out = PathBuf::new();
        for part in rest
            .split(['/', '\\'])
            .filter(|part| !part.is_empty() && *part != ".")
        {
            if part == ".." || part.contains(':') {
                return None;
            }
            out.push(part);
        }
        Some(out)
    };
    if path.is_empty() || path == "/" {
        return Some(ObsidianAttachments::Root);
    }
    if path == "./" || path == "." {
        return Some(ObsidianAttachments::NextToNote);
    }
    if let Some(rest) = path.strip_prefix("./") {
        let folder = inside(rest)?;
        return Some(if folder.as_os_str().is_empty() {
            ObsidianAttachments::NextToNote
        } else {
            ObsidianAttachments::UnderNote(folder)
        });
    }
    let folder = inside(path)?;
    Some(if folder.as_os_str().is_empty() {
        ObsidianAttachments::Root
    } else {
        ObsidianAttachments::Folder(folder)
    })
}

/// The folder, relative to the notebook, that attachments for the note at `note` (relative) go
/// in: `notebook.ini`'s `attachments_folder`, else Obsidian's setting, else `attachments`.
pub fn attachments_folder(
    configured: Option<&Path>,
    obsidian: Option<&ObsidianAttachments>,
    note: &Path,
) -> PathBuf {
    if let Some(configured) = configured {
        return configured.to_path_buf();
    }
    let note_folder = note.parent().unwrap_or(Path::new(""));
    match obsidian {
        Some(ObsidianAttachments::Root) => PathBuf::new(),
        Some(ObsidianAttachments::Folder(folder)) => folder.clone(),
        Some(ObsidianAttachments::NextToNote) => note_folder.to_path_buf(),
        Some(ObsidianAttachments::UnderNote(folder)) => note_folder.join(folder),
        None => PathBuf::from("attachments"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u8, minute: u8) -> Time {
        Time { hour, minute }
    }

    fn day() -> Date {
        Date::new(2026, 10, 19).unwrap()
    }

    #[test]
    fn first_words_name_the_capture() {
        assert_eq!(
            first_words("\n  consumer lag spikes when ledger batch runs??\nmore").as_deref(),
            Some("consumer lag spikes when ledger")
        );
        assert_eq!(
            first_words("what? <script>").as_deref(),
            Some("what script")
        );
        assert_eq!(first_words("  \n "), None);
        assert_eq!(
            capture_stem(day(), at(10, 30), ""),
            "2026-10-19 1030 capture"
        );
        assert_eq!(
            capture_stem(day(), at(9, 5), "Call Dana"),
            "2026-10-19 0905 Call Dana"
        );
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let taken = ["a.png", "a 2.png"];
        assert_eq!(unique_name("a", "png", |n| taken.contains(&n)), "a 3.png");
        assert_eq!(unique_name("b", "png", |n| taken.contains(&n)), "b.png");
    }

    #[test]
    fn links_are_relative_to_the_note_and_encode_spaces() {
        let attachments = [
            Attachment {
                path: PathBuf::from(r"attachments\2026-10-19 1030 lag.png"),
                image: true,
            },
            Attachment {
                path: PathBuf::from(r"attachments\ledger-batch.log"),
                image: false,
            },
        ];
        assert_eq!(
            link_lines(Path::new(r"inbox\note.md"), &attachments),
            [
                "![](../attachments/2026-10-19%201030%20lag.png)",
                "[ledger-batch.log](../attachments/ledger-batch.log)"
            ]
        );
        assert_eq!(
            link_lines(Path::new("inbox.md"), &attachments[1..]),
            ["[ledger-batch.log](attachments/ledger-batch.log)"]
        );
    }

    #[test]
    fn an_append_is_a_stamped_list_item_with_indented_lines() {
        let entry = append_entry(
            "# Inbox\n\n- 09:00 earlier\n",
            at(14, 32),
            "Call Dana #project/atlas\nsecond line\n\n",
            &["![](attachments/x.png)".to_owned()],
            None,
            "\n",
        );
        assert_eq!(
            entry,
            "- 14:32 Call Dana #project/atlas\n  second line\n  ![](attachments/x.png)\n"
        );
    }

    #[test]
    fn an_append_after_prose_starts_with_a_blank_line() {
        assert_eq!(
            append_entry("# Inbox\nSome notes", at(8, 0), "x", &[], None, "\r\n"),
            "\r\n\r\n- 08:00 x\r\n"
        );
        assert_eq!(
            append_entry("# Inbox\n\n", at(8, 0), "x", &[], None, "\n"),
            "- 08:00 x\n"
        );
        assert_eq!(
            append_entry("", at(8, 0), "x", &[], None, "\n"),
            "- 08:00 x\n"
        );
        assert_eq!(
            append_entry("- a\n  b", at(8, 0), "x", &[], Some("Chrome"), "\n"),
            "\n- 08:00 (Chrome) x\n"
        );
    }

    #[test]
    fn a_capture_note_has_its_time_in_front_matter() {
        assert_eq!(
            capture_note(
                day(),
                at(10, 30),
                "lag spikes #investigate\n",
                &["![](../attachments/a.png)".to_owned()],
                Some("Grafana \"payments\""),
            ),
            "---\ncaptured: 2026-10-19T10:30\nsource: \"Grafana \\\"payments\\\"\"\n---\n\n\
             lag spikes #investigate\n\n![](../attachments/a.png)\n"
        );
        assert_eq!(
            capture_note(day(), at(10, 30), "", &["[a](a)".to_owned()], None),
            "---\ncaptured: 2026-10-19T10:30\n---\n\n[a](a)\n"
        );
    }

    #[test]
    fn obsidians_attachment_folder_is_honoured_inside_the_vault() {
        let read =
            |path: &str| obsidian_attachments(&format!("{{\"attachmentFolderPath\": \"{path}\"}}"));
        assert_eq!(read("/"), Some(ObsidianAttachments::Root));
        assert_eq!(read("./"), Some(ObsidianAttachments::NextToNote));
        assert_eq!(
            read("./img"),
            Some(ObsidianAttachments::UnderNote(PathBuf::from("img")))
        );
        assert_eq!(
            read("assets/img"),
            Some(ObsidianAttachments::Folder(PathBuf::from(r"assets\img")))
        );
        assert_eq!(read("../outside"), None);
        assert_eq!(read("C:/elsewhere"), None);
        assert_eq!(obsidian_attachments("{}"), None);

        let note = Path::new(r"work\plan.md");
        assert_eq!(
            attachments_folder(None, None, note),
            PathBuf::from("attachments")
        );
        assert_eq!(
            attachments_folder(Some(Path::new("files")), read("/").as_ref(), note),
            PathBuf::from("files")
        );
        assert_eq!(
            attachments_folder(None, read("./img").as_ref(), note),
            PathBuf::from(r"work\img")
        );
    }
}
