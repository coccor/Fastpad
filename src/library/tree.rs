//! The notebook's folder tree: built from the scan's note list and the pins, kept current by
//! incremental updates, and flattened into the rows the Notebook view shows. Pure: no Win32 and
//! no disk.
//!
//! In each folder: pinned notes, then subfolders, then other notes, each group in natural,
//! case-insensitive name order ("Note 2" before "Note 10"), ties broken by extension, then by the
//! exact name. A folder exists only while it holds a note at some depth. Paths are relative to
//! the notebook and matched ignoring case, like NTFS. Nothing here recurses, so a very deep
//! folder chain cannot overflow a stack.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

/// An untitled tab, listed first at the root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsavedEntry {
    /// Identifies the tab to the caller (its document ID).
    pub key: u64,
    pub label: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RowKind {
    Unsaved(u64),
    Folder(PathBuf),
    Note(PathBuf),
}

/// One visible row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeRow {
    pub kind: RowKind,
    /// 0 for the root's contents.
    pub depth: u16,
    /// A folder's name, a note's file name without its extension, or an unsaved tab's label.
    pub name: String,
    pub pinned: bool,
    /// For a folder row: whether its contents follow it.
    pub expanded: bool,
}

#[derive(Debug, Default)]
pub struct NoteTree {
    root: Folder,
    count: usize,
}

#[derive(Debug, Default)]
struct Folder {
    /// As first seen; empty for the root.
    name: OsString,
    /// Sorted by `folder_order`.
    folders: Vec<Folder>,
    /// Pinned notes first, each group sorted by `note_order`.
    notes: Vec<Note>,
}

#[derive(Debug)]
struct Note {
    /// The file name, with its extension.
    file_name: OsString,
    pinned: bool,
}

impl Note {
    fn name(&self) -> Cow<'_, str> {
        self.file_name.to_string_lossy()
    }
}

/// Compares names the way people read them: case-insensitive, with digit runs compared as
/// numbers ("Note 2" before "Note 10"). Equal names can still differ in exact spelling ("x9" and
/// "x09", "a" and "A"); callers break those ties. Allocates nothing.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a, b);
    loop {
        let (Some(x), Some(y)) = (a.chars().next(), b.chars().next()) else {
            return b.is_empty().cmp(&a.is_empty());
        };
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let (left, rest_a) = split_digits(a);
            let (right, rest_b) = split_digits(b);
            let (left, right) = (left.trim_start_matches('0'), right.trim_start_matches('0'));
            let order = left.len().cmp(&right.len()).then_with(|| left.cmp(right));
            if order != Ordering::Equal {
                return order;
            }
            (a, b) = (rest_a, rest_b);
        } else {
            let order = x.to_lowercase().cmp(y.to_lowercase());
            if order != Ordering::Equal {
                return order;
            }
            (a, b) = (&a[x.len_utf8()..], &b[y.len_utf8()..]);
        }
    }
}

fn split_digits(text: &str) -> (&str, &str) {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    text.split_at(end)
}

/// Equal ignoring case, char by char, the way `natural_cmp` folds case.
fn eq_ignore_case(a: &str, b: &str) -> bool {
    let (mut left, mut right) = (a.chars(), b.chars());
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if x == y || x.to_lowercase().eq(y.to_lowercase()) => {}
            _ => return false,
        }
    }
}

fn starts_with_ignore_case(name: &str, prefix: &str) -> bool {
    let mut name = name.chars();
    prefix.chars().all(|wanted| {
        name.next()
            .is_some_and(|c| c == wanted || c.to_lowercase().eq(wanted.to_lowercase()))
    })
}

/// A file name's stem (what a row shows) and extension, split like `Path::file_stem`.
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        None | Some(0) => (name, ""),
        Some(index) => (&name[..index], &name[index + 1..]),
    }
}

