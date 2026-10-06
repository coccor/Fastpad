use super::{Editor, EditorDocument};
use crate::editor::scintilla_constants::{
    SC_ELEMENT_CARET_LINE_BACK, SC_ELEMENT_SELECTION_BACK, SC_ELEMENT_SELECTION_INACTIVE_BACK,
    SCI_ADDREFDOCUMENT, SCI_BEGINUNDOACTION, SCI_CANREDO, SCI_CANUNDO, SCI_COPYALLOWLINE,
    SCI_CUTALLOWLINE, SCI_ENDUNDOACTION, SCI_GETSELECTIONEND, SCI_GETSELECTIONSTART,
    SCI_GETSELTEXT, SCI_GETTARGETEND, SCI_PASTE, SCI_REDO, SCI_RELEASEDOCUMENT, SCI_REPLACETARGET,
    SCI_SEARCHINTARGET, SCI_SETDOCPOINTER, SCI_SETELEMENTCOLOUR, SCI_SETILEXER, SCI_SETMARGINLEFT,
    SCI_SETMARGINRIGHT, SCI_SETMARGINWIDTHN, SCI_SETSCROLLWIDTH, SCI_SETSCROLLWIDTHTRACKING,
    SCI_SETSEARCHFLAGS, SCI_SETSEL, SCI_SETTARGETRANGE, SCI_STYLECLEARALL, SCI_STYLESETBACK,
    SCI_STYLESETBOLD, SCI_STYLESETFONT, SCI_STYLESETFORE, SCI_STYLESETITALIC, SCI_UNDO,
};
use crate::editor::scintilla_constants::{
    SC_FOLDACTION_CONTRACT, SC_FOLDACTION_EXPAND, SC_FOLDFLAG_LINEAFTER_CONTRACTED,
    SC_MARGIN_SYMBOL, SC_MARK_BOXMINUS, SC_MARK_BOXMINUSCONNECTED, SC_MARK_BOXPLUS,
    SC_MARK_BOXPLUSCONNECTED, SC_MARK_LCORNER, SC_MARK_TCORNER, SC_MARK_VLINE, SC_MARKNUM_FOLDER,
    SC_MARKNUM_FOLDEREND, SC_MARKNUM_FOLDERMIDTAIL, SC_MARKNUM_FOLDEROPEN,
    SC_MARKNUM_FOLDEROPENMID, SC_MARKNUM_FOLDERSUB, SC_MARKNUM_FOLDERTAIL, SC_MASK_FOLDERS,
    SCI_FOLDALL, SCI_MARKERDEFINE, SCI_MARKERSETBACK, SCI_MARKERSETBACKSELECTED, SCI_MARKERSETFORE,
    SCI_SETAUTOMATICFOLD, SCI_SETFOLDFLAGS, SCI_SETFOLDMARGINCOLOUR, SCI_SETFOLDMARGINHICOLOUR,
    SCI_SETMARGINMASKN, SCI_SETMARGINSENSITIVEN,
};
use crate::editor::scintilla_constants::{
    SC_MARGIN_NUMBER, SCI_GETLINECOUNT, SCI_SETMARGINTYPEN, SCI_SETZOOM, SCI_STYLEGETBACK,
    SCI_TEXTWIDTH, SCI_ZOOMIN, SCI_ZOOMOUT, STYLE_DEFAULT, STYLE_LINENUMBER,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP};

/// Destroys the win32 host window created for [`test_editor`] once nothing needs it any more.
struct HostWindow(HWND);

impl Drop for HostWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}

/// Owns a real, native-backed Scintilla [`Editor`] for tests that need genuine buffer memory
/// (`SCI_GETRANGEPOINTER`) or genuine line/visible-line bookkeeping that the fake
/// `TestDirectHarness` below cannot provide. Mirrors how `main_window.rs`'s tests manage the
/// same two resources: `load_native_scintilla` returns an `OwnedModule` that calls
/// `FreeLibrary` on drop, and `ProductionWindow` destroys its window on drop. Dropping a
/// `TestEditor` runs its fields' drops top to bottom in declaration order: `editor` first
/// (Scintilla's own `Drop` destroys the child control window), then `_host` (destroys the
/// parent window), then `_module` (unloads the DLL) last.
struct TestEditor {
    editor: Editor,
    _host: HostWindow,
    _module: crate::platform::OwnedModule,
}

impl std::ops::Deref for TestEditor {
    type Target = Editor;

    fn deref(&self) -> &Editor {
        &self.editor
    }
}

