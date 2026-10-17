//! Links between notes (second brain phase 1 spec §7): what a link points at, which notes link
//! to a note, which link targets don't exist, which notes nothing links to, and the edits that
//! keep links pointing at notes that moved. Pure: works on the note list and the note index.

use super::facts::{LinkKind, LinkUse};
use super::index::NoteIndex;
use super::path_key;
use super::tree::natural_cmp;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

/// Finds the notes links point at. Built from the notebook's note list (every note, not only
/// Markdown ones, may be a link target) and the index's front matter aliases.
#[derive(Debug, Default)]
pub struct Resolver {
    by_path: HashMap<String, PathBuf>,
    by_stem: HashMap<String, Vec<PathBuf>>,
    by_name: HashMap<String, Vec<PathBuf>>,
    by_alias: HashMap<String, Vec<PathBuf>>,
}

/// `Path::file_stem`, lowercased.
fn stem_key(path: &Path) -> Option<String> {
    Some(path.file_stem()?.to_string_lossy().to_lowercase())
}

impl Resolver {
    /// `notes` and alias owners are relative to the notebook.
    pub fn new<'a>(
        notes: impl IntoIterator<Item = &'a Path>,
        aliases: impl IntoIterator<Item = (String, PathBuf)>,
    ) -> Self {
        let mut resolver = Self::default();
        for note in notes {
            resolver.by_path.insert(path_key(note), note.to_path_buf());
            if let Some(stem) = stem_key(note) {
                resolver
                    .by_stem
                    .entry(stem)
                    .or_default()
                    .push(note.to_path_buf());
            }
            if let Some(name) = note.file_name() {
                resolver
                    .by_name
                    .entry(name.to_string_lossy().to_lowercase())
                    .or_default()
                    .push(note.to_path_buf());
            }
        }
        for (alias, owner) in aliases {
            resolver
                .by_alias
                .entry(alias.to_lowercase())
                .or_default()
                .push(owner);
        }
        resolver
    }

    /// The resolver for `notes` with the aliases `index` knows.
    pub fn of<'a>(notes: impl IntoIterator<Item = &'a Path>, index: &NoteIndex) -> Self {
        Self::new(notes, aliases_of(index, &|path| path.to_path_buf()))
    }

    /// The note `link`, written in the note at `from`, points at.
    pub fn resolve(&self, link: &LinkUse, from: &Path) -> Option<PathBuf> {
        match link.kind {
            LinkKind::Wiki => self.resolve_wiki(&link.target, from),
            LinkKind::Markdown | LinkKind::Image => self.resolve_relative(&link.target, from),
        }
    }

    /// A wikilink target (spec §7.2): a path form relative to the root, then to `from`'s folder;
    /// else a file stem or file name; else an alias. Ties go to `from`'s folder, then to the
    /// fewest folders, then to natural order.
    pub fn resolve_wiki(&self, target: &str, from: &Path) -> Option<PathBuf> {
        let target = target.trim().replace('/', "\\");
        if target.is_empty() {
            return None;
        }
        if target.contains('\\') {
            let folder = from.parent().unwrap_or(Path::new(""));
            for base in [Path::new(""), folder] {
                let Some(joined) = normalize(&base.join(&target)) else {
                    continue;
                };
                for candidate in with_note_extensions(&joined) {
                    if let Some(found) = self.by_path.get(&path_key(&candidate)) {
                        return Some(found.clone());
                    }
                }
            }
            return None;
        }
        let key = target.to_lowercase();
        let has_extension = Path::new(&target)
            .extension()
            .is_some_and(|extension| super::title::is_note_extension(&extension.to_string_lossy()));
        let candidates = if has_extension {
            self.by_name.get(&key)
        } else {
            self.by_stem.get(&key)
        };
        candidates
            .or_else(|| self.by_alias.get(&key))
            .and_then(|candidates| best(candidates, from))
    }

    /// A Markdown link's destination: a path relative to `from`'s folder, percent-decoded.
    pub fn resolve_relative(&self, dest: &str, from: &Path) -> Option<PathBuf> {
        let decoded = percent_decode(dest).replace('/', "\\");
        let folder = from.parent().unwrap_or(Path::new(""));
        let joined = normalize(&folder.join(decoded))?;
        self.by_path.get(&path_key(&joined)).cloned()
    }

    /// Whether some note is at `path` (relative, any case).
    pub fn contains(&self, path: &Path) -> bool {
        self.by_path.contains_key(&path_key(path))
    }

    /// How many notes a wikilink by stem `stem` could mean.
    fn stem_count(&self, stem: &str) -> usize {
        self.by_stem.get(&stem.to_lowercase()).map_or(0, Vec::len)
            + self.by_alias.get(&stem.to_lowercase()).map_or(0, Vec::len)
    }
}

