//! The Keyboard Shortcuts page's rows and what each input does to them (keyboard shortcuts spec
//! §6.2–§6.6). Pure: no window handles.

use crate::window::commands::CommandId;
use crate::window::keymap::{COMMAND_IDS, KeyStroke, Keymap, bindable};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_NEXT, VK_PRIOR, VK_RETURN, VK_UP,
};

/// Names for the bound commands the palette doesn't list (spec §2).
const EXTRA_TITLES: [(CommandId, &str); 22] = [
    (CommandId::SelectTab1, "View: Select tab 1"),
    (CommandId::SelectTab2, "View: Select tab 2"),
    (CommandId::SelectTab3, "View: Select tab 3"),
    (CommandId::SelectTab4, "View: Select tab 4"),
    (CommandId::SelectTab5, "View: Select tab 5"),
    (CommandId::SelectTab6, "View: Select tab 6"),
    (CommandId::SelectTab7, "View: Select tab 7"),
    (CommandId::SelectTab8, "View: Select tab 8"),
    (CommandId::SelectTab9, "View: Select tab 9"),
    (CommandId::CommandPalette, "View: Show command palette"),
    (CommandId::MarkdownPreviewCycle, "Markdown preview: Cycle"),
    (CommandId::FocusNextPane, "View: Focus next pane"),
    (CommandId::FocusPreviousPane, "View: Focus previous pane"),
    (CommandId::FocusGroup1, "View: Focus editor group 1"),
    (CommandId::FocusGroup2, "View: Focus editor group 2"),
    (CommandId::FocusGroup3, "View: Focus editor group 3"),
    (CommandId::FocusGroup4, "View: Focus editor group 4"),
    (CommandId::FocusGroup5, "View: Focus editor group 5"),
    (CommandId::FocusGroup6, "View: Focus editor group 6"),
    (CommandId::FocusGroup7, "View: Focus editor group 7"),
    (CommandId::FocusGroup8, "View: Focus editor group 8"),
    (CommandId::FocusLastGroup, "View: Focus last editor group"),
];

/// Names for the Markdown-scoped commands: their keys work only in Markdown files, so the page
/// says so (live mode spec §9).
const MARKDOWN_SCOPED_TITLES: [(CommandId, &str); 4] = [
    (CommandId::MarkdownBold, "Markdown: Bold (Markdown files)"),
    (
        CommandId::MarkdownItalic,
        "Markdown: Italic (Markdown files)",
    ),
    (
        CommandId::MarkdownCode,
        "Markdown: Inline code (Markdown files)",
    ),
    (CommandId::MarkdownLink, "Markdown: Link (Markdown files)"),
];

/// What the page calls `command`: its Markdown-scoped name, its palette label, or its name from
/// `EXTRA_TITLES`.
pub(crate) fn title(command: CommandId) -> Option<&'static str> {
    MARKDOWN_SCOPED_TITLES
        .iter()
        .find(|(candidate, _)| *candidate == command)
        .map(|(_, title)| *title)
        .or_else(|| {
            crate::window::command_palette::ENTRIES
                .iter()
                .find(|entry| entry.command == command)
                .map(|entry| entry.label)
        })
        .or_else(|| {
            EXTRA_TITLES
                .iter()
                .find(|(candidate, _)| *candidate == command)
                .map(|(_, title)| *title)
        })
}

/// One row: one key of a command, or a command with none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShortcutRow {
    pub command: CommandId,
    pub title: &'static str,
    pub id: &'static str,
    pub stroke: Option<KeyStroke>,
    /// Whether the command's keys are the user's.
    pub user: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Filter {
    Text(String),
    Key(KeyStroke),
}

fn matches(row: &ShortcutRow, filter: &Filter) -> bool {
    match filter {
        Filter::Key(stroke) => row.stroke == Some(*stroke),
        Filter::Text(text) => {
            let compact = |text: &str| {
                text.chars()
                    .filter(|c| !c.is_whitespace())
                    .flat_map(char::to_lowercase)
                    .collect::<String>()
            };
            let needle = text.trim().to_lowercase();
            needle.is_empty()
                || row.title.to_lowercase().contains(&needle)
                || row.id.to_lowercase().contains(&needle)
                || row
                    .stroke
                    .is_some_and(|stroke| compact(&stroke.text()).contains(&compact(&needle)))
        }
    }
}

