/// Where one view of a document was: the selection (anchor to caret), the first visible display
/// line and the horizontal scroll (split editors spec §3.2).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViewState {
    pub caret: usize,
    pub anchor: usize,
    pub first_line: usize,
    pub x_offset: i32,
}