fn test_editor() -> TestEditor {
    let dll_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("native/out/x64/Scintilla.dll");
    let wide_path = crate::platform::wide_null(dll_path.to_str().unwrap());
    let module = unsafe {
        LoadLibraryExW(
            wide_path.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    let module = unsafe { crate::platform::OwnedModule::from_raw_owned(module) }
        .expect("failed to load native Scintilla.dll for tests");

    let host_class = crate::platform::wide_null("STATIC");
    let parent = unsafe {
        CreateWindowExW(
            0,
            host_class.as_ptr(),
            std::ptr::null(),
            WS_POPUP,
            0,
            0,
            800,
            600,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    assert!(
        !parent.is_null(),
        "failed to create a host window for tests"
    );

    let editor =
        Editor::create(parent).expect("failed to create a native Scintilla editor for tests");
    TestEditor {
        editor,
        _host: HostWindow(parent),
        _module: module,
    }
}

#[test]
fn a_host_document_shows_in_two_editors_and_outlives_both() {
    // Break caught: a document tied to the editor that created it, so a second group's editor
    // refuses it, or closing a group frees text another group still shows.
    let first = test_editor();
    let host = Editor::create_document_host().expect("document host");
    let left = Editor::create_with_host(first._host.0, &host).expect("left editor");
    let right = Editor::create_with_host(first._host.0, &host).expect("right editor");
    let document = left.create_document().unwrap();
    left.use_document(&document).unwrap();
    right.use_document(&document).unwrap();
    left.set_text("shared").unwrap();
    assert_eq!(right.text().unwrap(), "shared");
    drop(left);
    drop(right);
    host.use_document(&document).unwrap();
    assert_eq!(host.text().unwrap(), "shared");
}

#[test]
fn an_editor_refuses_documents_from_another_host() {
    // Break caught: SCI_SETDOCPOINTER with a document whose owner can be destroyed under it.
    let fixture = test_editor();
    let host = Editor::create_document_host().expect("document host");
    let hosted = Editor::create_with_host(fixture._host.0, &host).expect("hosted editor");
    let foreign = fixture.create_document().unwrap();
    assert!(hosted.use_document(&foreign).is_err());
    assert!(hosted.shares_documents_with(&host));
    assert!(!hosted.shares_documents_with(&fixture));
}

#[test]
fn view_state_round_trips_and_clamps_to_a_shorter_document() {
    // Break caught: switching back to a tab lands at the top, or a saved caret past the end of
    // a file that shrank on disk panics or selects garbage.
    let editor = test_editor();
    editor.set_text(&"line\n".repeat(200)).unwrap();
    let saved = crate::editor::ViewState {
        caret: 500,
        anchor: 495,
        first_line: 90,
        x_offset: 0,
    };
    editor.apply_view_state(saved).unwrap();
    assert_eq!(editor.view_state().unwrap(), saved);
    editor.set_text("short").unwrap();
    editor.apply_view_state(saved).unwrap();
    let clamped = editor.view_state().unwrap();
    assert_eq!((clamped.caret, clamped.anchor), (5, 5));
}

#[test]
fn range_bytes_returns_the_requested_slice_without_copying_the_document() {
    let editor = test_editor();
    editor.set_text("alpha\nbeta\ngamma").unwrap();
    assert_eq!(editor.range_bytes(6..10).unwrap(), b"beta");
    assert_eq!(editor.range_bytes(0..0).unwrap(), b"");
}

#[test]
fn document_text_is_the_whole_buffer_even_after_an_edit_moves_the_gap() {
    // Break caught: a pointer read before the gap is closed (text after the caret missing
    // or garbled), a stale length, or an empty document refused.
    let editor = test_editor();
    editor.set_text("o mașină nouă").unwrap();
    editor.set_selection(2..2).unwrap();
    editor.replace_target(2..2, "altă ").unwrap();
    assert_eq!(
        editor.with_document_text(str::to_owned).unwrap(),
        "o altă mașină nouă"
    );
    editor.set_text("").unwrap();
    assert_eq!(editor.with_document_text(str::len).unwrap(), 0);
}

#[test]
fn replace_ranges_with_puts_each_text_in_its_range_from_the_end_as_one_undo_step() {
    // Break caught: ascending edits (as `Matcher::replacements` gives them) replaced front to
    // back (later ranges shifted onto the wrong text), an edit given another edit's text,
    // one undo step per range, or an empty list failing instead of replacing nothing.
    let editor = test_editor();
    editor.populate_clean("foo bar foo baz foo").unwrap();
    let edits = [
        (0..3, "a".to_owned()),
        (8..11, "ță".to_owned()),
        (16..19, "quux".to_owned()),
    ];
    assert_eq!(editor.replace_ranges_with(&edits).unwrap(), 3);
    assert_eq!(editor.text().unwrap(), "a bar ță baz quux");
    editor.undo().unwrap();
    assert_eq!(editor.text().unwrap(), "foo bar foo baz foo");
    assert_eq!(editor.replace_ranges_with(&[]).unwrap(), 0);
}

#[test]
fn line_queries_map_positions_and_visible_lines() {
    let editor = test_editor();
    editor.set_text("a\nb\nc\nd\n").unwrap();
    assert_eq!(editor.line_from_position(4).unwrap(), 2);
    assert_eq!(editor.doc_line_from_visible(3).unwrap(), 3);
    assert_eq!(editor.visible_from_doc_line(3).unwrap(), 3);
    editor.set_first_visible_line(2).unwrap();
    assert!(editor.first_visible_line().unwrap() <= 2);
}

#[test]
fn notification_struct_matches_scnotification_layout() {
    use std::mem::offset_of;
    assert_eq!(offset_of!(super::ScintillaNotification, position), 24);
    assert_eq!(
        offset_of!(super::ScintillaNotification, modification_type),
        40
    );
    assert_eq!(offset_of!(super::ScintillaNotification, lines_added), 64);
    // 144, not the task brief's stated 136: native/src/scintilla/include/Sci_Position.h
    // defines `Sci_Position` (used by `annotationLinesAdded`, just before `updated`) as
    // `ptrdiff_t`, 8 bytes on x64, and native/src/scintilla/include/Scintilla.h has no
    // `#pragma pack`, so the default MSVC x64 ABI pads `annotationLinesAdded` to an 8-byte
    // boundary after the seven `int` fields (foldLevelNow..token) that precede it. Verified by
    // hand against the C header field-by-field, matching `offset_of!`'s own computed value.
    assert_eq!(offset_of!(super::ScintillaNotification, updated), 144);
}

#[test]
fn test_fixture_clone_and_drop_are_inert() {
    // Break caught: test-only fake document handles trying to refcount through a null endpoint.
    let fixture = EditorDocument::test_fixture();
    let clone = fixture.clone();
    assert_eq!(clone.raw(), 0);
    drop(clone);
    drop(fixture);
}

#[test]
fn release_counter_does_not_claim_a_call_after_endpoint_destruction() {
    // Break caught: counting Drop attempts before the liveness gate overstates native releases.
    let releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let document = EditorDocument::test_fixture_with_release_counter(Arc::clone(&releases));
    document
        .endpoint
        .destroyed
        .store(true, std::sync::atomic::Ordering::Release);
    drop(document);
    assert_eq!(releases.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn document_refcounts_keep_using_the_cached_endpoint_after_editor_drop() {
    // Break caught: storing only an HWND in EditorDocument makes clone/drop target a dead
    // editor endpoint after the Editor value is dropped.
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());
    let document = EditorDocument::test_fixture_with_raw(41, &editor);
    let clone = document.clone();

    drop(editor);
    drop(clone);
    drop(document);

    assert_eq!(
        harness.messages(),
        vec![SCI_ADDREFDOCUMENT, SCI_RELEASEDOCUMENT, SCI_RELEASEDOCUMENT]
    );
}

#[test]
fn search_in_target_sets_range_and_flags_before_searching() {
    // Break caught: omitting the requested target range or search flags can reuse stale
    // Scintilla target state and return the wrong match.
    let harness = TestDirectHarness::new();
    harness.push_response(7);
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    let found = editor.search_in_target("needle", 3..15, 99).unwrap();

    assert_eq!(found, Some(7..13));
    assert_eq!(
        harness.messages(),
        vec![
            SCI_SETTARGETRANGE,
            SCI_SETSEARCHFLAGS,
            SCI_SEARCHINTARGET,
            SCI_GETTARGETEND
        ]
    );
    assert_eq!(harness.target_range(), Some((3, 15)));
    assert_eq!(harness.search_flags(), Some(99));
    assert_eq!(harness.search_needle(), Some(b"needle".to_vec()));
}

#[test]
fn search_in_target_reports_the_length_scintilla_matched() {
    // Break caught: a regex hit reported as long as the pattern, so `\d+` over "12345"
    // selects three characters, or a find-next that starts inside the previous match.
    let harness = TestDirectHarness::new();
    harness.push_response(2);
    harness.push_target_end(7);
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    assert_eq!(
        editor.search_in_target(r"\d+", 0..10, 0).unwrap(),
        Some(2..7)
    );
}

#[test]
fn a_pattern_scintilla_cannot_compile_is_a_miss_not_an_error() {
    // Break caught: Scintilla's -2 (a bad regex in some versions) turned into an error or a
    // bogus range, which the find bar would report or panic on.
    let harness = TestDirectHarness::new();
    harness.push_response(-2);
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    assert_eq!(editor.search_in_target("(", 0..10, 0).unwrap(), None);
}

#[test]
fn search_in_target_returns_none_when_scintilla_reports_no_match() {
    // Break caught: turning Scintilla's not-found sentinel into a bogus byte range instead of
    // reporting the absence of a match.
    let harness = TestDirectHarness::new();
    harness.push_response(-1);
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    let found = editor.search_in_target("needle", 0..6, 0).unwrap();

    assert_eq!(found, None);
}

#[test]
fn length_reads_the_document_length() {
    let harness = TestDirectHarness::new();
    harness.push_response(42);
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    assert_eq!(editor.length().unwrap(), 42);
}

#[test]
fn undo_and_redo_send_the_matching_scintilla_messages() {
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.undo().unwrap();
    editor.redo().unwrap();

    assert_eq!(harness.messages(), vec![SCI_UNDO, SCI_REDO]);
}

#[test]
fn can_undo_and_can_redo_report_scintillas_boolean_state() {
    // Break caught: treating any nonzero Scintilla response as `true` incorrectly, or
    // collapsing distinct CANUNDO/CANREDO answers into one shared flag.
    let harness = TestDirectHarness::new();
    harness.push_response(1);
    harness.push_response(0);
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    assert!(editor.can_undo().unwrap());
    assert!(!editor.can_redo().unwrap());
    assert_eq!(harness.messages(), vec![SCI_CANUNDO, SCI_CANREDO]);
}

#[test]
fn cut_copy_paste_send_the_matching_scintilla_messages() {
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.cut().unwrap();
    editor.copy().unwrap();
    editor.paste().unwrap();

    assert_eq!(
        harness.messages(),
        vec![SCI_CUTALLOWLINE, SCI_COPYALLOWLINE, SCI_PASTE]
    );
}

#[test]
fn selection_reads_start_and_end_from_scintilla() {
    let harness = TestDirectHarness::new();
    harness.push_response(3);
    harness.push_response(9);
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    let range = editor.selection().unwrap();

    assert_eq!(range, 3..9);
    assert_eq!(
        harness.messages(),
        vec![SCI_GETSELECTIONSTART, SCI_GETSELECTIONEND]
    );
}

#[test]
fn selection_rejects_an_end_before_start() {
    // Break caught: trusting Scintilla's raw start/end without validating ordering can hand
    // callers a range that panics on use (e.g. slicing) instead of a clear error.
    let harness = TestDirectHarness::new();
    harness.push_response(9);
    harness.push_response(3);
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    assert!(editor.selection().is_err());
}

#[test]
fn set_selection_sends_anchor_and_caret_as_start_and_end() {
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.set_selection(4..10).unwrap();

    assert_eq!(harness.messages(), vec![SCI_SETSEL]);
    assert_eq!(harness.set_sel_calls(), vec![(4, 10)]);
}

#[test]
fn selected_text_reads_the_current_selection_without_a_full_document_fetch() {
    let harness = TestDirectHarness::new();
    harness.set_selected_text("needle");
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    let text = editor.selected_text().unwrap();

    assert_eq!(text, "needle");
    assert_eq!(harness.messages(), vec![SCI_GETSELTEXT, SCI_GETSELTEXT]);
}

#[test]
fn replace_target_sets_the_range_then_replaces_and_returns_the_new_end() {
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    let replaced = editor.replace_target(4..7, "longer").unwrap();

    assert_eq!(replaced, 4..10);
    assert_eq!(
        harness.messages(),
        vec![SCI_SETTARGETRANGE, SCI_REPLACETARGET]
    );
    assert_eq!(harness.target_range(), Some((4, 7)));
    assert_eq!(harness.replace_bytes(), vec![b"longer".to_vec()]);
}

#[test]
fn replace_all_replaces_every_match_as_exactly_one_undo_action() {
    // Break caught: wrapping each individual replacement in its own undo action instead of one
    // action for the whole operation would require multiple Ctrl+Z presses to undo Replace All.
    let harness = TestDirectHarness::new();
    // Iteration 1: document length 11 ("one two one"), match "one" at 0.
    harness.push_response(0); // SCI_BEGINUNDOACTION (ignored)
    harness.push_response(11); // SCI_GETLENGTH
    harness.push_response(0); // SCI_SEARCHINTARGET finds "one" at 0
    harness.push_response(0); // SCI_REPLACETARGET (ignored)
    // Iteration 2: document length now 12 (replaced 3 bytes with 4), match "one" at 9.
    harness.push_response(12); // SCI_GETLENGTH
    harness.push_response(9); // SCI_SEARCHINTARGET finds "one" at 9
    harness.push_response(0); // SCI_REPLACETARGET (ignored)
    // Iteration 3: no more matches.
    harness.push_response(13); // SCI_GETLENGTH
    harness.push_response(-1); // SCI_SEARCHINTARGET finds nothing
    harness.push_response(0); // SCI_ENDUNDOACTION (ignored)
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    let count = editor.replace_all("one", "1111", 0).unwrap();

    assert_eq!(count, 2);
    assert_eq!(
        harness.replace_bytes(),
        vec![b"1111".to_vec(), b"1111".to_vec()]
    );
    assert_eq!(
        harness.event_log(),
        vec!["begin", "replace", "replace", "end"]
    );
}

#[test]
fn set_lexer_sends_the_raw_pointer_via_sci_setilexer() {
    // Break caught: not forwarding the exact opaque ILexer5 pointer Lexilla returned (or
    // routing it through the wrong message) would hand Scintilla a value it cannot own.
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.set_lexer(0x1234).unwrap();

    assert_eq!(harness.messages(), vec![SCI_SETILEXER]);
    assert_eq!(harness.lexer_calls(), vec![0x1234]);
}

#[test]
fn set_lexer_with_null_sends_the_null_lexer() {
    // Break caught: treating a null (plain text) lexer as a no-op instead of explicitly
    // clearing any previously installed lexer.
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.set_lexer(0).unwrap();

    assert_eq!(harness.lexer_calls(), vec![0]);
}

#[test]
fn clear_all_styles_sends_sci_styleclearall() {
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.clear_all_styles().unwrap();

    assert_eq!(harness.messages(), vec![SCI_STYLECLEARALL]);
}

#[test]
fn set_style_sends_foreground_background_bold_and_font_for_the_style_id() {
    // Break caught: dropping one of fore/back/bold/font, or sending them for the wrong style
    // id, leaves a lexer's styling stale or bleeding across style numbers.
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor
        .set_style(2, 0xff0000, 0x00ff00, true, true, "Consolas")
        .unwrap();

    assert_eq!(
        harness.messages(),
        vec![
            SCI_STYLESETFORE,
            SCI_STYLESETBACK,
            SCI_STYLESETBOLD,
            SCI_STYLESETITALIC,
            SCI_STYLESETFONT
        ]
    );
    assert_eq!(
        harness.style_calls(),
        vec![
            (SCI_STYLESETFORE, 2, 0xff0000),
            (SCI_STYLESETBACK, 2, 0x00ff00),
            (SCI_STYLESETBOLD, 2, 1),
            (SCI_STYLESETITALIC, 2, 1),
        ]
    );
    assert_eq!(harness.font_calls(), vec![(2, b"Consolas".to_vec())]);
}

#[test]
fn set_style_sends_zero_bold_and_italic_flags_when_plain() {
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.set_style(0, 0, 0, false, false, "Consolas").unwrap();

    assert_eq!(
        harness.style_calls(),
        vec![
            (SCI_STYLESETFORE, 0, 0),
            (SCI_STYLESETBACK, 0, 0),
            (SCI_STYLESETBOLD, 0, 0),
            (SCI_STYLESETITALIC, 0, 0),
        ]
    );
}

#[test]
fn set_style_rejects_a_font_face_containing_nul_bytes() {
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    assert!(
        editor
            .set_style(0, 0, 0, false, false, "bad\0face")
            .is_err()
    );
}

#[test]
fn a_failed_cosmetic_chrome_default_still_yields_a_usable_editor() {
    // Break caught: a cosmetic margin or scroll-width failure aborts Editor::create, so FastPad
    // exits with a startup-fatal code instead of opening an editable window (spec 228).
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    let result =
        editor.initialize_view(|_| Err(crate::FastPadError::Invariant("cosmetic chrome failure")));

    assert!(result.is_ok());
    assert_eq!(
        harness.messages(),
        vec![crate::editor::scintilla_constants::SCI_SETCODEPAGE]
    );
}

#[test]
fn chrome_defaults_show_only_a_line_number_margin_and_track_scroll_width() {
    // Break caught: Scintilla's default 16 px symbol margin and 2000 px scroll width show an
    // unthemed grey gutter and a permanent horizontal scrollbar, and a number margin left at
    // its default grey background or zero width hides the line numbers shown by default.
    let harness = TestDirectHarness::new();
    harness.set_line_count(1);
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.apply_chrome_defaults(144).unwrap();

    assert_eq!(
        harness.calls(),
        vec![
            (SCI_SETMARGINTYPEN, 0, SC_MARGIN_NUMBER as isize),
            (SCI_SETMARGINWIDTHN, 1, 0),
            (SCI_SETMARGINWIDTHN, 2, 0),
            (SCI_SETMARGINTYPEN, 2, SC_MARGIN_SYMBOL as isize),
            (SCI_SETMARGINMASKN, 2, SC_MASK_FOLDERS as isize),
            (SCI_SETMARGINSENSITIVEN, 2, 1),
            (
                SCI_MARKERDEFINE,
                SC_MARKNUM_FOLDEROPEN as usize,
                SC_MARK_BOXMINUS as isize
            ),
            (
                SCI_MARKERDEFINE,
                SC_MARKNUM_FOLDER as usize,
                SC_MARK_BOXPLUS as isize
            ),
            (
                SCI_MARKERDEFINE,
                SC_MARKNUM_FOLDERSUB as usize,
                SC_MARK_VLINE as isize
            ),
            (
                SCI_MARKERDEFINE,
                SC_MARKNUM_FOLDERTAIL as usize,
                SC_MARK_LCORNER as isize
            ),
            (
                SCI_MARKERDEFINE,
                SC_MARKNUM_FOLDEREND as usize,
                SC_MARK_BOXPLUSCONNECTED as isize
            ),
            (
                SCI_MARKERDEFINE,
                SC_MARKNUM_FOLDEROPENMID as usize,
                SC_MARK_BOXMINUSCONNECTED as isize
            ),
            (
                SCI_MARKERDEFINE,
                SC_MARKNUM_FOLDERMIDTAIL as usize,
                SC_MARK_TCORNER as isize
            ),
            (SCI_SETAUTOMATICFOLD, 7, 0),
            (
                SCI_SETFOLDFLAGS,
                SC_FOLDFLAG_LINEAFTER_CONTRACTED as usize,
                0
            ),
            (SCI_STYLEGETBACK, STYLE_DEFAULT as usize, 0),
            (
                SCI_STYLESETBACK,
                STYLE_LINENUMBER as usize,
                TEST_DEFAULT_BACKGROUND
            ),
            (SCI_SETMARGINLEFT, 0, 12),
            (SCI_SETMARGINRIGHT, 0, 12),
            (SCI_SETSCROLLWIDTH, 1, 0),
            (SCI_SETSCROLLWIDTHTRACKING, 1, 0),
            (SCI_GETLINECOUNT, 0, 0),
            (SCI_TEXTWIDTH, STYLE_LINENUMBER as usize, 0),
            (SCI_SETMARGINWIDTHN, 0, 30),
        ]
    );
    // Two digits plus one digit of breathing room, so short files do not resize at line 10.
    assert_eq!(harness.text_width_texts(), vec![b"999".to_vec()]);
}

#[test]
fn code_folding_shows_the_fold_margin_scaled_to_dpi_and_hiding_it_expands_every_fold() {
    // Break caught: a fold margin left at width 0 when enabled (folding invisible), or left
    // collapsed-but-folded after turning folding off, hiding text behind an unclickable margin.
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.set_code_folding(true, 144).unwrap();
    editor.set_code_folding(false, 144).unwrap();

    assert_eq!(
        harness.calls(),
        vec![
            (SCI_SETMARGINWIDTHN, 2, 21),
            (SCI_SETMARGINWIDTHN, 2, 0),
            (SCI_FOLDALL, SC_FOLDACTION_EXPAND as usize, 0),
        ]
    );
}

#[test]
fn fold_all_contracts_or_expands_with_scintillas_fold_actions() {
    // Break caught: Fold All and Unfold All sending the same action.
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.fold_all(true).unwrap();
    editor.fold_all(false).unwrap();

    assert_eq!(
        harness.calls(),
        vec![
            (SCI_FOLDALL, SC_FOLDACTION_CONTRACT as usize, 0),
            (SCI_FOLDALL, SC_FOLDACTION_EXPAND as usize, 0),
        ]
    );
}

#[test]
fn fold_colors_paint_the_margin_and_every_folder_marker_from_the_palette() {
    // Break caught: markers keeping Scintilla's default white and grey, invisible or garish on
    // the dark, Paper, Lamp and high-contrast themes.
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.set_fold_colors(0x0033_4455, 0x0000_1122).unwrap();

    let calls = harness.calls();
    assert_eq!(
        &calls[..2],
        [
            (SCI_SETFOLDMARGINCOLOUR, 1, 0x1122),
            (SCI_SETFOLDMARGINHICOLOUR, 1, 0x1122),
        ]
    );
    for message in [
        SCI_MARKERSETFORE,
        SCI_MARKERSETBACK,
        SCI_MARKERSETBACKSELECTED,
    ] {
        let markers = calls
            .iter()
            .filter(|call| call.0 == message)
            .map(|call| (call.1, call.2))
            .collect::<Vec<_>>();
        let colour = if message == SCI_MARKERSETFORE {
            0x1122
        } else {
            0x0033_4455
        };
        assert_eq!(markers.len(), 7, "{message}");
        assert!(
            markers
                .iter()
                .all(|&(marker, value)| (25..=31).contains(&marker) && value == colour)
        );
    }
}

#[test]
fn line_number_margin_resizes_only_when_the_line_count_gains_a_digit() {
    // Break caught: re-measuring on every edit costs a font measurement per keystroke, while
    // never re-measuring clips line 100 in a margin sized for two digits.
    let harness = TestDirectHarness::new();
    harness.set_line_count(9);
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());
    editor.set_line_numbers(true).unwrap();

    harness.set_line_count(99);
    editor.refresh_line_numbers().unwrap();
    harness.set_line_count(100);
    editor.refresh_line_numbers().unwrap();

    let widths: Vec<_> = harness
        .calls()
        .into_iter()
        .filter(|call| call.0 == SCI_SETMARGINWIDTHN)
        .collect();
    assert_eq!(
        widths,
        vec![(SCI_SETMARGINWIDTHN, 0, 30), (SCI_SETMARGINWIDTHN, 0, 40)]
    );
    assert_eq!(
        harness.text_width_texts(),
        vec![b"999".to_vec(), b"9999".to_vec()]
    );
}

#[test]
fn hidden_line_numbers_collapse_the_margin_and_ignore_refreshes() {
    // Break caught: line_numbers=false still showing a gutter, or a later edit re-opening it.
    let harness = TestDirectHarness::new();
    harness.set_line_count(500);
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());
    editor.set_line_numbers(true).unwrap();

    editor.set_line_numbers(false).unwrap();
    let hidden_at = harness.calls().len();
    editor.refresh_line_numbers().unwrap();
    editor.remeasure_line_numbers().unwrap();

    assert_eq!(harness.calls()[hidden_at - 1], (SCI_SETMARGINWIDTHN, 0, 0));
    assert_eq!(harness.calls().len(), hidden_at);
}

#[test]
fn remeasuring_line_numbers_resizes_even_when_the_digit_count_is_unchanged() {
    // Break caught: a font-size or DPI change keeping the old pixel width, clipping the numbers.
    let harness = TestDirectHarness::new();
    harness.set_line_count(9);
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());
    editor.set_line_numbers(true).unwrap();

    editor.remeasure_line_numbers().unwrap();

    assert_eq!(harness.text_width_texts().len(), 2);
}

#[test]
fn view_settings_also_restyle_the_line_number_font() {
    // Break caught: STYLE_LINENUMBER sits just past STYLE_DEFAULT, so a loop ending at
    // STYLE_DEFAULT leaves line numbers in Scintilla's default font and size.
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor
        .apply_view_settings("Cascadia Code", 12, 4, false)
        .unwrap();

    assert!(
        harness
            .font_calls()
            .contains(&(STYLE_LINENUMBER as usize, b"Cascadia Code".to_vec()))
    );
}

#[test]
fn line_number_colors_target_the_line_number_style() {
    // Break caught: STYLE_LINENUMBER sits outside the base-color loop, so the gutter keeps
    // Scintilla's grey band, or full-contrast text after a lexer's style reset.
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor
        .set_line_number_colors(0x0060_6060, 0x00FF_FFFF)
        .unwrap();

    assert_eq!(
        harness.style_calls(),
        vec![
            (SCI_STYLESETFORE, STYLE_LINENUMBER as usize, 0x0060_6060),
            (SCI_STYLESETBACK, STYLE_LINENUMBER as usize, 0x00FF_FFFF),
        ]
    );
}

#[test]
fn text_padding_is_eight_pixels_at_96_dpi() {
    // Break caught: zero margins leave the caret and first glyph touching the window frame.
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.set_text_padding(96).unwrap();

    assert_eq!(
        harness.calls(),
        vec![(SCI_SETMARGINLEFT, 0, 8), (SCI_SETMARGINRIGHT, 0, 8)]
    );
}

#[test]
fn zoom_commands_step_the_view_and_reset_to_the_configured_size() {
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.zoom_in().unwrap();
    editor.zoom_out().unwrap();
    editor.reset_zoom().unwrap();

    assert_eq!(
        harness.calls(),
        vec![(SCI_ZOOMIN, 0, 0), (SCI_ZOOMOUT, 0, 0), (SCI_SETZOOM, 0, 0)]
    );
}

#[test]
fn switching_documents_resets_the_scroll_width() {
    // Break caught: scroll-width tracking only grows, so a short tab after a wide one would
    // keep the wide tab's horizontal scrollbar.
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());
    let document = EditorDocument::test_fixture_with_raw(41, &editor);

    editor.use_document(&document).unwrap();

    assert_eq!(
        harness.calls(),
        vec![(SCI_SETDOCPOINTER, 0, 41), (SCI_SETSCROLLWIDTH, 1, 0)]
    );
}

#[test]
fn chrome_colors_are_sent_as_opaque_element_colours() {
    // Break caught: Scintilla 5 element colours carry alpha in the top byte; a bare COLORREF has
    // alpha 0 and would leave the selection and caret line invisible.
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor
        .set_chrome_colors(0x0078_4F26, 0x0041_3D3A, Some(0x0028_2828))
        .unwrap();

    assert_eq!(
        harness.calls(),
        vec![
            (
                SCI_SETELEMENTCOLOUR,
                SC_ELEMENT_SELECTION_BACK as usize,
                0xFF78_4F26_u32 as isize
            ),
            (
                SCI_SETELEMENTCOLOUR,
                SC_ELEMENT_SELECTION_INACTIVE_BACK as usize,
                0xFF41_3D3A_u32 as isize
            ),
            (
                SCI_SETELEMENTCOLOUR,
                SC_ELEMENT_CARET_LINE_BACK as usize,
                0xFF28_2828_u32 as isize
            ),
        ]
    );
}

#[test]
fn high_contrast_selection_text_is_forced_and_otherwise_reset() {
    // Break caught: leaving the selected-text element unset paints lexer-colored text on the
    // system highlight background in high contrast, which is frequently unreadable; never
    // resetting it would then keep those system colors after leaving high contrast.
    use crate::editor::scintilla_constants::{
        SC_ELEMENT_SELECTION_INACTIVE_TEXT, SC_ELEMENT_SELECTION_TEXT, SCI_RESETELEMENTCOLOUR,
    };
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    editor.set_selection_text_colors(Some(0x00FF_FFFF)).unwrap();
    editor.set_selection_text_colors(None).unwrap();

    assert_eq!(
        harness.calls(),
        vec![
            (
                SCI_SETELEMENTCOLOUR,
                SC_ELEMENT_SELECTION_TEXT as usize,
                0xFFFF_FFFF_u32 as isize
            ),
            (
                SCI_SETELEMENTCOLOUR,
                SC_ELEMENT_SELECTION_INACTIVE_TEXT as usize,
                0xFFFF_FFFF_u32 as isize
            ),
            (
                SCI_RESETELEMENTCOLOUR,
                SC_ELEMENT_SELECTION_TEXT as usize,
                0
            ),
            (
                SCI_RESETELEMENTCOLOUR,
                SC_ELEMENT_SELECTION_INACTIVE_TEXT as usize,
                0
            ),
        ]
    );
}

#[test]
fn replace_all_with_an_empty_query_does_nothing() {
    let harness = TestDirectHarness::new();
    let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

    let count = editor.replace_all("", "x", 0).unwrap();

    assert_eq!(count, 0);
    assert!(harness.messages().is_empty());
}

#[derive(Default)]
struct TestDirectState {
    messages: Vec<u32>,
    responses: VecDeque<isize>,
    target_range: Option<(usize, isize)>,
    search_flags: Option<usize>,
    search_needle: Option<Vec<u8>>,
    /// Where the last hit's target ends: its position plus the needle's length, as a plain
    /// search reports it, unless `target_ends` scripts another end (a regex match).
    last_target_end: isize,
    target_ends: VecDeque<isize>,
    replace_bytes: Vec<Vec<u8>>,
    set_sel_calls: Vec<(usize, isize)>,
    selected_text: Option<Vec<u8>>,
    event_log: Vec<&'static str>,
    lexer_calls: Vec<isize>,
    style_calls: Vec<(u32, usize, isize)>,
    font_calls: Vec<(usize, Vec<u8>)>,
    line_count: isize,
    text_width_texts: Vec<Vec<u8>>,
    calls: Vec<(u32, usize, isize)>,
}

const TEST_DEFAULT_BACKGROUND: isize = 0x00AB_CDEF;

struct TestDirectHarness {
    state: Arc<Mutex<TestDirectState>>,
}

impl TestDirectHarness {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(TestDirectState::default())),
        }
    }

    fn direct_ptr(&self) -> isize {
        Arc::as_ptr(&self.state) as isize
    }

    fn push_response(&self, response: isize) {
        self.state.lock().unwrap().responses.push_back(response);
    }

    /// Scripts `SCI_GETTARGETEND` for the next hit, as a regex match of another length would.
    fn push_target_end(&self, end: isize) {
        self.state.lock().unwrap().target_ends.push_back(end);
    }

    fn messages(&self) -> Vec<u32> {
        self.state.lock().unwrap().messages.clone()
    }

    fn target_range(&self) -> Option<(usize, isize)> {
        self.state.lock().unwrap().target_range
    }

    fn search_flags(&self) -> Option<usize> {
        self.state.lock().unwrap().search_flags
    }

    fn search_needle(&self) -> Option<Vec<u8>> {
        self.state.lock().unwrap().search_needle.clone()
    }

    fn replace_bytes(&self) -> Vec<Vec<u8>> {
        self.state.lock().unwrap().replace_bytes.clone()
    }

    fn set_sel_calls(&self) -> Vec<(usize, isize)> {
        self.state.lock().unwrap().set_sel_calls.clone()
    }

    fn set_selected_text(&self, text: &str) {
        self.state.lock().unwrap().selected_text = Some(text.as_bytes().to_vec());
    }

    fn event_log(&self) -> Vec<&'static str> {
        self.state.lock().unwrap().event_log.clone()
    }

    fn lexer_calls(&self) -> Vec<isize> {
        self.state.lock().unwrap().lexer_calls.clone()
    }

    fn style_calls(&self) -> Vec<(u32, usize, isize)> {
        self.state.lock().unwrap().style_calls.clone()
    }

    fn font_calls(&self) -> Vec<(usize, Vec<u8>)> {
        self.state.lock().unwrap().font_calls.clone()
    }

    /// Every direct call, with `SCI_TEXTWIDTH`'s string pointer zeroed so it can be compared.
    fn calls(&self) -> Vec<(u32, usize, isize)> {
        self.state.lock().unwrap().calls.clone()
    }

    fn set_line_count(&self, line_count: isize) {
        self.state.lock().unwrap().line_count = line_count;
    }

    fn text_width_texts(&self) -> Vec<Vec<u8>> {
        self.state.lock().unwrap().text_width_texts.clone()
    }
}

