//! The notebook's folder tree: built from the scan's note list and the pins, kept current by
//! incremental updates, and flattened into the rows the Notebook view shows. Pure: no Win32 and
//! no disk.
//!
//! In each folder: pinned notes, then subfolders, then other notes, each group in natural,
//! case-insensitive name order ("Note 2" before "Note 10"), ties broken by extension, then by the
//! exact name. Every folder the scan listed has a row, empty or not, and so does every folder a
//! note is in; a folder leaves only through `remove_folder`, `rename_folder` or a rebuild
//! without it (notebook folders spec §3.2). Paths are relative to
//! the notebook and matched ignoring case, like NTFS. Nothing here recurses, so a very deep
//! folder chain cannot overflow a stack.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RowKind {
    Folder(PathBuf),
    Note(PathBuf),
    /// The Notebook view's row for a note or folder being named in the tree (inline naming spec
    /// §3.1). The view puts it in; `rows` never builds one.
    Draft,
}

/// One visible row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeRow {
    pub kind: RowKind,
    /// 0 for the root's contents.
    pub depth: u16,
    /// A folder's name, or a note's file name with its extension.
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
    /// The notes' file names back to back, as `OsStr::as_encoded_bytes` gives them. A note holds
    /// its range, so a notebook of 10,000 notes costs a few buffers, not 10,000 allocations.
    names: Vec<u8>,
    /// Bytes of `names` that no note uses any more. `compact` drops them once they are half.
    unused: usize,
}

/// One note of a folder: its file name (with its extension) is `Folder::names[start..start+len]`.
#[derive(Clone, Copy, Debug)]
struct Note {
    start: u32,
    len: u16,
    pinned: bool,
}

/// `note`'s file name in `names`.
fn file_name<'a>(names: &'a [u8], note: &Note) -> &'a OsStr {
    let start = note.start as usize;
    let bytes = &names[start..start + usize::from(note.len)];
    // SAFETY: `Folder::push_name` stored these bytes whole from `OsStr::as_encoded_bytes` in
    // this process, and every range a note holds is exactly one of them.
    unsafe { OsStr::from_encoded_bytes_unchecked(bytes) }
}

fn note_name<'a>(names: &'a [u8], note: &Note) -> Cow<'a, str> {
    file_name(names, note).to_string_lossy()
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

/// A file name's stem and extension, split like `Path::file_stem`.
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

fn note_order(names: &[u8], a: &Note, b: &Note) -> Ordering {
    b.pinned
        .cmp(&a.pinned)
        .then_with(|| note_loose(&note_name(names, a), &note_name(names, b)))
        .then_with(|| file_name(names, a).cmp(file_name(names, b)))
}

fn folder_order(a: &Folder, b: &Folder) -> Ordering {
    natural_cmp(&a.name.to_string_lossy(), &b.name.to_string_lossy())
        .then_with(|| a.name.cmp(&b.name))
}

/// The folder names and the file name of a plain relative path; `None` for anything else (an
/// absolute or rooted path, `.` or `..`, no file name, or one longer than any file name can be).
fn split_path(path: &Path) -> Option<(Vec<&OsStr>, &OsStr)> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            _ => return None,
        }
    }
    let file_name = parts.pop()?;
    u16::try_from(file_name.as_encoded_bytes().len()).ok()?;
    Some((parts, file_name))
}

/// Whether `path` is a plain relative folder path: at least one name and nothing else (not
/// empty, absolute, rooted, `.` or `..`). The destructive folder commands refuse anything else,
/// since joined onto the notebook root it could name the root itself or a folder outside it.
pub fn is_plain_relative_folder(path: &Path) -> bool {
    folder_parts(path).is_some()
}

/// The names of a plain relative folder path; `None` for anything else (absolute, rooted, `.`,
/// `..` or empty).
fn folder_parts(path: &Path) -> Option<Vec<&OsStr>> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            _ => return None,
        }
    }
    (!parts.is_empty()).then_some(parts)
}