/// Natural stem order, then natural extension order. Equal for names that differ only in case.
fn note_loose(a: &str, b: &str) -> Ordering {
    let (stem_a, extension_a) = split_name(a);
    let (stem_b, extension_b) = split_name(b);
    natural_cmp(stem_a, stem_b).then_with(|| natural_cmp(extension_a, extension_b))
}

fn note_order(a: &Note, b: &Note) -> Ordering {
    b.pinned
        .cmp(&a.pinned)
        .then_with(|| note_loose(&a.name(), &b.name()))
        .then_with(|| a.file_name.cmp(&b.file_name))
}

fn folder_order(a: &Folder, b: &Folder) -> Ordering {
    natural_cmp(&a.name.to_string_lossy(), &b.name.to_string_lossy())
        .then_with(|| a.name.cmp(&b.name))
}

/// The folder names and the file name of a plain relative path; `None` for anything else (an
/// absolute or rooted path, `.` or `..`, or no file name).
fn split_path(path: &Path) -> Option<(Vec<&OsStr>, &OsStr)> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            _ => return None,
        }
    }
    let file_name = parts.pop()?;
    Some((parts, file_name))
}

/// One key per note or folder, whatever the letter case or separator.
fn path_key(path: &Path) -> String {
    let mut key = String::new();
    for component in path.components() {
        if !key.is_empty() {
            key.push('\\');
        }
        key.push_str(&component.as_os_str().to_string_lossy().to_lowercase());
    }
    key
}

impl Folder {
    fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.notes.is_empty()
    }

    fn pinned_count(&self) -> usize {
        self.notes.partition_point(|note| note.pinned)
    }

    fn find_folder(&self, name: &str) -> Option<usize> {
        let start = self.folders.partition_point(|folder| {
            natural_cmp(&folder.name.to_string_lossy(), name) == Ordering::Less
        });
        self.folders[start..]
            .iter()
            .take_while(|folder| {
                natural_cmp(&folder.name.to_string_lossy(), name) == Ordering::Equal
            })
            .position(|folder| eq_ignore_case(&folder.name.to_string_lossy(), name))
            .map(|offset| start + offset)
    }

    fn child_or_insert(&mut self, name: &OsStr) -> &mut Folder {
        let index = match self.find_folder(&name.to_string_lossy()) {
            Some(index) => index,
            None => {
                let folder = Folder {
                    name: name.to_os_string(),
                    ..Folder::default()
                };
                let index = self
                    .folders
                    .partition_point(|existing| folder_order(existing, &folder) == Ordering::Less);
                self.folders.insert(index, folder);
                index
            }
        };
        &mut self.folders[index]
    }

    /// The note named `name` (ignoring case), by binary search in each pin group.
    fn find_note(&self, name: &str) -> Option<usize> {
        let split = self.pinned_count();
        for (offset, group) in [(0, &self.notes[..split]), (split, &self.notes[split..])] {
            let start =
                group.partition_point(|note| note_loose(&note.name(), name) == Ordering::Less);
            let found = group[start..]
                .iter()
                .take_while(|note| note_loose(&note.name(), name) == Ordering::Equal)
                .position(|note| eq_ignore_case(&note.name(), name));
            if let Some(found) = found {
                return Some(offset + start + found);
            }
        }
        None
    }

    fn insert_note(&mut self, note: Note) {
        let index = self
            .notes
            .partition_point(|existing| note_order(existing, &note) == Ordering::Less);
        self.notes.insert(index, note);
    }

    fn finish(&mut self) {
        self.folders.sort_by(folder_order);
        self.notes.sort_by(note_order);
    }
}

