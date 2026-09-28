//! The session manifest: which tabs the primary window had open when it closed, and how the editor
//! area was split, so the next launch can reopen them. Unsaved text is never stored here. It stays
//! in the Recovery snapshots the manifest names, so a lost or corrupt manifest still leaves crash
//! recovery to bring it back.
//!
//! Format (UTF-8, one `key=value` per line), version 2 (split editors spec §8): `version=2`,
//! `layout=<split tree>`, `active_group=<zero-based group index>`, then per editor group a
//! `group=<number>|active=<zero-based entry index>` line followed by that group's tabs in strip
//! order, one `file=<caret>|<anchor>|<first visible line>|<path>` or
//! `snapshot=<caret>|<anchor>|<first visible line>|<recovery id as 32 hex digits>` each. The
//! target is always the last field, so a path containing `|` still parses. The split tree is
//! `row(<child>:<ratio>,...)`, `column(...)`, or a bare group number.
//!
//! Version 1 (0.2.0) had no groups: `active=<index>` and the entry lines. It is read as one group.

use crate::Result;
use crate::document::{DocumentId, RecoveryId};
use std::path::{Path, PathBuf};

const VERSION: &str = "2";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionSource {
    /// A clean file, reopened from disk.
    File(PathBuf),
    /// An unsaved tab, reopened from the Recovery snapshot with this ID.
    Snapshot(RecoveryId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionEntry {
    pub source: SessionSource,
    pub caret: usize,
    pub anchor: usize,
    pub first_line: usize,
}

impl SessionEntry {
    pub fn new(source: SessionSource) -> Self {
        Self {
            source,
            caret: 0,
            anchor: 0,
            first_line: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionAxis {
    /// Children side by side, left to right.
    Row,
    /// Children stacked, top to bottom.
    Column,
}

/// How the editor area was split: a group, or a row or column of parts with their shares.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionLayout {
    /// The group whose `group=` line has this number.
    Leaf(usize),
    Branch {
        axis: SessionAxis,
        children: Vec<(SessionLayout, f32)>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionGroup {
    /// The name `layout` uses for this group.
    pub number: usize,
    /// The index in `entries` of the group's active tab.
    pub active: usize,
    pub entries: Vec<SessionEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub layout: SessionLayout,
    /// The index in `groups` of the active group.
    pub active_group: usize,
    pub groups: Vec<SessionGroup>,
}

impl Session {
    /// One group holding `entries`, with the one at `active` active.
    pub fn single(active: usize, entries: Vec<SessionEntry>) -> Self {
        Self {
            layout: SessionLayout::Leaf(1),
            active_group: 0,
            groups: vec![SessionGroup {
                number: 1,
                active,
                entries,
            }],
        }
    }

    /// Whether there is no tab to reopen.
    pub fn is_empty(&self) -> bool {
        self.groups.iter().all(|group| group.entries.is_empty())
    }

    /// Every entry, group after group, and the index of the active group's active entry among them.
    pub fn flattened(&self) -> (usize, Vec<SessionEntry>) {
        let before = self
            .groups
            .iter()
            .take(self.active_group)
            .map(|group| group.entries.len())
            .sum::<usize>();
        let active = before
            + self
                .groups
                .get(self.active_group)
                .map_or(0, |group| group.active);
        let entries = self
            .groups
            .iter()
            .flat_map(|group| group.entries.iter().cloned())
            .collect();
        (active, entries)
    }

    pub fn encode(&self) -> String {
        let mut output = format!(
            "version={VERSION}\r\nlayout={}\r\nactive_group={}\r\n",
            self.layout.encode(),
            self.active_group
        );
        for group in &self.groups {
            output.push_str(&format!(
                "group={}|active={}\r\n",
                group.number, group.active
            ));
            for entry in &group.entries {
                let (key, target) = match &entry.source {
                    SessionSource::File(path) => ("file", path.to_string_lossy().into_owned()),
                    SessionSource::Snapshot(id) => ("snapshot", format!("{:032x}", id.0)),
                };
                output.push_str(&format!(
                    "{key}={}|{}|{}|{target}\r\n",
                    entry.caret, entry.anchor, entry.first_line
                ));
            }
        }
        output
    }

    /// `None` for anything but a version 1 or 2 manifest. Malformed entry lines and unknown keys
    /// are skipped. A layout that fails to parse or doesn't name exactly the groups present puts
    /// every tab into one group in file order. Groups left without tabs leave the layout, and
    /// out-of-range active indices fall back to the first.
    pub fn parse(source: &str) -> Option<Self> {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let mut version = None;
        let mut active = 0;
        let mut layout = None;
        let mut active_group = 0;
        let mut loose = Vec::new();
        let mut groups: Vec<SessionGroup> = Vec::new();
        // Entry lines after a malformed `group=` line belong to no group and are dropped.
        let mut in_group = false;
        for line in source.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "version" => version = Some(value),
                "active" => active = value.parse().unwrap_or(0),
                "layout" => layout = Some(value),
                "active_group" => active_group = value.parse().unwrap_or(0),
                "group" => match parse_group(value) {
                    Some(group) => {
                        groups.push(group);
                        in_group = true;
                    }
                    None => in_group = false,
                },
                "file" | "snapshot" => {
                    if let Some(entry) = parse_entry(key, value) {
                        match groups.last_mut() {
                            Some(group) if in_group => group.entries.push(entry),
                            _ => loose.push(entry),
                        }
                    }
                }
                _ => {}
            }
        }
        let mut session = match version? {
            "1" => Self::single(active, loose),
            VERSION => {
                let layout = layout
                    .and_then(SessionLayout::parse)
                    .filter(|layout| layout.names_exactly(&groups));
                match layout {
                    Some(layout) => Self {
                        layout,
                        active_group,
                        groups,
                    },
                    None => Self::single(
                        0,
                        groups.into_iter().flat_map(|group| group.entries).collect(),
                    ),
                }
            }
            _ => return None,
        };
        session.drop_empty_groups();
        if session.active_group >= session.groups.len() {
            session.active_group = 0;
        }
        for group in &mut session.groups {
            if group.active >= group.entries.len() {
                group.active = 0;
            }
        }
        Some(session)
    }

    fn drop_empty_groups(&mut self) {
        let active = self.groups.get(self.active_group).map(|group| group.number);
        let empty = self
            .groups
            .iter()
            .filter(|group| group.entries.is_empty())
            .map(|group| group.number)
            .collect::<Vec<_>>();
        for number in empty {
            self.layout = self
                .layout
                .without(number)
                .unwrap_or(SessionLayout::Leaf(1));
        }
        self.groups.retain(|group| !group.entries.is_empty());
        self.active_group = active
            .and_then(|number| self.groups.iter().position(|group| group.number == number))
            .unwrap_or(0);
    }
}

impl SessionLayout {
    pub fn encode(&self) -> String {
        match self {
            Self::Leaf(number) => number.to_string(),
            Self::Branch { axis, children } => {
                let name = match axis {
                    SessionAxis::Row => "row",
                    SessionAxis::Column => "column",
                };
                let children = children
                    .iter()
                    .map(|(child, ratio)| format!("{}:{}", child.encode(), format_ratio(*ratio)))
                    .collect::<Vec<_>>()
                    .join(",");
                format!("{name}({children})")
            }
        }
    }

    /// `None` unless all of `text` is one well-formed tree with positive ratios and no group named
    /// twice. A branch whose ratios don't sum to 1 is scaled so they do.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parser = LayoutParser {
            text: text.as_bytes(),
            position: 0,
        };
        let layout = parser.node()?;
        if parser.position != text.len() {
            return None;
        }
        let mut leaves = layout.leaves();
        let count = leaves.len();
        leaves.sort_unstable();
        leaves.dedup();
        (leaves.len() == count).then_some(layout)
    }

    /// The group numbers, in layout order.
    pub fn leaves(&self) -> Vec<usize> {
        match self {
            Self::Leaf(number) => vec![*number],
            Self::Branch { children, .. } => children
                .iter()
                .flat_map(|(child, _)| child.leaves())
                .collect(),
        }
    }

    fn names_exactly(&self, groups: &[SessionGroup]) -> bool {
        let mut leaves = self.leaves();
        let mut numbers = groups.iter().map(|group| group.number).collect::<Vec<_>>();
        leaves.sort_unstable();
        numbers.sort_unstable();
        leaves == numbers
    }

    /// This layout without group `number`: its share goes to its siblings, and a branch left with
    /// one part becomes that part. `None` when nothing is left.
    pub fn without(&self, number: usize) -> Option<Self> {
        match self {
            Self::Leaf(leaf) => (*leaf != number).then(|| self.clone()),
            Self::Branch { axis, children } => {
                let mut kept = children
                    .iter()
                    .filter_map(|(child, ratio)| Some((child.without(number)?, *ratio)))
                    .collect::<Vec<_>>();
                match kept.len() {
                    0 => None,
                    1 => kept.pop().map(|(child, _)| child),
                    _ => {
                        normalize(&mut kept);
                        Some(Self::Branch {
                            axis: *axis,
                            children: kept,
                        })
                    }
                }
            }
        }
    }
}

fn format_ratio(ratio: f32) -> String {
    let text = format!("{ratio:.4}");
    let text = text.trim_end_matches('0');
    text.trim_end_matches('.').to_owned()
}

/// Scales the shares to sum to 1, unless they already do to within rounding.
fn normalize(children: &mut [(SessionLayout, f32)]) {
    let sum = children.iter().map(|(_, ratio)| ratio).sum::<f32>();
    if sum > 0.0 && (sum - 1.0).abs() > 1e-4 {
        for (_, ratio) in children.iter_mut() {
            *ratio /= sum;
        }
    }
}

struct LayoutParser<'a> {
    text: &'a [u8],
    position: usize,
}

impl LayoutParser<'_> {
    fn node(&mut self) -> Option<SessionLayout> {
        let axis = if self.eat(b"row(") {
            SessionAxis::Row
        } else if self.eat(b"column(") {
            SessionAxis::Column
        } else {
            return Some(SessionLayout::Leaf(self.number()?));
        };
        let mut children = Vec::new();
        loop {
            let child = self.node()?;
            if !self.eat(b":") {
                return None;
            }
            let ratio = self.ratio()?;
            children.push((child, ratio));
            if self.eat(b",") {
                continue;
            }
            if self.eat(b")") {
                break;
            }
            return None;
        }
        normalize(&mut children);
        Some(SessionLayout::Branch { axis, children })
    }

    fn eat(&mut self, token: &[u8]) -> bool {
        let matched = self.text[self.position..].starts_with(token);
        if matched {
            self.position += token.len();
        }
        matched
    }

    fn take_while(&mut self, accept: impl Fn(u8) -> bool) -> &str {
        let start = self.position;
        while self
            .text
            .get(self.position)
            .is_some_and(|byte| accept(*byte))
        {
            self.position += 1;
        }
        std::str::from_utf8(&self.text[start..self.position]).unwrap_or_default()
    }

    fn number(&mut self) -> Option<usize> {
        self.take_while(|byte| byte.is_ascii_digit()).parse().ok()
    }

    fn ratio(&mut self) -> Option<f32> {
        let ratio = self
            .take_while(|byte| byte.is_ascii_digit() || byte == b'.')
            .parse::<f32>()
            .ok()?;
        (ratio.is_finite() && ratio > 0.0).then_some(ratio)
    }
}

