//! The notebooks Search, Quick Open and Tags cover, their labels, and a result's way back to its
//! file (multi-root search spec §3).

use super::*;
use crate::window::library_host::{self, NoteRef, Skipped};

#[test]
fn searchable_lists_loaded_notebooks_in_sidebar_order() {
    // Break caught: a view searching only the target notebook, or in an order other than the
    // sidebar's, so ties and labels come out differently from what the tree shows.
    let _scintilla = load_native_scintilla();
    let ab = notebooks_a_and_b("searchable_order", "a", "b");
    let list = library_host::searchable_notebooks(ab.window.hwnd);
    let got: Vec<_> = list.iter().map(|s| (s.id, s.label.as_str())).collect();
    assert_eq!(got, [(ab.a_id, "A"), (ab.b_id, "B")]);
    assert!(library_host::several_notebooks(ab.window.hwnd));
    assert_eq!(
        library_host::skipped_suffix(library_host::skipped_notebooks(ab.window.hwnd)),
        ""
    );
}

#[test]
fn skipped_suffix_wording() {
    // Break caught: a header that hides that some notebook wasn't searched.
    use library_host::skipped_suffix;
    let s = |loading, unavailable| {
        skipped_suffix(Skipped {
            loading,
            unavailable,
        })
    };
    assert_eq!(s(1, 0), " (1 notebook still loading)");
    assert_eq!(s(2, 0), " (2 notebooks still loading)");
    assert_eq!(s(0, 3), " (3 notebooks unavailable)");
    assert_eq!(
        s(1, 1),
        " (1 notebook still loading, 1 notebook unavailable)"
    );
}

#[test]
fn a_note_ref_resolves_in_its_own_notebook() {
    // Break caught: a result's relative path joined to some other notebook's folder.
    let _scintilla = load_native_scintilla();
    let ab = notebooks_a_and_b("noteref_open", "a", "b");
    let note = NoteRef {
        notebook: ab.b_id,
        path: "b.md".into(),
    };
    assert_eq!(
        library_host::note_ref_path(ab.window.hwnd, &note),
        Some(ab.b.join("b.md"))
    );
}

#[test]
fn a_note_ref_of_a_closed_notebook_notifies() {
    // Break caught: a stale row opening a path under whichever notebook is left, or silently
    // doing nothing.
    let _scintilla = load_native_scintilla();
    let ab = notebooks_a_and_b("noteref_closed", "a", "b");
    assert!(library_host::close_notebook_of(ab.window.hwnd, ab.b_id));
    let note = NoteRef {
        notebook: ab.b_id,
        path: "b.md".into(),
    };
    assert_eq!(library_host::note_ref_path(ab.window.hwnd, &note), None);
    assert!(notices(ab.window.hwnd).contains(&"That note's notebook was closed.".to_owned()));
}
