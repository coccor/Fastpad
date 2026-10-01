//! Language activation, line numbers, the JSON commands, and crash-recovery snapshots.

use super::*;

#[test]
fn failed_language_activation_leaves_document_language_unchanged_and_records_a_warning() {
    // Break caught: a failed Lexilla load/lexer-creation must not record the requested
    // language on Document metadata when the editor itself was left exactly as it was
    // (LanguageManager::apply never installs a lexer before a real pointer is in hand), and
    // must surface something the caller can show in a notification instead of failing silently.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    unsafe {
        super::super::initialize_editor_with(window.hwnd, &identity, crate::editor::Editor::create)
    }
    .unwrap();

    // Seed a LanguageManager pointed at a Lexilla.dll path that cannot possibly load, so the
    // activation below fails deterministically without depending on the real native DLL.
    let missing = std::env::temp_dir().join(format!(
        "fastpad-main-window-missing-lexilla-test-{}.dll",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&missing);
    unsafe {
        super::super::app_ptr(window.hwnd)
            .unwrap()
            .as_mut()
            .language_manager = Some(LanguageManager::with_dll_path_for_test(missing));
    }
    let language_before = unsafe { super::super::app_ptr(window.hwnd).unwrap().as_ref() }
        .tabs
        .active()
        .unwrap()
        .language;
    assert_eq!(language_before, Language::PlainText);

    execute_command(window.hwnd, CommandId::LanguageJson);

    assert_eq!(
        notices(window.hwnd),
        vec![
            "FastPad could not enable syntax highlighting for this file. It will remain in \
             plain text."
                .to_owned()
        ]
    );
    let language_after = unsafe { super::super::app_ptr(window.hwnd).unwrap().as_ref() }
        .tabs
        .active()
        .unwrap()
        .language;
    assert_eq!(language_after, Language::PlainText);
}

#[test]
fn an_svg_keeps_its_preview_when_highlighting_cannot_load() {
    // Break caught: SVG moved from the null lexer to Lexilla's xml lexer, so a missing
    // Lexilla.dll left the tab as plain text and hid the SVG preview it never needed Lexilla
    // for.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    unsafe {
        super::super::initialize_editor_with(window.hwnd, &identity, crate::editor::Editor::create)
    }
    .unwrap();
    let missing = std::env::temp_dir().join(format!(
        "fastpad-main-window-missing-lexilla-svg-test-{}.dll",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&missing);
    unsafe {
        super::super::app_ptr(window.hwnd)
            .unwrap()
            .as_mut()
            .language_manager = Some(LanguageManager::with_dll_path_for_test(missing));
    }

    execute_command(window.hwnd, CommandId::LanguageSvg);

    assert_eq!(notices(window.hwnd).len(), 1);
    let language_after = unsafe { super::super::app_ptr(window.hwnd).unwrap().as_ref() }
        .tabs
        .active()
        .unwrap()
        .language;
    assert_eq!(language_after, Language::Svg);
    assert!(crate::window::preview_host::buttons_visible(window.hwnd));
}