/// `<number>|active=<index>`.
fn parse_group(value: &str) -> Option<SessionGroup> {
    let (number, active) = value.split_once('|')?;
    Some(SessionGroup {
        number: number.parse().ok()?,
        active: active.strip_prefix("active=")?.parse().ok()?,
        entries: Vec::new(),
    })
}

fn parse_entry(key: &str, value: &str) -> Option<SessionEntry> {
    let mut fields = value.splitn(4, '|');
    let caret = fields.next()?.parse().ok()?;
    let anchor = fields.next()?.parse().ok()?;
    let first_line = fields.next()?.parse().ok()?;
    let target = fields.next()?;
    let source = if key == "file" {
        if target.is_empty() {
            return None;
        }
        SessionSource::File(PathBuf::from(target))
    } else {
        if target.len() != 32 || !target.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        SessionSource::Snapshot(RecoveryId::from_u128(
            u128::from_str_radix(target, 16).ok()?,
        ))
    };
    Some(SessionEntry {
        source,
        caret,
        anchor,
        first_line,
    })
}

/// `%LocalAppData%\FastPad\session.ini`, next to `fastpad.ini`.
pub fn session_file_path() -> Result<PathBuf> {
    Ok(crate::platform::paths::fastpad_data_dir()?.join("session.ini"))
}

