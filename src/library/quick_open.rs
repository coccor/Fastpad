//! Quick open (Ctrl+P, quick-open spec §3.3–3.4): fuzzy matching of note names and folders the
//! way VS Code's Ctrl+P matches files, and the `:<line>` suffix. Pure: no Win32 and no disk.

use super::tree::natural_cmp;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::ffi::OsStr;
use std::path::{Component, Components, Path, PathBuf};

/// Every matched char.
const PER_CHAR: u32 = 1;
/// A char right after the previous matched one.
const CONTIGUOUS: u32 = 5;
/// A char that starts a word.
const WORD_START: u32 = 8;
/// The target's first char, on top of its word start.
const FIRST: u32 = 4;
/// Each term matched in the name rather than in the folder path.
const NAME_MATCH: u32 = 20;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuickMatch {
    /// As given: relative to the notebook.
    pub path: PathBuf,
    /// The file name without its extension, as the tree shows it.
    pub name: String,
    /// The folder the note is in, relative to the notebook and joined with `\`; `""` at the root.
    pub folder: String,
    /// The matched chars of `name`, as ascending char indices (not byte offsets).
    pub name_hits: Vec<usize>,
    /// The matched chars of `folder`, as ascending char indices.
    pub folder_hits: Vec<usize>,
}

impl QuickMatch {
    /// `path` as a row with nothing matched: how the open tabs are listed before anything is
    /// typed. `None` for a path with no file name.
    pub fn plain(path: &Path) -> Option<Self> {
        let (components, name) = split(path)?;
        let mut folder = String::new();
        extend_joined(components, &mut folder);
        Some(Self {
            path: path.to_path_buf(),
            name: stem_of(name.to_string_lossy()).into_owned(),
            folder,
            name_hits: Vec::new(),
            folder_hits: Vec::new(),
        })
    }
}

/// Splits a trailing `:<digits>` off `query`, trailing spaces ignored: the text to match and
/// the 1-based line. A trailing `:` with no digits yet is dropped too, so the list doesn't empty
/// while the number is typed; anything else after the last `:` is text (`a:x`). A line too
/// large for `u32` is `u32::MAX`, which goes to the last line.
pub fn split_line(query: &str) -> (&str, Option<u32>) {
    let query = query.trim_end();
    let Some(colon) = query.rfind(':') else {
        return (query, None);
    };
    let digits = &query[colon + 1..];
    if digits.is_empty() {
        return (&query[..colon], None);
    }
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return (query, None);
    }
    (&query[..colon], Some(digits.parse().unwrap_or(u32::MAX)))
}

/// `path`'s parent components (root to leaf) and its last component's raw name, in one walk over
/// `path`: `Scratch::score`, `folder_of` and `QuickMatch::plain` all start here instead of each
/// walking `file_stem()`, `parent()` and `parent().components()` separately. `None` when the
/// path's last component isn't a plain name (it ends in `..`, `.`, a root or a prefix) — the
/// same case `Path::file_stem` treats as "no file name".
fn split(path: &Path) -> Option<(Components<'_>, &OsStr)> {
    let mut components = path.components();
    match components.next_back()? {
        Component::Normal(name) => Some((components, name)),
        _ => None,
    }
}