#[test]
fn corrupt_settings_are_reported_on_the_bottom_bar_that_chrome_reserves() {
    // Break caught: nothing else asserts that invalid fastpad.ini lines actually reach the
    // user. If load_settings stopped queuing warnings, the painted bottom bar stopped showing
    // them, layout stopped reserving room for it, or dismissing a notice collapsed the bar,
    // every other test would still pass.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    unsafe {
        super::super::initialize_editor_with(window.hwnd, &identity, crate::editor::Editor::create)
    }
    .unwrap();
    let editor_hwnd = unsafe { super::super::app_ptr(window.hwnd).unwrap().as_ref() }
        .editor()
        .unwrap()
        .hwnd();

    // No bottom bar exists before WM_FASTPAD_BUILD_CHROME, regardless of pending warnings.
    assert_eq!(super::super::current_status_text(window.hwnd), None);
    assert_eq!(super::super::current_status_bar(window.hwnd), None);

    let warnings = vec![
        crate::config::SettingWarning {
            line: 3,
            message: "invalid value for tab_width: \"nope\"".to_owned(),
        },
        crate::config::SettingWarning {
            line: 5,
            message: "unknown setting key: bogus".to_owned(),
        },
    ];
    super::super::apply_loaded_settings(window.hwnd, crate::config::default_settings(), warnings);

    assert_eq!(
        unsafe { super::super::app_ptr(window.hwnd).unwrap().as_ref() }
            .notifications
            .len(),
        2
    );
    assert_eq!(
        super::super::current_status_text(window.hwnd),
        None,
        "chrome has not been built yet, so there is still nowhere to paint the warning"
    );

    super::super::build_chrome(window.hwnd);

    let status = super::super::current_status_text(window.hwnd);
    assert!(
        status
            .as_deref()
            .is_some_and(|text| text.contains("fastpad.ini line 3")),
        "expected the first warning's line reference in {status:?}"
    );
    let mut client = RECT::default();
    let mut shown = RECT::default();
    unsafe {
        GetClientRect(window.hwnd, &mut client);
        GetClientRect(editor_hwnd, &mut shown);
    }
    let dpi = unsafe { GetDpiForWindow(window.hwnd) };
    // The editor group's tab strip is the title row, so the editor starts right below it.
    let title_height = super::super::title_layout(window.hwnd).height;
    assert_eq!(
        (client.bottom - client.top) - (shown.bottom - shown.top),
        title_height + crate::window::status::status_height(dpi),
        "the editor should leave exactly the bottom bar's height below it"
    );

    super::super::dismiss_notifications(window.hwnd);

    assert_eq!(super::super::current_status_text(window.hwnd), None);
    let bar = super::super::current_status_bar(window.hwnd).unwrap();
    assert_eq!(bar.left, "Ln 1, Col 1");
    assert_eq!(bar.right, "Plain text    UTF-8");
    let mut dismissed = RECT::default();
    unsafe {
        GetClientRect(editor_hwnd, &mut dismissed);
    }
    assert_eq!(
        dismissed.bottom - dismissed.top,
        shown.bottom - shown.top,
        "the bottom bar stays after its notices are dismissed"
    );
}

#[test]
fn line_numbers_follow_edits_and_the_line_numbers_setting() {
    // Break caught: typing or pasting past line 99 without re-sizing the gutter clips the
    // numbers, and line_numbers=false in fastpad.ini leaving the gutter visible.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let two_digits = line_number_margin_width(&editor);
    assert!(two_digits > 0, "line numbers are shown by default");

    editor.replace_target(0..0, &"\n".repeat(150)).unwrap();
    let three_digits = line_number_margin_width(&editor);
    assert!(
        three_digits > two_digits,
        "151 lines need a wider gutter than {two_digits}px, got {three_digits}px"
    );

    editor.undo().unwrap();
    assert_eq!(line_number_margin_width(&editor), two_digits);

    let mut settings = crate::config::default_settings();
    settings.line_numbers = false;
    super::super::apply_loaded_settings(window.hwnd, settings, Vec::new());
    assert_eq!(line_number_margin_width(&editor), 0);
}

#[test]
fn format_json_command_reformats_with_two_spaces_in_one_undo_step() {
    // Break caught: Format JSON not actually rewriting the buffer, or splitting the rewrite
    // into more than one undo action (which would force repeated Ctrl+Z to fully undo it).
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.populate_clean("{\"a\":[1,2]}").unwrap();

    execute_command(window.hwnd, CommandId::FormatJson);

    assert_eq!(
        editor.text().unwrap(),
        "{\n  \"a\": [\n    1,\n    2\n  ]\n}"
    );
    assert!(notices(window.hwnd).is_empty());
    assert!(editor.can_undo().unwrap());
    editor.undo().unwrap();
    assert_eq!(editor.text().unwrap(), "{\"a\":[1,2]}");
    assert!(!editor.can_undo().unwrap());
}

