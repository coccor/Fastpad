//! The note index (second brain phase 1 spec §6): what each Markdown note of the open notebook
//! says about itself (tags, front matter aliases and properties), kept by size and write time so a
//! rebuild reads only the notes that changed, and cached on disk between runs. Built on a worker;
//! the UI thread only swaps a finished index in.

use super::facts::{self, NoteFacts, TagSource, TagUse};
use super::{NoteEntry, path_key};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Notes larger than this are left out of the index.
pub const MAX_NOTE_BYTES: u64 = 2 * 1024 * 1024;
const CACHE_VERSION: &str = "1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexedNote {
    /// Relative to the notebook.
    pub path: PathBuf,
    pub size: u64,
    /// FILETIME ticks, as the note list keeps them.
    pub mtime: u64,
    pub facts: NoteFacts,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NoteIndex {
    /// By lowercase relative path.
    pub notes: HashMap<String, IndexedNote>,
}

/// Whether the index reads `path`: a Markdown note.
pub fn is_indexed(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
    })
}

impl NoteIndex {
    /// The index of `listed` (the notebook's note list), reusing `previous` facts whose size and
    /// write time still match and reading the rest from `folder`. Also returns whether anything
    /// differs from `previous`.
    pub fn build(folder: &Path, listed: &[NoteEntry], previous: &NoteIndex) -> (NoteIndex, bool) {
        let mut notes = HashMap::new();
        let mut changed = false;
        for entry in listed {
            if !is_indexed(&entry.path) || entry.size > MAX_NOTE_BYTES || entry.online_only {
                continue;
            }
            let key = path_key(&entry.path);
            if let Some(known) = previous.notes.get(&key)
                && known.size == entry.size
                && known.mtime == entry.mtime
                && known.path == entry.path
            {
                notes.insert(key, known.clone());
                continue;
            }
            changed = true;
            // A note that can't be read now is left out; the next build tries again.
            let Ok(file) = crate::file::loader::load(&folder.join(&entry.path)) else {
                continue;
            };
            notes.insert(
                key,
                IndexedNote {
                    path: entry.path.clone(),
                    size: entry.size,
                    mtime: entry.mtime,
                    facts: facts::note_facts(&file.text),
                },
            );
        }
        changed |= notes.len() != previous.notes.len();
        (NoteIndex { notes }, changed)
    }

    /// The facts of the note at `relative`, if it is indexed.
    pub fn facts(&self, relative: &Path) -> Option<&NoteFacts> {
        self.notes.get(&path_key(relative)).map(|note| &note.facts)
    }