pub fn write(path: &Path, session: &Session) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::file::saver::save_atomic(path, session.encode().as_bytes())
}

pub fn read(path: &Path) -> Option<Session> {
    Session::parse(&std::fs::read_to_string(path).ok()?)
}

pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Progress through a manifest being reopened, one entry per `WM_FASTPAD_RESTORE_SESSION`. With a
/// single editor group, every group's entries are reopened into it in order.
#[derive(Debug)]
pub struct SessionRestore {
    pub entries: Vec<SessionEntry>,
    /// The index in `entries` of the saved active tab.
    pub active: usize,
    pub next: usize,
    pub failed: usize,
    /// The tab each processed entry became, by entry index. `None` when the entry failed.
    pub restored: Vec<Option<DocumentId>>,
    /// The empty startup tab, closed at the end once something else was restored.
    pub placeholder: Option<DocumentId>,
}

impl SessionRestore {
    pub fn new(session: &Session, placeholder: Option<DocumentId>) -> Self {
        let (active, entries) = session.flattened();
        Self {
            entries,
            active,
            next: 0,
            failed: 0,
            restored: Vec::new(),
            placeholder,
        }
    }

    pub fn next_entry(&self) -> Option<&SessionEntry> {
        self.entries.get(self.next)
    }

    pub fn record(&mut self, restored: Option<DocumentId>) {
        if restored.is_none() {
            self.failed += 1;
        }
        self.restored.push(restored);
        self.next += 1;
    }