#[test]
fn format_json_command_on_invalid_json_leaves_bytes_unchanged_and_reports_an_issue() {
    // Break caught: Format JSON starting an undo action or mutating the buffer before
    // discovering the source does not parse, and/or swallowing the failure instead of
    // surfacing it.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.populate_clean("{ bad").unwrap();

    execute_command(window.hwnd, CommandId::FormatJson);

    assert_eq!(editor.text().unwrap(), "{ bad");
    assert!(!editor.can_undo().unwrap());
    let issues = notices(window.hwnd);
    assert_eq!(issues.len(), 1);
    assert!(issues[0].contains("line 1"), "{}", issues[0]);
}

#[test]
fn format_json_command_clamps_the_restored_selection_to_the_new_shorter_length() {
    // Break caught: restoring the pre-format selection verbatim after formatting shrank the
    // document, leaving an out-of-range SCI_SETSEL instead of a clamped one.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let padding = " ".repeat(50);
    let source = format!("{{{padding}\"a\":1}}");
    editor.populate_clean(&source).unwrap();
    editor.set_selection(55..58).unwrap();

    execute_command(window.hwnd, CommandId::FormatJson);

    let formatted_len = editor.text().unwrap().len();
    assert!(formatted_len < 55, "expected formatting to shrink the text");
    assert_eq!(editor.selection().unwrap(), formatted_len..formatted_len);
}

#[test]
fn format_json_command_snaps_the_restored_selection_to_a_utf8_char_boundary() {
    // Break caught: reusing a pre-format byte offset verbatim (once only clamped to the new
    // length) against the post-format text can land mid-character, since it has no guaranteed
    // relationship to character boundaries in the reformatted bytes. Here the caret sits at an
    // ordinary, valid boundary in the *compact* source (right after "héllo"'s closing quote);
    // at that exact raw byte offset, the *pretty-printed* text — which keeps "é"'s literal
    // two-byte UTF-8 encoding but reflows the surrounding whitespace — instead lands squarely
    // between "é"'s two bytes.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let source = "{\"a\":\"h\u{e9}llo\",\"b\":1}";
    let formatted = crate::languages::format_json(source).unwrap();

    let boundary_in_source = source.find("\",\"b\"").unwrap(); // right before the closing '"'
    assert!(source.is_char_boundary(boundary_in_source));

    let e_char_start = formatted.find('\u{e9}').unwrap();
    assert_eq!(
        boundary_in_source,
        e_char_start + 1,
        "test setup: expected the reused raw byte offset to land inside é's encoding"
    );
    assert!(!formatted.is_char_boundary(boundary_in_source));

    editor.populate_clean(source).unwrap();
    editor
        .set_selection(boundary_in_source..boundary_in_source)
        .unwrap();

    execute_command(window.hwnd, CommandId::FormatJson);

    assert_eq!(editor.text().unwrap(), formatted);
    let restored = editor.selection().unwrap();
    assert!(
        formatted.is_char_boundary(restored.start) && formatted.is_char_boundary(restored.end),
        "restored selection {restored:?} is not on a UTF-8 character boundary"
    );
    // Snapped backward to the boundary immediately before "é", not forward past it.
    assert_eq!(restored, e_char_start..e_char_start);
}

#[test]
fn validate_json_command_reports_success_and_never_mutates_the_document() {
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.populate_clean("{\"a\":1}").unwrap();

    execute_command(window.hwnd, CommandId::ValidateJson);

    let reported = notices(window.hwnd);
    assert_eq!(reported.len(), 1);
    assert!(reported[0].contains("valid JSON"), "{reported:?}");
    assert_eq!(editor.text().unwrap(), "{\"a\":1}");
    assert!(!editor.can_undo().unwrap());
}

#[test]
fn validate_json_command_reports_the_line_and_column_for_invalid_json() {
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.populate_clean("{\n  bad\n}").unwrap();

    execute_command(window.hwnd, CommandId::ValidateJson);

    let issues = notices(window.hwnd);
    assert_eq!(issues.len(), 1);
    assert!(
        issues[0].contains("line 2") && issues[0].contains("column 3"),
        "{}",
        issues[0]
    );
    assert_eq!(editor.text().unwrap(), "{\n  bad\n}");
}