unsafe extern "C" fn test_direct(
    direct_ptr: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    let shared = unsafe { &*(direct_ptr as *const Mutex<TestDirectState>) };
    let mut state = shared.lock().unwrap();
    state.messages.push(message);
    let recorded_lparam = if message == SCI_TEXTWIDTH { 0 } else { lparam };
    state.calls.push((message, wparam, recorded_lparam));
    match message {
        SCI_GETLINECOUNT => state.line_count,
        SCI_STYLEGETBACK => TEST_DEFAULT_BACKGROUND,
        // Ten pixels per measured character keeps the expected widths readable.
        SCI_TEXTWIDTH => {
            let bytes = unsafe { std::ffi::CStr::from_ptr(lparam as *const std::ffi::c_char) }
                .to_bytes()
                .to_vec();
            let width = bytes.len() as isize * 10;
            state.text_width_texts.push(bytes);
            width
        }
        SCI_SETTARGETRANGE => {
            state.target_range = Some((wparam, lparam));
            0
        }
        SCI_SETSEARCHFLAGS => {
            state.search_flags = Some(wparam);
            0
        }
        SCI_SEARCHINTARGET => {
            let bytes = unsafe { std::slice::from_raw_parts(lparam as *const u8, wparam) };
            state.search_needle = Some(bytes.to_vec());
            let found = state.responses.pop_front().unwrap_or(-1);
            if found >= 0 {
                state.last_target_end = found + wparam as isize;
            }
            found
        }
        SCI_GETTARGETEND => {
            let scripted = state.target_ends.pop_front();
            scripted.unwrap_or(state.last_target_end)
        }
        SCI_REPLACETARGET => {
            let bytes = unsafe { std::slice::from_raw_parts(lparam as *const u8, wparam) };
            state.replace_bytes.push(bytes.to_vec());
            state.event_log.push("replace");
            state.responses.pop_front().unwrap_or(0)
        }
        SCI_SETSEL => {
            state.set_sel_calls.push((wparam, lparam));
            0
        }
        SCI_BEGINUNDOACTION => {
            state.event_log.push("begin");
            state.responses.pop_front().unwrap_or(0)
        }
        SCI_ENDUNDOACTION => {
            state.event_log.push("end");
            state.responses.pop_front().unwrap_or(0)
        }
        SCI_GETSELTEXT => {
            let text = state.selected_text.clone().unwrap_or_default();
            if lparam != 0 {
                let buffer =
                    unsafe { std::slice::from_raw_parts_mut(lparam as *mut u8, text.len() + 1) };
                buffer[..text.len()].copy_from_slice(&text);
                buffer[text.len()] = 0;
            }
            text.len() as isize
        }
        SCI_SETILEXER => {
            state.lexer_calls.push(lparam);
            0
        }
        SCI_STYLESETFORE | SCI_STYLESETBACK | SCI_STYLESETBOLD | SCI_STYLESETITALIC => {
            state.style_calls.push((message, wparam, lparam));
            0
        }
        SCI_STYLESETFONT => {
            let bytes = unsafe { std::ffi::CStr::from_ptr(lparam as *const std::ffi::c_char) }
                .to_bytes()
                .to_vec();
            state.font_calls.push((wparam, bytes));
            0
        }
        _ => state.responses.pop_front().unwrap_or(0),
    }
}

