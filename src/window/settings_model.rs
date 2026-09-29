//! The Settings dialog's rows and what each input does to them (settings dialog spec §3, §4.1).
//! Pure: no window handles, so the dialog's behaviour is tested without a window.

use crate::config::{FileIconSet, Settings, ThemePreference};
use crate::window::commands::CommandId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Section {
    Appearance,
    Editor,
    NotesAndSession,
}

impl Section {
    pub(crate) const ALL: [Self; 3] = [Self::Appearance, Self::Editor, Self::NotesAndSession];

    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Editor => "Editor",
            Self::NotesAndSession => "Notes and session",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Control {
    Check,
    Segmented,
    Dropdown,
    Stepper,
}

/// One row of the dialog, declared top to bottom: `row as usize` is its index in `Row::ALL`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Row {
    Theme,
    FileIcons,
    Font,
    FontSize,
    TabWidth,
    InsertSpaces,
    WordWrap,
    LineNumbers,
    ShowWhitespace,
    HighlightCurrentLine,
    NotesMode,
    RestoreSession,
    NotebookAutosave,
}

impl Row {
    pub(crate) const ALL: [Self; 13] = [
        Self::Theme,
        Self::FileIcons,
        Self::Font,
        Self::FontSize,
        Self::TabWidth,
        Self::InsertSpaces,
        Self::WordWrap,
        Self::LineNumbers,
        Self::ShowWhitespace,
        Self::HighlightCurrentLine,
        Self::NotesMode,
        Self::RestoreSession,
        Self::NotebookAutosave,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::FileIcons => "File icons",
            Self::Font => "Font",
            Self::FontSize => "Font size",
            Self::TabWidth => "Tab width",
            Self::InsertSpaces => "Indent with spaces",
            Self::WordWrap => "Word wrap",
            Self::LineNumbers => "Line numbers",
            Self::ShowWhitespace => "Show whitespace",
            Self::HighlightCurrentLine => "Highlight current line",
            Self::NotesMode => "Notes mode",
            Self::RestoreSession => "Restore session",
            Self::NotebookAutosave => "Notebook autosave",
        }
    }

    pub(crate) const fn section(self) -> Section {
        match self {
            Self::Theme | Self::FileIcons => Section::Appearance,
            Self::NotesMode | Self::RestoreSession | Self::NotebookAutosave => {
                Section::NotesAndSession
            }
            _ => Section::Editor,
        }
    }

    pub(crate) const fn control(self) -> Control {
        match self {
            Self::Theme | Self::Font => Control::Dropdown,
            Self::FileIcons | Self::TabWidth => Control::Segmented,
            Self::FontSize => Control::Stepper,
            _ => Control::Check,
        }
    }

    /// A checkbox row's setting.
    pub(crate) const fn toggle(self) -> Option<Toggle> {
        Some(match self {
            Self::InsertSpaces => Toggle::InsertSpaces,
            Self::WordWrap => Toggle::WordWrap,
            Self::LineNumbers => Toggle::LineNumbers,
            Self::ShowWhitespace => Toggle::ShowWhitespace,
            Self::HighlightCurrentLine => Toggle::HighlightCurrentLine,
            Self::NotesMode => Toggle::NotesMode,
            Self::RestoreSession => Toggle::RestoreSession,
            Self::NotebookAutosave => Toggle::NotebookAutosave,
            _ => return None,
        })
    }
}

/// A checkbox's setting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Toggle {
    InsertSpaces,
    WordWrap,
    LineNumbers,
    ShowWhitespace,
    HighlightCurrentLine,
    NotesMode,
    RestoreSession,
    NotebookAutosave,
}

impl Toggle {
    /// The palette command that flips it: the dialog runs exactly what the palette runs, notices
    /// and side effects included.
    pub(crate) const fn command(self) -> CommandId {
        match self {
            Self::InsertSpaces => CommandId::ToggleInsertSpaces,
            Self::WordWrap => CommandId::ToggleWordWrap,
            Self::LineNumbers => CommandId::ToggleLineNumbers,
            Self::ShowWhitespace => CommandId::ToggleShowWhitespace,
            Self::HighlightCurrentLine => CommandId::ToggleHighlightCurrentLine,
            Self::NotesMode => CommandId::ToggleNotesMode,
            Self::RestoreSession => CommandId::ToggleRestoreSession,
            Self::NotebookAutosave => CommandId::ToggleFolderAutosave,
        }
    }
}

