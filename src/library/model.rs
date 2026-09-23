//! Notebooks, tags and note records, and the primitive mutations on them. Every name rule of the
//! spec lives here: trimmed, not empty, unique ignoring case; tags never store a leading `#`.

use super::ids::{NoteId, NotebookId, TagId};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotebookColor {
    Red,
    Orange,
    Yellow,
    Green,
    Teal,
    Blue,
    Purple,
    Pink,
}

impl NotebookColor {
    pub const ALL: [Self; 8] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Teal,
        Self::Blue,
        Self::Purple,
        Self::Pink,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Teal => "teal",
            Self::Blue => "blue",
            Self::Purple => "purple",
            Self::Pink => "pink",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.name() == text)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notebook {
    pub id: NotebookId,
    pub name: String,
    pub color: Option<NotebookColor>,
    pub sort: u32,
    pub created: u64,
    pub modified: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tag {
    pub id: TagId,
    pub name: String,
}

/// One organized note. `path` is relative to the folder, or absolute for a file outside it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteRecord {
    pub id: NoteId,
    pub notebook: Option<NotebookId>,
    pub favorite: bool,
    pub pinned: bool,
    pub deleted: bool,
    pub tags: Vec<TagId>,
    pub size: u64,
    pub hash: u64,
    pub path: PathBuf,
}

impl NoteRecord {
    pub fn new(id: NoteId, path: PathBuf) -> Self {
        Self {
            id,
            notebook: None,
            favorite: false,
            pinned: false,
            deleted: false,
            tags: Vec::new(),
            size: 0,
            hash: 0,
            path,
        }
    }

    pub fn has_metadata(&self) -> bool {
        self.notebook.is_some() || self.favorite || self.pinned || !self.tags.is_empty()
    }
}

/// Names a note for an operation: by stable ID first, then by path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteRef {
    pub id: NoteId,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameError {
    Empty,
    Duplicate,
}

impl std::fmt::Display for NameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "A name cannot be empty.",
            Self::Duplicate => "That name is already used.",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LibraryError {
    Name(NameError),
    NotFound,
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Name(error) => error.fmt(formatter),
            Self::NotFound => formatter.write_str("That item no longer exists."),
        }
    }
}

pub fn normalize_name(name: &str) -> Result<String, NameError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        Err(NameError::Empty)
    } else {
        Ok(trimmed.to_owned())
    }
}

/// Like `normalize_name`, but the leading `#` is presentation only and never stored.
pub fn normalize_tag_name(name: &str) -> Result<String, NameError> {
    normalize_name(name.trim().trim_start_matches('#'))
}