/// `(alias, owner)` for every alias in `index`, the owner mapped through `place`.
fn aliases_of(index: &NoteIndex, place: &dyn Fn(&Path) -> PathBuf) -> Vec<(String, PathBuf)> {
    index
        .notes
        .values()
        .flat_map(|note| {
            let owner = place(&note.path);
            note.facts
                .aliases
                .iter()
                .map(move |alias| (alias.clone(), owner.clone()))
        })
        .collect()
}

/// `path`, then with `.md` and `.markdown` when it has no extension.
fn with_note_extensions(path: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![path.to_path_buf()];
    if path.extension().is_none() {
        candidates.push(path.with_extension("md"));
        candidates.push(path.with_extension("markdown"));
    }
    candidates
}

/// `path` without `.` and `..` parts, or `None` when it climbs above the notebook or isn't a
/// plain relative path.
fn normalize(path: &Path) -> Option<PathBuf> {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
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

/// The candidate a link from `from` means: the one in `from`'s folder, then the one with the
/// fewest folders, then the first in natural order.
fn best(candidates: &[PathBuf], from: &Path) -> Option<PathBuf> {
    let folder = from.parent().unwrap_or(Path::new(""));
    candidates
        .iter()
        .min_by(|a, b| {
            let near = |path: &PathBuf| {
                !path
                    .parent()
                    .is_some_and(|parent| super::model::same_path(parent, folder))
            };
            near(a)
                .cmp(&near(b))
                .then(a.components().count().cmp(&b.components().count()))
                .then_with(|| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()))
        })
        .cloned()
}

pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(hex) = text.get(index + 1..index + 3)
            && let Ok(value) = u8::from_str_radix(hex, 16)
        {
            output.push(value);
            index += 3;
            continue;
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

/// A note that links to another, with the links and their lines.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Backlink {
    pub from: PathBuf,
    pub links: Vec<LinkUse>,
}

/// The notes that link to `note`, in natural order of their paths (spec §7.5).
pub fn backlinks(index: &NoteIndex, resolver: &Resolver, note: &Path) -> Vec<Backlink> {
    let mut found: Vec<Backlink> = index
        .notes
        .values()
        .filter(|source| !super::model::same_path(&source.path, note))
        .filter_map(|source| {
            let links: Vec<LinkUse> = source
                .facts
                .links
                .iter()
                .filter(|link| link.kind != LinkKind::Image)
                .filter(|link| {
                    resolver
                        .resolve(link, &source.path)
                        .is_some_and(|target| super::model::same_path(&target, note))
                })
                .cloned()
                .collect();
            (!links.is_empty()).then(|| Backlink {
                from: source.path.clone(),
                links,
            })
        })
        .collect();
    found.sort_by(|a, b| natural_cmp(&a.from.to_string_lossy(), &b.from.to_string_lossy()));
    found
}

/// A link out of a note, and the note it reaches, if any.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Outgoing {
    pub link: LinkUse,
    pub target: Option<PathBuf>,
}

/// `note`'s links in order, once per target (spec §7.5).
pub fn outgoing(index: &NoteIndex, resolver: &Resolver, note: &Path) -> Vec<Outgoing> {
    let Some(facts) = index.facts(note) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    facts
        .links
        .iter()
        .filter(|link| link.kind != LinkKind::Image)
        .filter_map(|link| {
            let target = resolver.resolve(link, note);
            let key = match &target {
                Some(path) => path_key(path),
                None => format!("?{}", link.target.to_lowercase()),
            };
            seen.insert(key).then(|| Outgoing {
                link: link.clone(),
                target,
            })
        })
        .collect()
}