/// The Theme dropdown's items, in order.
pub(crate) const THEME_CHOICES: [(ThemePreference, &str); 8] = [
    (ThemePreference::System, "System"),
    (ThemePreference::Light, "Light"),
    (ThemePreference::Dark, "Dark"),
    (ThemePreference::Catppuccin, "Catppuccin"),
    (ThemePreference::CatppuccinLatte, "Catppuccin Latte"),
    (ThemePreference::CatppuccinFrappe, "Catppuccin Frappé"),
    (ThemePreference::CatppuccinMacchiato, "Catppuccin Macchiato"),
    (ThemePreference::CatppuccinMocha, "Catppuccin Mocha"),
];

pub(crate) const FILE_ICON_CHOICES: [(FileIconSet, &str); 3] = [
    (FileIconSet::Material, "Material"),
    (FileIconSet::Minimal, "Minimal"),
    (FileIconSet::Solid, "Solid"),
];

pub(crate) const TAB_WIDTH_CHOICES: [u8; 3] = [2, 4, 8];

/// The font size the stepper and the palette's font-size commands keep within.
pub(crate) const MIN_FONT_SIZE: u16 = 6;
pub(crate) const MAX_FONT_SIZE: u16 = 72;

/// Digits the font size field accepts before ignoring more.
const MAX_TYPED_DIGITS: usize = 3;

/// One change the dialog asks `main_window::apply_settings_action` to make.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SettingsAction {
    SetTheme(ThemePreference),
    SetFileIcons(FileIconSet),
    SetFontFace(String),
    SetFontSize(u16),
    SetTabWidth(u8),
    Toggle(Toggle),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Segment {
    pub label: String,
    pub selected: bool,
}

/// What the dialog shows: the current settings, and the open notebook's autosave switch, which
/// is `None` while no notebook is open or its state is still loading.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SettingsView {
    pub settings: Settings,
    pub notebook_autosave: Option<bool>,
}

impl SettingsView {
    /// Whether `row` can be changed now. Only Notebook autosave is ever greyed out.
    pub(crate) fn enabled(&self, row: Row) -> bool {
        row != Row::NotebookAutosave || self.notebook_autosave.is_some()
    }

    pub(crate) fn checked(&self, toggle: Toggle) -> bool {
        let settings = &self.settings;
        match toggle {
            Toggle::InsertSpaces => settings.insert_spaces,
            Toggle::WordWrap => settings.word_wrap,
            Toggle::LineNumbers => settings.line_numbers,
            Toggle::ShowWhitespace => settings.show_whitespace,
            Toggle::HighlightCurrentLine => settings.highlight_current_line,
            Toggle::NotesMode => settings.notes_mode,
            Toggle::RestoreSession => settings.restore_session,
            Toggle::NotebookAutosave => self.notebook_autosave.unwrap_or(false),
        }
    }