impl NoteTree {
    /// The tree of `notes`, with the ones in `pinned` pinned. Both are relative to the notebook;
    /// a path that is not a plain relative path is skipped, and a second spelling of one note
    /// (another letter case) is dropped.
    pub fn build(notes: &[PathBuf], pinned: &[PathBuf]) -> NoteTree {
        let pinned: HashSet<String> = pinned.iter().map(|path| path_key(path)).collect();
        // Folders live in an arena keyed by lower-case relative path, so each note finds its
        // folder in one lookup however many folders there are; they are nested once, at the end.
        let mut arena = vec![Folder::default()];
        let mut parents = vec![0_usize];
        let mut by_key: HashMap<String, usize> = HashMap::new();
        let mut seen = HashSet::new();
        let mut count = 0;
        for path in notes {
            let Some((folders, file_name)) = split_path(path) else {
                continue;
            };
            let key = path_key(path);
            let is_pinned = pinned.contains(&key);
            if !seen.insert(key) {
                continue;
            }
            let mut current = 0;
            let mut folder_key = String::new();
            for part in folders {
                if !folder_key.is_empty() {
                    folder_key.push('\\');
                }
                folder_key.push_str(&part.to_string_lossy().to_lowercase());
                current = match by_key.get(&folder_key) {
                    Some(&index) => index,
                    None => {
                        arena.push(Folder {
                            name: part.to_os_string(),
                            ..Folder::default()
                        });
                        parents.push(current);
                        by_key.insert(folder_key.clone(), arena.len() - 1);
                        arena.len() - 1
                    }
                };
            }
            arena[current].notes.push(Note {
                file_name: file_name.to_os_string(),
                pinned: is_pinned,
            });
            count += 1;
        }
        // Every folder comes after its parent in the arena, so taking them from the end nests
        // each one before its parent is taken.
        while arena.len() > 1 {
            let (Some(mut folder), Some(parent)) = (arena.pop(), parents.pop()) else {
                break;
            };
            folder.finish();
            arena[parent].folders.push(folder);
        }
        let mut root = arena.pop().unwrap_or_default();
        root.finish();
        NoteTree { root, count }
    }

    pub fn note_count(&self) -> usize {
        self.count
    }

    /// Adds a note, or replaces the entry for the same file (any letter case, any pin).
    pub fn insert_note(&mut self, path: &Path, pinned: bool) {
        let Some((folders, file_name)) = split_path(path) else {
            return;
        };
        self.remove_note(path);
        let mut folder = &mut self.root;
        for part in folders {
            folder = folder.child_or_insert(part);
        }
        folder.insert_note(Note {
            file_name: file_name.to_os_string(),
            pinned,
        });
        self.count += 1;
    }

    /// Removes a note, and every folder that holds nothing after it. Unknown paths do nothing.
    pub fn remove_note(&mut self, path: &Path) {
        let Some((folders, file_name)) = split_path(path) else {
            return;
        };
        let Some(mut trail) = self.trail(&folders) else {
            return;
        };
        let folder = self.folder_at_mut(&trail);
        let Some(index) = folder.find_note(&file_name.to_string_lossy()) else {
            return;
        };
        folder.notes.remove(index);
        self.count -= 1;
        // Deepest first: each folder left holding nothing goes, up to the first that keeps
        // something.
        while let Some(index) = trail.pop() {
            let parent = self.folder_at_mut(&trail);
            if !parent.folders[index].is_empty() {
                break;
            }
            parent.folders.remove(index);
        }
    }

    /// Moves `old`'s row to `new`, keeping its pin. Does nothing when `old` is not in the tree.
    pub fn rename_note(&mut self, old: &Path, new: &Path) {
        if let Some(pinned) = self.note_pinned(old) {
            self.remove_note(old);
            self.insert_note(new, pinned);
        }
    }

    /// Pins or unpins a note in place; its row moves to its new group. Unknown paths do nothing.
    pub fn set_pinned(&mut self, path: &Path, pinned: bool) {
        let Some((folders, file_name)) = split_path(path) else {
            return;
        };
        let Some(trail) = self.trail(&folders) else {
            return;
        };
        let folder = self.folder_at_mut(&trail);
        let Some(index) = folder.find_note(&file_name.to_string_lossy()) else {
            return;
        };
        if folder.notes[index].pinned == pinned {
            return;
        }
        let mut note = folder.notes.remove(index);
        note.pinned = pinned;
        folder.insert_note(note);
    }