#[test]
fn container_styling_runs_land_on_the_right_bytes() {
    let editor = test_editor();
    editor.set_text("ab**cd**").unwrap();
    editor.set_lexer(0).unwrap();
    editor
        .apply_styling(0, &[(2, 0), (2, 1), (2, 3), (2, 1)])
        .unwrap();
    let styles: Vec<u8> = (0..8).map(|at| editor.style_at(at).unwrap()).collect();
    assert_eq!(styles, [0, 0, 1, 1, 3, 3, 1, 1]);
    assert_eq!(editor.end_styled().unwrap(), 8);
}

#[test]
fn annotation_lines_reserve_and_clear() {
    let editor = test_editor();
    editor.set_text("# Title\nbody\n").unwrap();
    editor.show_annotations(true).unwrap();
    editor.set_annotation_lines(0, 2, 40).unwrap();
    assert_eq!(editor.annotation_lines(0).unwrap(), 2);
    editor.set_annotation_lines(0, 0, 40).unwrap();
    assert_eq!(editor.annotation_lines(0).unwrap(), 0);
}

#[test]
fn geometry_round_trips_a_position() {
    let editor = test_editor();
    editor.set_text("hello\nworld\n").unwrap();
    let start = editor.line_start(1).unwrap();
    let (x, y) = editor.point_of(start).unwrap();
    assert_eq!(editor.position_at(x + 1, y + 1).unwrap(), start);
    assert!(editor.text_height().unwrap() > 0);
}