#[test]
fn recovery_discovery_opens_foreign_snapshots_as_dirty_recovered_tabs_with_one_notice() {
    // Break caught: recovered text opened clean, untitled-looking, without a notice, or this
    // process's own live snapshots reopened as duplicates.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let root = RecoveryScratch::new("discover");
    let source = write_snapshot(
        root.path(),
        &Snapshot::new(
            RecoveryId::from_u128(0x77),
            Some(PathBuf::from(r"C:\docs\notes.md")),
            Encoding::Utf16Le,
            "recovered body",
        ),
    )
    .unwrap();
    let own_id = app_mut(window.hwnd).allocate_recovery_id();
    let own = write_snapshot(
        root.path(),
        &Snapshot::new(own_id, None, Encoding::Utf8, "live"),
    )
    .unwrap();
    std::fs::write(root.path().join("torn.fps"), b"FPS1").unwrap();
    app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());

    super::super::recover_snapshots(window.hwnd);

    let (tabs, title, dirty, path, encoding, origin, notices) = {
        let app = app_mut(window.hwnd);
        let active = app.tabs.active().unwrap();
        (
            app.tabs.len(),
            active.title(),
            active.dirty,
            active.path.clone(),
            active.encoding,
            active.recovery_origin.clone(),
            app.notifications.len(),
        )
    };
    assert_eq!(tabs, 2);
    assert_eq!(title, "Recovered: notes.md *");
    assert!(dirty);
    assert_eq!(path, None);
    assert_eq!(encoding, Encoding::Utf16Le);
    assert_eq!(origin.unwrap().snapshot_path, source);
    assert_eq!(notices, 1);
    assert_eq!(editor.text().unwrap(), "recovered body");
    assert_ne!(
        unsafe { SendMessageW(editor.hwnd(), SCI_GETMODIFY, 0, 0) },
        0,
        "Scintilla itself must treat the recovered text as unsaved"
    );
    assert!(source.exists() && own.exists());
    assert!(root.path().join("torn.fps.invalid").exists());
}

#[test]
fn snapshots_write_one_changed_dirty_document_per_tick_without_disturbing_the_view() {
    // Break caught: several documents written per tick, unchanged generations rewritten, or an
    // inactive tab's snapshot swapping the visible document or selection.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let root = RecoveryScratch::new("tick");
    app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
    editor.set_text("alpha").unwrap();
    super::super::create_new_document(window.hwnd).unwrap();
    editor.set_text("beta\nline").unwrap();
    editor.set_selection(2..3).unwrap();
    let ids = app_mut(window.hwnd)
        .tabs
        .documents()
        .map(|document| document.recovery_id)
        .collect::<Vec<_>>();
    let first = snapshot_path(root.path(), ids[0]);
    let second = snapshot_path(root.path(), ids[1]);

    super::super::snapshot_next_document(window.hwnd);

    assert_eq!(read_snapshot_text(&first), "alpha");
    assert!(!second.exists());
    assert_eq!(editor.text().unwrap(), "beta\nline");
    assert_eq!(editor.selection().unwrap(), 2..3);
    assert!(app_mut(window.hwnd).last_snapshot_duration.is_some());

    super::super::snapshot_next_document(window.hwnd);
    assert_eq!(read_snapshot_text(&second), "beta\nline");

    std::fs::remove_file(&first).unwrap();
    std::fs::remove_file(&second).unwrap();
    super::super::snapshot_next_document(window.hwnd);
    assert!(!first.exists() && !second.exists());
}