/// The file name's stem the way `Path::file_stem` defines it, run on the already-decoded name
/// (`split`'s last component) so a note's path is parsed once instead of three times: the whole
/// name when it has no `.`, or begins with the only `.` it has; otherwise the portion before the
/// final `.`.
fn stem_of(name: Cow<'_, str>) -> Cow<'_, str> {
    match name {
        Cow::Borrowed(text) => Cow::Borrowed(match text.rfind('.') {
            Some(0) | None => text,
            Some(dot) => &text[..dot],
        }),
        Cow::Owned(mut text) => {
            if let Some(dot) = text.rfind('.')
                && dot != 0
            {
                text.truncate(dot);
            }
            Cow::Owned(text)
        }
    }
}

/// Appends `components`' names to `out`, joined by `\`: how a folder is shown, and how the
/// path-fallback target sees it ahead of the name. The one join written for both `folder_of` and
/// the folder chars `Scratch::score` builds for a path-fallback match.
fn extend_joined(components: Components<'_>, out: &mut impl Extend<char>) {
    for (index, component) in components.enumerate() {
        if index > 0 {
            out.extend(std::iter::once('\\'));
        }
        out.extend(component.as_os_str().to_string_lossy().chars());
    }
}

/// One char of a match target.
#[derive(Clone, Copy)]
struct Unit {
    /// The char folded to one lowercase char (`fold`).
    folded: char,
    /// Whether a word starts here (spec §3.3).
    start: bool,
}

/// `c` in lowercase as one char: the first char of its lowercase form, so a char never becomes
/// two (`İ` lowercases to `i` plus a combining dot) and positions stay char indices.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// `chars` as match targets. A word starts at the first char, after a space, `-`, `_`, `.`,
/// `\` or `/`, and at an uppercase letter after a lowercase one.
fn units(chars: impl IntoIterator<Item = char>, out: &mut Vec<Unit>) {
    out.clear();
    let mut previous: Option<char> = None;
    for c in chars {
        let start = previous.is_none_or(|previous| {
            matches!(previous, ' ' | '-' | '_' | '.' | '\\' | '/')
                || (c.is_uppercase() && previous.is_lowercase())
        });
        out.push(Unit {
            folded: fold(c),
            start,
        });
        previous = Some(c);
    }
}

/// Whether `term`'s folded letters all appear in `target`, in order (not necessarily
/// contiguous), with no scoring. A cheap pre-check before the DP: when this fails, `align` would
/// score 0 too (and when it holds, `align` always finds a positive-scoring alignment, since
/// every matched char scores at least `PER_CHAR`), so most notes a query doesn't match are
/// rejected without ever touching `score` or `from`.
fn contains_in_order(term: &[char], target: &[Unit]) -> bool {
    let mut letters = term.iter();
    let Some(mut wanted) = letters.next() else {
        return true;
    };
    for unit in target {
        if unit.folded == *wanted {
            match letters.next() {
                Some(next) => wanted = next,
                None => return true,
            }
        }
    }
    false
}

/// The best score of `term`'s letters, in order, in `target`, or 0 when they don't all appear.
/// `score[j * n + i]` is the best score of the first `j + 1` letters with letter `j` on char `i`
/// (0: impossible), and `from[j * n + i]` the char letter `j - 1` sits on then. With
/// `positions`, also records where each letter of the best alignment landed; `from` is only
/// written (and only worth clearing first) when `positions` asks for that backtrack.
fn align(
    term: &[char],
    target: &[Unit],
    score: &mut Vec<u32>,
    from: &mut Vec<u32>,
    mut positions: Option<&mut Vec<usize>>,
) -> u32 {
    let (letters, n) = (term.len(), target.len());
    if letters == 0 || letters > n || !contains_in_order(term, target) {
        return 0;
    }
    let track = positions.is_some();
    score.clear();
    score.resize(letters * n, 0);
    if track {
        from.clear();
        from.resize(letters * n, 0);
    }
    for (j, &letter) in term.iter().enumerate() {
        // The best score of the previous letter on a char at least two before `i`, and where.
        let (mut before, mut before_at) = (0, 0);
        for (i, unit) in target.iter().enumerate() {
            if j > 0 && i >= 2 {
                let candidate = score[(j - 1) * n + i - 2];
                if candidate > before {
                    (before, before_at) = (candidate, i - 2);
                }
            }
            if unit.folded != letter {
                continue;
            }
            let own =
                PER_CHAR + if unit.start { WORD_START } else { 0 } + if i == 0 { FIRST } else { 0 };
            if j == 0 {
                score[i] = own;
                continue;
            }
            let adjacent = if i >= 1 {
                score[(j - 1) * n + i - 1]
            } else {
                0
            };
            let (previous, at) = if adjacent > 0 && adjacent + CONTIGUOUS >= before {
                (adjacent + CONTIGUOUS, i - 1)
            } else {
                (before, before_at)
            };
            if previous > 0 {
                score[j * n + i] = previous + own;
                if track {
                    from[j * n + i] = at as u32;
                }
            }
        }
    }
    let (mut end, mut best) = (0, 0);
    for (i, &value) in score[(letters - 1) * n..].iter().enumerate() {
        if value > best {
            (end, best) = (i, value);
        }
    }
    if best > 0
        && let Some(positions) = positions.take()
    {
        positions.clear();
        positions.resize(letters, 0);
        let mut at = end;
        for (j, slot) in positions.iter_mut().enumerate().rev() {
            *slot = at;
            if j > 0 {
                at = from[j * n + at] as usize;
            }
        }
    }
    best
}

/// Buffers reused from note to note, so a keystroke doesn't allocate per note for matching. The
/// name lives only for the duration of one `score` call (borrowed from the note's own path), so
/// it needs no buffer here; the folder is built into `folder` only when a term actually needs
/// the path fallback (spec §3.6: most notes fail on the name alone, and most of those never had
/// a folder to search regardless).
#[derive(Default)]
struct Scratch {
    folder: Vec<char>,
    name_units: Vec<Unit>,
    path_units: Vec<Unit>,
    score: Vec<u32>,
    from: Vec<u32>,
    positions: Vec<usize>,
}

impl Scratch {
    /// Whether every term matched in the name, the summed score, and the note's name (borrowed
    /// from `path` when it decodes without a lossy copy) with its length in chars; `None` when a
    /// term matches neither the name nor `folder\name`. With `hits`, also collects the name's
    /// and the folder's matched chars.
    fn score<'p>(
        &mut self,
        terms: &[Vec<char>],
        path: &'p Path,
        mut hits: Option<(&mut Vec<usize>, &mut Vec<usize>)>,
    ) -> Option<(bool, u32, Cow<'p, str>, usize)> {
        let (folder_components, name) = split(path)?;
        let mut folder_components = Some(folder_components);
        let stem = stem_of(name.to_string_lossy());
        units(stem.chars(), &mut self.name_units);
        let name_len = self.name_units.len();

        self.folder.clear();
        let keep = hits.is_some();
        let mut folder_ready = false;
        let mut path_ready = false;
        let (mut all_name, mut total) = (true, 0);
        for term in terms {
            let positions = keep.then_some(&mut self.positions);
            let in_name = align(
                term,
                &self.name_units,
                &mut self.score,
                &mut self.from,
                positions,
            );
            if in_name > 0 {
                total += in_name + NAME_MATCH;
                if let Some((name_hits, _)) = hits.as_mut() {
                    name_hits.extend_from_slice(&self.positions);
                }
                continue;
            }
            // Only a term that fails on the name needs the folder at all.
            if !folder_ready {
                if let Some(components) = folder_components.take() {
                    extend_joined(components, &mut self.folder);
                }
                folder_ready = true;
            }
            if self.folder.is_empty() {
                return None;
            }
            if !path_ready {
                units(
                    self.folder
                        .iter()
                        .copied()
                        .chain(std::iter::once('\\'))
                        .chain(stem.chars()),
                    &mut self.path_units,
                );
                path_ready = true;
            }
            let positions = keep.then_some(&mut self.positions);
            let in_path = align(
                term,
                &self.path_units,
                &mut self.score,
                &mut self.from,
                positions,
            );
            if in_path == 0 {
                return None;
            }
            all_name = false;
            total += in_path;
            if let Some((name_hits, folder_hits)) = hits.as_mut() {
                let split = self.folder.len();
                for &position in &self.positions {
                    match position.cmp(&split) {
                        Ordering::Less => folder_hits.push(position),
                        // The `\` joining the folder to the name is in neither.
                        Ordering::Equal => {}
                        Ordering::Greater => name_hits.push(position - split - 1),
                    }
                }
            }
        }
        if let Some((name_hits, folder_hits)) = hits {
            for list in [name_hits, folder_hits] {
                list.sort_unstable();
                list.dedup();
            }
        }
        Some((all_name, total, stem, name_len))
    }
}