/// The arena index of the folder at `parts`, adding it and any missing ancestor. `key` is
/// scratch space, reused so a whole build allocates one key buffer.
fn arena_folder(
    arena: &mut Vec<Folder>,
    parents: &mut Vec<usize>,
    by_key: &mut HashMap<String, usize>,
    key: &mut String,
    parts: &[&OsStr],
) -> usize {
    let mut current = 0;
    key.clear();
    for part in parts {
        if !key.is_empty() {
            key.push('\\');
        }
        key.push_str(&part.to_string_lossy().to_lowercase());
        current = match by_key.get(key.as_str()) {
            Some(&index) => index,
            None => {
                arena.push(Folder {
                    name: part.to_os_string(),
                    ..Folder::default()
                });
                parents.push(current);
                by_key.insert(key.clone(), arena.len() - 1);
                arena.len() - 1
            }
        };
    }
    current
}

/// Drops a folder taken out of the tree one level at a time, as `NoteTree`'s own drop does.
fn drop_flat(folder: Folder) {
    let mut pending = vec![folder];
    while let Some(mut folder) = pending.pop() {
        pending.append(&mut folder.folders);
    }
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
    fn pinned_count(&self) -> usize {
        self.notes.partition_point(|note| note.pinned)
    }

    /// The notes in this folder and every folder under it.
    fn note_count(&self) -> usize {
        let mut count = 0;
        let mut pending = vec![self];
        while let Some(folder) = pending.pop() {
            count += folder.notes.len();
            pending.extend(folder.folders.iter());
        }
        count
    }

    /// Every note (with its pin) and every folder under this one, as paths relative to it.
    fn entries(&self) -> Vec<(PathBuf, Option<bool>)> {
        let mut entries = Vec::new();
        let mut pending = vec![(PathBuf::new(), self)];
        while let Some((path, folder)) = pending.pop() {
            for note in &folder.notes {
                entries.push((path.join(file_name(&folder.names, note)), Some(note.pinned)));
            }
            for child in &folder.folders {
                let child_path = path.join(&child.name);
                entries.push((child_path.clone(), None));
                pending.push((child_path, child));
            }
        }
        entries
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
        let names = &self.names[..];
        let split = self.pinned_count();
        for (offset, group) in [(0, &self.notes[..split]), (split, &self.notes[split..])] {
            let start = group.partition_point(|note| {
                note_loose(&note_name(names, note), name) == Ordering::Less
            });
            let found = group[start..]
                .iter()
                .take_while(|note| note_loose(&note_name(names, note), name) == Ordering::Equal)
                .position(|note| eq_ignore_case(&note_name(names, note), name));
            if let Some(found) = found {
                return Some(offset + start + found);
            }
        }
        None
    }

    /// Stores `file_name` in `names` and returns its note, unplaced. `None` for a name longer
    /// than any file name can be.
    fn push_name(&mut self, file_name: &OsStr, pinned: bool) -> Option<Note> {
        let bytes = file_name.as_encoded_bytes();
        let len = u16::try_from(bytes.len()).ok()?;
        let start = u32::try_from(self.names.len()).ok()?;
        self.names.extend_from_slice(bytes);
        Some(Note { start, len, pinned })
    }

    /// Adds a note in its place. `note` comes from `push_name` or from `take_note`.
    fn place_note(&mut self, note: Note) {
        let names = &self.names[..];
        let index = self
            .notes
            .partition_point(|existing| note_order(names, existing, &note) == Ordering::Less);
        self.notes.insert(index, note);
    }

    /// Takes the note at `index` out of the order; its name stays in `names`.
    fn take_note(&mut self, index: usize) -> Note {
        self.notes.remove(index)
    }

    /// Removes the note at `index` and its name.
    fn remove_note_at(&mut self, index: usize) {
        let note = self.notes.remove(index);
        self.unused += usize::from(note.len);
        if self.unused * 2 > self.names.len() {
            self.compact();
        }
    }

    /// Rewrites `names` with only the names notes use.
    fn compact(&mut self) {
        let mut names = Vec::with_capacity(self.names.len() - self.unused);
        for note in &mut self.notes {
            let start = note.start as usize;
            let bytes = &self.names[start..start + usize::from(note.len)];
            // At most the old length, which fit.
            note.start = names.len() as u32;
            names.extend_from_slice(bytes);
        }
        self.names = names;
        self.unused = 0;
    }

    /// Sorts what `NoteTree::build` gathered and drops second spellings. Returns how many notes
    /// were dropped.
    fn finish(&mut self) -> usize {
        self.folders.sort_by(folder_order);
        let names = &self.names[..];
        self.notes.sort_by(|a, b| note_order(names, a, b));
        let dropped = self.drop_second_spellings();
        self.notes.shrink_to_fit();
        if self.unused > 0 {
            self.compact();
        }
        self.names.shrink_to_fit();
        dropped
    }

    /// A note given twice in different letter case keeps the spelling given first. Both spellings
    /// have the same pin and sort next to each other; the one given first has the lower `start`.
    fn drop_second_spellings(&mut self) -> usize {
        let names = &self.names[..];
        let notes = &self.notes;
        let mut dropped = vec![false; notes.len()];
        let mut run = 0;
        for end in 1..=notes.len() {
            if end < notes.len()
                && notes[end].pinned == notes[run].pinned
                && note_loose(
                    &note_name(names, &notes[end]),
                    &note_name(names, &notes[run]),
                ) == Ordering::Equal
            {
                continue;
            }
            for kept in run..end {
                if dropped[kept] {
                    continue;
                }
                let key = note_name(names, &notes[kept]).to_lowercase();
                for other in run..end {
                    if other != kept
                        && !dropped[other]
                        && notes[other].start > notes[kept].start
                        && note_name(names, &notes[other]).to_lowercase() == key
                    {
                        dropped[other] = true;
                    }
                }
            }
            run = end;
        }
        let count = dropped.iter().filter(|&&gone| gone).count();
        if count > 0 {
            let mut index = 0;
            let mut unused = 0;
            self.notes.retain(|note| {
                let keep = !dropped[index];
                index += 1;
                if !keep {
                    unused += usize::from(note.len);
                }
                keep
            });
            self.unused += unused;
        }
        count
    }
}