    /// The tags of the notebook as a tree split on `/` (spec §8.1).
    pub fn tags(&self) -> Vec<TagNode> {
        // Every spelling of each tag, by lowercase full name, and the notes that use it directly.
        let mut spellings: BTreeMap<String, HashMap<String, usize>> = BTreeMap::new();
        let mut direct: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for note in self.notes.values() {
            let mut seen = Vec::new();
            for tag in &note.facts.tags {
                let key = tag.name.to_lowercase();
                *spellings
                    .entry(key.clone())
                    .or_default()
                    .entry(tag.name.clone())
                    .or_default() += 1;
                for parent in facts::parent_tags(&tag.name) {
                    spellings.entry(parent.to_lowercase()).or_default();
                }
                if !seen.contains(&key) {
                    direct
                        .entry(key.clone())
                        .or_default()
                        .push(note.path.clone());
                    seen.push(key);
                }
            }
        }
        let display = |key: &str| -> String {
            let last = key.rsplit('/').next().unwrap_or(key);
            spellings
                .get(key)
                .and_then(|counts| {
                    let best = counts.values().max()?;
                    counts
                        .iter()
                        .filter(|(_, count)| *count == best)
                        .map(|(spelling, _)| spelling.as_str())
                        .min_by(|a, b| super::tree::natural_cmp(a, b))
                })
                .map(|spelling| spelling.rsplit('/').next().unwrap_or(spelling).to_owned())
                .unwrap_or_else(|| last.to_owned())
        };
        fn build_level(
            parent: Option<&str>,
            keys: &[String],
            direct: &HashMap<String, Vec<PathBuf>>,
            display: &dyn Fn(&str) -> String,
        ) -> Vec<TagNode> {
            let mut nodes: Vec<TagNode> = keys
                .iter()
                .filter(|key| match parent {
                    None => !key.contains('/'),
                    Some(parent) => key
                        .strip_prefix(parent)
                        .and_then(|rest| rest.strip_prefix('/'))
                        .is_some_and(|rest| !rest.contains('/')),
                })
                .map(|key| {
                    let children = build_level(Some(key), keys, direct, display);
                    let mut notes = direct.get(key).cloned().unwrap_or_default();
                    notes.sort_by(|a, b| {
                        super::tree::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy())
                    });
                    let mut all: Vec<&PathBuf> = notes.iter().collect();
                    let mut stack: Vec<&TagNode> = children.iter().collect();
                    while let Some(node) = stack.pop() {
                        all.extend(node.notes.iter());
                        stack.extend(node.children.iter());
                    }
                    all.sort();
                    all.dedup();
                    TagNode {
                        name: display(key),
                        full: key.clone(),
                        count: all.len(),
                        notes,
                        children,
                    }
                })
                .collect();
            nodes.sort_by(|a, b| super::tree::natural_cmp(&a.name, &b.name));
            nodes
        }
        let keys: Vec<String> = spellings.keys().cloned().collect();
        build_level(None, &keys, &direct, &display)
    }

    /// The cache file's text.
    pub fn encode(&self) -> String {
        let mut notes: Vec<&IndexedNote> = self.notes.values().collect();
        notes.sort_by(|a, b| a.path.cmp(&b.path));
        let mut output = format!("version={CACHE_VERSION}\r\n");
        for note in notes {
            output.push_str(&format!(
                "note={}|{}|{}\r\n",
                note.size,
                note.mtime,
                note.path.to_string_lossy()
            ));
            for tag in &note.facts.tags {
                let source = match tag.source {
                    TagSource::FrontMatter => 'f',
                    TagSource::Inline => 'i',
                };
                output.push_str(&format!("tag={source}|{}|{}\r\n", tag.line, tag.name));
            }
            for alias in &note.facts.aliases {
                output.push_str(&format!("alias={}\r\n", escape(alias)));
            }
            for (key, value) in &note.facts.properties {
                output.push_str(&format!("prop={}|{}\r\n", escape(key), escape(value)));
            }
        }
        output
    }

    /// The index a cache file holds, or `None` for a file of another version or with a line
    /// that doesn't parse: the whole cache is thrown away then, and every note is read again.
    pub fn parse(source: &str) -> Option<NoteIndex> {
        let mut lines = source.lines();
        if lines.next()? != format!("version={CACHE_VERSION}") {
            return None;
        }
        let mut notes = HashMap::new();
        let mut current: Option<IndexedNote> = None;
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let (key, value) = line.split_once('=')?;
            match key {
                "note" => {
                    if let Some(note) = current.take() {
                        notes.insert(path_key(&note.path), note);
                    }
                    let mut fields = value.splitn(3, '|');
                    current = Some(IndexedNote {
                        size: fields.next()?.parse().ok()?,
                        mtime: fields.next()?.parse().ok()?,
                        path: PathBuf::from(fields.next()?),
                        facts: NoteFacts::default(),
                    });
                }
                "tag" => {
                    let mut fields = value.splitn(3, '|');
                    let source = match fields.next()? {
                        "f" => TagSource::FrontMatter,
                        "i" => TagSource::Inline,
                        _ => return None,
                    };
                    let line = fields.next()?.parse().ok()?;
                    let name = fields.next()?.to_owned();
                    current
                        .as_mut()?
                        .facts
                        .tags
                        .push(TagUse { name, line, source });
                }
                "alias" => current.as_mut()?.facts.aliases.push(unescape(value)?),
                "prop" => {
                    let (name, text) = split_escaped(value)?;
                    current
                        .as_mut()?
                        .facts
                        .properties
                        .push((unescape(name)?, unescape(text)?));
                }
                _ => return None,
            }
        }
        if let Some(note) = current {
            notes.insert(path_key(&note.path), note);
        }
        Some(NoteIndex { notes })
    }
}

/// One tag in the tree: its last segment as most often spelled, its full lowercase name, the
/// notes that use it directly, how many notes use it or a tag under it, and its child tags.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagNode {
    pub name: String,
    pub full: String,
    pub notes: Vec<PathBuf>,
    pub count: usize,
    pub children: Vec<TagNode>,
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\p")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

fn unescape(value: &str) -> Option<String> {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            output.push(c);
            continue;
        }
        output.push(match chars.next()? {
            '\\' => '\\',
            'p' => '|',
            'r' => '\r',
            'n' => '\n',
            _ => return None,
        });
    }
    Some(output)
}

/// Splits `escaped|escaped` at its one unescaped `|` (escaping turns every `|` into `\p`).
fn split_escaped(value: &str) -> Option<(&str, &str)> {
    value.split_once('|')
}

