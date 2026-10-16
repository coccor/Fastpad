//! How a file changed on disk while FastPad had it open, and what a tab does about it (spec
//! 2026-10-16-second-brain-phase-1 §5.2-§5.4). Pure: the caller stats and reads the file; nothing
//! here touches a window or the disk.

use super::DiskStamp;
use super::ids::fnv1a;

/// A clean tab whose file grew past this is not reloaded by itself: a huge log being written
/// must not freeze the window. It is treated as a conflict instead (spec §5.3).
pub const AUTO_RELOAD_LIMIT: u64 = 8 * 1024 * 1024;

/// The text a tab last loaded from or saved to its file: its UTF-8 length and a hash, enough to
/// tell whether the file on disk only grew at the end since then.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextBase {
    pub len: usize,
    pub hash: u64,
}

impl TextBase {
    pub fn of(text: &str) -> Self {
        Self {
            len: text.len(),
            hash: fnv1a(text.as_bytes()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiskChange {
    Same,
    /// The disk text is the base followed by more text, starting at this byte offset.
    Appended {
        at: usize,
    },
    Other,
}

/// How `disk` relates to the text the tab last loaded or saved. Without a base nothing can be
/// told, so any difference is `Other`.
pub fn classify(base: Option<TextBase>, disk: &str) -> DiskChange {
    let Some(base) = base else {
        return DiskChange::Other;
    };
    if TextBase::of(disk) == base {
        return DiskChange::Same;
    }
    if disk.len() > base.len
        && disk.is_char_boundary(base.len)
        && fnv1a(&disk.as_bytes()[..base.len]) == base.hash
    {
        return DiskChange::Appended { at: base.len };
    }
    DiskChange::Other
}

/// The smallest single edit that turns `old` into `new`: bytes `start..old_end` of `old` become
/// `new_text`. Replacing only the middle keeps Scintilla's caret, selection, folds and scroll
/// position for everything outside it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Replacement {
    pub start: usize,
    pub old_end: usize,
    pub new_text: String,
}

pub fn minimal_replacement(old: &str, new: &str) -> Option<Replacement> {
    if old == new {
        return None;
    }
    let (old_bytes, new_bytes) = (old.as_bytes(), new.as_bytes());
    let mut prefix = old_bytes
        .iter()
        .zip(new_bytes)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    // The suffix may not reach into the prefix of either text.
    let room = old_bytes.len().min(new_bytes.len()) - prefix;
    let mut suffix = old_bytes
        .iter()
        .rev()
        .zip(new_bytes.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    Some(Replacement {
        start: prefix,
        old_end: old.len() - suffix,
        new_text: new[prefix..new.len() - suffix].to_owned(),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReloadPolicy {
    /// A clean tab takes the file's new text by itself (`reload_changed_files=auto`).
    Auto,
    /// Every change is a conflict for the user to resolve (`reload_changed_files=ask`).
    Ask,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictKind {
    Changed,
    Deleted,
}

/// What a tab does after its file was checked (spec §5.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabVerdict {
    Unchanged,
    /// Replace the tab's text with the file's, as one undoable edit.
    Reload,
    /// Insert the file's text from this byte offset at the end of the dirty tab.
    AppendMerge {
        at: usize,
    },
    Conflict(ConflictKind),
    /// The file is gone and the tab has nothing to lose: it shows "(deleted)".
    Deleted,
    /// The file is back after being deleted, unchanged: the "(deleted)" mark goes.
    Restored,
}

/// The state of a tab that a verdict depends on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TabState {
    pub dirty: bool,
    pub known: Option<DiskStamp>,
    pub deleted_on_disk: bool,
}

/// Decides what a tab does about its file. `now` is the file's stamp, `None` when it is gone.
/// `change` reads and classifies the file; it runs only when the stamps differ.
pub fn verdict(
    tab: TabState,
    now: Option<DiskStamp>,
    policy: ReloadPolicy,
    change: impl FnOnce() -> DiskChange,
) -> TabVerdict {
    let Some(now) = now else {
        if tab.deleted_on_disk {
            return TabVerdict::Unchanged;
        }
        return if tab.dirty {
            TabVerdict::Conflict(ConflictKind::Deleted)
        } else {
            TabVerdict::Deleted
        };
    };
    if tab.known == Some(now) {
        return if tab.deleted_on_disk {
            TabVerdict::Restored
        } else {
            TabVerdict::Unchanged
        };
    }
    // A tab restored from a snapshot has no known stamp: it is treated as dirty, as autosave
    // always did, so its file is never silently replaced, and nothing it remembers about the
    // file's text is trusted.
    let dirty = tab.dirty || tab.known.is_none();
    if !dirty && policy == ReloadPolicy::Auto && now.size > AUTO_RELOAD_LIMIT {
        return TabVerdict::Conflict(ConflictKind::Changed);
    }
    let change = if tab.known.is_none() {
        DiskChange::Other
    } else {
        change()
    };
    match change {
        DiskChange::Same if tab.deleted_on_disk => TabVerdict::Restored,
        // Touched but not changed (a save of the same bytes elsewhere): only the stamp moves.
        DiskChange::Same => TabVerdict::Unchanged,
        DiskChange::Appended { at } if dirty => TabVerdict::AppendMerge { at },
        _ if dirty || policy == ReloadPolicy::Ask => TabVerdict::Conflict(ConflictKind::Changed),
        _ => TabVerdict::Reload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(old: &str, edit: &Option<Replacement>) -> String {
        match edit {
            None => old.to_owned(),
            Some(edit) => format!(
                "{}{}{}",
                &old[..edit.start],
                edit.new_text,
                &old[edit.old_end..]
            ),
        }
    }

    #[test]
    fn minimal_replacement_turns_old_into_new() {
        let cases = [
            ("", "text"),
            ("text", ""),
            ("abc", "abXc"),
            ("abc", "Xbc"),
            ("abc", "abX"),
            ("café au lait", "cafe au lait"),
            ("a😀b", "a😁b"),
            ("aaaa", "aaaaaa"),
            ("abab", "ab"),
            ("line 1\r\nline 2\r\n", "line 1\r\nline 1.5\r\nline 2\r\n"),
        ];
        for (old, new) in cases {
            let edit = minimal_replacement(old, new);
            assert_eq!(apply(old, &edit), new, "{old:?} -> {new:?}");
        }
    }

    #[test]
    fn minimal_replacement_touches_only_the_changed_middle() {
        let edit = minimal_replacement("one two three", "one 2 three").unwrap();
        assert_eq!((edit.start, edit.old_end), (4, 7));
        assert_eq!(edit.new_text, "2");
        assert_eq!(minimal_replacement("same", "same"), None);
    }

    #[test]
    fn minimal_replacement_cuts_at_character_boundaries() {
        // é (C3 A9) and è (C3 A8) share their first byte.
        let edit = minimal_replacement("é", "è").unwrap();
        assert_eq!((edit.start, edit.old_end), (0, 2));
        assert_eq!(edit.new_text, "è");
        // 😀 (F0 9F 98 80) and 😁 (F0 9F 98 81) share three leading bytes.
        let edit = minimal_replacement("x😀", "x😁").unwrap();
        assert_eq!((edit.start, edit.old_end), (1, 5));
    }

    #[test]
    fn append_is_detected_only_for_a_pure_append() {
        let base = Some(TextBase::of("- 09:00 first\n"));
        assert_eq!(classify(base, "- 09:00 first\n"), DiskChange::Same);
        assert_eq!(
            classify(base, "- 09:00 first\n- 10:00 second\n"),
            DiskChange::Appended { at: 14 }
        );
        // An edit before the end, a truncation, or the same length with other text: not appends.
        assert_eq!(
            classify(base, "- 09:00 FIRST\n- 10:00 second\n"),
            DiskChange::Other
        );
        assert_eq!(classify(base, "- 09:00"), DiskChange::Other);
        assert_eq!(classify(base, "- 09:00 FIRST\n"), DiskChange::Other);
        assert_eq!(classify(None, "anything"), DiskChange::Other);
    }

    #[test]
    fn a_base_ending_inside_a_character_is_not_an_append() {
        // The base is the first byte of é; the disk text is é: same prefix bytes, but no boundary.
        let base = Some(TextBase {
            len: 1,
            hash: fnv1a(&[0xC3]),
        });
        assert_eq!(classify(base, "é"), DiskChange::Other);
    }

    const A: DiskStamp = DiskStamp {
        size: 10,
        modified: 1,
    };
    const B: DiskStamp = DiskStamp {
        size: 12,
        modified: 2,
    };

    fn tab(dirty: bool) -> TabState {
        TabState {
            dirty,
            known: Some(A),
            deleted_on_disk: false,
        }
    }

    fn never() -> DiskChange {
        panic!("the file must not be read when its stamp is unchanged")
    }

    #[test]
    fn verdict_follows_the_spec_table() {
        let auto = ReloadPolicy::Auto;
        assert_eq!(
            verdict(tab(false), Some(A), auto, never),
            TabVerdict::Unchanged
        );
        assert_eq!(
            verdict(tab(true), Some(A), auto, never),
            TabVerdict::Unchanged
        );
        assert_eq!(
            verdict(tab(false), Some(B), auto, || DiskChange::Other),
            TabVerdict::Reload
        );
        assert_eq!(
            verdict(tab(false), Some(B), auto, || DiskChange::Appended {
                at: 10
            }),
            TabVerdict::Reload
        );
        assert_eq!(
            verdict(tab(true), Some(B), auto, || DiskChange::Appended { at: 10 }),
            TabVerdict::AppendMerge { at: 10 }
        );
        assert_eq!(
            verdict(tab(true), Some(B), auto, || DiskChange::Other),
            TabVerdict::Conflict(ConflictKind::Changed)
        );
        assert_eq!(
            verdict(tab(false), Some(B), auto, || DiskChange::Same),
            TabVerdict::Unchanged
        );
        assert_eq!(verdict(tab(false), None, auto, never), TabVerdict::Deleted);
        assert_eq!(
            verdict(tab(true), None, auto, never),
            TabVerdict::Conflict(ConflictKind::Deleted)
        );
    }

    #[test]
    fn a_deleted_file_coming_back_clears_the_mark() {
        let gone = TabState {
            deleted_on_disk: true,
            ..tab(false)
        };
        assert_eq!(
            verdict(gone, None, ReloadPolicy::Auto, never),
            TabVerdict::Unchanged
        );
        assert_eq!(
            verdict(gone, Some(A), ReloadPolicy::Auto, never),
            TabVerdict::Restored
        );
        assert_eq!(
            verdict(gone, Some(B), ReloadPolicy::Auto, || DiskChange::Same),
            TabVerdict::Restored
        );
        assert_eq!(
            verdict(gone, Some(B), ReloadPolicy::Auto, || DiskChange::Other),
            TabVerdict::Reload
        );
    }

    #[test]
    fn ask_policy_turns_every_change_into_a_conflict() {
        assert_eq!(
            verdict(tab(false), Some(B), ReloadPolicy::Ask, || DiskChange::Other),
            TabVerdict::Conflict(ConflictKind::Changed)
        );
        // A deleted file still only shows "(deleted)": there is nothing to choose between.
        assert_eq!(
            verdict(tab(false), None, ReloadPolicy::Ask, never),
            TabVerdict::Deleted
        );
    }

    #[test]
    fn a_huge_change_to_a_clean_tab_is_a_conflict_without_reading_it() {
        let huge = DiskStamp {
            size: AUTO_RELOAD_LIMIT + 1,
            modified: 2,
        };
        assert_eq!(
            verdict(tab(false), Some(huge), ReloadPolicy::Auto, never),
            TabVerdict::Conflict(ConflictKind::Changed)
        );
    }

    #[test]
    fn a_tab_without_a_known_stamp_is_treated_as_dirty() {
        let restored = TabState {
            known: None,
            ..tab(false)
        };
        assert_eq!(
            verdict(restored, Some(B), ReloadPolicy::Auto, || DiskChange::Other),
            TabVerdict::Conflict(ConflictKind::Changed)
        );
        // Even text that looks unchanged or appended is not trusted without a stamp.
        assert_eq!(
            verdict(restored, Some(B), ReloadPolicy::Auto, never),
            TabVerdict::Conflict(ConflictKind::Changed)
        );
    }
}