    /// The visible rows: `unsaved` first, then the root's contents, and inside each folder that
    /// `expanded` says is open, its contents one level deeper.
    pub fn rows(&self, expanded: &dyn Fn(&Path) -> bool, unsaved: &[UnsavedEntry]) -> Vec<TreeRow> {
        struct Frame<'a> {
            folder: &'a Folder,
            path: PathBuf,
            depth: u16,
            next: usize,
        }
        let mut rows =
            Vec::with_capacity(unsaved.len() + self.root.folders.len() + self.root.notes.len());
        rows.extend(unsaved.iter().map(|entry| TreeRow {
            kind: RowKind::Unsaved(entry.key),
            depth: 0,
            name: entry.label.clone(),
            pinned: false,
            expanded: false,
        }));
        push_notes(&mut rows, &self.root, Path::new(""), 0, true);
        let mut stack = vec![Frame {
            folder: &self.root,
            path: PathBuf::new(),
            depth: 0,
            next: 0,
        }];
        while let Some(frame) = stack.last_mut() {
            let folder = frame.folder;
            let Some(child) = folder.folders.get(frame.next) else {
                let done = stack.pop().expect("the loop just saw this frame");
                push_notes(&mut rows, done.folder, &done.path, done.depth, false);
                continue;
            };
            frame.next += 1;
            let depth = frame.depth;
            let path = frame.path.join(&child.name);
            let open = expanded(&path);
            rows.push(TreeRow {
                kind: RowKind::Folder(path.clone()),
                depth,
                name: child.name.to_string_lossy().into_owned(),
                pinned: false,
                expanded: open,
            });
            if open {
                let depth = depth.saturating_add(1);
                push_notes(&mut rows, child, &path, depth, true);
                stack.push(Frame {
                    folder: child,
                    path,
                    depth,
                    next: 0,
                });
            }
        }
        rows
    }

    /// The child index of each folder on the way down to `folders`, or `None` if one is missing.
    fn trail(&self, folders: &[&OsStr]) -> Option<Vec<usize>> {
        let mut trail = Vec::with_capacity(folders.len());
        let mut folder = &self.root;
        for part in folders {
            let index = folder.find_folder(&part.to_string_lossy())?;
            trail.push(index);
            folder = &folder.folders[index];
        }
        Some(trail)
    }

    fn folder_at_mut(&mut self, trail: &[usize]) -> &mut Folder {
        let mut folder = &mut self.root;
        for &index in trail {
            folder = &mut folder.folders[index];
        }
        folder
    }

    fn note_pinned(&self, path: &Path) -> Option<bool> {
        let (folders, file_name) = split_path(path)?;
        let mut folder = &self.root;
        for part in folders {
            folder = &folder.folders[folder.find_folder(&part.to_string_lossy())?];
        }
        let index = folder.find_note(&file_name.to_string_lossy())?;
        Some(folder.notes[index].pinned)
    }
}

impl Drop for NoteTree {
    /// Nested folders would drop recursively; this empties them one level at a time instead.
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.root.folders);
        while let Some(mut folder) = pending.pop() {
            pending.append(&mut folder.folders);
        }
    }
}

/// Appends a folder's pinned notes (`pinned`) or its other notes.
fn push_notes(rows: &mut Vec<TreeRow>, folder: &Folder, path: &Path, depth: u16, pinned: bool) {
    let split = folder.pinned_count();
    let notes = if pinned {
        &folder.notes[..split]
    } else {
        &folder.notes[split..]
    };
    rows.extend(notes.iter().map(|note| TreeRow {
        kind: RowKind::Note(path.join(&note.file_name)),
        depth,
        name: split_name(&note.name()).0.to_owned(),
        pinned: note.pinned,
        expanded: false,
    }));
}

fn same_row_path(a: &Path, b: &Path) -> bool {
    eq_ignore_case(&a.to_string_lossy(), &b.to_string_lossy())
}