#[test]
fn saving_a_recovered_tab_never_touches_the_original_and_removes_its_snapshots() {
    // Break caught: Save silently writing the original path, or a saved recovered document
    // leaving snapshots that resurrect it after the next crash.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let root = RecoveryScratch::new("save");
    let original = root.path().join("original.txt");
    std::fs::write(&original, b"original").unwrap();
    let source = write_snapshot(
        root.path(),
        &Snapshot::new(
            RecoveryId::from_u128(0x99),
            Some(original.clone()),
            Encoding::Utf8,
            "recovered",
        ),
    )
    .unwrap();
    app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
    super::super::recover_snapshots(window.hwnd);
    editor.set_text("recovered and edited").unwrap();

    super::super::snapshot_next_document(window.hwnd);
    let own = snapshot_path(
        root.path(),
        app_mut(window.hwnd).tabs.active().unwrap().recovery_id,
    );
    assert_eq!(read_snapshot_text(&own), "recovered and edited");
    assert!(
        !source.exists(),
        "the stale source would reopen as a duplicate"
    );

    let target = root.path().join("saved.txt");
    super::super::save_path_as(window.hwnd, &target);

    assert_eq!(std::fs::read(&target).unwrap(), b"recovered and edited");
    assert_eq!(std::fs::read(&original).unwrap(), b"original");
    assert!(!own.exists());
    let app = app_mut(window.hwnd);
    assert_eq!(app.tabs.active().unwrap().title(), "saved.txt");
    assert_eq!(app.tabs.active().unwrap().recovery_origin, None);
}

#[test]
fn clean_window_close_removes_this_sessions_snapshots() {
    // Break caught: snapshots outliving a clean exit and restoring tabs on the next launch.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let root = RecoveryScratch::new("close");
    app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
    editor.set_text("typed").unwrap();
    super::super::snapshot_next_document(window.hwnd);
    let own = snapshot_path(
        root.path(),
        app_mut(window.hwnd).tabs.active().unwrap().recovery_id,
    );
    assert!(own.exists());
    editor.set_save_point();

    unsafe {
        SendMessageW(window.hwnd, WM_CLOSE, 0, 0);
    }

    assert_eq!(unsafe { IsWindow(window.hwnd) }, 0);
    assert!(!own.exists());
}

#[test]
fn undoing_a_recovered_tab_keeps_it_dirty_and_its_source_through_clean_close_cleanup() {
    // Break caught: undo reaching Scintilla's empty save point marks the recovered tab clean,
    // so closing skips the prompt and deletes the only copy of its text.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let root = RecoveryScratch::new("undo");
    let source = write_snapshot(
        root.path(),
        &Snapshot::new(
            RecoveryId::from_u128(0x55),
            None,
            Encoding::Utf8,
            "only copy",
        ),
    )
    .unwrap();
    app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
    super::super::recover_snapshots(window.hwnd);

    while editor.can_undo().unwrap() {
        editor.undo().unwrap();
    }

    assert_eq!(editor.text().unwrap(), "");
    assert_eq!(
        unsafe { SendMessageW(editor.hwnd(), SCI_GETMODIFY, 0, 0) },
        0,
        "test setup: Scintilla reached its save point"
    );
    let app = app_mut(window.hwnd);
    let active = app.tabs.active().unwrap().id;
    assert!(app.tabs.active().unwrap().dirty);
    assert_eq!(
        app.tabs.next_dirty_review(&[]).map(|review| review.id),
        Some(active),
        "window close must still prompt for the recovered tab"
    );
    super::super::remove_session_snapshots(window.hwnd, &[]);
    assert!(source.exists());
}

#[test]
fn discovery_skips_snapshots_whose_owner_process_is_still_running() {
    // Break caught: a second instance opening, quarantining, or later deleting a live
    // instance's snapshots.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let root = RecoveryScratch::new("live-owner");
    let process_start = 0x5EED_0000_0000_0000 | u64::from(std::process::id());
    let live = RecoveryId::compose(process_start, 4_000_000_001, 1);
    let torn = snapshot_path(
        root.path(),
        RecoveryId::compose(process_start, 4_000_000_001, 2),
    );
    let owner = crate::recovery::create_owner_mutex(live).unwrap();
    let valid = write_snapshot(
        root.path(),
        &Snapshot::new(live, None, Encoding::Utf8, "live elsewhere"),
    )
    .unwrap();
    std::fs::write(&torn, b"FPS1").unwrap();
    app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());

    super::super::recover_snapshots(window.hwnd);

    assert_eq!(app_mut(window.hwnd).tabs.len(), 1);
    assert!(valid.exists() && torn.exists());

    drop(owner);
    super::super::recover_snapshots(window.hwnd);

    assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
    assert!(valid.exists());
    assert!(
        !torn.exists(),
        "a dead owner's torn snapshot is quarantined"
    );
}
