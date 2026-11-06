//! Moving notes and folders between notebooks (multi-root move spec §2.1): where each item
//! lands, which names are taken there, how many links would stop reaching their note, which
//! images the moved notes embed from outside what moves, and the question to ask about them.
//! Pure: the caller passes the note lists, indexes and resolvers, and answers what exists.

use super::facts::{LinkKind, LinkUse};
use super::index::NoteIndex;
use super::links::Resolver;
use super::path_key;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

/// One notebook's side of a move.
pub struct Side<'a> {
    /// Every note, relative to the notebook.
    pub notes: &'a [PathBuf],
    /// `None` while the notebook's index isn't built.
    pub index: Option<&'a NoteIndex>,
    pub resolver: &'a Resolver,
    /// The notebook's attachments folder (relative), where a wiki embed by name is looked for
    /// after the note's folder and the root.
    pub attachments: &'a Path,
}

/// A moved item, relative to the source notebook.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Item {
    Note(PathBuf),
    Folder(PathBuf),
}

impl Item {
    pub fn path(&self) -> &Path {
        match self {
            Self::Note(path) | Self::Folder(path) => path,
        }
    }
}

/// One use of an image in a moved note: the note (source-relative, before the move), the byte
/// range of the link's target, its kind and its text as written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageUse {
    pub note: PathBuf,
    pub range: Range<usize>,
    pub kind: LinkKind,
    pub target: String,
}

/// An image a moved note embeds from outside what moves.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Image {
    /// Relative to the source notebook.
    pub source: PathBuf,
    /// No note that stays embeds it: it moves rather than being copied.
    pub users_only_moving: bool,
    pub uses: Vec<ImageUse>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MovePlan {
    /// Each item and where it lands, relative to the destination notebook.
    pub moves: Vec<(Item, PathBuf)>,
    /// Every note that moves, before and after.
    pub notes: Vec<(PathBuf, PathBuf)>,
    /// The names of items already taken at the destination.
    pub clashes: Vec<String>,
    /// Link uses that would stop reaching their note; `None` when the source isn't indexed.
    pub breaking: Option<usize>,
    pub images: Vec<Image>,
}

/// `path` without `.` and `..` parts, or `None` when it climbs above the notebook or isn't a
/// plain relative path.
fn normalize(path: &Path) -> Option<PathBuf> {
    let mut parts: Vec<&OsStr> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.iter().collect())
}

/// Whether a wikilink's target names a file that isn't a note (an image, a PDF).
fn names_a_file(target: &str) -> bool {
    Path::new(target.trim())
        .extension()
        .is_some_and(|extension| !super::title::is_note_extension(&extension.to_string_lossy()))
}

/// The file (relative) an image link of the note at `note` shows, when it exists: a Markdown
/// image's destination relative to the note's folder; a wiki embed by path from the root then
/// the note's folder, or by name in the note's folder, the root, then `attachments`.
fn image_target(
    note: &Path,
    link: &LinkUse,
    attachments: &Path,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let folder = note.parent().unwrap_or(Path::new(""));
    let candidates: Vec<PathBuf> = match link.kind {
        LinkKind::Image => {
            let decoded = super::links::percent_decode(&link.target).replace('/', "\\");
            normalize(&folder.join(decoded)).into_iter().collect()
        }
        LinkKind::Wiki if names_a_file(&link.target) => {
            let target = link.target.trim().replace('/', "\\");
            let bases: &[&Path] = if target.contains('\\') {
                &[Path::new(""), folder]
            } else {
                &[folder, Path::new(""), attachments]
            };
            bases
                .iter()
                .filter_map(|base| normalize(&base.join(&target)))
                .collect()
        }
        LinkKind::Wiki | LinkKind::Markdown => Vec::new(),
    };
    candidates.into_iter().find(|candidate| exists(candidate))
}

pub fn plan(
    items: &[Item],
    dest_folder: &Path,
    source: &Side,
    dest: Option<&Side>,
    dest_exists: &dyn Fn(&Path) -> bool,
    source_exists: &dyn Fn(&Path) -> bool,
) -> MovePlan {
    let mut plan = MovePlan::default();
    for item in items {
        let Some(name) = item.path().file_name() else {
            continue;
        };
        let landing = dest_folder.join(name);
        if dest_exists(&landing) {
            plan.clashes.push(name.to_string_lossy().into_owned());
        }
        match item {
            Item::Note(path) => plan.notes.push((path.clone(), landing.clone())),
            Item::Folder(folder) => {
                for note in source.notes {
                    if super::is_inside(folder, note) {
                        let inner = super::record_path(folder, note);
                        plan.notes.push((note.clone(), landing.join(inner)));
                    }
                }
            }
        }
        plan.moves.push((item.clone(), landing));
    }
    let new_of: HashMap<String, PathBuf> = plan
        .notes
        .iter()
        .map(|(old, new)| (path_key(old), new.clone()))
        .collect();
    let Some(index) = source.index else {
        return plan;
    };
    plan.breaking = Some(breaking(index, source, dest, &new_of));
    plan.images = images(index, source, items, &new_of, source_exists);
    plan
}