struct Candidate<'a> {
    /// Every term matched in the name.
    all_name: bool,
    score: u32,
    /// The name's length in chars.
    name_len: usize,
    name: Cow<'a, str>,
    path: &'a Path,
}

/// Name matches first, then the higher score, the shorter name, natural name order, natural
/// folder order (the root first) and the exact path (spec §3.3). The folder is only joined into
/// a string when a comparison actually reaches it, which for distinct names it rarely does.
fn rank(a: &Candidate<'_>, b: &Candidate<'_>) -> Ordering {
    b.all_name
        .cmp(&a.all_name)
        .then_with(|| b.score.cmp(&a.score))
        .then_with(|| a.name_len.cmp(&b.name_len))
        .then_with(|| natural_cmp(&a.name, &b.name))
        .then_with(|| natural_cmp(&folder_of(a.path), &folder_of(b.path)))
        .then_with(|| a.path.cmp(b.path))
}

/// The notes matching every space-separated term of `query`, best first, at most `limit`, each
/// with its matched chars. `query` has no line suffix here: `split_line` took it off.
pub fn search<'a, P>(
    notes: impl IntoIterator<Item = &'a P>,
    query: &str,
    limit: usize,
) -> Vec<QuickMatch>
where
    P: AsRef<Path> + ?Sized + 'a,
{
    let terms: Vec<Vec<char>> = query
        .split_whitespace()
        .map(|term| term.chars().map(fold).collect())
        .collect();
    if terms.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut scratch = Scratch::default();
    let mut found: Vec<Candidate<'_>> = Vec::new();
    for path in notes {
        let path = path.as_ref();
        let Some((all_name, score, name, name_len)) = scratch.score(&terms, path, None) else {
            continue;
        };
        found.push(Candidate {
            all_name,
            score,
            name_len,
            name,
            path,
        });
    }
    // Only the best `limit` need a full sort: one letter can match every note.
    if found.len() > limit {
        found.select_nth_unstable_by(limit, rank);
        found.truncate(limit);
    }
    found.sort_by(rank);
    found
        .into_iter()
        .map(|candidate| {
            let (mut name_hits, mut folder_hits) = (Vec::new(), Vec::new());
            // The same alignment again, this time keeping where each letter landed.
            let _ = scratch.score(
                &terms,
                candidate.path,
                Some((&mut name_hits, &mut folder_hits)),
            );
            QuickMatch {
                path: candidate.path.to_path_buf(),
                name: candidate.name.into_owned(),
                folder: folder_of(candidate.path),
                name_hits,
                folder_hits,
            }
        })
        .collect()
}