    /// A segmented row's segments, left to right. A tab width other than 2, 4 or 8 gets a fourth,
    /// selected segment showing it (spec §3.2).
    pub(crate) fn segments(&self, row: Row) -> Vec<Segment> {
        match row {
            Row::FileIcons => FILE_ICON_CHOICES
                .iter()
                .map(|&(set, label)| Segment {
                    label: label.to_owned(),
                    selected: self.settings.file_icons == set,
                })
                .collect(),
            Row::TabWidth => {
                let width = self.settings.tab_width;
                let mut segments = TAB_WIDTH_CHOICES
                    .iter()
                    .map(|&choice| Segment {
                        label: choice.to_string(),
                        selected: width == choice,
                    })
                    .collect::<Vec<_>>();
                if !TAB_WIDTH_CHOICES.contains(&width) {
                    segments.push(Segment {
                        label: width.to_string(),
                        selected: true,
                    });
                }
                segments
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn selected_segment(&self, row: Row) -> Option<usize> {
        self.segments(row)
            .iter()
            .position(|segment| segment.selected)
    }

    /// What picking segment `index` of `row` does. The custom tab width segment is already the
    /// setting, so picking it does nothing.
    pub(crate) fn segment_action(&self, row: Row, index: usize) -> Option<SettingsAction> {
        match row {
            Row::FileIcons => FILE_ICON_CHOICES
                .get(index)
                .map(|&(set, _)| SettingsAction::SetFileIcons(set)),
            Row::TabWidth => TAB_WIDTH_CHOICES
                .get(index)
                .map(|&width| SettingsAction::SetTabWidth(width)),
            _ => None,
        }
    }

    /// A dropdown row's items and the selected one. `fonts` is the Font row's list.
    pub(crate) fn dropdown(&self, row: Row, fonts: &[String]) -> (Vec<String>, Option<usize>) {
        match row {
            Row::Theme => (
                THEME_CHOICES
                    .iter()
                    .map(|(_, label)| (*label).to_owned())
                    .collect(),
                THEME_CHOICES
                    .iter()
                    .position(|(theme, _)| *theme == self.settings.theme),
            ),
            Row::Font => (
                fonts.to_vec(),
                fonts
                    .iter()
                    .position(|font| font.eq_ignore_ascii_case(&self.settings.font_face)),
            ),
            _ => (Vec::new(), None),
        }
    }

    /// The text a closed dropdown shows.
    pub(crate) fn dropdown_text(&self, row: Row) -> String {
        match row {
            Row::Theme => THEME_CHOICES
                .iter()
                .find(|(theme, _)| *theme == self.settings.theme)
                .map_or_else(String::new, |(_, label)| (*label).to_owned()),
            Row::Font => self.settings.font_face.clone(),
            _ => String::new(),
        }
    }
}

/// What picking item `index` of `row`'s dropdown does.
pub(crate) fn dropdown_action(row: Row, index: usize, fonts: &[String]) -> Option<SettingsAction> {
    match row {
        Row::Theme => THEME_CHOICES
            .get(index)
            .map(|&(theme, _)| SettingsAction::SetTheme(theme)),
        Row::Font => fonts
            .get(index)
            .map(|font| SettingsAction::SetFontFace(font.clone())),
        _ => None,
    }
}

/// One step of the stepper from `size`. The result is within 6–72, so a size set outside that
/// range in `fastpad.ini` comes back into it on the first step (spec §3.2).
pub(crate) fn step_font_size(size: u16, up: bool) -> u16 {
    let next = if up {
        size.saturating_add(1)
    } else {
        size.saturating_sub(1)
    };
    next.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
}

/// The size a typed entry commits: its number pulled into 6–72, or `current` when the text is
/// empty or not a number.
pub(crate) fn typed_font_size(text: &str, current: u16) -> u16 {
    match text.trim().parse::<u32>() {
        Ok(value) => value.clamp(u32::from(MIN_FONT_SIZE), u32::from(MAX_FONT_SIZE)) as u16,
        Err(_) => current,
    }
}

/// What has the keyboard focus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Focus {
    Row(Row),
    EditIni,
    Close,
}

/// Tab order: the rows top to bottom, skipping greyed-out ones, then the Edit fastpad.ini link,
/// then Close. It wraps around.
pub(crate) fn next_focus(current: Focus, forward: bool, view: &SettingsView) -> Focus {
    let order = Row::ALL
        .into_iter()
        .filter(|row| view.enabled(*row))
        .map(Focus::Row)
        .chain([Focus::EditIni, Focus::Close])
        .collect::<Vec<_>>();
    let count = order.len();
    let next = match (order.iter().position(|focus| *focus == current), forward) {
        (Some(index), true) => (index + 1) % count,
        (Some(index), false) => (index + count - 1) % count,
        (None, true) => 0,
        (None, false) => count - 1,
    };
    order[next]
}

/// A key the dialog passes on, already decoded from its window message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Key {
    Tab { back: bool },
    Space,
    Enter,
    Left,
    Right,
    Up,
    Down,
    AltDown,
    Escape,
    Backspace,
    Char(char),
}

/// What the dialog does after an input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Effect {
    None,
    Repaint,
    Apply(SettingsAction),
    OpenDropdown(Row),
    EditIni,
    Close,
}

/// The dialog's keyboard state: what has the focus, and a font size being typed but not yet
/// committed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DialogModel {
    pub focus: Focus,
    pub typed: Option<String>,
}

impl DialogModel {
    pub(crate) fn new() -> Self {
        Self {
            focus: Focus::Row(Row::ALL[0]),
            typed: None,
        }
    }

