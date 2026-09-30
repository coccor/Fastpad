use super::Tabs;
use super::{CloseReview, CloseReviewError};
use crate::document::{Document, DocumentId};
use crate::editor::ViewState;
use std::fs;

use crate::document::CloseDecision;

fn order(tabs: &Tabs) -> Vec<u64> {
    tabs.activation_order().iter().map(|(_, id)| id.0).collect()
}

fn document(id: u64) -> Document {
    Document::test_fixture(DocumentId(id), false)
}

#[test]
fn a_clean_background_tab_closes_where_it_is_and_the_active_tab_stays() {
    // Break caught: a middle-click on another tab switching to it first (the editor flashes),
    // closing the active tab instead, the active index left pointing one tab too far right,
    // or a dirty tab closed without its prompt.
    let mut tabs = Tabs::with_document(document(1));
    tabs.push(document(2)).unwrap();
    tabs.push(document(3)).unwrap();
    let review = |tabs: &Tabs, id: u64| CloseReview {
        id: DocumentId(id),
        generation: tabs.document(DocumentId(id)).unwrap().generation,
    };
    let first = review(&tabs, 1);
    let closed = tabs.close_clean_background(first).unwrap().unwrap();
    assert_eq!(closed.id, DocumentId(1));
    assert_eq!(tabs.active().unwrap().id, DocumentId(3));
    assert_eq!(tabs.active_index(), 1);
    assert_eq!(order(&tabs), [3, 2]);
    assert_eq!(tabs.view().snapshot().tabs.len(), 2);

    let active = tabs.active_close_review().unwrap();
    assert_eq!(
        tabs.close_clean_background(active),
        Err(CloseReviewError::Stale)
    );
    tabs.document_mut(DocumentId(2)).unwrap().dirty = true;
    let dirty = review(&tabs, 2);
    assert_eq!(
        tabs.close_clean_background(dirty),
        Err(CloseReviewError::Unsaved)
    );
    let stale = CloseReview {
        generation: dirty.generation + 1,
        ..dirty
    };
    assert_eq!(
        tabs.close_clean_background(stale),
        Err(CloseReviewError::Stale)
    );
    assert_eq!(tabs.len(), 2);
}

/// `push` canonicalizes paths through the disk, so these pure tests use untitled documents.
fn preview(id: u64) -> Document {
    let mut document = document(id);
    document.preview = true;
    document
}

#[test]
fn a_background_edit_marks_only_that_tab_dirty_and_keeps_its_preview() {
    // Break caught: a Search replace into a background tab leaving it clean (closing it
    // would drop the replacement without asking), marking the active tab instead, or leaving
    // a preview that the next click replaces with its edits in it.
    let mut tabs = Tabs::with_document(document(1));
    tabs.push(preview(2)).unwrap();
    tabs.activate(DocumentId(1)).unwrap();
    let before = tabs.document(DocumentId(2)).unwrap().generation;
    assert!(tabs.note_background_edit(DocumentId(2)));
    let edited = tabs.document(DocumentId(2)).unwrap();
    assert!(edited.dirty && !edited.preview);
    assert_eq!(edited.generation, before + 1);
    assert!(!tabs.active().unwrap().dirty);
    assert!(!tabs.note_background_edit(DocumentId(2)), "already dirty");
    assert!(!tabs.note_background_edit(DocumentId(9)), "no such tab");
}

#[test]
fn replacing_the_preview_keeps_its_place_and_selects_it() {
    // Break caught: a second preview appended at the end (the strip grows with every click),
    // or replaced in place but left unselected.
    let mut tabs = Tabs::with_document(document(1));
    tabs.push(preview(2)).unwrap();
    tabs.push(document(3)).unwrap();
    assert_eq!(tabs.preview_id(), Some(DocumentId(2)));

    let old = tabs.replace_preview(preview(4)).unwrap();
    assert_eq!(old.id, DocumentId(2));
    assert_eq!(
        tabs.documents()
            .map(|document| document.id)
            .collect::<Vec<_>>(),
        [DocumentId(1), DocumentId(4), DocumentId(3)]
    );
    assert_eq!(tabs.active_index(), 1);
    assert_eq!(tabs.preview_id(), Some(DocumentId(4)));
    assert!(tabs.view().snapshot().tabs[1].preview);
}