/// A wikilink target no note matches, and the notes that link to it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MissingNote {
    /// As first written.
    pub target: String,
    pub from: Vec<PathBuf>,
}

/// Where a notebook keeps the notes the link lists treat apart (spec §7.5, §9.1, §9.4).
#[derive(Clone, Copy, Debug)]
pub struct Conventions<'a> {
    pub daily_folder: &'a Path,
    pub inbox: &'a Path,
    /// Links written in templates are placeholders, not links.
    pub templates: &'a Path,
}

/// Whether `path` (relative) is in `folder` or a folder under it.
pub fn in_folder(path: &Path, folder: &Path) -> bool {
    path.parent().is_some_and(|parent| {
        super::is_inside(folder, parent) || super::model::same_path(parent, folder)
    })
}

/// Every wikilink target in the notebook that matches no note, most linked first (spec §7.5).
/// Links in templates don't count.
pub fn missing(index: &NoteIndex, resolver: &Resolver, templates: &Path) -> Vec<MissingNote> {
    let mut by_target: HashMap<String, MissingNote> = HashMap::new();
    let mut notes: Vec<_> = index
        .notes
        .values()
        .filter(|note| !in_folder(&note.path, templates))
        .collect();
    notes.sort_by(|a, b| natural_cmp(&a.path.to_string_lossy(), &b.path.to_string_lossy()));
    for source in notes {
        for link in &source.facts.links {
            if link.kind != LinkKind::Wiki || resolver.resolve(link, &source.path).is_some() {
                continue;
            }
            let entry = by_target
                .entry(link.target.to_lowercase())
                .or_insert_with(|| MissingNote {
                    target: link.target.clone(),
                    from: Vec::new(),
                });
            if !entry
                .from
                .iter()
                .any(|from| super::model::same_path(from, &source.path))
            {
                entry.from.push(source.path.clone());
            }
        }
    }
    let mut missing: Vec<MissingNote> = by_target.into_values().collect();
    missing.sort_by(|a, b| {
        b.from
            .len()
            .cmp(&a.from.len())
            .then_with(|| natural_cmp(&a.target, &b.target))
    });
    missing
}

/// Notes nothing is expected to link to (spec §7.5): the daily notes, the inbox, templates, the
/// index and log of agent-maintained vaults, and instructions and readmes.
fn expected_unlinked(path: &Path, conventions: &Conventions<'_>) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    ["index.md", "log.md", "claude.md", "readme.md"].contains(&name.as_str())
        || super::model::same_path(path, conventions.inbox)
        || in_folder(path, conventions.daily_folder)
        || in_folder(path, conventions.templates)
}

/// Markdown notes no other note links to, most recently changed first (spec §7.5).
pub fn unlinked(
    index: &NoteIndex,
    resolver: &Resolver,
    conventions: &Conventions<'_>,
) -> Vec<PathBuf> {
    let mut linked = HashSet::new();
    for source in index.notes.values() {
        for link in &source.facts.links {
            if let Some(target) = resolver.resolve(link, &source.path)
                && !super::model::same_path(&target, &source.path)
            {
                linked.insert(path_key(&target));
            }
        }
    }
    let mut notes: Vec<_> = index
        .notes
        .values()
        .filter(|note| !linked.contains(&path_key(&note.path)))
        .filter(|note| !expected_unlinked(&note.path, conventions))
        .collect();
    notes.sort_by(|a, b| {
        b.mtime
            .cmp(&a.mtime)
            .then_with(|| natural_cmp(&a.path.to_string_lossy(), &b.path.to_string_lossy()))
    });
    notes.into_iter().map(|note| note.path.clone()).collect()
}

/// The link edits one note needs after notes moved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteEdits {
    /// Where the note is now (after the move, if it moved itself).
    pub note: PathBuf,
    /// Its size and write time when indexed, so a note changed since is left alone.
    pub size: u64,
    pub mtime: u64,
    /// The note is one of the ones that moved: these are its own relative links.
    pub own: bool,
    /// Byte ranges of link targets and their new text, in order.
    pub edits: Vec<(Range<usize>, String)>,
}