#[test]
fn a_hook_that_takes_enter_also_swallows_its_char() {
    use crate::editor::EditorHooks;
    use std::cell::Cell;
    use std::rc::Rc;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_CHAR, WM_KEYDOWN};
    #[derive(Debug)]
    struct TakeEnter(Cell<u32>);
    impl EditorHooks for TakeEnter {
        fn key_down(&self, vk: u16, _: bool, _: bool, _: bool) -> bool {
            self.0.set(self.0.get() + 1);
            vk == VK_RETURN
        }
    }
    let editor = test_editor();
    editor.set_text("a").unwrap();
    editor.set_selection(1..1).unwrap();
    let hook = Rc::new(TakeEnter(Cell::new(0)));
    editor.set_hooks(Some(hook.clone()));
    unsafe {
        SendMessageW(editor.hwnd(), WM_KEYDOWN, usize::from(VK_RETURN), 0);
        // What TranslateMessage would post after the key-down.
        SendMessageW(editor.hwnd(), WM_CHAR, 0x0D, 0);
    }
    assert_eq!(hook.0.get(), 1);
    assert_eq!(editor.text().unwrap(), "a");
    editor.set_hooks(None);
    unsafe {
        SendMessageW(editor.hwnd(), WM_KEYDOWN, usize::from(VK_RETURN), 0);
    }
    assert_ne!(
        editor.text().unwrap(),
        "a",
        "without a hook Enter reaches Scintilla"
    );
}