#[test]
fn a_dirty_preview_is_kept_as_a_normal_tab_and_the_new_one_is_added() {
    // Break caught: a preview whose promotion was missed being replaced with its edits in it.
    let mut tabs = Tabs::with_document(document(1));
    let mut edited = preview(2);
    edited.dirty = true;
    tabs.push(edited).unwrap();

    assert!(tabs.replace_preview(preview(3)).is_none());
    assert_eq!(tabs.len(), 3);
    assert!(!tabs.document(DocumentId(2)).unwrap().preview);
    assert_eq!(tabs.preview_id(), Some(DocumentId(3)));
    assert_eq!(tabs.active_index(), 2);
}

#[test]
fn with_no_preview_replace_preview_adds_a_tab() {
    // Break caught: the first preview of a session silently dropped because there was nothing
    // to replace.
    let mut tabs = Tabs::with_document(document(1));
    assert!(tabs.replace_preview(preview(2)).is_none());
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs.preview_id(), Some(DocumentId(2)));
}

#[test]
fn the_first_edit_promotes_the_active_preview_once() {
    // Break caught: typing into a preview leaving it a preview, so the next click replaces it.
    let mut tabs = Tabs::with_document(preview(1));
    assert!(tabs.note_text_change(DocumentId(1)));
    assert!(!tabs.note_text_change(DocumentId(1)));
    assert_eq!(tabs.preview_id(), None);
    assert!(!tabs.promote(DocumentId(1)));
    let mut tabs = Tabs::with_document(preview(1));
    assert!(tabs.promote(DocumentId(1)));
    assert!(!tabs.view().snapshot().tabs[0].preview);
}

#[test]
fn recovered_documents_stay_dirty_until_their_origin_is_cleared() {
    // Break caught: undoing a recovered tab to Scintilla's save point marks it clean, so
    // closing skips the prompt and deletes the only copy of its text.
    let mut recovered = Document::test_fixture(DocumentId(1), true);
    recovered.recovery_origin = Some(crate::document::RecoveryOrigin {
        snapshot_path: std::path::PathBuf::from("a.fps"),
        original_path: None,
        from_session: false,
    });
    let mut tabs = Tabs::with_document(recovered);

    assert!(!tabs.set_active_dirty(false));
    assert!(tabs.active().unwrap().dirty);
    assert!(tabs.take_active_recovery_origin().is_some());
    assert!(tabs.set_active_dirty(false));
}

#[test]
fn native_document_tab_is_selected() {
    let tabs = Tabs::with_document(Document::test_fixture(DocumentId(1), false));
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs.active_index(), 0);
    assert_eq!(tabs.titles().collect::<Vec<_>>(), ["Untitled"]);
}

#[test]
fn set_active_language_updates_only_the_active_document() {
    // Break caught: explicit language selection or detection-on-open applying to the wrong
    // tab, or leaving Document::language stale after a real lexer switch.
    use crate::document::Language;
    let mut tabs = Tabs::from_documents([document(1), document(2)]).unwrap();
    tabs.activate(DocumentId(2)).unwrap();

    assert!(tabs.set_active_language(Language::Json));
    assert!(!tabs.set_active_language(Language::Json));
    assert_eq!(
        tabs.document(DocumentId(1)).unwrap().language,
        Language::PlainText
    );
    assert_eq!(
        tabs.document(DocumentId(2)).unwrap().language,
        Language::Json
    );
}

#[test]
fn shared_selection_updates_the_tab_model_and_rejects_out_of_range_indices() {
    // Break caught: accessibility can keep a provider-private selection that title painting
    // cannot observe, or accept a button/out-of-range index as a tab.
    let tabs = Tabs::from_documents([
        Document::test_fixture(DocumentId(1), false),
        Document::test_fixture(DocumentId(2), false),
    ])
    .unwrap();
    let selection = tabs.selection();

    assert!(selection.select(1, tabs.len()));
    assert_eq!(tabs.active_index(), 1);
    assert!(!selection.select(2, tabs.len()));
    assert_eq!(tabs.active_index(), 1);
}