/// `libraries\<folder key>.notes.cache`, beside the per-PC library file.
pub fn cache_file(local_path: &Path) -> PathBuf {
    local_path.with_extension("notes.cache")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(label: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("fastpad-index-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn note(&self, name: &str, text: &str) -> NoteEntry {
            let path = self.0.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, text).unwrap();
            NoteEntry {
                path: PathBuf::from(name),
                size: text.len() as u64,
                mtime: 1,
                online_only: false,
            }
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tag_names(index: &NoteIndex, path: &str) -> Vec<String> {
        index
            .facts(Path::new(path))
            .unwrap()
            .tags
            .iter()
            .map(|tag| tag.name.clone())
            .collect()
    }

    #[test]
    fn a_build_reads_markdown_notes_and_reuses_what_did_not_change() {
        let scratch = Scratch::new("build");
        let listed = vec![
            scratch.note("a.md", "#alpha"),
            scratch.note("b.markdown", "#beta"),
            scratch.note("c.txt", "#not-indexed"),
        ];
        let (index, changed) = NoteIndex::build(&scratch.0, &listed, &NoteIndex::default());
        assert!(changed);
        assert_eq!(index.notes.len(), 2);
        assert_eq!(tag_names(&index, "a.md"), ["alpha"]);

        // Same stamps: nothing is read again, even though the file changed on disk.
        std::fs::write(scratch.0.join("a.md"), "#rewritten").unwrap();
        let (again, changed) = NoteIndex::build(&scratch.0, &listed, &index);
        assert!(!changed);
        assert_eq!(tag_names(&again, "a.md"), ["alpha"]);

        // A new stamp reads it.
        let mut moved = listed.clone();
        moved[0].mtime = 2;
        let (fresh, changed) = NoteIndex::build(&scratch.0, &moved, &again);
        assert!(changed);
        assert_eq!(tag_names(&fresh, "a.md"), ["rewritten"]);

        // A note gone from the list leaves the index.
        let (fewer, changed) = NoteIndex::build(&scratch.0, &moved[..1], &fresh);
        assert!(changed);
        assert_eq!(fewer.notes.len(), 1);
    }

    #[test]
    fn large_and_online_only_notes_are_left_out() {
        let scratch = Scratch::new("skip");
        let mut big = scratch.note("big.md", "#x");
        big.size = MAX_NOTE_BYTES + 1;
        let mut cloud = scratch.note("cloud.md", "#y");
        cloud.online_only = true;
        let (index, _) = NoteIndex::build(&scratch.0, &[big, cloud], &NoteIndex::default());
        assert!(index.notes.is_empty());
    }

    #[test]
    fn the_cache_round_trips_and_a_bad_one_is_discarded() {
        let scratch = Scratch::new("cache");
        let listed = vec![scratch.note(
            r"sub\a.md",
            "---\ntags: [x]\naliases: [A | B]\nsummary: |\n  multi\n  line\n---\nBody #y\n",
        )];
        let (index, _) = NoteIndex::build(&scratch.0, &listed, &NoteIndex::default());
        let text = index.encode();
        assert_eq!(NoteIndex::parse(&text), Some(index));
        assert_eq!(NoteIndex::parse("version=2\r\n"), None);
        assert_eq!(
            NoteIndex::parse("version=1\r\ntag=i|0|x\r\n"),
            None,
            "a tag before a note"
        );
        assert_eq!(NoteIndex::parse("version=1\r\nwhat=ever\r\n"), None);
    }

    #[test]
    fn tags_form_a_tree_with_counts_and_the_most_common_spelling() {
        let mut index = NoteIndex::default();
        let mut add = |path: &str, tags: &[&str]| {
            index.notes.insert(
                path.to_owned(),
                IndexedNote {
                    path: PathBuf::from(path),
                    size: 0,
                    mtime: 0,
                    facts: NoteFacts {
                        tags: tags
                            .iter()
                            .map(|name| TagUse {
                                name: (*name).to_owned(),
                                line: 0,
                                source: TagSource::Inline,
                            })
                            .collect(),
                        ..NoteFacts::default()
                    },
                },
            );
        };
        add("a.md", &["Project/Atlas", "todo"]);
        add("b.md", &["project/fastpad", "project/fastpad"]);
        add("c.md", &["project", "TODO", "todo"]);
        let tree = index.tags();
        let summary = |nodes: &[TagNode]| {
            nodes
                .iter()
                .map(|node| (node.name.clone(), node.count))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            summary(&tree),
            [("project".to_owned(), 3), ("todo".to_owned(), 2)]
        );
        assert_eq!(
            summary(&tree[0].children),
            [("Atlas".to_owned(), 1), ("fastpad".to_owned(), 1)]
        );
        assert_eq!(tree[0].notes, [PathBuf::from("c.md")]);
        assert_eq!(tree[1].full, "todo");
    }
}