    /// The tab the saved active entry became, if that entry was restored.
    pub fn saved_active_restored(&self) -> Option<DocumentId> {
        self.restored.get(self.active).copied().flatten()
    }

    /// The tab to show at the end: the saved active one, else the last one restored.
    pub fn active_tab(&self) -> Option<DocumentId> {
        self.saved_active_restored()
            .or_else(|| self.restored.iter().rev().find_map(|id| *id))
    }
}

pub fn restore_failure_notice(count: usize) -> String {
    if count == 1 {
        "1 item from the last session could not be reopened.".to_owned()
    } else {
        format!("{count} items from the last session could not be reopened.")
    }
}

pub fn toggle_notice(enabled: bool) -> &'static str {
    if enabled {
        "Session restore is on. Open tabs will reopen on the next launch."
    } else {
        "Session restore is off. Closing FastPad will ask about unsaved changes."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Session {
        Session {
            layout: SessionLayout::Branch {
                axis: SessionAxis::Row,
                children: vec![
                    (SessionLayout::Leaf(1), 0.5),
                    (
                        SessionLayout::Branch {
                            axis: SessionAxis::Column,
                            children: vec![
                                (SessionLayout::Leaf(2), 0.6),
                                (SessionLayout::Leaf(3), 0.4),
                            ],
                        },
                        0.5,
                    ),
                ],
            },
            active_group: 1,
            groups: vec![
                SessionGroup {
                    number: 1,
                    active: 0,
                    entries: vec![SessionEntry::new(SessionSource::File(PathBuf::from(
                        r"C:\notes\a b|c.txt",
                    )))],
                },
                SessionGroup {
                    number: 2,
                    active: 0,
                    entries: vec![SessionEntry {
                        source: SessionSource::Snapshot(RecoveryId::from_u128(0xabc)),
                        caret: 12,
                        anchor: 4,
                        first_line: 3,
                    }],
                },
                SessionGroup {
                    number: 3,
                    active: 0,
                    entries: vec![SessionEntry::new(SessionSource::File(PathBuf::from(
                        r"C:\x.md",
                    )))],
                },
            ],
        }
    }

    #[test]
    fn a_manifest_round_trips_through_its_text_form() {
        // Break caught: a layout, a group boundary, a path with `|` or a snapshot id that does not
        // survive a write and read.
        let text = sample().encode();
        assert!(text.starts_with("version=2\r\n"));
        assert!(text.contains("layout=row(1:0.5,column(2:0.6,3:0.4):0.5)\r\n"));
        assert!(
            text.contains(
                "group=2|active=0\r\nsnapshot=12|4|3|00000000000000000000000000000abc\r\n"
            )
        );
        assert_eq!(Session::parse(&text), Some(sample()));
    }

    #[test]
    fn a_version_one_manifest_reads_as_one_group() {
        // Break caught: upgrading from 0.2.0 opening to an empty window.
        let parsed = Session::parse(
            "version=1\r\nactive=1\r\nfile=0|0|0|C:\\a.txt\r\nfile=5|2|1|C:\\b.txt\r\n",
        )
        .unwrap();
        assert_eq!(parsed.layout, SessionLayout::Leaf(1));
        assert_eq!(parsed.groups.len(), 1);
        assert_eq!(parsed.groups[0].active, 1);
        assert_eq!(parsed.groups[0].entries[1].caret, 5);
        assert_eq!(parsed.flattened().0, 1);
    }

    #[test]
    fn only_versions_one_and_two_are_accepted() {
        // Break caught: a future or hand-damaged manifest restoring garbage instead of nothing.
        assert_eq!(Session::parse("active=0\r\nfile=0|0|0|C:\\a.txt\r\n"), None);
        assert_eq!(
            Session::parse("version=3\r\nfile=0|0|0|C:\\a.txt\r\n"),
            None
        );
        assert!(Session::parse("\u{feff}version=2\r\n").is_some());
    }

    #[test]
    fn a_bad_or_mismatched_layout_falls_back_to_one_group_in_file_order() {
        // Break caught: a damaged layout line losing every tab of the session.
        for layout in [
            "row(1:0.5",
            "row(1:0.5,9:0.5)",
            "column()",
            "row(1:x,2:1)",
            "",
        ] {
            let text = format!(
                "version=2\r\nlayout={layout}\r\nactive_group=1\r\ngroup=1|active=0\r\n\
                 file=0|0|0|C:\\a.txt\r\ngroup=2|active=0\r\nfile=0|0|0|C:\\b.txt\r\n"
            );
            let parsed = Session::parse(&text).unwrap();
            assert_eq!(parsed.layout, SessionLayout::Leaf(1), "{layout}");
            assert_eq!(parsed.groups.len(), 1, "{layout}");
            assert_eq!(parsed.groups[0].entries.len(), 2, "{layout}");
        }
    }

    #[test]
    fn ratios_are_normalized_and_out_of_range_indices_fall_back() {
        // Break caught: hand-edited ratios that no longer sum to 1 giving groups no width, or an
        // active index past the end activating nothing.
        let parsed = Session::parse(
            "version=2\r\nlayout=row(1:1,2:3)\r\nactive_group=7\r\ngroup=1|active=4\r\n\
             file=0|0|0|C:\\a.txt\r\ngroup=2|active=0\r\nfile=0|0|0|C:\\b.txt\r\n",
        )
        .unwrap();
        let SessionLayout::Branch { children, .. } = &parsed.layout else {
            panic!("expected a branch");
        };
        assert!((children[0].1 - 0.25).abs() < 1e-6 && (children[1].1 - 0.75).abs() < 1e-6);
        assert_eq!(parsed.active_group, 0);
        assert_eq!(parsed.groups[0].active, 0);
    }

    #[test]
    fn a_group_left_empty_leaves_the_layout() {
        // Break caught: a group whose every entry was malformed leaving an empty pane behind.
        let parsed = Session::parse(
            "version=2\r\nlayout=row(1:0.5,2:0.5)\r\nactive_group=0\r\ngroup=1|active=0\r\n\
             file=0|0|0|C:\\a.txt\r\ngroup=2|active=0\r\nfile=x|0|0|C:\\bad.txt\r\n",
        )
        .unwrap();
        assert_eq!(parsed.layout, SessionLayout::Leaf(1));
        assert_eq!(parsed.groups.len(), 1);
    }

    #[test]
    fn flattening_puts_groups_in_order_and_finds_the_active_entry() {
        // Break caught: a multi-group file restoring into one group with the wrong tab active.
        let (active, entries) = sample().flattened();
        assert_eq!(entries.len(), 3);
        assert_eq!(active, 1);
    }

    #[test]
    fn malformed_entries_are_skipped() {
        // Break caught: one damaged line discarding the whole session.
        let parsed = Session::parse(
            "version=1\nactive=9\nfile=x|0|0|C:\\bad.txt\nsnapshot=0|0|0|xyz\nfile=0|0|0|\n\
             bogus=1\nfile=1|2|3|C:\\good.txt\n",
        )
        .unwrap();
        assert_eq!(parsed.groups[0].active, 0);
        assert_eq!(
            parsed.groups[0].entries,
            vec![SessionEntry {
                source: SessionSource::File(PathBuf::from(r"C:\good.txt")),
                caret: 1,
                anchor: 2,
                first_line: 3,
            }]
        );
    }

    #[test]
    fn write_then_read_returns_the_session_and_a_missing_file_reads_as_none() {
        let dir = std::env::temp_dir().join(format!("fastpad-session-io-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("FastPad").join("session.ini");
        assert_eq!(read(&path), None);
        write(&path, &sample()).unwrap();
        assert_eq!(read(&path), Some(sample()));
        remove(&path);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_progress_activates_the_saved_tab_or_the_last_restored_one() {
        // Break caught: a failed active entry leaving no tab activated, or counting successes as
        // failures in the notice.
        let session = sample();
        let mut restore = SessionRestore::new(&session, None);
        assert_eq!(restore.next_entry(), Some(&session.groups[0].entries[0]));
        restore.record(Some(DocumentId(7)));
        restore.record(None);
        restore.record(Some(DocumentId(9)));
        assert_eq!(restore.next_entry(), None);
        assert_eq!(restore.failed, 1);
        assert_eq!(restore.saved_active_restored(), None);
        assert_eq!(restore.active_tab(), Some(DocumentId(9)));

        let mut restore = SessionRestore::new(&session, None);
        restore.record(Some(DocumentId(7)));
        restore.record(Some(DocumentId(8)));
        assert_eq!(restore.active_tab(), Some(DocumentId(8)));
        assert_eq!(
            restore_failure_notice(1),
            "1 item from the last session could not be reopened."
        );
        assert_eq!(
            restore_failure_notice(2),
            "2 items from the last session could not be reopened."
        );
    }
}