/// How many link uses stop reaching their note (spec §2.1).
fn breaking(
    index: &NoteIndex,
    source: &Side,
    dest: Option<&Side>,
    new_of: &HashMap<String, PathBuf>,
) -> usize {
    // The destination as it will be: its notes and aliases, and the moving ones at their new
    // paths.
    let combined = dest.and_then(|dest| {
        let dest_index = dest.index?;
        let notes: Vec<PathBuf> = dest
            .notes
            .iter()
            .cloned()
            .chain(new_of.values().cloned())
            .collect();
        let aliases =
            dest_index
                .notes
                .values()
                .map(|note| (note.path.clone(), &note.facts))
                .chain(index.notes.values().filter_map(|note| {
                    Some((new_of.get(&path_key(&note.path))?.clone(), &note.facts))
                }))
                .flat_map(|(owner, facts)| {
                    facts
                        .aliases
                        .iter()
                        .map(move |alias| (alias.clone(), owner.clone()))
                })
                .collect::<Vec<_>>();
        Some(Resolver::new(notes.iter().map(PathBuf::as_path), aliases))
    });
    let mut count = 0;
    for note in index.notes.values() {
        let from_new = new_of.get(&path_key(&note.path));
        for link in note
            .facts
            .links
            .iter()
            .filter(|link| link.kind != LinkKind::Image)
        {
            let Some(target) = source.resolver.resolve(link, &note.path) else {
                continue;
            };
            let target_new = new_of.get(&path_key(&target));
            match (from_new, target_new) {
                (None, None) => {}
                (None, Some(_)) | (Some(_), None) => count += 1,
                (Some(from), Some(to)) => {
                    if let Some(combined) = &combined
                        && combined
                            .resolve(link, from)
                            .is_none_or(|found| !super::model::same_path(&found, to))
                    {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

/// The images moved notes embed from outside every moving folder, in order of first use.
fn images(
    index: &NoteIndex,
    source: &Side,
    items: &[Item],
    new_of: &HashMap<String, PathBuf>,
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<Image> {
    let folders: Vec<&Path> = items
        .iter()
        .filter_map(|item| match item {
            Item::Folder(folder) => Some(folder.as_path()),
            Item::Note(_) => None,
        })
        .collect();
    let mut found: Vec<Image> = Vec::new();
    let mut moving_notes: Vec<&super::index::IndexedNote> = index
        .notes
        .values()
        .filter(|note| new_of.contains_key(&path_key(&note.path)))
        .collect();
    moving_notes.sort_by(|a, b| {
        super::tree::natural_cmp(&a.path.to_string_lossy(), &b.path.to_string_lossy())
    });
    for note in moving_notes {
        for link in &note.facts.links {
            let Some(target) = image_target(&note.path, link, source.attachments, exists) else {
                continue;
            };
            if folders
                .iter()
                .any(|folder| super::is_inside(folder, &target))
                || new_of.contains_key(&path_key(&target))
            {
                continue;
            }
            let usage = ImageUse {
                note: note.path.clone(),
                range: link.start as usize..link.end as usize,
                kind: link.kind,
                target: link.target.clone(),
            };
            match found
                .iter_mut()
                .find(|image| super::model::same_path(&image.source, &target))
            {
                Some(image) => image.uses.push(usage),
                None => found.push(Image {
                    source: target,
                    users_only_moving: true,
                    uses: vec![usage],
                }),
            }
        }
    }
    for note in index.notes.values() {
        if new_of.contains_key(&path_key(&note.path)) {
            continue;
        }
        for link in &note.facts.links {
            if let Some(target) = image_target(&note.path, link, source.attachments, exists)
                && let Some(image) = found
                    .iter_mut()
                    .find(|image| super::model::same_path(&image.source, &target))
            {
                image.users_only_moving = false;
            }
        }
    }
    found
}

/// A button's meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Answer {
    Move,
    MoveWithImages,
    MoveWithoutImages,
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Question {
    pub text: String,
    pub buttons: Vec<(&'static str, Answer)>,
}

fn counted(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// The question a move asks before it goes ahead (spec §3.2), or `None` when nothing would
/// break and no image stays behind.
pub fn question(
    plan: &MovePlan,
    item: &str,
    source_name: &str,
    dest_name: &str,
) -> Option<Question> {
    let start = format!("Move {item} to {dest_name}?");
    let move_or_cancel = vec![("Move", Answer::Move), ("Cancel", Answer::Cancel)];
    let Some(breaking) = plan.breaking else {
        return Some(Question {
            text: format!(
                "{start} {source_name} is still being indexed, so FastPad can't tell which links will stop working."
            ),
            buttons: move_or_cancel,
        });
    };
    let mut text = start;
    if breaking > 0 {
        text.push_str(&format!(
            " {} in {source_name} will stop working.",
            counted(breaking, "link", "links")
        ));
    }
    if !plan.images.is_empty() {
        text.push_str(&format!(
            " It uses {} from {source_name}'s attachments folder.",
            counted(plan.images.len(), "image", "images")
        ));
        return Some(Question {
            text,
            buttons: vec![
                ("Move with images", Answer::MoveWithImages),
                ("Move without images", Answer::MoveWithoutImages),
                ("Cancel", Answer::Cancel),
            ],
        });
    }
    (breaking > 0).then_some(Question {
        text,
        buttons: move_or_cancel,
    })
}

/// How the question and the notices name what moves: one item's name, or "*N* items".
pub fn item_label(items: &[Item]) -> String {
    match items {
        [item] => item
            .path()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        _ => format!("{} items", items.len()),
    }
}

/// Where an image named `name` goes in `folder`: its own name, or " (2)", " (3)" … before the
/// extension while that is `taken`.
pub fn image_destination(folder: &Path, name: &OsStr, taken: &dyn Fn(&Path) -> bool) -> PathBuf {
    let first = folder.join(name);
    if !taken(&first) {
        return first;
    }
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or(name).to_string_lossy();
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy());
    (2..)
        .map(|n| {
            folder.join(match &extension {
                Some(extension) => format!("{stem} ({n}).{extension}"),
                None => format!("{stem} ({n})"),
            })
        })
        .find(|candidate| !taken(candidate))
        .unwrap_or(first)
}

/// The edits to the note that moved from `note_old` to `note_new` (relative to the source and
/// the destination) that point its image links at where `moved` (source → destination) put
/// the images: a Markdown image gets its new relative path, a wiki embed the new file name.
/// In order of position.
pub fn image_link_edits(
    images: &[Image],
    note_old: &Path,
    note_new: &Path,
    moved: &[(PathBuf, PathBuf)],
) -> Vec<(Range<usize>, String)> {
    let mut edits: Vec<(Range<usize>, String)> = images
        .iter()
        .filter_map(|image| {
            let (_, new) = moved
                .iter()
                .find(|(old, _)| super::model::same_path(old, &image.source))?;
            Some((image, new))
        })
        .flat_map(|(image, new)| {
            image
                .uses
                .iter()
                .filter(|usage| super::model::same_path(&usage.note, note_old))
                .map(move |usage| {
                    let text = match usage.kind {
                        LinkKind::Image | LinkKind::Markdown => {
                            super::links::relative_target(note_new, new, &usage.target)
                        }
                        LinkKind::Wiki => new
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                    };
                    (usage.range.clone(), text)
                })
        })
        .collect();
    edits.sort_by_key(|(range, _)| range.start);
    edits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::facts::note_facts;
    use crate::library::index::IndexedNote;
    use crate::library::path_key;
    use std::collections::HashSet;

    fn index(notes: &[(&str, &str)]) -> NoteIndex {
        let mut index = NoteIndex::default();
        for (path, text) in notes {
            index.notes.insert(
                path_key(Path::new(path)),
                IndexedNote {
                    path: PathBuf::from(path),
                    size: text.len() as u64,
                    mtime: 1,
                    facts: note_facts(text),
                },
            );
        }
        index
    }

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    /// A notebook built from `(path, text)` notes.
    struct Book {
        notes: Vec<PathBuf>,
        index: NoteIndex,
        resolver: Resolver,
    }

    fn book(notes: &[(&str, &str)]) -> Book {
        let index = index(notes);
        let list = paths(&notes.iter().map(|(path, _)| *path).collect::<Vec<_>>());
        let resolver = Resolver::of(list.iter().map(PathBuf::as_path), &index);
        Book {
            notes: list,
            index,
            resolver,
        }
    }

    fn side(book: &Book) -> Side<'_> {
        Side {
            notes: &book.notes,
            index: Some(&book.index),
            resolver: &book.resolver,
            attachments: Path::new("attachments"),
        }
    }

    fn files(list: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<String> = list.iter().map(|p| path_key(Path::new(p))).collect();
        move |path: &Path| set.contains(&path_key(path))
    }

    fn none(_: &Path) -> bool {
        false
    }

    #[test]
    fn a_note_and_a_folder_map_into_the_destination_folder() {
        let a = book(&[
            ("a.md", ""),
            (r"work\b.md", ""),
            (r"work\deep\c.md", ""),
            ("stay.md", ""),
        ]);
        let b = book(&[]);
        let items = [Item::Note("a.md".into()), Item::Folder("work".into())];
        let plan = plan(
            &items,
            Path::new("projects"),
            &side(&a),
            Some(&side(&b)),
            &none,
            &none,
        );
        assert_eq!(
            plan.moves,
            vec![
                (Item::Note("a.md".into()), PathBuf::from(r"projects\a.md")),
                (Item::Folder("work".into()), PathBuf::from(r"projects\work")),
            ]
        );
        assert!(
            plan.notes
                .contains(&(r"work\deep\c.md".into(), r"projects\work\deep\c.md".into()))
        );
        assert!(
            plan.notes
                .contains(&("a.md".into(), r"projects\a.md".into()))
        );
        assert_eq!(plan.notes.len(), 3, "stay.md stays");
        assert!(plan.clashes.is_empty());
        assert_eq!(plan.breaking, Some(0));
    }

    #[test]
    fn any_clash_refuses_and_case_counts() {
        let a = book(&[("a.md", ""), ("b.md", "")]);
        let b = book(&[(r"projects\A.MD", "")]);
        let items = [Item::Note("a.md".into()), Item::Note("b.md".into())];
        let taken = files(&[r"projects\A.MD"]);
        let plan = plan(
            &items,
            Path::new("projects"),
            &side(&a),
            Some(&side(&b)),
            &taken,
            &none,
        );
        assert_eq!(plan.clashes, vec!["a.md".to_owned()]);
    }

    #[test]
    fn each_breaking_kind_counts_and_links_inside_the_set_do_not() {
        let a = book(&[
            // stay → move: counts.
            ("stay.md", "[[a]]"),
            // move → stay: counts. a → notes\Plan (moving): fine in the source, but at the
            // destination's root a same-named Plan.md wins: counts.
            ("a.md", "[[stay]] [[Plan]] ![i](attachments/x.png)"),
            (r"notes\Plan.md", ""),
            // Inside the moving folder: no count.
            (r"work\b.md", r"[[work\deep\c]]"),
            (r"work\deep\c.md", ""),
        ]);
        let b = book(&[("Plan.md", "")]);
        let items = [
            Item::Note("a.md".into()),
            Item::Folder("notes".into()),
            Item::Folder("work".into()),
        ];
        let plan = plan(
            &items,
            Path::new(""),
            &side(&a),
            Some(&side(&b)),
            &none,
            &none,
        );
        assert_eq!(plan.breaking, Some(3));
    }

    #[test]
    fn an_unbuilt_source_index_gives_none() {
        let a = book(&[("a.md", "![i](attachments/x.png)")]);
        let b = book(&[]);
        let mut source = side(&a);
        source.index = None;
        let exists = files(&[r"attachments\x.png"]);
        let plan = plan(
            &[Item::Note("a.md".into())],
            Path::new(""),
            &source,
            Some(&side(&b)),
            &none,
            &exists,
        );
        assert_eq!(plan.breaking, None);
        assert!(plan.images.is_empty());
    }

    #[test]
    fn images_used_only_by_moving_notes_move_and_shared_ones_copy() {
        let a = book(&[
            (
                "a.md",
                "![x](attachments/x.png)\n![y](attachments/y.png)\n![[z.png]]",
            ),
            ("stay.md", "![y](attachments/y.png)"),
        ]);
        let b = book(&[]);
        let exists = files(&[
            r"attachments\x.png",
            r"attachments\y.png",
            r"attachments\z.png",
        ]);
        let plan = plan(
            &[Item::Note("a.md".into())],
            Path::new(""),
            &side(&a),
            Some(&side(&b)),
            &none,
            &exists,
        );
        let found: Vec<(PathBuf, bool)> = plan
            .images
            .iter()
            .map(|image| (image.source.clone(), image.users_only_moving))
            .collect();
        assert_eq!(
            found,
            vec![
                (r"attachments\x.png".into(), true),
                (r"attachments\y.png".into(), false),
                (r"attachments\z.png".into(), true),
            ]
        );
        assert_eq!(plan.images[0].uses.len(), 1);
        assert_eq!(plan.images[0].uses[0].note, PathBuf::from("a.md"));
    }

    #[test]
    fn images_inside_a_moving_folder_are_left_out() {
        let a = book(&[(r"work\b.md", "![p](img/p.png)")]);
        let b = book(&[]);
        let exists = files(&[r"work\img\p.png"]);
        let plan = plan(
            &[Item::Folder("work".into())],
            Path::new(""),
            &side(&a),
            Some(&side(&b)),
            &none,
            &exists,
        );
        assert!(plan.images.is_empty());
    }

    fn with(breaking: Option<usize>, images: usize) -> MovePlan {
        MovePlan {
            breaking,
            images: (0..images)
                .map(|n| Image {
                    source: format!("attachments\\{n}.png").into(),
                    users_only_moving: true,
                    uses: Vec::new(),
                })
                .collect(),
            ..MovePlan::default()
        }
    }

    #[test]
    fn the_question_wording_and_buttons() {
        let ask = |plan: MovePlan| question(&plan, "a.md", "Work", "Personal");
        assert_eq!(ask(with(Some(0), 0)), None, "nothing breaks");
        let links = ask(with(Some(3), 0)).unwrap();
        assert_eq!(
            links.text,
            "Move a.md to Personal? 3 links in Work will stop working."
        );
        assert_eq!(
            links.buttons,
            vec![("Move", Answer::Move), ("Cancel", Answer::Cancel)]
        );
        assert_eq!(
            ask(with(Some(1), 0)).unwrap().text,
            "Move a.md to Personal? 1 link in Work will stop working."
        );
        let images = ask(with(Some(0), 2)).unwrap();
        assert_eq!(
            images.text,
            "Move a.md to Personal? It uses 2 images from Work's attachments folder."
        );
        assert_eq!(
            images.buttons,
            vec![
                ("Move with images", Answer::MoveWithImages),
                ("Move without images", Answer::MoveWithoutImages),
                ("Cancel", Answer::Cancel),
            ]
        );
        assert_eq!(
            ask(with(Some(1), 1)).unwrap().text,
            "Move a.md to Personal? 1 link in Work will stop working. It uses 1 image from Work's attachments folder."
        );
        let unknown = ask(with(None, 0)).unwrap();
        assert_eq!(
            unknown.text,
            "Move a.md to Personal? Work is still being indexed, so FastPad can't tell which links will stop working."
        );
        assert_eq!(
            unknown.buttons,
            vec![("Move", Answer::Move), ("Cancel", Answer::Cancel)]
        );
    }

    #[test]
    fn the_item_label_names_one_item_or_counts_several() {
        assert_eq!(item_label(&[Item::Note(r"x\a.md".into())]), "a.md");
        assert_eq!(
            item_label(&[Item::Folder(r"x\Projects".into())]),
            "Projects"
        );
        assert_eq!(
            item_label(&[Item::Note("a.md".into()), Item::Folder("b".into())]),
            "2 items"
        );
    }

    #[test]
    fn image_names_avoid_taken_ones() {
        let taken = files(&[r"assets\x.png", r"assets\x (2).png"]);
        assert_eq!(
            image_destination(Path::new("assets"), OsStr::new("x.png"), &taken),
            PathBuf::from(r"assets\x (3).png")
        );
        assert_eq!(
            image_destination(Path::new("assets"), OsStr::new("y.png"), &taken),
            PathBuf::from(r"assets\y.png")
        );
    }

    #[test]
    fn image_link_edits_change_only_targets() {
        let text = "See ![a](../attachments/x.png) and ![[x.png]].";
        let a = book(&[(r"work\b.md", text)]);
        let b = book(&[]);
        let exists = files(&[r"attachments\x.png"]);
        let plan = plan(
            &[Item::Note(r"work\b.md".into())],
            Path::new("p"),
            &side(&a),
            Some(&side(&b)),
            &none,
            &exists,
        );
        let moved = [(
            PathBuf::from(r"attachments\x.png"),
            PathBuf::from(r"assets\x (2).png"),
        )];
        let edits = image_link_edits(
            &plan.images,
            Path::new(r"work\b.md"),
            Path::new(r"p\b.md"),
            &moved,
        );
        let mut out = text.to_owned();
        for (range, new) in edits.iter().rev() {
            out.replace_range(range.clone(), new);
        }
        assert_eq!(out, "See ![a](../assets/x%20(2).png) and ![[x (2).png]].");
    }
}