#[test]
fn renaming_the_active_document_updates_its_path_and_the_tab_view() {
    // Break caught: Save As writing the path field directly instead of going through a
    // validated method can desync the tab view from the document model.
    let root = std::env::temp_dir().join(format!(
        "fastpad-task11-rename-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let target = root.join("saved-as.txt");
    fs::write(&target, b"saved").unwrap();
    let mut tabs = Tabs::with_document(document(1));
    let view = tabs.view();
    let before = view.snapshot().revision;

    tabs.set_active_path(target.clone()).unwrap();

    assert_eq!(
        tabs.active().unwrap().path.as_deref(),
        Some(target.as_path())
    );
    assert!(view.snapshot().revision > before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn renaming_to_a_path_that_does_not_yet_exist_succeeds() {
    // Break caught: reusing canonical_key's existence requirement verbatim can reject every
    // brand-new Save As destination, since a not-yet-written file cannot be canonicalized.
    let root = std::env::temp_dir().join(format!(
        "fastpad-task11-rename-new-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let target = root.join("not-written-yet.txt");
    let mut tabs = Tabs::with_document(document(1));

    assert!(tabs.set_active_path(target.clone()).is_ok());
    assert_eq!(
        tabs.active().unwrap().path.as_deref(),
        Some(target.as_path())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn renaming_the_active_document_onto_another_open_tabs_canonical_path_is_rejected() {
    // Break caught: Save As can create two tabs that own the same canonical path, breaking
    // Task 9's one-native-document-per-path invariant.
    let root = std::env::temp_dir().join(format!(
        "fastpad-task11-rename-collision-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let other_path = root.join("other.txt");
    fs::write(&other_path, b"other").unwrap();
    let mut other = document(2);
    other.path = Some(other_path.clone());
    let mut tabs = Tabs::from_documents([document(1), other]).unwrap();
    tabs.activate(DocumentId(1)).unwrap();

    let alternate = root.join(".").join("other.txt");
    assert!(tabs.set_active_path(alternate).is_err());
    assert_eq!(tabs.active().unwrap().path, None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rebinding_a_path_rejects_one_another_tab_has() {
    // Break caught: rebinding a document's path (e.g. after a note rename) can create two
    // tabs that own the same canonical path, breaking Task 9's invariant, or can fail to
    // update the tab view so the title strip shows a stale name.
    let root = std::env::temp_dir().join(format!(
        "fastpad-task13-rebind-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let path_a = root.join("a.md");
    let path_b = root.join("b.md");
    let path_c = root.join("c.md");
    fs::write(&path_a, b"a").unwrap();
    fs::write(&path_b, b"b").unwrap();
    fs::write(&path_c, b"c").unwrap();

    let mut first = document(1);
    first.path = Some(path_a);
    let mut second = document(2);
    second.path = Some(path_b.clone());
    let mut tabs = Tabs::from_documents([first, second]).unwrap();

    assert!(tabs.rebind_path(DocumentId(1), path_b).is_err());
    tabs.rebind_path(DocumentId(1), path_c.clone()).unwrap();
    assert_eq!(
        tabs.document(DocumentId(1)).unwrap().path.as_deref(),
        Some(path_c.as_path())
    );
    tabs.document_mut(DocumentId(2)).unwrap().autosave_paused = true;
    assert!(tabs.document(DocumentId(2)).unwrap().autosave_paused);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn activating_or_opening_a_tab_moves_it_to_the_front_of_the_activation_order() {
    // Break caught: Ctrl+P listing tabs in strip order, so Ctrl+P then Enter doesn't go back
    // to the previous note, or a new tab missing from the list.
    let mut tabs = Tabs::with_document(document(1));
    tabs.push(document(2)).unwrap();
    tabs.push(document(3)).unwrap();
    assert_eq!(order(&tabs), [3, 2, 1]);
    tabs.activate(DocumentId(1)).unwrap();
    assert_eq!(order(&tabs), [1, 3, 2]);
    tabs.activate_index(2).unwrap();
    assert_eq!(order(&tabs), [3, 1, 2]);
    tabs.activate(DocumentId(3)).unwrap();
    assert_eq!(order(&tabs), [3, 1, 2], "the active tab stays first");
}

#[test]
fn a_closed_tab_leaves_the_order_and_the_tab_taking_its_place_comes_first() {
    // Break caught: Ctrl+P offering a closed tab's dead document, or leaving the tab now on
    // screen second, so Ctrl+P then Enter re-selects the tab already shown.
    let mut tabs = Tabs::with_document(document(1));
    tabs.push(document(2)).unwrap();
    tabs.push(document(3)).unwrap();
    tabs.activate(DocumentId(2)).unwrap();
    assert_eq!(order(&tabs), [2, 3, 1]);
    tabs.close_active(CloseDecision::Discard).unwrap();
    assert_eq!(tabs.active().unwrap().id, DocumentId(3));
    assert_eq!(order(&tabs), [3, 1]);
    let review = tabs.active_close_review().unwrap();
    tabs.close_reviewed(review, CloseDecision::Discard).unwrap();
    assert_eq!(order(&tabs), [1]);
}

#[test]
fn a_tab_replaced_in_place_takes_the_front_and_the_old_document_leaves_the_order() {
    // Break caught: a replaced preview (or reused untitled tab) still listed by Ctrl+P under
    // its old document, or the note now in it missing.
    let mut tabs = Tabs::with_document(document(1));
    tabs.push(preview(2)).unwrap();
    tabs.push(document(3)).unwrap();
    tabs.replace_preview(preview(4)).unwrap();
    assert_eq!(order(&tabs), [4, 3, 1]);
    tabs.activate(DocumentId(1)).unwrap();
    tabs.replace_active_untitled(document(5)).unwrap();
    assert_eq!(order(&tabs), [5, 4, 3]);
    tabs.clear_for_shutdown();
    assert!(order(&tabs).is_empty());
}

#[test]
fn restored_tabs_restart_the_order_from_the_strip_with_the_active_tab_first() {
    // Break caught: after a session restore, the tabs listed last-restored first (each one
    // entered at the front as it opened).
    let mut tabs = Tabs::with_document(document(1));
    tabs.push(document(2)).unwrap();
    tabs.push(document(3)).unwrap();
    tabs.push(document(4)).unwrap();
    tabs.activate(DocumentId(3)).unwrap();
    tabs.reset_activation_order();
    assert_eq!(order(&tabs), [3, 1, 2, 4]);
    let from = Tabs::from_documents([document(5), document(6)]).unwrap();
    assert_eq!(order(&from), [5, 6]);
}

#[test]
fn each_tab_keeps_its_own_view_state() {
    // Break caught: switching tabs forgetting where you were, or one tab's caret landing in
    // another.
    let mut tabs = Tabs::with_document(document(1));
    tabs.push(document(2)).unwrap();
    let state = crate::editor::ViewState {
        caret: 7,
        anchor: 3,
        first_line: 2,
        x_offset: 0,
    };
    tabs.set_view_state(DocumentId(1), state);
    assert_eq!(tabs.view_state(DocumentId(1)), state);
    assert_eq!(
        tabs.view_state(DocumentId(2)),
        crate::editor::ViewState::default()
    );
}

#[test]
fn closing_the_shown_tab_keeps_the_next_tabs_view_state() {
    // Break caught: the tab activated by a close inheriting the closed tab's caret.
    let mut tabs = Tabs::with_document(document(1));
    tabs.push(document(2)).unwrap();
    let second = crate::editor::ViewState {
        caret: 9,
        anchor: 9,
        first_line: 4,
        x_offset: 0,
    };
    tabs.set_view_state(DocumentId(2), second);
    tabs.activate(DocumentId(1)).unwrap();
    tabs.close_active(CloseDecision::Discard).unwrap();
    assert_eq!(
        tabs.active().map(|document| document.id),
        Some(DocumentId(2))
    );
    assert_eq!(tabs.view_state(DocumentId(2)), second);
}

#[test]
fn a_document_can_have_a_view_in_each_group_and_leaves_with_its_last_view() {
    // Break caught: a second group's tab removing the document from the store when the first
    // group's tab closes, or a close leaving an orphan in the store.
    let mut tabs = Tabs::new();
    tabs.push(document(1)).unwrap();
    let first = tabs.active_group();
    let second = tabs.add_group();
    assert!(tabs.add_view(second, DocumentId(1), ViewState::default()));
    assert_eq!(tabs.views_of(DocumentId(1)), vec![first, second]);
    let kept = tabs.close_active(CloseDecision::Discard).unwrap();
    assert!(kept.is_none(), "not the last view: the document stays");
    assert_eq!(tabs.views_of(DocumentId(1)), vec![second]);
    assert!(tabs.document(DocumentId(1)).is_some());
    tabs.set_active_group(second);
    let closed = tabs.close_active(CloseDecision::Discard).unwrap().unwrap();
    assert_eq!(closed.id, DocumentId(1));
    assert!(tabs.document(DocumentId(1)).is_none());
}

#[test]
fn the_facade_methods_act_on_the_active_group() {
    // Break caught: `active()` or `len()` reading the first group after the user moved to
    // another one, so commands act on the wrong tab.
    let mut tabs = Tabs::new();
    tabs.push(document(1)).unwrap();
    let second = tabs.add_group();
    assert!(tabs.set_active_group(second));
    assert!(tabs.is_empty());
    assert!(tabs.active().is_none());
    tabs.push(document(2)).unwrap();
    assert_eq!(
        tabs.active().map(|document| document.id),
        Some(DocumentId(2))
    );
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs.documents().count(), 2, "documents() spans every group");
    assert_eq!(
        tabs.activation_order(),
        &[
            (second, DocumentId(2)),
            (tabs.group_ids()[0], DocumentId(1))
        ]
    );
}

#[test]
fn a_second_view_promotes_a_preview_and_a_preview_is_replaced_only_in_its_group() {
    // Break caught: an italic tab whose document is also open elsewhere being replaced by the
    // next tree click, closing a view the user did not click away from.
    let mut tabs = Tabs::new();
    let mut preview = document(1);
    preview.preview = true;
    tabs.push(preview).unwrap();
    let first = tabs.active_group();
    let second = tabs.add_group();
    tabs.set_active_group(second);
    let mut other = document(2);
    other.preview = true;
    assert!(
        tabs.replace_preview(other).is_none(),
        "group 2 had no preview"
    );
    assert_eq!(tabs.group(first).unwrap().len(), 1);
    assert_eq!(tabs.preview_id(), Some(DocumentId(2)));
    assert!(tabs.add_view(first, DocumentId(2), ViewState::default()));
    assert!(!tabs.document(DocumentId(2)).unwrap().preview);
}

#[test]
fn moving_a_view_keeps_its_state_and_the_document() {
    // Break caught: Move to Next Group dropping the caret, or removing the document between
    // the removal and the insertion.
    let mut tabs = Tabs::new();
    tabs.push(document(1)).unwrap();
    let first = tabs.active_group();
    let second = tabs.add_group();
    let state = ViewState {
        caret: 7,
        anchor: 3,
        first_line: 2,
        x_offset: 0,
    };
    tabs.set_view_state_in(first, DocumentId(1), state);
    assert!(tabs.move_view(first, DocumentId(1), second));
    assert!(tabs.group(first).unwrap().is_empty());
    assert_eq!(tabs.view_state_in(second, DocumentId(1)), state);
    assert!(tabs.document(DocumentId(1)).is_some());
    assert!(tabs.remove_group(first));
    assert!(!tabs.remove_group(second), "the last group stays");
}

#[test]
fn a_text_change_is_recorded_once_on_the_document() {
    // Break caught: a document-level change applied per view, bumping the generation twice
    // for one keystroke.
    let mut tabs = Tabs::new();
    tabs.push(document(1)).unwrap();
    let second = tabs.add_group();
    tabs.add_view(second, DocumentId(1), ViewState::default());
    let before = tabs.document(DocumentId(1)).unwrap().generation;
    tabs.note_text_change(DocumentId(1));
    assert_eq!(tabs.document(DocumentId(1)).unwrap().generation, before + 1);
    assert!(tabs.set_dirty(DocumentId(1), true));
    assert!(!tabs.set_dirty(DocumentId(1), true));
}

#[test]
fn a_view_goes_in_at_the_insertion_point() {
    // Break caught: a tab dropped between two tabs landing at the end of the strip.
    let mut tabs = Tabs::new();
    let first = tabs.active_group();
    tabs.push(document(1)).unwrap();
    let second = tabs.add_group();
    tabs.set_active_group(second);
    tabs.push(document(2)).unwrap();
    tabs.push(document(3)).unwrap();
    assert!(tabs.move_view_at(first, DocumentId(1), second, Some(1)));
    assert_eq!(
        tabs.group(second).unwrap().document_ids(),
        [DocumentId(2), DocumentId(1), DocumentId(3)]
    );
    assert_eq!(
        tabs.group(second).unwrap().active_document(),
        Some(DocumentId(1))
    );
    assert!(tabs.group(first).unwrap().is_empty());
    assert!(tabs.add_view_at(first, DocumentId(3), ViewState::default(), Some(9)));
    assert_eq!(tabs.group(first).unwrap().document_ids(), [DocumentId(3)]);
}

#[test]
fn a_view_already_in_the_target_is_selected_where_it_is() {
    // Break caught: a second view of one document in one group (spec §6.2).
    let mut tabs = Tabs::new();
    let first = tabs.active_group();
    tabs.push(document(1)).unwrap();
    let second = tabs.add_group();
    assert!(tabs.add_view(second, DocumentId(1), ViewState::default()));
    tabs.set_active_group(second);
    tabs.push(document(2)).unwrap();
    assert!(tabs.move_view_at(first, DocumentId(1), second, Some(2)));
    assert_eq!(
        tabs.group(second).unwrap().document_ids(),
        [DocumentId(1), DocumentId(2)]
    );
    assert_eq!(
        tabs.group(second).unwrap().active_document(),
        Some(DocumentId(1))
    );
    assert!(
        tabs.group(first).unwrap().is_empty(),
        "a move still removes the source view"
    );
}

#[test]
fn reordering_moves_the_tab_and_keeps_the_active_document_selected() {
    // Break caught: the selection index left behind, so the strip highlights one tab while
    // the editor shows another.
    let mut tabs = Tabs::new();
    let group = tabs.active_group();
    for id in 1..=3 {
        tabs.push(document(id)).unwrap();
    }
    tabs.activate(DocumentId(2)).unwrap();
    assert!(tabs.reorder(group, 0, 2));
    assert_eq!(
        tabs.group(group).unwrap().document_ids(),
        [DocumentId(2), DocumentId(3), DocumentId(1)]
    );
    assert_eq!(tabs.active().unwrap().id, DocumentId(2));
    assert!(!tabs.reorder(group, 0, 3), "out of range");
}

#[test]
fn a_preview_arriving_where_a_preview_already_is_becomes_a_normal_tab() {
    // Break caught (Review Focus 3): two italic tabs in one group, so the next tree click
    // replaces one of them and the other lingers forever.
    let mut tabs = Tabs::new();
    let first = tabs.active_group();
    tabs.push(preview(1)).unwrap();
    let second = tabs.add_group();
    tabs.set_active_group(second);
    tabs.push(preview(2)).unwrap();
    assert!(tabs.move_view(first, DocumentId(1), second));
    assert!(!tabs.document(DocumentId(1)).unwrap().preview);
    assert!(tabs.document(DocumentId(2)).unwrap().preview);
    assert_eq!(tabs.preview_id(), Some(DocumentId(2)));
}

#[test]
fn a_preview_moved_into_a_group_without_one_stays_a_preview() {
    // Break caught: every move pinning its tab, so a quick look can never be replaced.
    let mut tabs = Tabs::new();
    let first = tabs.active_group();
    tabs.push(preview(1)).unwrap();
    let second = tabs.add_group();
    assert!(tabs.move_view(first, DocumentId(1), second));
    assert!(tabs.document(DocumentId(1)).unwrap().preview);
}