    /// Moves the focus, committing a typed font size on the way out.
    pub(crate) fn set_focus(&mut self, focus: Focus, view: &SettingsView) -> Effect {
        let commit = self.commit_typed(view);
        self.focus = focus;
        match commit {
            Effect::None => Effect::Repaint,
            other => other,
        }
    }

    /// The font size field's text: what is being typed, or the current size.
    pub(crate) fn font_size_text(&self, view: &SettingsView) -> String {
        self.typed
            .clone()
            .unwrap_or_else(|| view.settings.font_size.to_string())
    }

    pub(crate) fn key(&mut self, key: Key, view: &SettingsView) -> Effect {
        match key {
            Key::Tab { back } => {
                let next = next_focus(self.focus, !back, view);
                self.set_focus(next, view)
            }
            Key::Escape => Effect::Close,
            _ => match self.focus {
                Focus::EditIni if matches!(key, Key::Space | Key::Enter) => Effect::EditIni,
                Focus::Close if matches!(key, Key::Space | Key::Enter) => Effect::Close,
                Focus::Row(row) => self.row_key(row, key, view),
                _ => Effect::None,
            },
        }
    }

    fn row_key(&mut self, row: Row, key: Key, view: &SettingsView) -> Effect {
        if !view.enabled(row) {
            return Effect::None;
        }
        match row.control() {
            Control::Check => match (key, row.toggle()) {
                (Key::Space, Some(toggle)) => Effect::Apply(SettingsAction::Toggle(toggle)),
                _ => Effect::None,
            },
            Control::Segmented => {
                let Some(selected) = view.selected_segment(row) else {
                    return Effect::None;
                };
                let count = view.segments(row).len();
                let target = match key {
                    Key::Left => selected.checked_sub(1),
                    Key::Right => (selected + 1 < count).then_some(selected + 1),
                    _ => None,
                };
                target
                    .and_then(|index| view.segment_action(row, index))
                    .map_or(Effect::None, Effect::Apply)
            }
            Control::Dropdown => match key {
                Key::Enter | Key::AltDown => Effect::OpenDropdown(row),
                _ => Effect::None,
            },
            Control::Stepper => self.stepper_key(key, view),
        }
    }

    fn stepper_key(&mut self, key: Key, view: &SettingsView) -> Effect {
        let current = view.settings.font_size;
        match key {
            Key::Up | Key::Down => {
                // A step replaces whatever was being typed.
                self.typed = None;
                let size = step_font_size(current, key == Key::Up);
                if size == current {
                    Effect::Repaint
                } else {
                    Effect::Apply(SettingsAction::SetFontSize(size))
                }
            }
            Key::Char(digit) if digit.is_ascii_digit() => {
                let typed = self.typed.get_or_insert_with(String::new);
                if typed.len() < MAX_TYPED_DIGITS {
                    typed.push(digit);
                }
                Effect::Repaint
            }
            Key::Backspace => {
                self.typed.get_or_insert_with(|| current.to_string()).pop();
                Effect::Repaint
            }
            Key::Enter => self.commit_typed(view),
            _ => Effect::None,
        }
    }