/// Every row `filter` keeps: commands sorted by title, each command's keys in binding order.
pub(crate) fn rows(keymap: &Keymap, filter: &Filter) -> Vec<ShortcutRow> {
    let mut commands = COMMAND_IDS
        .iter()
        .filter_map(|&(command, id)| Some((command, title(command)?, id)))
        .collect::<Vec<_>>();
    commands.sort_by_key(|(_, title, _)| title.to_lowercase());
    let mut rows = Vec::new();
    for (command, title, id) in commands {
        let user = keymap.is_user(command);
        let keys = keymap.keys_of(command);
        let strokes = if keys.is_empty() {
            vec![None]
        } else {
            keys.into_iter().map(Some).collect()
        };
        rows.extend(
            strokes
                .into_iter()
                .map(|stroke| ShortcutRow {
                    command,
                    title,
                    id,
                    stroke,
                    user,
                })
                .filter(|row| matches(row, filter)),
        );
    }
    rows
}

/// The recording box: the command being given a key, the key it replaces (none when adding),
/// and what was pressed so far.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Recording {
    pub command: CommandId,
    pub replace: Option<KeyStroke>,
    pub stroke: Option<KeyStroke>,
    pub refusal: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ShortcutsEffect {
    None,
    Repaint,
    /// Give the command exactly these keys; the dialog applies and saves them.
    SetKeys(CommandId, Vec<KeyStroke>),
    Reset(CommandId),
    CopyId(&'static str),
    FocusSearch,
    FocusTable,
    /// Put this text in the search field.
    SetSearchText(String),
    Close,
}

const fn plain(vk: u16) -> KeyStroke {
    KeyStroke::new(false, false, false, vk)
}

#[derive(Clone, Debug)]
pub(crate) struct ShortcutsModel {
    pub keymap: Keymap,
    pub filter: Filter,
    pub rows: Vec<ShortcutRow>,
    pub selected: usize,
    /// The first row shown.
    pub top: usize,
    /// How many rows the table shows.
    pub visible: usize,
    pub recording: Option<Recording>,
    /// Whether the search field records keys instead of taking text.
    pub record_keys: bool,
    /// Commands with a `key.<id>=` line in the settings, even one the keymap ignored
    /// (`key.file.save=Bogus`): Reset removes the line, so it is offered for them too.
    pub lines: Vec<CommandId>,
    /// The key the last confirmed recording gave its command: the next `refresh` selects its
    /// row.
    changed: Option<(CommandId, KeyStroke)>,
}

impl ShortcutsModel {
    pub(crate) fn new(keymap: Keymap, visible: usize) -> Self {
        let filter = Filter::Text(String::new());
        let rows = rows(&keymap, &filter);
        Self {
            keymap,
            filter,
            rows,
            selected: 0,
            top: 0,
            visible: visible.max(1),
            recording: None,
            record_keys: false,
            lines: Vec::new(),
            changed: None,
        }
    }

    pub(crate) fn selected_row(&self) -> Option<&ShortcutRow> {
        self.rows.get(self.selected)
    }

    /// Re-reads the rows from `keymap`, and which commands have a `key.<id>=` line from
    /// `lines`. The selection moves to the key a recording just confirmed when the filter shows
    /// it; otherwise it keeps the selected command (on the same key when it still has it).
    pub(crate) fn refresh(&mut self, keymap: Keymap, lines: Vec<CommandId>) {
        let changed = self.changed.take();
        let kept = self.selected_row().map(|row| (row.command, row.stroke));
        self.keymap = keymap;
        self.lines = lines;
        self.rows = rows(&self.keymap, &self.filter);
        let row_of = |command: CommandId, stroke: Option<KeyStroke>| {
            self.rows
                .iter()
                .position(|row| row.command == command && row.stroke == stroke)
        };
        let new = changed.and_then(|(command, stroke)| row_of(command, Some(stroke)));
        if let Some(index) = new {
            self.selected = index;
        } else if let Some((command, stroke)) = kept {
            let index = row_of(command, stroke)
                .or_else(|| self.rows.iter().position(|row| row.command == command));
            if let Some(index) = index {
                self.selected = index;
            }
        }
        self.clamp();
    }

    fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        self.rows = rows(&self.keymap, &self.filter);
        self.selected = 0;
        self.top = 0;
    }

    pub(crate) fn set_text(&mut self, text: &str) -> ShortcutsEffect {
        self.set_filter(Filter::Text(text.to_owned()));
        ShortcutsEffect::Repaint
    }

    pub(crate) fn record_search_key(&mut self, stroke: KeyStroke) -> ShortcutsEffect {
        self.set_filter(Filter::Key(stroke));
        ShortcutsEffect::SetSearchText(stroke.text())
    }

    pub(crate) fn toggle_record_keys(&mut self) -> ShortcutsEffect {
        self.record_keys = !self.record_keys;
        self.set_filter(Filter::Text(String::new()));
        ShortcutsEffect::SetSearchText(String::new())
    }

    fn clamp(&mut self) {
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        let max_top = self.rows.len().saturating_sub(self.visible);
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + self.visible {
            self.top = self.selected + 1 - self.visible;
        }
        self.top = self.top.min(max_top);
    }

    pub(crate) fn select(&mut self, index: usize) {
        self.selected = index;
        self.clamp();
    }

    /// The table now shows `visible` rows (the dialog was resized): the selection stays in view
    /// and a taller table fills from the top rather than showing space under the last row.
    pub(crate) fn set_visible(&mut self, visible: usize) {
        self.visible = visible.max(1);
        self.clamp();
    }

    /// Scrolls by `rows` (negative: up) without moving the selection.
    pub(crate) fn scroll(&mut self, rows: isize) {
        let max_top = self.rows.len().saturating_sub(self.visible);
        self.top = self.top.saturating_add_signed(rows).min(max_top);
    }

    pub(crate) fn table_key(&mut self, stroke: KeyStroke) -> ShortcutsEffect {
        let page = self.visible.saturating_sub(1).max(1);
        let last = self.rows.len().saturating_sub(1);
        match stroke {
            s if s == plain(VK_UP) && self.selected == 0 => ShortcutsEffect::FocusSearch,
            s if s == plain(VK_UP) => self.move_to(self.selected - 1),
            s if s == plain(VK_DOWN) => self.move_to((self.selected + 1).min(last)),
            s if s == plain(VK_PRIOR) => self.move_to(self.selected.saturating_sub(page)),
            s if s == plain(VK_NEXT) => self.move_to((self.selected + page).min(last)),
            s if s == plain(VK_HOME) => self.move_to(0),
            s if s == plain(VK_END) => self.move_to(last),
            s if s == plain(VK_RETURN) => self.start_change(),
            s if s == KeyStroke::new(true, false, false, VK_RETURN) => self.start_add(),
            s if s == plain(VK_DELETE) => self.remove(),
            s if s == KeyStroke::new(true, false, false, u16::from(b'C')) => self.copy_id(),
            s if s == plain(VK_ESCAPE) => ShortcutsEffect::Close,
            _ => ShortcutsEffect::None,
        }
    }

    fn move_to(&mut self, index: usize) -> ShortcutsEffect {
        self.select(index);
        ShortcutsEffect::Repaint
    }

    fn start(&mut self, replace: bool) -> ShortcutsEffect {
        let Some(row) = self.selected_row() else {
            return ShortcutsEffect::None;
        };
        self.recording = Some(Recording {
            command: row.command,
            replace: if replace { row.stroke } else { None },
            stroke: None,
            refusal: None,
        });
        ShortcutsEffect::Repaint
    }

    pub(crate) fn start_change(&mut self) -> ShortcutsEffect {
        self.start(true)
    }

    pub(crate) fn start_add(&mut self) -> ShortcutsEffect {
        self.start(false)
    }

    pub(crate) fn cancel_recording(&mut self) -> ShortcutsEffect {
        if self.recording.take().is_some() {
            ShortcutsEffect::Repaint
        } else {
            ShortcutsEffect::None
        }
    }

    /// A key pressed while the recording box is open. Plain Enter confirms and plain Escape
    /// cancels; anything else is the key being recorded.
    pub(crate) fn record_key(&mut self, stroke: KeyStroke) -> ShortcutsEffect {
        if stroke == plain(VK_ESCAPE) {
            return self.cancel_recording();
        }
        let Some(recording) = self.recording.as_mut() else {
            return ShortcutsEffect::None;
        };
        if stroke != plain(VK_RETURN) {
            recording.stroke = Some(stroke);
            recording.refusal = bindable(stroke).err();
            return ShortcutsEffect::Repaint;
        }
        let (Some(new), None) = (recording.stroke, recording.refusal) else {
            return ShortcutsEffect::None;
        };
        let recording = self.recording.take().expect("checked above");
        let current = self.keymap.keys_of(recording.command);
        let mut keys = current.clone();
        match recording
            .replace
            .and_then(|old| keys.iter().position(|key| *key == old))
        {
            Some(index) => keys[index] = new,
            None => keys.push(new),
        }
        let mut unique = Vec::with_capacity(keys.len());
        for key in keys {
            if !unique.contains(&key) {
                unique.push(key);
            }
        }
        if unique == current {
            ShortcutsEffect::Repaint
        } else {
            self.changed = Some((recording.command, new));
            ShortcutsEffect::SetKeys(recording.command, unique)
        }
    }

    /// How many other commands already use the recorded key.
    pub(crate) fn conflict_count(&self) -> usize {
        self.recording
            .as_ref()
            .and_then(|recording| {
                Some(
                    self.keymap
                        .conflicts(recording.stroke?, recording.command)
                        .len(),
                )
            })
            .unwrap_or(0)
    }

    /// The "N existing commands" link: closes the box and searches for the recorded key.
    pub(crate) fn follow_conflicts(&mut self) -> ShortcutsEffect {
        let Some(stroke) = self.recording.take().and_then(|recording| recording.stroke) else {
            return ShortcutsEffect::None;
        };
        self.record_keys = true;
        self.record_search_key(stroke)
    }

    pub(crate) fn remove(&mut self) -> ShortcutsEffect {
        let Some(row) = self.selected_row() else {
            return ShortcutsEffect::None;
        };
        let Some(stroke) = row.stroke else {
            return ShortcutsEffect::None;
        };
        let keys = self
            .keymap
            .keys_of(row.command)
            .into_iter()
            .filter(|key| *key != stroke)
            .collect();
        ShortcutsEffect::SetKeys(row.command, keys)
    }

    /// Whether the selected row's command can be reset: its keys are the user's, or it has a
    /// `key.<id>=` line the keymap ignored.
    pub(crate) fn can_reset(&self) -> bool {
        self.selected_row()
            .is_some_and(|row| row.user || self.lines.contains(&row.command))
    }

    pub(crate) fn reset(&mut self) -> ShortcutsEffect {
        match self.selected_row() {
            Some(row) if self.can_reset() => ShortcutsEffect::Reset(row.command),
            _ => ShortcutsEffect::None,
        }
    }

    pub(crate) fn copy_id(&mut self) -> ShortcutsEffect {
        self.selected_row()
            .map_or(ShortcutsEffect::None, |row| ShortcutsEffect::CopyId(row.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::keymap::{KeyStroke, Keymap, TYPING_REFUSAL};

    fn stroke(text: &str) -> KeyStroke {
        KeyStroke::parse(text).unwrap()
    }

    fn model(query: &str) -> ShortcutsModel {
        let mut model = ShortcutsModel::new(Keymap::defaults(), 10);
        model.set_text(query);
        model
    }

    #[test]
    fn every_command_has_a_title_and_a_row() {
        // Break caught: a command the page can't show (no title), so its keys can't be changed.
        let model = model("");
        for (command, _) in crate::window::keymap::COMMAND_IDS {
            assert!(title(*command).is_some(), "{command:?}");
            assert!(
                model.rows.iter().any(|row| row.command == *command),
                "{command:?}"
            );
        }
        assert_eq!(title(CommandId::SelectTab3), Some("View: Select tab 3"));
    }

    #[test]
    fn markdown_scoped_titles_say_where_their_keys_work() {
        // Break caught: Ctrl+B listed twice on the page with nothing telling the two rows apart.
        for (command, _) in crate::window::keymap::COMMAND_IDS {
            let scoped = command.scope() == crate::window::commands::Scope::Markdown;
            let suffixed =
                title(*command).is_some_and(|title| title.ends_with(" (Markdown files)"));
            assert_eq!(scoped, suffixed, "{command:?}");
        }
        assert_eq!(
            title(CommandId::MarkdownBold),
            Some("Markdown: Bold (Markdown files)")
        );
    }

    #[test]
    fn a_command_has_one_row_per_key_and_an_unbound_one_has_one_empty_row() {
        // Break caught: a multi-key command collapsing to one row, or an unbound command vanishing.
        let model = model("zoom in");
        let keys = model.rows.iter().map(|row| row.stroke).collect::<Vec<_>>();
        assert_eq!(
            keys,
            [
                Some(stroke("Ctrl+=")),
                Some(stroke("Ctrl+Shift+=")),
                Some(stroke("Ctrl+NumpadAdd"))
            ]
        );
        let about = model_rows_for("about");
        assert_eq!(about, [None]);
    }

    fn model_rows_for(query: &str) -> Vec<Option<KeyStroke>> {
        model(query).rows.iter().map(|row| row.stroke).collect()
    }

    #[test]
    fn text_matches_title_id_or_key_text() {
        // Break caught: typing "ctrl+s" or a command ID finding nothing.
        let titles = |query: &str| {
            model(query)
                .rows
                .iter()
                .map(|row| row.title)
                .collect::<Vec<_>>()
        };
        assert!(titles("save as").contains(&"File: Save as..."));
        assert!(titles("file.saveAs").contains(&"File: Save as..."));
        let by_key = model("ctrl + s");
        assert!(by_key.rows.iter().all(|row| {
            row.stroke
                .is_some_and(|s| s.text().to_lowercase().replace(' ', "").contains("ctrl+s"))
        }));
        assert!(by_key.rows.iter().any(|row| row.command == CommandId::Save));
    }

    #[test]
    fn record_keys_search_filters_by_the_exact_key() {
        // Break caught: key search matching by substring or the toggle not clearing the filter.
        let mut model = model("");
        assert_eq!(
            model.toggle_record_keys(),
            ShortcutsEffect::SetSearchText(String::new())
        );
        assert!(model.record_keys);
        assert_eq!(
            model.record_search_key(stroke("Ctrl+S")),
            ShortcutsEffect::SetSearchText("Ctrl+S".into())
        );
        assert_eq!(model.rows.len(), 1);
        assert_eq!(model.rows[0].command, CommandId::Save);
        model.toggle_record_keys();
        assert!(!model.record_keys);
        assert_eq!(model.filter, Filter::Text(String::new()));
    }

    #[test]
    fn arrows_move_the_selection_and_up_from_the_top_returns_to_the_search() {
        // Break caught: the selection running off the table or Up at the top trapping focus.
        let mut model = model("");
        assert_eq!(model.table_key(stroke("Down")), ShortcutsEffect::Repaint);
        assert_eq!(model.selected, 1);
        assert_eq!(model.table_key(stroke("End")), ShortcutsEffect::Repaint);
        assert_eq!(model.selected, model.rows.len() - 1);
        assert_eq!(model.top, model.rows.len() - 10);
        model.table_key(stroke("Home"));
        assert_eq!((model.selected, model.top), (0, 0));
        assert_eq!(model.table_key(stroke("Up")), ShortcutsEffect::FocusSearch);
        assert_eq!(model.table_key(stroke("Escape")), ShortcutsEffect::Close);
    }

    #[test]
    fn changing_a_key_replaces_only_that_key() {
        // Break caught: changing Zoom In's second key dropping the other two.
        let mut model = model("zoom in");
        model.select(1);
        assert_eq!(model.table_key(stroke("Enter")), ShortcutsEffect::Repaint);
        assert!(model.recording.is_some());
        assert_eq!(model.record_key(stroke("F9")), ShortcutsEffect::Repaint);
        assert_eq!(
            model.record_key(stroke("Enter")),
            ShortcutsEffect::SetKeys(
                CommandId::ZoomIn,
                vec![stroke("Ctrl+="), stroke("F9"), stroke("Ctrl+NumpadAdd")]
            )
        );
        assert!(model.recording.is_none());
    }

    #[test]
    fn adding_appends_and_an_empty_row_adds() {
        // Break caught: Ctrl+Enter replacing the key instead of adding, or an unbound command not gaining one.
        let mut model = model("save as");
        model.table_key(stroke("Ctrl+Enter"));
        model.record_key(stroke("F9"));
        assert_eq!(
            model.record_key(stroke("Enter")),
            ShortcutsEffect::SetKeys(
                CommandId::SaveAs,
                vec![stroke("Ctrl+Shift+S"), stroke("F9")]
            )
        );
        let mut about = self::model("about");
        about.table_key(stroke("Enter"));
        about.record_key(stroke("F9"));
        assert_eq!(
            about.record_key(stroke("Enter")),
            ShortcutsEffect::SetKeys(CommandId::About, vec![stroke("F9")])
        );
    }

    #[test]
    fn recording_refuses_typing_keys_and_escape_cancels() {
        // Break caught: plain A accepted as a shortcut, or Enter with a refused key saving it.
        let mut model = model("save as");
        model.table_key(stroke("Enter"));
        model.record_key(stroke("A"));
        assert_eq!(
            model.recording.as_ref().unwrap().refusal,
            Some(TYPING_REFUSAL)
        );
        assert_eq!(model.record_key(stroke("Enter")), ShortcutsEffect::None);
        assert!(model.recording.is_some());
        // Keys that Enter and Escape mean in the box are recorded with modifiers.
        model.record_key(stroke("Ctrl+Enter"));
        assert_eq!(model.recording.as_ref().unwrap().refusal, None);
        assert_eq!(model.record_key(stroke("Escape")), ShortcutsEffect::Repaint);
        assert!(model.recording.is_none());
    }

    #[test]
    fn confirming_the_same_key_changes_nothing() {
        // Break caught: a no-op confirm writing the ini.
        let mut model = model("save as");
        model.table_key(stroke("Enter"));
        model.record_key(stroke("Ctrl+Shift+S"));
        assert_eq!(model.record_key(stroke("Enter")), ShortcutsEffect::Repaint);
    }

    #[test]
    fn conflicts_count_other_commands_and_the_link_filters_by_the_key() {
        // Break caught: the conflict link not searching the recorded key.
        let mut model = model("save as");
        model.table_key(stroke("Enter"));
        model.record_key(stroke("F3"));
        assert_eq!(model.conflict_count(), 1);
        assert_eq!(
            model.follow_conflicts(),
            ShortcutsEffect::SetSearchText("F3".into())
        );
        assert!(model.recording.is_none());
        assert!(model.record_keys);
        assert_eq!(
            model.rows.iter().map(|row| row.command).collect::<Vec<_>>(),
            [CommandId::FindNext]
        );
    }

    #[test]
    fn remove_unbinds_the_key_and_reset_is_only_for_user_rows() {
        // Break caught: Reset offered on default rows, or Delete on an empty row.
        let mut model = model("save as");
        assert_eq!(model.reset(), ShortcutsEffect::None);
        assert_eq!(
            model.table_key(stroke("Delete")),
            ShortcutsEffect::SetKeys(CommandId::SaveAs, vec![])
        );
        model.refresh(
            Keymap::defaults().with_keys(CommandId::SaveAs, vec![]),
            vec![CommandId::SaveAs],
        );
        assert_eq!(model.rows.len(), 1);
        assert_eq!(model.rows[0].stroke, None);
        assert!(model.rows[0].user);
        assert_eq!(model.table_key(stroke("Delete")), ShortcutsEffect::None);
        assert_eq!(model.reset(), ShortcutsEffect::Reset(CommandId::SaveAs));
        assert_eq!(
            model.table_key(stroke("Ctrl+C")),
            ShortcutsEffect::CopyId("file.saveAs")
        );
    }

    #[test]
    fn refresh_keeps_the_selected_command() {
        // Break caught: the selection jumping to the top after every change.
        let mut model = model("zoom");
        let index = model
            .rows
            .iter()
            .position(|row| row.command == CommandId::ZoomOut)
            .unwrap();
        model.select(index);
        model.refresh(
            Keymap::defaults().with_keys(CommandId::ZoomOut, vec![stroke("F9")]),
            vec![CommandId::ZoomOut],
        );
        assert_eq!(model.selected_row().unwrap().command, CommandId::ZoomOut);
        assert_eq!(model.selected_row().unwrap().stroke, Some(stroke("F9")));
    }

    #[test]
    fn a_confirmed_key_selects_its_own_row() {
        // Break caught: changing Zoom In's third key to F9 selecting Ctrl+= (the command's
        // first row) instead of the new F9 row.
        let mut model = model("zoom in");
        model.select(2);
        model.start_change();
        model.record_key(stroke("F9"));
        let ShortcutsEffect::SetKeys(command, keys) = model.record_key(stroke("Enter")) else {
            panic!("no change");
        };
        model.refresh(
            Keymap::defaults().with_keys(command, keys),
            vec![CommandId::ZoomIn],
        );
        assert_eq!(model.selected_row().unwrap().stroke, Some(stroke("F9")));
        // A later refresh with no recording behind it keeps the selection where it is.
        model.select(0);
        model.refresh(model.keymap.clone(), vec![CommandId::ZoomIn]);
        assert_eq!(model.selected, 0);
    }

    #[test]
    fn reset_is_offered_for_an_ignored_ini_line() {
        // Break caught: `key.file.save=Bogus` (a line the keymap ignored, so the keys are the
        // defaults) offering no Reset, so the stale line can't be removed from the page.
        let mut model = model("file: save");
        let index = model
            .rows
            .iter()
            .position(|row| row.command == CommandId::Save)
            .unwrap();
        model.select(index);
        assert!(!model.selected_row().unwrap().user);
        assert!(!model.can_reset());
        assert_eq!(model.reset(), ShortcutsEffect::None);
        model.refresh(Keymap::defaults(), vec![CommandId::Save]);
        assert!(model.can_reset());
        assert_eq!(model.reset(), ShortcutsEffect::Reset(CommandId::Save));
    }

    #[test]
    fn an_empty_filter_result_keeps_the_keys_harmless() {
        // Break caught: a panic or effect from keys on an empty table.
        let mut model = model("no such command anywhere");
        assert!(model.rows.is_empty());
        assert_eq!(model.table_key(stroke("Enter")), ShortcutsEffect::None);
        assert_eq!(model.table_key(stroke("Delete")), ShortcutsEffect::None);
        assert_eq!(model.table_key(stroke("Down")), ShortcutsEffect::Repaint);
        assert_eq!(model.selected, 0);
    }

    #[test]
    fn scrolling_moves_the_window_not_the_selection_and_stays_in_range() {
        // Break caught: the wheel scrolling past the last row or dragging the selection along.
        let mut model = model("");
        model.scroll(3);
        assert_eq!((model.top, model.selected), (3, 0));
        model.scroll(-10);
        assert_eq!(model.top, 0);
        model.scroll(isize::MAX);
        assert_eq!(model.top, model.rows.len() - 10);
    }

    #[test]
    fn resizing_the_table_keeps_the_selection_in_view_and_fills_it() {
        // Break caught: after a resize, the selected row hidden below a shorter table, or a
        // taller one scrolled to the end showing empty space under the last row.
        let mut model = model("");
        let last = model.rows.len() - 1;
        model.select(last);
        assert_eq!(model.top, model.rows.len() - 10);
        model.set_visible(20);
        assert_eq!(
            model.top,
            model.rows.len() - 20,
            "no space under the last row"
        );
        model.select(15);
        model.set_visible(4);
        assert!(
            model.top <= 15 && 15 < model.top + 4,
            "the selection stays in view"
        );
        model.set_visible(0);
        assert_eq!(model.visible, 1);
    }
}
