# Themed prompts and sentence-case copy: design

- Status: approved in conversation on 2026-10-01. Written spec awaiting review.
- Branch: `feat/dialogs-copy`, stacked on `feat/selection-accent` (PR #50), which is stacked on #49, #48, #47, #46, #45 and #44.
- Last step of the Windows design alignment. Two parts: the prompts (part A) and the copy (part B). The Windows colors link from the original step list is dropped.

## 1. Goal

- **Part A:** the save, replace, copy-replace and delete prompts open in the app's own colors, shape and fonts instead of the light system `MessageBoxW`, with buttons named for the action.
- **Part B:** user-facing labels follow Windows 11 sentence case.
- **A build you can run** at the end, so the prompts are judged by eye in Light, Dark, Latte and high contrast.

## 2. Findings that set the scope

- `window/modal.rs` owns two `MessageBoxW` prompts: `prompt_close_decision(hwnd, title)` (Yes / No / Cancel, "Save changes to {title} before closing?") and `confirm(hwnd, text) -> bool` (OK / Cancel). Both enter a `ModalScope` and have test hooks (`answer_next_close_prompt`, `answer_next_confirm`, `take_last_confirm`).
- `confirm` has these callers: Search replace (`text_search_host.rs`, plan text from `confirm_text` and `row_confirm_text`), and, through `library_host::confirmed` (`library_host/pins.rs`), the tree-copy replace (`copy_host.rs`, `tree_copy::replace_question`), note delete (`library_host/note_actions.rs`) and folder delete (`library_host/folders.rs`, `delete_folder_question`).
- `main.rs` shows one `MessageBoxW` for startup-fatal errors, before Direct2D or any chrome exists. It stays a system dialog.
- `about.rs` is the pattern: an owned `WS_POPUP | WS_CAPTION` window with its own modal loop, `DwmSetWindowAttribute` rounded corners, DWM shadow through a 1px frame margin, a `Canvas` (Direct2D with a GDI fallback) and `type_ramp` fonts, with a test hook (`answer_next`). The Settings dialog supplies the accent primary-button look (`soft_paint`, `settings_dialog/painting.rs`).
- Title-case user-facing strings found by grep (non-test source): `menus.rs` "Split Right", "Split Down"; `settings_model.rs` "Notes and Session", "Keyboard Shortcuts"; `shortcuts_input.rs` "Reset Keybinding"; `accessibility.rs` and `preview_host.rs` "Open Preview", "Open Preview to the Side"; `command_palette.rs` "Close Markdown Preview"; `languages/registry.rs` "Plain Text". Not changed because they are proper nouns or product names: "Reveal in Explorer", "Catppuccin ...", font names, "FastPad".

## 3. Decisions

| Question | Decision |
|---|---|
| Window | A new `window/prompt.rs`, shaped like About's popup, with its own class and modal loop. About is not refactored; the shell is repeated rather than shared, to leave About untouched. |
| Entry points | `prompt_close_decision(hwnd, title)` keeps its signature. `confirm(hwnd, text)` becomes `confirm(hwnd, text, action)`, and `library_host::confirmed` gains the same `action` parameter. |
| Buttons | Close prompt: **Save** (primary, default), **Don't save**, **Cancel**. Confirm: `{action}` (primary, default) and **Cancel**. Callers pass **Replace** (Search replace, tree-copy replace) and **Delete** (note delete, folder delete). |
| Message text | Unchanged except the close prompt: "Save changes to {title}?" (the old "before closing" is dropped, since the buttons say what each does). |
| Title line | "FastPad", as the system prompts show today. |
| Fonts | `type_ramp` styles: Title for the title line, Body for the message and buttons, so the Windows 11 faces and the text-size setting apply. |
| Size | 400px wide at 96 DPI. Height follows the wrapped message, measured with the real fonts, floored at a minimum so the button row always fits. |
| Keys | Enter presses the focused button, Esc is Cancel, Tab and Shift+Tab move focus, and the initial focus is the primary button. For the close prompt, `S` is Save and `D` is Don't save while the dialog is up (no accelerator letters shown). |
| High contrast | Square corners, system colors only, the primary button filled with the highlight pair, and the focus ring on the focused button. |
| Test hooks | The existing `answer_next_*` hooks keep working: in tests the hook answers instead of the window opening. `take_last_confirm` also records the action label. |
| Not in this piece | The startup-fatal dialog, the About and Settings dialogs, message text rewrites, the Windows colors link, and shared-shell refactoring. |

## 4. Design

### 4.1 Layout (`prompt.rs`)

- A pure `Layout::calculate(dpi, title_height, body_lines_height, button_count, label_widths)` giving the client rect, title rect, message rect, button rects (right-aligned, 88px wide at 96 DPI minimum or the label's width plus padding, 30px tall scaled with the text size via `scale_text`) and the total height. Padding 20px, gaps 16px at 96 DPI, as About uses.
- The message wraps to the content width: the real `DrawTextW(DT_CALCRECT | DT_WORDBREAK)` measure, like About's `measure`, on the body font.

### 4.2 Window and modal loop

- Registers its class once (lazily, at the first prompt), creates the popup over `owner`, centers it over the owner, rounds corners with `DWMWCP_ROUND`, extends the frame by 1px for the shadow, runs a `GetMessageW` loop until the window is destroyed, re-posts `WM_QUIT` like About, and re-enables the owner before destroying.
- `show(owner, colors, spec) -> Choice` where `spec` has the message, the button labels and which is primary, and `Choice` is the index of the pressed button (Cancel is the last button, and Esc, the title close and a closing owner give it).
- `modal.rs`'s production branches map `Choice` to `CloseDecision` and to `bool`. The `ModalScope` stays where it is.

### 4.3 Painting

- Direct2D through `Canvas` with the GDI fallback, as About: the window background `editor_background`, a 1px `stroke` border, the title in `editor_foreground`, the message in `muted_foreground` or `editor_foreground` as About's body text, the primary button `accent` with `on_accent` text, secondary buttons `hover_background` on hover and `stroke` outlined at rest, the focus ring `accent` 2px with `FOCUS_GAP`.

### 4.4 Copy pass (part B)

Sentence case changes: "Split Right" -> "Split right", "Split Down" -> "Split down", "Notes and Session" -> "Notes and session", "Keyboard Shortcuts" -> "Keyboard shortcuts", "Reset Keybinding" -> "Reset keybinding", "Open Preview" -> "Open preview", "Open Preview to the Side" -> "Open preview to the side", "Close Markdown Preview" -> "Close Markdown preview", language name "Plain Text" -> "Plain text". Every test that pins an old string changes with it; the implementation plan lists each occurrence found by grep, including accessible names and comments that quote the string.

## 5. Tests

- `Layout::calculate`: at 96, 120, 144 and 192 DPI and at text sizes 100 and 225, the message, the button row and the padding fit inside the client rect, and the height grows with a longer message.
- Pixel tests on a memory bitmap through the GDI fallback: the primary button is `accent` at its middle, a secondary button is not, the rounded corners differ from the background, and in high contrast the primary button is the highlight color and the corners are square.
- Key handling (pure): Enter on the primary button chooses it, Esc is Cancel, Tab and Shift+Tab cycle, `S` and `D` map to Save and Don't save only for the close prompt.
- `modal.rs`: the existing close-prompt and confirm tests keep passing through the hooks; `take_last_confirm` also returns the action label and the call sites' labels are asserted (Replace for Search and tree-copy replace, Delete for note and folder delete).
- Copy: the changed labels, in menus, settings, the shortcuts reset item, the preview buttons and their accessible names, the palette entry and the language name.

## 6. Risks and checks

- **Nested modal loop.** The prompt runs its own loop, like About and `MessageBoxW`. Callers already ask with nothing of the `App` borrowed (see `text_search_host.rs`), and `ModalScope` holds the messages the main window must not process meanwhile. The build is checked by opening a prompt over a dirty tab and closing the window.
- **Focus and activation on return.** The owner is re-enabled before the popup is destroyed, so activation returns to the main window (About's order).
- **Text-size and DPI.** The height comes from measuring, so a long message grows the box instead of clipping; the layout tests pin the fit.
- **Startup.** Nothing loads at startup: the class is registered and Direct2D loaded at the first prompt, as About does.
- **Verification:** Clippy, the targeted tests for the changed modules, the full suite once, then the build. Before the build, `fastpad.ini` is backed up.

## 7. After this piece

The design alignment's step list is complete. Parked: the five known-short contrast pairs, high-contrast focus visibility on the active editor row, the optional Windows accent color, accent on tab and activity-bar indicators, the Settings dialog's `selection_background`-colored pills, and the Windows colors link.