pub fn same_name(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

/// NTFS compares names ignoring case, so the library does too.
pub fn same_path(left: &Path, right: &Path) -> bool {
    left.as_os_str().to_string_lossy().to_lowercase()
        == right.as_os_str().to_string_lossy().to_lowercase()
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Library {
    pub notebooks: Vec<Notebook>,
    pub tags: Vec<Tag>,
    pub notes: Vec<NoteRecord>,
}

impl Library {
    pub fn notebook(&self, id: NotebookId) -> Option<&Notebook> {
        self.notebooks.iter().find(|notebook| notebook.id == id)
    }

    pub fn notebooks_in_order(&self) -> Vec<&Notebook> {
        let mut ordered: Vec<_> = self.notebooks.iter().collect();
        ordered.sort_by(|a, b| a.sort.cmp(&b.sort).then_with(|| a.name.cmp(&b.name)));
        ordered
    }

    pub fn tag(&self, id: TagId) -> Option<&Tag> {
        self.tags.iter().find(|tag| tag.id == id)
    }

    pub fn tag_by_name(&self, name: &str) -> Option<&Tag> {
        self.tags.iter().find(|tag| same_name(&tag.name, name))
    }

    pub fn note(&self, id: NoteId) -> Option<&NoteRecord> {
        self.notes.iter().find(|note| note.id == id)
    }

    pub fn note_by_path(&self, path: &Path) -> Option<&NoteRecord> {
        self.notes.iter().find(|note| same_path(&note.path, path))
    }

    /// An existing record, by ID and then by path. Never creates one.
    pub fn find_note_mut(&mut self, target: &NoteRef) -> Option<&mut NoteRecord> {
        let index = self
            .notes
            .iter()
            .position(|note| note.id == target.id)
            .or_else(|| self.notes.iter().position(|note| same_path(&note.path, &target.path)))?;
        self.notes.get_mut(index)
    }

    /// The record for `target`, created with its ID and path when none exists.
    pub fn resolve_note(&mut self, target: &NoteRef) -> &mut NoteRecord {
        let index = self
            .notes
            .iter()
            .position(|note| note.id == target.id)
            .or_else(|| self.notes.iter().position(|note| same_path(&note.path, &target.path)));
        let index = index.unwrap_or_else(|| {
            self.notes.push(NoteRecord::new(target.id, target.path.clone()));
            self.notes.len() - 1
        });
        &mut self.notes[index]
    }

    fn check_notebook_name(
        &self,
        name: &str,
        except: Option<NotebookId>,
    ) -> Result<String, LibraryError> {
        let name = normalize_name(name).map_err(LibraryError::Name)?;
        let taken = self
            .notebooks
            .iter()
            .any(|notebook| Some(notebook.id) != except && same_name(&notebook.name, &name));
        if taken {
            Err(LibraryError::Name(NameError::Duplicate))
        } else {
            Ok(name)
        }
    }

    pub fn create_notebook(
        &mut self,
        id: NotebookId,
        name: &str,
        now: u64,
    ) -> Result<(), LibraryError> {
        if self.notebook(id).is_some() {
            return Ok(());
        }
        let name = self.check_notebook_name(name, None)?;
        let sort = self.notebooks.iter().map(|n| n.sort + 1).max().unwrap_or(0);
        self.notebooks.push(Notebook {
            id,
            name,
            color: None,
            sort,
            created: now,
            modified: now,
        });
        Ok(())
    }

    pub fn rename_notebook(
        &mut self,
        id: NotebookId,
        name: &str,
        now: u64,
    ) -> Result<(), LibraryError> {
        let name = self.check_notebook_name(name, Some(id))?;
        let notebook = self
            .notebooks
            .iter_mut()
            .find(|notebook| notebook.id == id)
            .ok_or(LibraryError::NotFound)?;
        notebook.name = name;
        notebook.modified = now;
        Ok(())
    }

    pub fn set_notebook_color(
        &mut self,
        id: NotebookId,
        color: Option<NotebookColor>,
        now: u64,
    ) -> Result<(), LibraryError> {
        let notebook = self
            .notebooks
            .iter_mut()
            .find(|notebook| notebook.id == id)
            .ok_or(LibraryError::NotFound)?;
        notebook.color = color;
        notebook.modified = now;
        Ok(())
    }

    /// Moves the notebook to `index` in display order (clamped) and renumbers every sort key.
    pub fn move_notebook(&mut self, id: NotebookId, index: usize) -> Result<(), LibraryError> {
        let mut order: Vec<NotebookId> = self.notebooks_in_order().iter().map(|n| n.id).collect();
        let from = order.iter().position(|&n| n == id).ok_or(LibraryError::NotFound)?;
        order.remove(from);
        order.insert(index.min(order.len()), id);
        for (sort, id) in order.into_iter().enumerate() {
            if let Some(notebook) = self.notebooks.iter_mut().find(|n| n.id == id) {
                notebook.sort = sort as u32;
            }
        }
        Ok(())
    }

    /// Removes the notebook and moves its notes to Notes. Returns how many notes moved.
    pub fn delete_notebook(&mut self, id: NotebookId) -> Result<usize, LibraryError> {
        let before = self.notebooks.len();
        self.notebooks.retain(|notebook| notebook.id != id);
        if self.notebooks.len() == before {
            return Err(LibraryError::NotFound);
        }
        let mut moved = 0;
        for note in &mut self.notes {
            if note.notebook == Some(id) {
                note.notebook = None;
                moved += 1;
            }
        }
        Ok(moved)
    }

    /// Returns the existing tag with this name, or creates one with `id`.
    pub fn create_tag(&mut self, id: TagId, name: &str) -> Result<TagId, LibraryError> {
        let name = normalize_tag_name(name).map_err(LibraryError::Name)?;
        if let Some(existing) = self.tag_by_name(&name) {
            return Ok(existing.id);
        }
        if self.tag(id).is_some() {
            return Ok(id);
        }
        self.tags.push(Tag { id, name });
        Ok(id)
    }

    pub fn rename_tag(&mut self, id: TagId, name: &str) -> Result<(), LibraryError> {
        let name = normalize_tag_name(name).map_err(LibraryError::Name)?;
        if self.tags.iter().any(|tag| tag.id != id && same_name(&tag.name, &name)) {
            return Err(LibraryError::Name(NameError::Duplicate));
        }
        let tag = self.tags.iter_mut().find(|tag| tag.id == id).ok_or(LibraryError::NotFound)?;
        tag.name = name;
        Ok(())
    }

    /// Removes the tag from every note and from the library. Returns how many notes had it.
    pub fn remove_tag_everywhere(&mut self, id: TagId) -> Result<usize, LibraryError> {
        if self.tag(id).is_none() {
            return Err(LibraryError::NotFound);
        }
        let count = self.tag_count(id);
        for note in &mut self.notes {
            note.tags.retain(|&tag| tag != id);
        }
        self.tags.retain(|tag| tag.id != id);
        Ok(count)
    }

    pub fn tag_count(&self, id: TagId) -> usize {
        self.notes
            .iter()
            .filter(|note| !note.deleted && note.tags.contains(&id))
            .count()
    }

    /// Drops records that carry no metadata and tags no record uses. Run before every write.
    pub fn prune(&mut self) {
        self.notes.retain(NoteRecord::has_metadata);
        let notes = &self.notes;
        self.tags.retain(|tag| notes.iter().any(|note| note.tags.contains(&tag.id)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nb(value: u128) -> NotebookId {
        NotebookId(value)
    }

    #[test]
    fn notebook_names_are_trimmed_non_empty_and_unique_ignoring_case() {
        // Break caught: "Work" and " work " becoming two notebooks, or an empty name accepted.
        let mut library = Library::default();
        library.create_notebook(nb(1), "  Work ", 10).unwrap();
        assert_eq!(library.notebook(nb(1)).unwrap().name, "Work");
        assert_eq!(
            library.create_notebook(nb(2), "work", 10),
            Err(LibraryError::Name(NameError::Duplicate))
        );
        assert_eq!(
            library.create_notebook(nb(3), "   ", 10),
            Err(LibraryError::Name(NameError::Empty))
        );
        library.create_notebook(nb(4), "Personal", 10).unwrap();
        assert_eq!(
            library.rename_notebook(nb(4), "WORK", 11),
            Err(LibraryError::Name(NameError::Duplicate))
        );
        // Renaming to a different case of its own name is allowed.
        library.rename_notebook(nb(1), "WORK", 12).unwrap();
        assert_eq!(library.notebook(nb(1)).unwrap().modified, 12);
    }

    #[test]
    fn creating_a_notebook_with_an_existing_id_is_a_no_op() {
        // Break caught: replaying "create notebook" after a merge failing as a duplicate and
        // being dropped, or creating a second copy.
        let mut library = Library::default();
        library.create_notebook(nb(1), "Work", 10).unwrap();
        library.create_notebook(nb(1), "Work", 20).unwrap();
        assert_eq!(library.notebooks.len(), 1);
    }

    #[test]
    fn notebooks_reorder_and_new_ones_go_last() {
        let mut library = Library::default();
        for (id, name) in [(1, "A"), (2, "B"), (3, "C")] {
            library.create_notebook(nb(id), name, 0).unwrap();
        }
        library.move_notebook(nb(3), 0).unwrap();
        let order: Vec<_> = library.notebooks_in_order().iter().map(|n| n.name.as_str()).collect();
        assert_eq!(order, ["C", "A", "B"]);
        library.create_notebook(nb(4), "D", 0).unwrap();
        assert_eq!(library.notebooks_in_order().last().unwrap().name, "D");
        library.move_notebook(nb(1), 99).unwrap();
        assert_eq!(library.notebooks_in_order().last().unwrap().name, "A");
    }

    #[test]
    fn deleting_a_notebook_moves_its_notes_to_notes_and_keeps_them() {
        // Break caught: deleting a notebook deleting its notes' records (and their tags).
        let mut library = Library::default();
        library.create_notebook(nb(1), "Work", 0).unwrap();
        let note = library.resolve_note(&NoteRef { id: NoteId(9), path: "a.md".into() });
        note.notebook = Some(nb(1));
        note.favorite = true;
        assert_eq!(library.delete_notebook(nb(1)), Ok(1));
        let note = library.note(NoteId(9)).unwrap();
        assert_eq!(note.notebook, None);
        assert!(note.favorite);
        assert_eq!(library.delete_notebook(nb(1)), Err(LibraryError::NotFound));
    }

    #[test]
    fn resolve_finds_by_id_then_by_path_ignoring_case_then_creates() {
        let mut library = Library::default();
        library.resolve_note(&NoteRef { id: NoteId(1), path: "Plan.md".into() });
        let by_path = library.resolve_note(&NoteRef { id: NoteId(2), path: "plan.MD".into() });
        assert_eq!(by_path.id, NoteId(1));
        let by_id = library.resolve_note(&NoteRef { id: NoteId(1), path: "other.md".into() });
        assert_eq!(by_id.path, PathBuf::from("Plan.md"));
        library.resolve_note(&NoteRef { id: NoteId(3), path: "new.md".into() });
        assert_eq!(library.notes.len(), 2);
    }

    #[test]
    fn tags_strip_a_leading_hash_reuse_names_and_disappear_when_unused() {
        // Break caught: "#idea" and "idea" becoming two tags, or an unused tag lingering.
        let mut library = Library::default();
        assert_eq!(library.create_tag(TagId(1), "#idea"), Ok(TagId(1)));
        assert_eq!(library.tag(TagId(1)).unwrap().name, "idea");
        assert_eq!(library.create_tag(TagId(2), "IDEA"), Ok(TagId(1)));
        assert_eq!(
            library.create_tag(TagId(3), " # "),
            Err(LibraryError::Name(NameError::Empty))
        );
        library.resolve_note(&NoteRef { id: NoteId(5), path: "a.md".into() }).tags.push(TagId(1));
        library.create_tag(TagId(4), "todo").unwrap();
        library.prune();
        assert!(library.tag(TagId(1)).is_some());
        assert!(library.tag(TagId(4)).is_none());
    }

    #[test]
    fn removing_a_tag_everywhere_counts_notes_and_prune_drops_bare_records() {
        let mut library = Library::default();
        library.create_tag(TagId(1), "todo").unwrap();
        for (id, path) in [(1, "a.md"), (2, "b.md")] {
            library.resolve_note(&NoteRef { id: NoteId(id), path: path.into() }).tags.push(TagId(1));
        }
        library.resolve_note(&NoteRef { id: NoteId(2), path: "b.md".into() }).favorite = true;
        assert_eq!(library.tag_count(TagId(1)), 2);
        assert_eq!(library.remove_tag_everywhere(TagId(1)), Ok(2));
        library.prune();
        assert_eq!(library.notes.len(), 1);
        assert_eq!(library.notes[0].id, NoteId(2));
        assert!(library.tags.is_empty());
    }

    #[test]
    fn paths_compare_ignoring_case_like_ntfs() {
        assert!(same_path(Path::new(r"Sub\Plan.md"), Path::new(r"sub\plan.MD")));
        assert!(!same_path(Path::new("a.md"), Path::new("b.md")));
    }
}