    /// The typed size's change, if it differs from the current size, clearing the typed text.
    fn commit_typed(&mut self, view: &SettingsView) -> Effect {
        let Some(text) = self.typed.take() else {
            return Effect::None;
        };
        let current = view.settings.font_size;
        let size = typed_font_size(&text, current);
        if size == current {
            Effect::Repaint
        } else {
            Effect::Apply(SettingsAction::SetFontSize(size))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> SettingsView {
        SettingsView {
            settings: crate::config::default_settings(),
            notebook_autosave: None,
        }
    }

    #[test]
    fn rows_follow_the_spec_order_and_sections() {
        // Break caught: a row painted under the wrong heading, or `row as usize` drifting from
        // its position in `Row::ALL`, which the layout indexes by.
        for (index, row) in Row::ALL.into_iter().enumerate() {
            assert_eq!(row as usize, index, "{row:?}");
        }
        let sections = Row::ALL.map(Row::section);
        assert_eq!(&sections[..2], [Section::Appearance; 2]);
        assert_eq!(&sections[2..10], [Section::Editor; 8]);
        assert_eq!(&sections[10..], [Section::NotesAndSession; 3]);
        for (index, section) in Section::ALL.into_iter().enumerate() {
            assert_eq!(section as usize, index);
        }
    }

    #[test]
    fn every_checkbox_runs_the_palette_command_for_its_setting() {
        // Break caught: a checkbox that flips a different setting than its label says.
        let checks = Row::ALL
            .into_iter()
            .filter(|row| row.control() == Control::Check)
            .map(|row| row.toggle().unwrap().command())
            .collect::<Vec<_>>();
        assert_eq!(
            checks,
            [
                CommandId::ToggleInsertSpaces,
                CommandId::ToggleWordWrap,
                CommandId::ToggleLineNumbers,
                CommandId::ToggleShowWhitespace,
                CommandId::ToggleHighlightCurrentLine,
                CommandId::ToggleNotesMode,
                CommandId::ToggleRestoreSession,
                CommandId::ToggleFolderAutosave,
            ]
        );
        assert_eq!(Row::Theme.toggle(), None);
    }

    #[test]
    fn tab_order_wraps_and_skips_notebook_autosave_without_a_notebook() {
        // Break caught: Tab stopping on a greyed-out row that ignores every key.
        let closed = view();
        assert_eq!(
            next_focus(Focus::Row(Row::RestoreSession), true, &closed),
            Focus::EditIni
        );
        assert_eq!(
            next_focus(Focus::Close, true, &closed),
            Focus::Row(Row::Theme)
        );
        assert_eq!(
            next_focus(Focus::Row(Row::Theme), false, &closed),
            Focus::Close
        );
        let open = SettingsView {
            notebook_autosave: Some(true),
            ..view()
        };
        assert_eq!(
            next_focus(Focus::Row(Row::RestoreSession), true, &open),
            Focus::Row(Row::NotebookAutosave)
        );
        assert!(open.checked(Toggle::NotebookAutosave));
        assert!(!closed.checked(Toggle::NotebookAutosave));
    }

    #[test]
    fn a_custom_tab_width_shows_as_a_fourth_selected_segment() {
        // Break caught: tab_width=3 from fastpad.ini shown as none selected, or picking the
        // custom segment writing a value.
        let mut custom = view();
        custom.settings.tab_width = 3;
        let segments = custom.segments(Row::TabWidth);
        assert_eq!(
            segments
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>(),
            ["2", "4", "8", "3"]
        );
        assert_eq!(custom.selected_segment(Row::TabWidth), Some(3));
        assert_eq!(custom.segment_action(Row::TabWidth, 3), None);
        assert_eq!(
            custom.segment_action(Row::TabWidth, 0),
            Some(SettingsAction::SetTabWidth(2))
        );
        assert_eq!(view().segments(Row::TabWidth).len(), 3);
        assert_eq!(view().selected_segment(Row::TabWidth), Some(1));
        assert_eq!(view().selected_segment(Row::FileIcons), Some(0));
    }

    #[test]
    fn the_stepper_stays_within_6_to_72_and_brings_outside_sizes_back() {
        assert_eq!(step_font_size(11, true), 12);
        assert_eq!(step_font_size(11, false), 10);
        assert_eq!(step_font_size(72, true), 72);
        assert_eq!(step_font_size(6, false), 6);
        assert_eq!(
            step_font_size(100, true),
            72,
            "a hand-edited 100 comes back"
        );
        assert_eq!(step_font_size(3, false), 6);
    }

    #[test]
    fn a_typed_size_is_clamped_and_text_that_is_not_a_number_keeps_the_current_size() {
        assert_eq!(typed_font_size("16", 11), 16);
        assert_eq!(typed_font_size("999", 11), 72);
        assert_eq!(typed_font_size("0", 11), 6);
        assert_eq!(typed_font_size("", 11), 11);
        assert_eq!(typed_font_size("abc", 11), 11);
    }

    #[test]
    fn tab_commits_a_typed_size_and_moves_on() {
        // Break caught: a typed size lost when the keyboard leaves the field (review focus 3).
        let view = view();
        let mut model = DialogModel {
            focus: Focus::Row(Row::FontSize),
            typed: None,
        };
        assert_eq!(model.key(Key::Char('1'), &view), Effect::Repaint);
        assert_eq!(model.key(Key::Char('6'), &view), Effect::Repaint);
        assert_eq!(model.font_size_text(&view), "16");
        assert_eq!(
            model.key(Key::Tab { back: false }, &view),
            Effect::Apply(SettingsAction::SetFontSize(16))
        );
        assert_eq!(model.focus, Focus::Row(Row::TabWidth));
        assert_eq!(model.typed, None);
    }

    #[test]
    fn the_font_size_field_takes_three_digits_enter_commits_and_a_step_discards_typing() {
        let view = view();
        let mut model = DialogModel {
            focus: Focus::Row(Row::FontSize),
            typed: None,
        };
        for digit in ['1', '2', '3', '4'] {
            model.key(Key::Char(digit), &view);
        }
        assert_eq!(model.font_size_text(&view), "123");
        assert_eq!(
            model.key(Key::Enter, &view),
            Effect::Apply(SettingsAction::SetFontSize(72))
        );
        model.key(Key::Backspace, &view);
        assert_eq!(
            model.font_size_text(&view),
            "1",
            "backspace edits the current 11"
        );
        assert_eq!(
            model.key(Key::Up, &view),
            Effect::Apply(SettingsAction::SetFontSize(12))
        );
        assert_eq!(model.typed, None);
        model.key(Key::Char('x'), &view);
        assert_eq!(model.typed, None, "letters are ignored");
        model.key(Key::Char('1'), &view);
        assert_eq!(model.key(Key::Char('1'), &view), Effect::Repaint);
        assert_eq!(
            model.key(Key::Enter, &view),
            Effect::Repaint,
            "typing the current size changes nothing"
        );
    }

    #[test]
    fn keys_act_on_the_focused_control() {
        let view = view();
        let mut model = DialogModel::new();
        assert_eq!(model.focus, Focus::Row(Row::Theme));
        assert_eq!(
            model.key(Key::Enter, &view),
            Effect::OpenDropdown(Row::Theme)
        );
        assert_eq!(
            model.key(Key::AltDown, &view),
            Effect::OpenDropdown(Row::Theme)
        );
        assert_eq!(model.key(Key::Space, &view), Effect::None);

        model.focus = Focus::Row(Row::FileIcons);
        assert_eq!(
            model.key(Key::Left, &view),
            Effect::None,
            "already the first"
        );
        assert_eq!(
            model.key(Key::Right, &view),
            Effect::Apply(SettingsAction::SetFileIcons(FileIconSet::Minimal))
        );

        model.focus = Focus::Row(Row::WordWrap);
        assert_eq!(
            model.key(Key::Space, &view),
            Effect::Apply(SettingsAction::Toggle(Toggle::WordWrap))
        );
        model.focus = Focus::Row(Row::NotebookAutosave);
        assert_eq!(model.key(Key::Space, &view), Effect::None, "greyed out");

        model.focus = Focus::EditIni;
        assert_eq!(model.key(Key::Enter, &view), Effect::EditIni);
        model.focus = Focus::Close;
        assert_eq!(model.key(Key::Space, &view), Effect::Close);
        assert_eq!(model.key(Key::Escape, &view), Effect::Close);
    }

    #[test]
    fn dropdowns_select_the_current_value_and_pick_by_index() {
        let mut view = view();
        view.settings.theme = ThemePreference::CatppuccinMocha;
        view.settings.font_face = "consolas".to_owned();
        let fonts = vec!["Cascadia Mono".to_owned(), "Consolas".to_owned()];
        let (themes, selected) = view.dropdown(Row::Theme, &fonts);
        assert_eq!(themes.len(), 8);
        assert_eq!(selected, Some(7));
        assert_eq!(view.dropdown_text(Row::Theme), "Catppuccin Mocha");
        assert_eq!(view.dropdown(Row::Font, &fonts), (fonts.clone(), Some(1)));
        assert_eq!(view.dropdown_text(Row::Font), "consolas");
        assert_eq!(
            dropdown_action(Row::Theme, 2, &fonts),
            Some(SettingsAction::SetTheme(ThemePreference::Dark))
        );
        assert_eq!(
            dropdown_action(Row::Font, 0, &fonts),
            Some(SettingsAction::SetFontFace("Cascadia Mono".to_owned()))
        );
        assert_eq!(dropdown_action(Row::Font, 9, &fonts), None);
    }
}