/// The folder `path` is in, relative to the notebook and joined with `\`; `""` at the root.
pub(super) fn folder_of(path: &Path) -> String {
    let Some((components, _)) = split(path) else {
        return String::new();
    };
    let mut folder = String::new();
    extend_joined(components, &mut folder);
    folder
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn names(matches: &[QuickMatch]) -> Vec<&str> {
        matches.iter().map(|found| found.name.as_str()).collect()
    }

    #[test]
    fn a_term_matches_when_its_letters_appear_in_order_ignoring_case() {
        // Break caught: a substring-only matcher ("nt" finding nothing in "note"), letters taken
        // out of order ("tn" finding "note"), or the extension searched.
        let notes = paths(&["note.md", "tone.md", "Readme"]);
        assert_eq!(names(&search(&notes, "nt", 50)), ["note"]);
        assert_eq!(names(&search(&notes, "NT", 50)), ["note"]);
        assert_eq!(names(&search(&notes, "tn", 50)), ["tone"]);
        assert_eq!(names(&search(&notes, "rdm", 50)), ["Readme"]);
        assert!(
            search(&notes, "md", 50).is_empty(),
            "the extension is not searched"
        );
    }

    #[test]
    fn every_term_must_match_in_the_name_or_the_folder_path() {
        // Break caught: terms ORed (every note with "al" listed), a term that only matches the
        // folder dropping the note, or a query of spaces listing every note.
        let notes = paths(&[
            "alpha beta.md",
            "alpha.md",
            "beta.md",
            r"alpha\gamma beta.md",
        ]);
        let found = search(&notes, "al be", 50);
        assert_eq!(names(&found), ["alpha beta", "gamma beta"]);
        assert_eq!(found[1].folder, "alpha");
        assert!(search(&notes, "   ", 50).is_empty());
        assert!(search(&notes, "al zz", 50).is_empty());
        assert!(search(&notes, "al", 0).is_empty());
    }

    #[test]
    fn a_name_match_comes_before_a_higher_scoring_folder_match() {
        // Break caught: "meet" listing every note of a folder called "meet" above the note whose
        // own name holds the letters.
        let notes = paths(&[r"meet\zz.md", "xmxexet.md"]);
        assert_eq!(names(&search(&notes, "meet", 50)), ["xmxexet", "zz"]);
    }

    #[test]
    fn contiguous_and_word_start_letters_beat_scattered_ones() {
        // Break caught: plain in-order matching that ranks "xaxbxc" level with "abc" typed as a
        // word, so the note the user meant sinks under the noise.
        let notes = paths(&["xaxbxc.md", "xxabcxx.md", "zz-a-b-c.md"]);
        assert_eq!(
            names(&search(&notes, "abc", 50)),
            ["zz-a-b-c", "xxabcxx", "xaxbxc"]
        );
        let camel = paths(&["xaxbxc.md", "zzAxBxCx.md"]);
        assert_eq!(names(&search(&camel, "abc", 50)), ["zzAxBxCx", "xaxbxc"]);
    }

    #[test]
    fn hits_are_char_positions_in_the_name_and_the_folder() {
        // Break caught (review focus 2): byte offsets reported as char positions, so "Über
        // Straße" bolds the wrong letters; a folder match's hits left in the name; or the `\`
        // joining folder and name counted as a hit.
        let notes = paths(&["Über Straße.md"]);
        let found = search(&notes, "st", 50);
        assert_eq!(found[0].name_hits, [5, 6]);
        assert!(found[0].folder_hits.is_empty());
        assert_eq!(search(&notes, "ÜB", 50)[0].name_hits, [0, 1]);

        let notes = paths(&[r"Work\2026\meeting notes.md"]);
        let found = search(&notes, "w26 mn", 50);
        assert_eq!(found[0].folder, r"Work\2026");
        assert_eq!(found[0].folder_hits, [0, 5, 8]);
        assert_eq!(found[0].name_hits, [0, 8]);
    }

    #[test]
    fn at_most_the_limit_best_matches_are_kept() {
        // Break caught: a one-letter query listing all 300 notes, or the cap keeping arbitrary
        // notes instead of the best (the shorter names, then natural order).
        let many: Vec<PathBuf> = (0..300)
            .map(|index| PathBuf::from(format!("n{index}.md")))
            .collect();
        let found = search(&many, "n", 50);
        assert_eq!(found.len(), 50);
        assert_eq!(found[0].name, "n0");
        assert_eq!(found[9].name, "n9");
        assert_eq!(found[10].name, "n10");
        assert_eq!(found[49].name, "n49");
    }

    #[test]
    fn a_trailing_colon_and_digits_is_a_line() {
        // Break caught (review focus 1): "meet:42" matched as text (nothing found), a colon
        // further in cutting the text short, ":12" offering notes instead of the line, a
        // trailing colon emptying the list while the number is typed, or a huge number panicking.
        assert_eq!(split_line("a:12"), ("a", Some(12)));
        assert_eq!(split_line(":12"), ("", Some(12)));
        assert_eq!(split_line("a:"), ("a", None));
        assert_eq!(split_line("a:x"), ("a:x", None));
        assert_eq!(split_line("12"), ("12", None));
        assert_eq!(split_line("a:b:3"), ("a:b", Some(3)));
        assert_eq!(split_line("meet:42  "), ("meet", Some(42)));
        assert_eq!(split_line("a:99999999999"), ("a", Some(u32::MAX)));
        assert_eq!(split_line("   "), ("", None));
        assert!(search(&paths(&["a.md", "b.md"]), split_line("a:b").0, 50).is_empty());
    }

    #[test]
    fn plain_rows_name_the_note_and_its_folder_with_nothing_matched() {
        // Break caught: the open-tab rows shown before anything is typed losing their folder, or
        // carrying stale highlights.
        let plain = QuickMatch::plain(Path::new(r"a\b\x.md")).unwrap();
        assert_eq!((plain.name.as_str(), plain.folder.as_str()), ("x", r"a\b"));
        assert!(plain.name_hits.is_empty() && plain.folder_hits.is_empty());
        assert_eq!(QuickMatch::plain(Path::new("x.md")).unwrap().folder, "");
    }

    #[test]
    fn ten_thousand_notes_match_quickly() {
        // Break caught: a keystroke that allocates or sorts per note so heavily that typing in
        // Ctrl+P lags on a big notebook (spec §3.6).
        let notes: Vec<PathBuf> = (0..10_000)
            .map(|index| PathBuf::from(format!(r"batch{}\note{index}.md", index / 500)))
            .collect();
        let started = std::time::Instant::now();
        let found = search(&notes, "nt 12", 50);
        let elapsed = started.elapsed();
        assert_eq!(found.len(), 50);
        assert_eq!(found[0].name, "note12");
        assert_eq!(found[0].name_hits, [0, 2, 4, 5]);
        if !cfg!(debug_assertions) {
            assert!(
                elapsed < std::time::Duration::from_millis(20),
                "{elapsed:?}"
            );
        }
    }
}