/// The edits that keep every link pointing at the note it pointed at before `moves` (old, new;
/// relative paths) took place (spec §7.6, §10.5). `notes` is the note list after the moves. A
/// link that already reaches the moved note needs no edit.
/// `edits_for_moves`, where links to files the notebook doesn't list (a PDF, a log) count too:
/// a moved note's own relative link to such a file, which `exists` (relative to the notebook)
/// says is still where the link pointed, is rewritten to keep reaching it (spec §10.5).
pub fn edits_for_moves(
    index: &NoteIndex,
    notes: &[PathBuf],
    moves: &[(PathBuf, PathBuf)],
) -> Vec<NoteEdits> {
    edits_for_moves_with(index, notes, moves, &|_| false)
}

/// `edits_for_moves`, also keeping a moved note's relative links to unlisted files that `exists`.
pub fn edits_for_moves_with(
    index: &NoteIndex,
    notes: &[PathBuf],
    moves: &[(PathBuf, PathBuf)],
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<NoteEdits> {
    let moved = |path: &Path| -> PathBuf {
        moves
            .iter()
            .find(|(old, _)| super::model::same_path(old, path))
            .map_or_else(|| path.to_path_buf(), |(_, new)| new.clone())
    };
    let before_notes: Vec<PathBuf> = notes
        .iter()
        .map(|note| {
            moves
                .iter()
                .find(|(_, new)| super::model::same_path(new, note))
                .map_or_else(|| note.clone(), |(old, _)| old.clone())
        })
        .collect();
    let before = Resolver::new(
        before_notes.iter().map(PathBuf::as_path),
        aliases_of(index, &|path| path.to_path_buf()),
    );
    let after = Resolver::new(
        notes.iter().map(PathBuf::as_path),
        aliases_of(index, &moved),
    );
    let mut all = Vec::new();
    let mut sources: Vec<_> = index.notes.values().collect();
    sources.sort_by(|a, b| a.path.cmp(&b.path));
    for source in sources {
        let old_from = &source.path;
        let new_from = moved(old_from);
        let own = !super::model::same_path(old_from, &new_from);
        let mut edits = Vec::new();
        for link in &source.facts.links {
            let Some(was) = before.resolve(link, old_from) else {
                if own
                    && link.kind != LinkKind::Wiki
                    && let Some(text) = unlisted_target(link, old_from, &new_from, exists)
                {
                    edits.push((link.start as usize..link.end as usize, text));
                }
                continue;
            };
            let now = moved(&was);
            if !own && super::model::same_path(&was, &now) {
                continue;
            }
            if after
                .resolve(link, &new_from)
                .is_some_and(|reached| super::model::same_path(&reached, &now))
            {
                continue;
            }
            let text = match link.kind {
                LinkKind::Wiki => wiki_target(&after, &now, &new_from),
                LinkKind::Markdown | LinkKind::Image => {
                    relative_target(&new_from, &now, &link.target)
                }
            };
            edits.push((link.start as usize..link.end as usize, text));
        }
        if !edits.is_empty() {
            all.push(NoteEdits {
                note: new_from,
                size: source.size,
                mtime: source.mtime,
                own,
                edits,
            });
        }
    }
    all
}

/// A moved note's relative link to a file the notebook doesn't list: its new text when the file
/// is still where the link pointed from `old_from` and `new_from` no longer reaches it.
fn unlisted_target(
    link: &LinkUse,
    old_from: &Path,
    new_from: &Path,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<String> {
    let decoded = percent_decode(&link.target).replace('/', "\\");
    let from_folder = |note: &Path| note.parent().unwrap_or(Path::new("")).to_path_buf();
    let target = normalize(&from_folder(old_from).join(&decoded))?;
    if !exists(&target) {
        return None;
    }
    let reached = normalize(&from_folder(new_from).join(&decoded));
    if reached.is_some_and(|reached| super::model::same_path(&reached, &target)) {
        return None;
    }
    Some(relative_target(new_from, &target, &link.target))
}

/// The shortest wikilink target that reaches `note`: its stem when no other note shares it, else
/// its path from the notebook root with `/`, leaving out `.md`.
pub fn wiki_target(resolver: &Resolver, note: &Path, from: &Path) -> String {
    let markdown = super::index::is_indexed(note);
    let stem = note
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path = if markdown {
        note.with_extension("")
    } else {
        note.to_path_buf()
    }
    .to_string_lossy()
    .replace('\\', "/");
    // The first form that reaches the note from where the link is: a bare name can mean a note
    // in the linking note's own folder, so a root note may need `./name`.
    let mut candidates = Vec::new();
    if markdown && resolver.stem_count(&stem) <= 1 {
        candidates.push(stem);
    }
    candidates.push(path.clone());
    candidates.push(format!("./{path}"));
    candidates
        .into_iter()
        .find(|candidate| {
            resolver
                .resolve_wiki(candidate, from)
                .is_some_and(|reached| super::model::same_path(&reached, note))
        })
        .unwrap_or(path)
}

/// `to` relative to `from`'s folder with `/`, spaces percent-encoded when `original` encoded
/// anything or had none to encode, so the destination stays one Markdown token.
pub fn relative_target(from: &Path, to: &Path, original: &str) -> String {
    let base: Vec<_> = from
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .collect();
    let target: Vec<_> = to.components().collect();
    let common = base
        .iter()
        .zip(&target)
        .take_while(|(a, b)| {
            a.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
        })
        .count();
    let mut parts: Vec<String> =
        std::iter::repeat_n("..".to_owned(), base.len() - common).collect();
    parts.extend(
        target[common..]
            .iter()
            .map(|part| part.as_os_str().to_string_lossy().into_owned()),
    );
    let joined = parts.join("/");
    if original.contains('%') || (!original.contains(' ') && joined.contains(' ')) {
        joined.replace('%', "%25").replace(' ', "%20")
    } else {
        joined
    }
}

/// A `[[wikilink]]` found in one line of text, for the editor (spec §7.3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineLink {
    /// Byte range of the whole `[[…]]` in the line.
    pub range: Range<usize>,
    pub target: String,
    pub heading: Option<String>,
}

/// The wikilinks in `line`, outside inline code spans. A quick scan for the editor's marks and
/// clicks, which only ever look at the lines on screen.
pub fn wikilinks_in_line(line: &str) -> Vec<LineLink> {
    let mut links = Vec::new();
    let mut at = 0;
    let mut in_code = false;
    let bytes = line.as_bytes();
    while at < bytes.len() {
        if bytes[at] == b'`' {
            in_code = !in_code;
            at += 1;
            continue;
        }
        if !in_code
            && line[at..].starts_with("[[")
            && let Some(close) = line[at + 2..].find("]]")
        {
            let inner = &line[at + 2..at + 2 + close];
            if !inner.contains("[[") && !inner.trim().is_empty() {
                let before_label = inner.find('|').map_or(inner, |bar| &inner[..bar]);
                let (target, heading) = match before_label.split_once('#') {
                    Some((target, heading)) => (target, Some(heading.trim().to_owned())),
                    None => (before_label, None),
                };
                let end = at + 2 + close + 2;
                links.push(LineLink {
                    range: at..end,
                    target: target.trim().to_owned(),
                    heading: heading.filter(|heading| !heading.is_empty()),
                });
                at = end;
                continue;
            }
        }
        at += line[at..].chars().next().map_or(1, char::len_utf8);
    }
    links
}

#[cfg(test)]
mod line_tests {
    use super::*;

    #[test]
    fn links_in_a_line_skip_inline_code_and_keep_their_parts() {
        let found = wikilinks_in_line("See [[Plan#Goals|goals]], `[[code]]` and [[Ünï cödé]].");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].range, 4..24);
        assert_eq!(found[0].target, "Plan");
        assert_eq!(found[0].heading.as_deref(), Some("Goals"));
        assert_eq!(found[1].target, "Ünï cödé");
        assert!(wikilinks_in_line("[[unclosed and [] ]").is_empty());
        assert!(wikilinks_in_line("[[ ]]").is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::facts::{NoteFacts, note_facts};
    use crate::library::index::IndexedNote;

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

    fn resolver(notes: &[&str], index: &NoteIndex) -> Resolver {
        let notes = paths(notes);
        Resolver::of(notes.iter().map(PathBuf::as_path), index)
    }

    fn wiki(target: &str) -> LinkUse {
        LinkUse {
            target: target.to_owned(),
            heading: None,
            line: 0,
            kind: LinkKind::Wiki,
            start: 0,
            end: 0,
        }
    }

    #[test]
    fn wikilinks_resolve_by_path_stem_name_and_alias_with_tie_breaks() {
        let index = index(&[("people/dana.md", "---\naliases: [D]\n---\n")]);
        let resolver = resolver(
            &[
                "plan.md",
                r"work\plan.md",
                r"work\deep\plan.md",
                r"people\dana.md",
                "data.json",
                r"sub\note.markdown",
            ],
            &index,
        );
        let from_root = Path::new("index.md");
        let from_work = Path::new(r"work\today.md");
        let r = |target: &str, from: &Path| resolver.resolve(&wiki(target), from);
        assert_eq!(r("Plan", from_root), Some(PathBuf::from("plan.md")));
        assert_eq!(
            r("plan", from_work),
            Some(PathBuf::from(r"work\plan.md")),
            "same folder wins"
        );
        assert_eq!(
            r("work/deep/plan", from_root),
            Some(PathBuf::from(r"work\deep\plan.md"))
        );
        assert_eq!(
            r("deep/plan", from_work),
            Some(PathBuf::from(r"work\deep\plan.md")),
            "relative to the note"
        );
        assert_eq!(r("data.json", from_root), Some(PathBuf::from("data.json")));
        assert_eq!(
            r("data", from_root),
            Some(PathBuf::from("data.json")),
            "a stem of any note"
        );
        assert_eq!(
            r("sub/note", from_root),
            Some(PathBuf::from(r"sub\note.markdown"))
        );
        assert_eq!(
            r("d", from_root),
            Some(PathBuf::from(r"people\dana.md")),
            "an alias"
        );
        assert_eq!(r("nowhere", from_root), None);
        assert_eq!(r("../plan", from_root), None, "never above the notebook");
    }

    #[test]
    fn markdown_links_resolve_relative_to_their_note_and_decode() {
        let resolver = resolver(&[r"work\a b.md", "top.md"], &NoteIndex::default());
        let md = |target: &str| LinkUse {
            kind: LinkKind::Markdown,
            ..wiki(target)
        };
        assert_eq!(
            resolver.resolve(&md("a%20b.md"), Path::new(r"work\x.md")),
            Some(PathBuf::from(r"work\a b.md"))
        );
        assert_eq!(
            resolver.resolve(&md("../top.md"), Path::new(r"work\x.md")),
            Some(PathBuf::from("top.md"))
        );
    }

    #[test]
    fn backlinks_outgoing_missing_and_unlinked() {
        let index = index(&[
            ("a.md", "See [[b]] and [[Ghost]], [[b|again]]."),
            ("b.md", "Back to [a](a.md) and [[Ghost]] and [[Phantom]]."),
            ("lonely.md", "nothing"),
            (r"journal\2026-10-16.md", "today"),
            ("inbox.md", "- x"),
            (r"templates\Meeting.md", "[[{{title}} notes]]"),
        ]);
        let notes = [
            "a.md",
            "b.md",
            "lonely.md",
            r"journal\2026-10-16.md",
            "inbox.md",
            r"templates\Meeting.md",
        ];
        let resolver = resolver(&notes, &index);
        let back = backlinks(&index, &resolver, Path::new("b.md"));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].from, PathBuf::from("a.md"));
        assert_eq!(back[0].links.len(), 2);

        let out = outgoing(&index, &resolver, Path::new("a.md"));
        let targets: Vec<_> = out
            .iter()
            .map(|o| (o.link.target.as_str(), o.target.is_some()))
            .collect();
        assert_eq!(targets, [("b", true), ("Ghost", false)]);

        let gaps = missing(&index, &resolver, Path::new("templates"));
        assert_eq!(gaps.len(), 2, "a template's links are placeholders");
        assert_eq!(gaps[0].target, "Ghost");
        assert_eq!(gaps[0].from.len(), 2);
        assert_eq!(gaps[1].target, "Phantom");

        assert_eq!(
            unlinked(
                &index,
                &resolver,
                &Conventions {
                    daily_folder: Path::new("journal"),
                    inbox: Path::new("inbox.md"),
                    templates: Path::new("templates"),
                }
            ),
            [PathBuf::from("lonely.md")]
        );
    }

    #[test]
    fn a_rename_rewrites_the_links_that_would_break_and_keeps_their_form() {
        let text_a = "See [[Old]], [[Old#Goals|goals]] and [old](Old.md).";
        let index = index(&[("a.md", text_a), ("Old.md", "x"), (r"sub\Other.md", "y")]);
        let after = paths(&["a.md", "New Name.md", r"sub\Other.md"]);
        let edits = edits_for_moves(
            &index,
            &after,
            &[(PathBuf::from("Old.md"), PathBuf::from("New Name.md"))],
        );
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].note, PathBuf::from("a.md"));
        assert!(!edits[0].own);
        let mut text = text_a.to_owned();
        for (range, new) in edits[0].edits.iter().rev() {
            text.replace_range(range.clone(), new);
        }
        assert_eq!(
            text,
            "See [[New Name]], [[New Name#Goals|goals]] and [old](New%20Name.md)."
        );
    }

    #[test]
    fn a_moved_note_keeps_its_relative_links_and_an_ambiguous_stem_becomes_a_path() {
        let index = index(&[
            ("note.md", "[top](top.md) [[plan]] ![i](att/s.png)"),
            ("top.md", ""),
            ("plan.md", ""),
            (r"deep\plan.md", ""),
        ]);
        let after = paths(&[
            r"deep\note.md",
            "top.md",
            "plan.md",
            r"deep\plan.md",
            r"att\s.png",
        ]);
        let edits = edits_for_moves(
            &index,
            &after,
            &[(PathBuf::from("note.md"), PathBuf::from(r"deep\note.md"))],
        );
        assert_eq!(edits.len(), 1);
        assert!(edits[0].own);
        let new: Vec<_> = edits[0]
            .edits
            .iter()
            .map(|(_, text)| text.as_str())
            .collect();
        // From `deep\`, `[[plan]]` would now mean `deep\plan.md`, so it names the root one; the
        // image is listed in the notebook, so its relative link follows the move too.
        assert_eq!(new, ["../top.md", "./plan", "../att/s.png"]);
    }

    #[test]
    fn a_moved_notes_links_to_unlisted_files_follow_it_while_the_files_stay() {
        // Break caught: a PDF or log attached to a note left unreachable when the note moved.
        let index = index(&[(
            "note.md",
            "[spec](attachments/spec%20v2.pdf) [gone](attachments/old.log)",
        )]);
        let after = paths(&[r"deep\note.md"]);
        let on_disk = [PathBuf::from(r"attachments\spec v2.pdf")];
        let edits = edits_for_moves_with(
            &index,
            &after,
            &[(PathBuf::from("note.md"), PathBuf::from(r"deep\note.md"))],
            &|path| {
                on_disk
                    .iter()
                    .any(|file| super::super::model::same_path(file, path))
            },
        );
        let new: Vec<_> = edits[0]
            .edits
            .iter()
            .map(|(_, text)| text.as_str())
            .collect();
        assert_eq!(
            new,
            ["../attachments/spec%20v2.pdf"],
            "a missing file is left alone"
        );
    }

    #[test]
    fn links_that_still_reach_the_moved_note_are_left_alone() {
        // A move to another folder keeps the stem: [[Old]] still finds it.
        let index = index(&[("a.md", "[[Old]]"), ("Old.md", "")]);
        let after = paths(&["a.md", r"archive\Old.md"]);
        let edits = edits_for_moves(
            &index,
            &after,
            &[(PathBuf::from("Old.md"), PathBuf::from(r"archive\Old.md"))],
        );
        assert!(edits.is_empty());
        let _ = NoteFacts::default();
    }
}