impl NoteTree {
    /// The tree of `notes` and `folders`, with the notes in `pinned` pinned. All are relative to
    /// the notebook; a path that is not a plain relative path is skipped, and a second spelling
    /// of one note (another letter case) is dropped. Every listed folder gets a row, even an
    /// empty one, and so does every folder a note is in. `notes` is only read: the library
    /// builds its tree straight from its own note list.
    pub fn build<'a, P>(
        notes: impl IntoIterator<Item = &'a P>,
        folders: &[PathBuf],
        pinned: &[PathBuf],
    ) -> NoteTree
    where
        P: AsRef<Path> + ?Sized + 'a,
    {
        let pinned: HashSet<String> = pinned.iter().map(|path| path_key(path)).collect();
        // Folders live in an arena keyed by lower-case relative path, so each note finds its
        // folder in one lookup however many folders there are; they are nested once, at the end.
        let mut arena = vec![Folder::default()];
        let mut parents = vec![0_usize];
        let mut by_key: HashMap<String, usize> = HashMap::new();
        let mut count = 0;
        let mut folder_key = String::new();
        // The listed folders go in first, so each keeps the spelling the scan saw on disk.
        for path in folders {
            if let Some(parts) = folder_parts(path) {
                arena_folder(
                    &mut arena,
                    &mut parents,
                    &mut by_key,
                    &mut folder_key,
                    &parts,
                );
            }
        }
        for path in notes {
            let path = path.as_ref();
            let Some((parts, file_name)) = split_path(path) else {
                continue;
            };
            let is_pinned = !pinned.is_empty() && pinned.contains(&path_key(path));
            let current = arena_folder(
                &mut arena,
                &mut parents,
                &mut by_key,
                &mut folder_key,
                &parts,
            );
            let folder = &mut arena[current];
            if let Some(note) = folder.push_name(file_name, is_pinned) {
                folder.notes.push(note);
                count += 1;
            }
        }
        // Every folder comes after its parent in the arena, so taking them from the end nests
        // each one before its parent is taken.
        while arena.len() > 1 {
            let (Some(mut folder), Some(parent)) = (arena.pop(), parents.pop()) else {
                break;
            };
            count -= folder.finish();
            arena[parent].folders.push(folder);
        }
        let mut root = arena.pop().unwrap_or_default();
        count -= root.finish();
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
        if let Some(note) = folder.push_name(file_name, pinned) {
            folder.place_note(note);
            self.count += 1;
        }
    }

    /// Removes a note. Its folder keeps its row even when it now holds nothing: folders leave
    /// only through `remove_folder`, `rename_folder` or a rebuild without them. Unknown paths
    /// do nothing.
    pub fn remove_note(&mut self, path: &Path) {
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
        folder.remove_note_at(index);
        self.count -= 1;
    }

    /// Whether the tree has a folder at `path`, ignoring case.
    pub fn contains_folder(&self, path: &Path) -> bool {
        folder_parts(path).is_some_and(|parts| self.trail(&parts).is_some())
    }

    /// Adds a folder row at `path`, with any missing ancestor, holding nothing. A folder already
    /// there in any letter case keeps its spelling; a path that is not a plain relative name
    /// does nothing.
    pub fn insert_folder(&mut self, path: &Path) {
        let Some(parts) = folder_parts(path) else {
            return;
        };
        parts
            .into_iter()
            .fold(&mut self.root, |folder, part| folder.child_or_insert(part));
    }

    /// Removes the folder at `path` and everything under it. Unknown paths do nothing.
    pub fn remove_folder(&mut self, path: &Path) {
        let Some(parts) = folder_parts(path) else {
            return;
        };
        let Some(mut trail) = self.trail(&parts) else {
            return;
        };
        let Some(index) = trail.pop() else {
            return;
        };
        let removed = self.folder_at_mut(&trail).folders.remove(index);
        self.count -= removed.note_count();
        drop_flat(removed);
    }

    /// Moves the folder at `old`, with everything under it, to `new`, keeping its notes and
    /// their pins; a change of letter case renames it in place. When `old` is not in the tree,
    /// `new` is added: a rescan's result that already saw the rename gets it replayed. When a
    /// folder is already at `new`, what `old` held merges into it.
    pub fn rename_folder(&mut self, old: &Path, new: &Path) {
        let (Some(old_parts), Some(new_parts)) = (folder_parts(old), folder_parts(new)) else {
            return;
        };
        let Some((&name, parents)) = new_parts.split_last() else {
            return;
        };
        let Some(mut trail) = self.trail(&old_parts) else {
            self.insert_folder(new);
            return;
        };
        let Some(index) = trail.pop() else {
            return;
        };
        let mut moved = self.folder_at_mut(&trail).folders.remove(index);
        let mut parent = &mut self.root;
        for part in parents {
            parent = parent.child_or_insert(part);
        }
        if parent.find_folder(&name.to_string_lossy()).is_none() {
            moved.name = name.to_os_string();
            let index = parent
                .folders
                .partition_point(|existing| folder_order(existing, &moved) == Ordering::Less);
            parent.folders.insert(index, moved);
            return;
        }
        self.count -= moved.note_count();
        let entries = moved.entries();
        drop_flat(moved);
        for (path, pinned) in entries {
            let path = new.join(path);
            match pinned {
                Some(pinned) => self.insert_note(&path, pinned),
                None => self.insert_folder(&path),
            }
        }
    }

    /// Moves `old`'s row to `new`, keeping its pin. Does nothing when `old` is not in the tree.
    #[cfg(test)]
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
        let mut note = folder.take_note(index);
        note.pinned = pinned;
        folder.place_note(note);
    }

    /// The visible rows: the root's contents, then inside each folder that `expanded` says is
    /// open, its contents one level deeper.
    pub fn rows(&self, expanded: &dyn Fn(&Path) -> bool) -> Vec<TreeRow> {
        struct Frame<'a> {
            folder: &'a Folder,
            path: PathBuf,
            depth: u16,
            next: usize,
        }
        let mut rows = Vec::with_capacity(self.root.folders.len() + self.root.notes.len());
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

    #[cfg(test)]
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
        kind: RowKind::Note(path.join(file_name(&folder.names, note))),
        depth,
        name: note_name(&folder.names, note).into_owned(),
        pinned: note.pinned,
        expanded: false,
    }));
}

fn same_row_path(a: &Path, b: &Path) -> bool {
    eq_ignore_case(&a.to_string_lossy(), &b.to_string_lossy())
}

/// The row that takes row `index`'s place once it and everything shown under it go: the next row
/// after its subtree, else the row before it.
pub fn row_in_place_of(rows: &[TreeRow], index: usize) -> Option<&TreeRow> {
    let depth = rows.get(index)?.depth;
    rows[index + 1..]
        .iter()
        .find(|row| row.depth <= depth)
        .or_else(|| index.checked_sub(1).and_then(|before| rows.get(before)))
}

pub fn row_index(rows: &[TreeRow], kind: &RowKind) -> Option<usize> {
    rows.iter().position(|row| match (&row.kind, kind) {
        (RowKind::Folder(a), RowKind::Folder(b)) | (RowKind::Note(a), RowKind::Note(b)) => {
            same_row_path(a, b)
        }
        (RowKind::Draft, RowKind::Draft) => true,
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
mod tests;