/// The row showing `kind`, with paths compared ignoring case.
pub fn row_index(rows: &[TreeRow], kind: &RowKind) -> Option<usize> {
    rows.iter().position(|row| match (&row.kind, kind) {
        (RowKind::Unsaved(a), RowKind::Unsaved(b)) => a == b,
        (RowKind::Folder(a), RowKind::Folder(b)) | (RowKind::Note(a), RowKind::Note(b)) => {
            same_row_path(a, b)
        }
        _ => false,
    })
}

/// The folder row that row `index` sits in; `None` at depth 0 or for an index past the end.
pub fn parent_index(rows: &[TreeRow], index: usize) -> Option<usize> {
    let depth = rows.get(index)?.depth;
    if depth == 0 {
        return None;
    }
    rows[..index]
        .iter()
        .rposition(|row| row.depth == depth - 1 && matches!(row.kind, RowKind::Folder(_)))
}

/// The first row at or after `from`, wrapping around, whose name starts with `prefix` ignoring
/// case. The caller passes the row after the selection for a new prefix, and the selection
/// itself while the user keeps typing the same one.
pub fn type_ahead(rows: &[TreeRow], from: usize, prefix: &str) -> Option<usize> {
    if prefix.is_empty() || rows.is_empty() {
        return None;
    }
    let start = from % rows.len();
    (start..rows.len())
        .chain(0..start)
        .find(|&index| starts_with_ignore_case(&rows[index].name, prefix))
}

/// The folders that contain `path`, outermost first: `a\b\c.md` gives `a` and `a\b`.
pub fn ancestors(path: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = path
        .ancestors()
        .skip(1)
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .collect();
    folders.reverse();
    folders
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: &Path) -> bool {
        true
    }

    fn build(notes: &[&str], pinned: &[&str]) -> NoteTree {
        let notes: Vec<PathBuf> = notes.iter().map(PathBuf::from).collect();
        let pinned: Vec<PathBuf> = pinned.iter().map(PathBuf::from).collect();
        NoteTree::build(&notes, &pinned)
    }

    /// Each row as `<indent><name><marker>`: `/` a folder, `*` a pinned note, `?` unsaved.
    fn outline(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|row| {
                let marker = match (&row.kind, row.pinned) {
                    (RowKind::Folder(_), _) => "/",
                    (RowKind::Unsaved(_), _) => "?",
                    (RowKind::Note(_), true) => "*",
                    (RowKind::Note(_), false) => "",
                };
                format!(
                    "{}{}{marker}",
                    "  ".repeat(usize::from(row.depth)),
                    row.name
                )
            })
            .collect()
    }

    #[test]
    fn natural_order_compares_digit_runs_as_numbers_and_ignores_case() {
        // Break caught: "Note 10" sorting before "Note 2", or "apple" after "Banana".
        assert_eq!(natural_cmp("Note 2", "Note 10"), Ordering::Less);
        assert_eq!(natural_cmp("note 2", "Note 2"), Ordering::Equal);
        assert_eq!(natural_cmp("apple", "Banana"), Ordering::Less);
        assert_eq!(natural_cmp("file10b", "file10a"), Ordering::Greater);
        assert_eq!(natural_cmp("x9", "x09"), Ordering::Equal);
        assert_eq!(natural_cmp("", "a"), Ordering::Less);
        assert_eq!(
            natural_cmp("n123456789012345678901234567890", "n2"),
            Ordering::Greater,
            "digit runs longer than any integer type"
        );
        assert_eq!(natural_cmp("Äpfel", "äpfel"), Ordering::Equal);
    }

    #[test]
    fn each_folder_lists_pinned_notes_then_subfolders_then_other_notes() {
        // Break caught: pins mixed into the name order, folders after the notes, or "Gamma 10"
        // before "Gamma 9".
        let tree = build(
            &[
                "b.md",
                "Note 10.md",
                "Note 2.md",
                r"beta\x.md",
                r"Alpha\y.md",
                r"Gamma 10\q.md",
                r"Gamma 9\q.md",
                "z.md",
                r"Alpha\pinned.md",
            ],
            &["z.md", r"Alpha\pinned.md"],
        );
        assert_eq!(
            outline(&tree.rows(&all, &[])),
            [
                "z*",
                "Alpha/",
                "  pinned*",
                "  y",
                "beta/",
                "  x",
                "Gamma 9/",
                "  q",
                "Gamma 10/",
                "  q",
                "b",
                "Note 2",
                "Note 10",
            ]
        );
        assert_eq!(tree.note_count(), 9);
    }

    #[test]
    fn ties_are_broken_by_extension_then_by_exact_name() {
        let tree = build(&["a.txt", "A2.md", "a1.md", "a.md", "a01.md"], &[]);
        let paths: Vec<PathBuf> = tree
            .rows(&all, &[])
            .into_iter()
            .map(|row| match row.kind {
                RowKind::Note(path) => path,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            paths,
            ["a.md", "a.txt", "a01.md", "a1.md", "A2.md"].map(PathBuf::from)
        );
    }

    #[test]
    fn rows_do_not_depend_on_the_order_the_scan_listed_notes_in() {
        // Break caught: a rescan after an edit (which changes a file's position in the scan)
        // moving its row, so opening or editing a note reorders the tree.
        let notes = [r"s\b.md", "c.md", r"s\a.md", "a.md", "b.md"];
        let mut reversed = notes;
        reversed.reverse();
        assert_eq!(
            build(&notes, &["b.md"]).rows(&all, &[]),
            build(&reversed, &["b.md"]).rows(&all, &[])
        );
    }

    #[test]
    fn collapsed_folders_hide_their_rows_and_the_root_is_always_open() {
        let tree = build(&["top.md", r"a\one.md", r"a\b\two.md"], &[]);
        let only_a = |path: &Path| path == Path::new("a");
        let rows = tree.rows(&only_a, &[]);
        assert_eq!(outline(&rows), ["a/", "  b/", "  one", "top"]);
        assert!(rows[0].expanded && !rows[1].expanded);
        assert_eq!(rows[1].kind, RowKind::Folder(PathBuf::from(r"a\b")));
        assert_eq!(rows[2].kind, RowKind::Note(PathBuf::from(r"a\one.md")));
        assert_eq!(outline(&tree.rows(&|_| false, &[])), ["a/", "top"]);
    }

    #[test]
    fn unsaved_entries_come_first_at_the_root() {
        let tree = build(&["a.md"], &["a.md"]);
        let unsaved = [
            UnsavedEntry {
                key: 7,
                label: "Idea".into(),
            },
            UnsavedEntry {
                key: 3,
                label: "Untitled".into(),
            },
        ];
        let rows = tree.rows(&all, &unsaved);
        assert_eq!(outline(&rows), ["Idea?", "Untitled?", "a*"]);
        assert_eq!(rows[1].kind, RowKind::Unsaved(3));
        assert_eq!(rows[1].depth, 0);
    }

    #[test]
    fn folders_and_notes_match_ignoring_case() {
        // Break caught: `Sub\a.md` and `sub\b.md` showing as two folders, or a case-only rename
        // leaving the old row behind.
        let mut tree = build(&[r"Sub\a.md", r"sub\b.md"], &[]);
        assert_eq!(outline(&tree.rows(&all, &[])), ["Sub/", "  a", "  b"]);
        tree.insert_note(Path::new(r"SUB\A.MD"), true);
        assert_eq!(outline(&tree.rows(&all, &[])), ["Sub/", "  A*", "  b"]);
        assert_eq!(tree.note_count(), 2);
        tree.remove_note(Path::new(r"sub\b.MD"));
        tree.set_pinned(Path::new(r"sub\a.md"), false);
        assert_eq!(outline(&tree.rows(&all, &[])), ["Sub/", "  A"]);
        assert_eq!(tree.note_count(), 1);
    }

    #[test]
    fn incremental_updates_match_a_full_rebuild() {
        // Break caught: an insert, removal, rename or pin change leaving the tree in an order
        // (or with an empty folder) that a fresh build of the same notes would not have.
        let names: Vec<PathBuf> = [
            "a.md",
            "b.md",
            r"x\c.md",
            r"x\d.md",
            r"x\y\e.md",
            r"z\f.md",
            "Note 2.md",
            "Note 10.md",
        ]
        .map(PathBuf::from)
        .to_vec();
        let mut model: Vec<(PathBuf, bool)> = Vec::new();
        let mut tree = NoteTree::default();
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |bound: usize| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % bound
        };
        for step in 0..400 {
            let path = names[next(names.len())].clone();
            match next(4) {
                0 => {
                    let pinned = next(2) == 0;
                    tree.insert_note(&path, pinned);
                    model.retain(|(existing, _)| existing != &path);
                    model.push((path, pinned));
                }
                1 => {
                    tree.remove_note(&path);
                    model.retain(|(existing, _)| existing != &path);
                }
                2 => {
                    let pinned = next(2) == 0;
                    tree.set_pinned(&path, pinned);
                    if let Some(entry) = model.iter_mut().find(|(existing, _)| existing == &path) {
                        entry.1 = pinned;
                    }
                }
                _ => {
                    let target = names[next(names.len())].clone();
                    // A rename onto an existing note never happens: the file system refuses it.
                    if model.iter().any(|(existing, _)| existing == &target) {
                        continue;
                    }
                    tree.rename_note(&path, &target);
                    if let Some(entry) = model.iter_mut().find(|(existing, _)| existing == &path) {
                        entry.0 = target;
                    }
                }
            }
            let notes: Vec<PathBuf> = model.iter().map(|(path, _)| path.clone()).collect();
            let pinned: Vec<PathBuf> = model
                .iter()
                .filter(|(_, pinned)| *pinned)
                .map(|(path, _)| path.clone())
                .collect();
            assert_eq!(
                tree.rows(&all, &[]),
                NoteTree::build(&notes, &pinned).rows(&all, &[]),
                "step {step}"
            );
            assert_eq!(tree.note_count(), model.len(), "step {step}");
        }
    }

    #[test]
    fn removing_the_last_note_in_a_folder_chain_removes_the_chain() {
        // Break caught: an empty folder row left behind after its only note was deleted or moved.
        let mut tree = build(&[r"a\b\c\n.md", r"a\keep.md", "top.md"], &[]);
        tree.remove_note(Path::new(r"a\b\c\n.md"));
        assert_eq!(outline(&tree.rows(&all, &[])), ["a/", "  keep", "top"]);
        tree.remove_note(Path::new(r"a\keep.md"));
        assert_eq!(outline(&tree.rows(&all, &[])), ["top"]);
        tree.remove_note(Path::new("missing.md"));
        assert_eq!(tree.note_count(), 1);
    }

    #[test]
    fn pins_and_renames_move_rows_and_renames_keep_the_pin() {
        let mut tree = build(&["a.md", "b.md", "c.md"], &[]);
        tree.set_pinned(Path::new("c.md"), true);
        assert_eq!(outline(&tree.rows(&all, &[])), ["c*", "a", "b"]);
        tree.rename_note(Path::new("c.md"), Path::new(r"sub\c2.md"));
        assert_eq!(outline(&tree.rows(&all, &[])), ["sub/", "  c2*", "a", "b"]);
        tree.rename_note(Path::new("gone.md"), Path::new("new.md"));
        assert_eq!(
            tree.note_count(),
            3,
            "renaming a note the tree lacks does nothing"
        );
    }

    #[test]
    fn rows_are_found_by_path_parent_and_typed_prefix() {
        // Break caught: Left on a nested note jumping to the wrong folder, or type-ahead stuck
        // on the current row instead of moving on.
        let tree = build(&[r"a\one.md", r"a\b\two.md", "top.md"], &[]);
        let rows = tree.rows(&all, &[]);
        assert_eq!(outline(&rows), ["a/", "  b/", "    two", "  one", "top"]);
        assert_eq!(
            row_index(&rows, &RowKind::Note(PathBuf::from(r"A\B\TWO.md"))),
            Some(2)
        );
        assert_eq!(
            row_index(&rows, &RowKind::Folder(PathBuf::from(r"a\b"))),
            Some(1)
        );
        assert_eq!(row_index(&rows, &RowKind::Unsaved(3)), None);
        assert_eq!(parent_index(&rows, 2), Some(1));
        assert_eq!(parent_index(&rows, 3), Some(0));
        assert_eq!(parent_index(&rows, 1), Some(0));
        assert_eq!(parent_index(&rows, 0), None);
        assert_eq!(parent_index(&rows, 4), None);
        assert_eq!(parent_index(&rows, 9), None);
        assert_eq!(type_ahead(&rows, 0, "t"), Some(2));
        assert_eq!(type_ahead(&rows, 3, "T"), Some(4));
        assert_eq!(type_ahead(&rows, 5, "on"), Some(3), "wraps around");
        assert_eq!(type_ahead(&rows, 0, ""), None);
        assert_eq!(type_ahead(&rows, 0, "zz"), None);
        assert_eq!(
            ancestors(Path::new(r"a\b\two.md")),
            [PathBuf::from("a"), PathBuf::from(r"a\b")]
        );
        assert!(ancestors(Path::new("top.md")).is_empty());
    }

    #[test]
    fn ten_thousand_notes_in_one_folder_flatten_quickly() {
        // Break caught: a quadratic build or flatten, so opening a big notebook or expanding its
        // one huge folder stalls.
        let notes: Vec<PathBuf> = (0..10_000)
            .map(|index| PathBuf::from(format!(r"big\Note {index}.md")))
            .collect();
        let pinned = vec![PathBuf::from(r"big\Note 9999.md")];
        let started = std::time::Instant::now();
        let tree = NoteTree::build(&notes, &pinned);
        let rows = tree.rows(&all, &[]);
        let elapsed = started.elapsed();
        assert_eq!(tree.note_count(), 10_000);
        assert_eq!(rows.len(), 10_001);
        assert_eq!(
            outline(&rows[..4]),
            ["big/", "  Note 9999*", "  Note 0", "  Note 1"]
        );
        assert_eq!(rows[10_000].name, "Note 9998");
        assert_eq!(tree.rows(&|_| false, &[]).len(), 1);
        if !cfg!(debug_assertions) {
            assert!(
                elapsed < std::time::Duration::from_millis(100),
                "{elapsed:?}"
            );
        }
    }

    #[test]
    fn a_very_deep_folder_chain_builds_flattens_and_empties_without_recursion() {
        // Break caught: a recursive walk (or drop) overflowing the UI thread's stack on a
        // notebook nested thousands of folders deep.
        let depth = 2_000;
        let folder: PathBuf = (0..depth).map(|index| format!("d{index}")).collect();
        let note = folder.join("deep.md");
        let mut tree = NoteTree::build(std::slice::from_ref(&note), &[]);
        let rows = tree.rows(&all, &[]);
        assert_eq!(rows.len(), depth + 1);
        assert_eq!(usize::from(rows[depth].depth), depth);
        assert_eq!(rows[depth].kind, RowKind::Note(note.clone()));
        tree.remove_note(&note);
        assert!(tree.rows(&all, &[]).is_empty());
        let deep = NoteTree::build(std::slice::from_ref(&note), &[]);
        drop(deep);
    }

    #[test]
    fn paths_that_are_not_plain_relative_names_are_ignored() {
        let mut tree = build(
            &[r"C:\abs\x.md", r"..\up.md", "", r".\dot.md", "ok.md"],
            &[],
        );
        tree.insert_note(Path::new(r"\rooted.md"), false);
        assert_eq!(outline(&tree.rows(&all, &[])), ["ok"]);
        assert_eq!(tree.note_count(), 1);
    }
}
