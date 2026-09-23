//! MSAA for the sidebar's two painted windows (spec §10). The activity bar is a toolbar of push
//! buttons. The panel is an outline of outline items (Notebook view) or a list of list items
//! (Search and Favorites), with its header buttons as push buttons.
//!
//! Children are flat child IDs answered by the provider itself, as in `accessibility.rs`. The
//! provider reads nothing but its window handle and a static `AccessibleSource`. Clients call in
//! on RPC threads, so every query goes to the window's own thread first, and App state is only
//! read there. A 10,000-row tree is counted and read one item at a time, never as a list.

use crate::library::tree::{RowKind, TreeRow};
use crate::window::accessibility::{
    AccessibleVtable, IID_IACCESSIBLE, IID_IDISPATCH, IID_IUNKNOWN, RawVariant, VariantValue,
    accessible_get_help_topic, accessible_get_ids_of_names, accessible_get_parent,
    accessible_get_type_info, accessible_get_type_info_count, accessible_invoke, allocate_bstr,
    guid_eq,
};
use crate::window::row_list::RowListState;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{
    E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, HWND, LPARAM, LRESULT, POINT, RECT, S_FALSE, S_OK,
    WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Accessibility::{
    LresultFromObject, NAVDIR_DOWN, NAVDIR_FIRSTCHILD, NAVDIR_LASTCHILD, NAVDIR_NEXT,
    NAVDIR_PREVIOUS, NAVDIR_UP, NotifyWinEvent, ROLE_SYSTEM_LISTITEM, ROLE_SYSTEM_OUTLINEITEM,
    ROLE_SYSTEM_PANE, ROLE_SYSTEM_PUSHBUTTON, SELFLAG_TAKEFOCUS, SELFLAG_TAKESELECTION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_FOCUS, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_REORDER, EVENT_OBJECT_SELECTION,
    EVENT_OBJECT_STATECHANGE, GUITHREADINFO, GetClientRect, GetGUIThreadInfo, GetWindowRect,
    GetWindowThreadProcessId, OBJID_CLIENT, PostMessageW, SendMessageW, WM_APP, WM_LBUTTONDOWN,
    WM_LBUTTONUP,
};
use windows_sys::core::{BSTR, GUID, HRESULT};

#[cfg(test)]
use windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_LIST;

/// Sent to a sidebar window with a `*mut Call` to run one query on the window's thread.
pub(crate) const WM_FASTPAD_SIDEBAR_ACCESSIBLE: u32 = WM_APP + 0x60;
/// Posted to a sidebar window: `wparam` is the child index, and `lparam` is one of `ACTION_*`.
pub(crate) const WM_FASTPAD_SIDEBAR_ACTION: u32 = WM_APP + 0x61;
pub(crate) const ACTION_ACTIVATE: LPARAM = 0;
pub(crate) const ACTION_SELECT: LPARAM = 1;
pub(crate) const ACTION_FOCUS: LPARAM = 2;

pub(crate) const STATE_SELECTED: u32 = 0x0000_0002;
pub(crate) const STATE_FOCUSED: u32 = 0x0000_0004;
pub(crate) const STATE_PRESSED: u32 = 0x0000_0008;
pub(crate) const STATE_EXPANDED: u32 = 0x0000_0200;
pub(crate) const STATE_COLLAPSED: u32 = 0x0000_0400;
pub(crate) const STATE_OFFSCREEN: u32 = 0x0001_0000;
pub(crate) const STATE_FOCUSABLE: u32 = 0x0010_0000;
pub(crate) const STATE_SELECTABLE: u32 = 0x0020_0000;
const MK_LBUTTON: WPARAM = 0x0001;

/// One MSAA child of a sidebar window, in the window's client coordinates.
#[derive(Clone)]
pub(crate) struct AccessibleItem {
    pub name: String,
    pub role: u32,
    pub state: u32,
    pub rect: RECT,
    /// An outline item's level (0 for the root's children), as tree views report it. Empty
    /// otherwise.
    pub value: String,
}

impl std::fmt::Debug for AccessibleItem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccessibleItem")
            .field("name", &self.name)
            .field("role", &self.role)
            .field("state", &format_args!("{:#x}", self.state))
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}

/// What a window's provider asks, always on the window's own thread.
pub(crate) struct AccessibleSource {
    /// The window itself: its name and role.
    pub container: fn(HWND) -> (String, u32),
    pub count: fn(HWND) -> usize,
    pub item: fn(HWND, usize) -> Option<AccessibleItem>,
    /// The child under a client point.
    pub hit: fn(HWND, POINT) -> Option<usize>,
    /// The selected child: the focused one while the window has the keyboard focus.
    pub current: fn(HWND) -> Option<usize>,
    /// Makes a child current without activating it.
    pub select: fn(HWND, usize),
    /// A child's default action.
    pub activate: fn(HWND, usize),
    /// A cheap identity of child `index` (a hash of its row or path) that survives a re-sort,
    /// or `None` for children with no identity of their own.
    pub identity: fn(HWND, usize) -> Option<u64>,
    /// Changes whenever the children's order does, even at the same count.
    pub generation: fn(HWND) -> u64,
}

/// A hash of `value`, as an `AccessibleSource::identity`.
pub(crate) fn identity_of(value: &impl std::hash::Hash) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    BuildHasherDefault::<DefaultHasher>::default().hash_one(value)
}

/// A view's children as MSAA sees them. Every method gets the panel's client rectangle and DPI.
pub(crate) trait AccessibleView {
    fn accessible_count(&self, client: RECT, dpi: u32) -> usize;
    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<AccessibleItem>;
    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize>;
    fn accessible_current(&self, client: RECT, dpi: u32) -> Option<usize>;
    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32);
    /// See `AccessibleSource::identity`.
    fn accessible_identity(&self, _index: usize, _client: RECT, _dpi: u32) -> Option<u64> {
        None
    }
    /// See `AccessibleSource::generation`.
    fn accessible_generation(&self) -> u64 {
        0
    }
}

pub(crate) fn button_item(name: &str, pressed: bool, focused: bool, rect: RECT) -> AccessibleItem {
    let mut state = STATE_FOCUSABLE;
    if pressed {
        state |= STATE_PRESSED;
    }
    if focused {
        state |= STATE_FOCUSED;
    }
    AccessibleItem {
        name: name.to_owned(),
        role: ROLE_SYSTEM_PUSHBUTTON,
        state,
        rect,
        value: String::new(),
    }
}

fn row_state(selected: bool, focused: bool, visible: bool) -> u32 {
    let mut state = STATE_SELECTABLE | STATE_FOCUSABLE;
    if selected {
        state |= STATE_SELECTED;
        if focused {
            state |= STATE_FOCUSED;
        }
    }
    if !visible {
        state |= STATE_OFFSCREEN;
    }
    state
}

/// A search result, favorite or recent notebook. `focused` means the list has the focus: only
/// the selected item is then focused.
pub(crate) fn list_item(
    name: &str,
    selected: bool,
    focused: bool,
    rect: RECT,
    visible: bool,
) -> AccessibleItem {
    AccessibleItem {
        name: name.to_owned(),
        role: ROLE_SYSTEM_LISTITEM,
        state: row_state(selected, focused, visible),
        rect,
        value: String::new(),
    }
}

/// A Notebook-view row. The name carries ", pinned" or ", unsaved", so neither is conveyed by the
/// icon alone.
pub(crate) fn tree_item(
    row: &TreeRow,
    selected: bool,
    focused: bool,
    rect: RECT,
    visible: bool,
) -> AccessibleItem {
    let mut name = row.name.clone();
    if row.pinned {
        name.push_str(", pinned");
    }
    if matches!(row.kind, RowKind::Unsaved(_)) {
        name.push_str(", unsaved");
    }
    let mut state = row_state(selected, focused, visible);
    if matches!(row.kind, RowKind::Folder(_)) {
        state |= if row.expanded {
            STATE_EXPANDED
        } else {
            STATE_COLLAPSED
        };
    }
    AccessibleItem {
        name,
        role: ROLE_SYSTEM_OUTLINEITEM,
        state,
        rect,
        value: row.depth.to_string(),
    }
}

/// Where row `index` of `list` is, scrolled or not, and whether any of it shows in `area`.
pub(crate) fn row_rect(area: RECT, list: &RowListState, index: usize) -> (RECT, bool) {
    let offset = (index as i64 - list.top as i64) * i64::from(list.row_height);
    let top = (i64::from(area.top) + offset).clamp(i64::from(i32::MIN / 2), i64::from(i32::MAX / 2))
        as i32;
    let rect = RECT {
        left: area.left,
        top,
        right: area.right,
        bottom: top.saturating_add(list.row_height),
    };
    let visible = rect.bottom > area.top && rect.top < area.bottom;
    (rect, visible)
}

pub(crate) fn default_action(item: &AccessibleItem) -> &'static str {
    if item.role == ROLE_SYSTEM_PUSHBUTTON {
        "Press"
    } else if item.state & STATE_EXPANDED != 0 {
        "Collapse"
    } else if item.state & STATE_COLLAPSED != 0 {
        "Expand"
    } else {
        "Open"
    }
}

/// Presses an item the way a click on its center does.
pub(crate) fn click_item(hwnd: HWND, rect: RECT) {
    let x = (rect.left + rect.right) / 2;
    let y = (rect.top + rect.bottom) / 2;
    let point = (x as u16 as u32 | ((y as u16 as u32) << 16)) as LPARAM;
    unsafe {
        SendMessageW(hwnd, WM_LBUTTONDOWN, MK_LBUTTON, point);
        SendMessageW(hwnd, WM_LBUTTONUP, 0, point);
    }
}

/// What screen readers last knew about the current child, compared across a change.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AccessibleMark {
    pub current: Option<usize>,
    /// Focus and scrolling aside: those raise their own events.
    pub state: u32,
    pub name: String,
    pub count: usize,
    /// The current child's `AccessibleSource::identity`.
    pub identity: Option<u64>,
    pub generation: u64,
}

impl AccessibleMark {
    /// Reads the mark on the window's own thread.
    pub(crate) fn read(hwnd: HWND, source: &AccessibleSource) -> Self {
        let current = (source.current)(hwnd);
        let item = current.and_then(|index| (source.item)(hwnd, index));
        Self {
            current,
            state: item
                .as_ref()
                .map_or(0, |item| item.state & !(STATE_FOCUSED | STATE_OFFSCREEN)),
            name: item.map(|item| item.name).unwrap_or_default(),
            count: (source.count)(hwnd),
            identity: current.and_then(|index| (source.identity)(hwnd, index)),
            generation: (source.generation)(hwnd),
        }
    }
}

/// The win events a change from `before` to `after` raises, as (event, child id).
pub(crate) fn events_between(
    before: &AccessibleMark,
    after: &AccessibleMark,
    focused: bool,
) -> Vec<(u32, i32)> {
    let mut events = Vec::new();
    // A re-sort at the same count still gives every moved sibling a new child ID.
    if before.count != after.count || before.generation != after.generation {
        events.push((EVENT_OBJECT_REORDER, 0));
    }
    let Some(current) = after.current else {
        return events;
    };
    let id = current as i32 + 1;
    // The same item is current: by identity where the children have one, else by index.
    let same_item = match (before.identity, after.identity) {
        (Some(before_identity), Some(after_identity)) => before_identity == after_identity,
        (None, None) => before.current == after.current,
        _ => false,
    };
    if before.current != after.current || !same_item {
        events.push((EVENT_OBJECT_SELECTION, id));
        if focused {
            events.push((EVENT_OBJECT_FOCUS, id));
        }
    }
    if same_item {
        // A pin changes only the name (", pinned"), and usually moves the row, but screen
        // readers listen for it as a state change (spec §10), so a renamed current child raises
        // both, in place or at its new ID.
        if before.state != after.state || before.name != after.name {
            events.push((EVENT_OBJECT_STATECHANGE, id));
        }
        if before.name != after.name {
            events.push((EVENT_OBJECT_NAMECHANGE, id));
        }
    }
    events
}

#[cfg(test)]
thread_local! {
    static RAISED: std::cell::RefCell<Vec<(usize, u32, i32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The (window, event, child ID) triples raised on this thread since the last call.
#[cfg(test)]
pub(crate) fn take_raised() -> Vec<(usize, u32, i32)> {
    RAISED.with(|raised| std::mem::take(&mut *raised.borrow_mut()))
}

pub(crate) fn raise(hwnd: HWND, events: &[(u32, i32)]) {
    for &(event, child) in events {
        #[cfg(test)]
        RAISED.with(|raised| raised.borrow_mut().push((hwnd as usize, event, child)));
        unsafe {
            NotifyWinEvent(event, hwnd, OBJID_CLIENT, child);
        }
    }
}

/// One event for child `index` (0-based), or for the window itself with `None`.
pub(crate) fn notify(event: u32, hwnd: HWND, index: Option<usize>) {
    raise(hwnd, &[(event, index.map_or(0, |index| index as i32 + 1))]);
}

#[repr(C)]
struct SidebarAccessible {
    vtable: &'static AccessibleVtable,
    references: AtomicU32,
    hwnd: HWND,
    source: &'static AccessibleSource,
}

pub(crate) static SIDEBAR_VTABLE: AccessibleVtable = AccessibleVtable {
    query_interface,
    add_ref,
    release,
    get_type_info_count: accessible_get_type_info_count,
    get_type_info: accessible_get_type_info,
    get_ids_of_names: accessible_get_ids_of_names,
    invoke: accessible_invoke,
    get_acc_parent: accessible_get_parent,
    get_acc_child_count: child_count,
    get_acc_child: child,
    get_acc_name: name,
    get_acc_value: value,
    get_acc_description: empty_text,
    get_acc_role: role,
    get_acc_state: state,
    get_acc_help: empty_text,
    get_acc_help_topic: accessible_get_help_topic,
    get_acc_keyboard_shortcut: empty_text,
    get_acc_focus: focus,
    get_acc_selection: selection,
    get_acc_default_action: default_action_text,
    acc_select: select,
    acc_location: location,
    acc_navigate: navigate,
    acc_hit_test: hit_test,
    acc_do_default_action: do_default_action,
    put_acc_name: put_text,
    put_acc_value: put_text,
};

fn create_provider(hwnd: HWND, source: &'static AccessibleSource) -> *mut c_void {
    Box::into_raw(Box::new(SidebarAccessible {
        vtable: &SIDEBAR_VTABLE,
        references: AtomicU32::new(1),
        hwnd,
        source,
    }))
    .cast()
}

/// Answers `WM_GETOBJECT(OBJID_CLIENT)` for a sidebar window. Call it with nothing of the App
/// borrowed: a client may call back in while `LresultFromObject` runs.
pub(crate) unsafe fn object_result(
    hwnd: HWND,
    source: &'static AccessibleSource,
    wparam: WPARAM,
) -> LRESULT {
    let provider = create_provider(hwnd, source);
    let result = unsafe { LresultFromObject(&IID_IACCESSIBLE, wparam, provider) };
    unsafe { release(provider) };
    result
}

#[cfg(test)]
pub(crate) fn create_for_test(hwnd: HWND, source: &'static AccessibleSource) -> *mut c_void {
    create_provider(hwnd, source)
}

#[derive(Clone, Copy)]
enum Query {
    Container,
    Count,
    Item(usize),
    Hit(POINT),
    Current,
}

enum Answer {
    Container(String, u32),
    Count(usize),
    Item(Option<AccessibleItem>),
    Index(Option<usize>),
}

/// A query sent to the window's thread; the window procedure fills in `answer`.
struct Call {
    source: &'static AccessibleSource,
    query: Query,
    answer: Option<Answer>,
}

fn evaluate(source: &AccessibleSource, hwnd: HWND, query: Query) -> Answer {
    match query {
        Query::Container => {
            let (name, role) = (source.container)(hwnd);
            Answer::Container(name, role)
        }
        Query::Count => Answer::Count((source.count)(hwnd)),
        Query::Item(index) => Answer::Item((source.item)(hwnd, index)),
        Query::Hit(point) => Answer::Index((source.hit)(hwnd, point)),
        Query::Current => Answer::Index((source.current)(hwnd)),
    }
}

/// The window procedure's `WM_FASTPAD_SIDEBAR_ACCESSIBLE` handler.
///
/// `lparam` is trusted only while it is in `LIVE_CALLS`: any window (another process, or a reused
/// handle) can send this message number with anything in `lparam`. Returns 0 for an unknown one.
pub(crate) unsafe fn answer(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let address = lparam as usize;
    // A registered call's sender is blocked in `SendMessageW` until it unregisters, so it is
    // live while registered. Read and write it only under the lock that unregistering takes.
    let request = {
        let live = live_calls();
        if !live.contains(&address) {
            return 0;
        }
        let call = unsafe { &*(address as *const Call) };
        (call.source, call.query)
    };
    // Evaluated without the lock: it reads App, and a nested query must not deadlock.
    let result = evaluate(request.0, hwnd, request.1);
    let live = live_calls();
    if !live.contains(&address) {
        return 0;
    }
    unsafe { (*(address as *mut Call)).answer = Some(result) };
    1
}

/// The `Call`s whose senders are waiting in `ask`, by address.
static LIVE_CALLS: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

fn live_calls() -> std::sync::MutexGuard<'static, Vec<usize>> {
    LIVE_CALLS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The window procedure's `WM_FASTPAD_SIDEBAR_ACTION` handler.
pub(crate) fn run_action(hwnd: HWND, source: &AccessibleSource, wparam: WPARAM, lparam: LPARAM) {
    let index = wparam;
    // The children may have changed since the client asked; an index past the end is dropped.
    if index >= (source.count)(hwnd) {
        return;
    }
    if lparam == ACTION_ACTIVATE {
        (source.activate)(hwnd, index);
        return;
    }
    (source.select)(hwnd, index);
    if lparam == ACTION_FOCUS {
        unsafe {
            SetFocus(hwnd);
        }
    }
}

fn on_window_thread(hwnd: HWND) -> bool {
    unsafe { GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) == GetCurrentThreadId() }
}

unsafe fn provider<'a>(this: *mut c_void) -> &'a SidebarAccessible {
    unsafe { &*this.cast::<SidebarAccessible>() }
}

fn ask(item: &SidebarAccessible, query: Query) -> Answer {
    if item.hwnd.is_null() || on_window_thread(item.hwnd) {
        return evaluate(item.source, item.hwnd, query);
    }
    let mut call = Call {
        source: item.source,
        query,
        answer: None,
    };
    let address = &raw mut call as usize;
    live_calls().push(address);
    unsafe {
        SendMessageW(
            item.hwnd,
            WM_FASTPAD_SIDEBAR_ACCESSIBLE,
            0,
            address as LPARAM,
        );
    }
    {
        let mut live = live_calls();
        if let Some(position) = live.iter().position(|&entry| entry == address) {
            live.swap_remove(position);
        }
    }
    // A destroyed window answers nothing: report no children.
    call.answer.unwrap_or(match query {
        Query::Container => Answer::Container(String::new(), ROLE_SYSTEM_PANE),
        Query::Count => Answer::Count(0),
        Query::Item(_) => Answer::Item(None),
        Query::Hit(_) | Query::Current => Answer::Index(None),
    })
}

fn count_of(item: &SidebarAccessible) -> usize {
    match ask(item, Query::Count) {
        Answer::Count(count) => count,
        _ => 0,
    }
}

fn index_answer(item: &SidebarAccessible, query: Query) -> Option<usize> {
    match ask(item, query) {
        Answer::Index(index) => index,
        _ => None,
    }
}

/// `Some(None)` for the window itself, `Some(Some(item))` for a child, `None` for a bad ID.
fn target(item: &SidebarAccessible, child: &RawVariant) -> Option<Option<AccessibleItem>> {
    match child.child_id()? {
        0 => Some(None),
        id if id > 0 => match ask(item, Query::Item(id as usize - 1)) {
            Answer::Item(Some(found)) => Some(Some(found)),
            _ => None,
        },
        _ => None,
    }
}

fn container(item: &SidebarAccessible) -> (String, u32) {
    match ask(item, Query::Container) {
        Answer::Container(name, role) => (name, role),
        _ => (String::new(), ROLE_SYSTEM_PANE),
    }
}

/// Asks the window's own thread: `GetFocus` on an RPC thread reports that thread's empty focus.
fn has_focus(item: &SidebarAccessible) -> bool {
    if item.hwnd.is_null() {
        return false;
    }
    let thread = unsafe { GetWindowThreadProcessId(item.hwnd, std::ptr::null_mut()) };
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    thread != 0
        && unsafe { GetGUIThreadInfo(thread, &mut info) } != 0
        && info.hwndFocus == item.hwnd
}

unsafe extern "system" fn query_interface(
    this: *mut c_void,
    iid: *const GUID,
    output: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || output.is_null() {
        return E_INVALIDARG;
    }
    let requested = unsafe { *iid };
    if guid_eq(&requested, &IID_IUNKNOWN)
        || guid_eq(&requested, &IID_IDISPATCH)
        || guid_eq(&requested, &IID_IACCESSIBLE)
    {
        unsafe {
            *output = this;
            add_ref(this);
        }
        S_OK
    } else {
        unsafe { *output = std::ptr::null_mut() };
        E_NOINTERFACE
    }
}

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    unsafe { provider(this) }
        .references
        .fetch_add(1, Ordering::Relaxed)
        + 1
}

unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    let remaining = unsafe { provider(this) }
        .references
        .fetch_sub(1, Ordering::Release)
        - 1;
    if remaining == 0 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(unsafe { Box::from_raw(this.cast::<SidebarAccessible>()) });
    }
    remaining
}

unsafe extern "system" fn child_count(this: *mut c_void, count: *mut i32) -> HRESULT {
    if count.is_null() {
        return E_INVALIDARG;
    }
    let children = count_of(unsafe { provider(this) });
    unsafe { *count = i32::try_from(children).unwrap_or(i32::MAX) };
    S_OK
}

unsafe extern "system" fn child(
    this: *mut c_void,
    child: RawVariant,
    output: *mut *mut c_void,
) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    unsafe { *output = std::ptr::null_mut() };
    match child.child_id() {
        Some(id) if id > 0 && (id as usize) <= count_of(unsafe { provider(this) }) => S_FALSE,
        _ => E_INVALIDARG,
    }
}

unsafe extern "system" fn name(this: *mut c_void, child: RawVariant, output: *mut BSTR) -> HRESULT {
    let item = unsafe { provider(this) };
    match target(item, &child) {
        Some(None) => unsafe { allocate_bstr(&container(item).0, output) },
        Some(Some(found)) => unsafe { allocate_bstr(&found.name, output) },
        None => E_INVALIDARG,
    }
}

unsafe extern "system" fn value(
    this: *mut c_void,
    child: RawVariant,
    output: *mut BSTR,
) -> HRESULT {
    match target(unsafe { provider(this) }, &child) {
        Some(None) => unsafe { allocate_bstr("", output) },
        Some(Some(found)) => unsafe { allocate_bstr(&found.value, output) },
        None => E_INVALIDARG,
    }
}

unsafe extern "system" fn empty_text(
    this: *mut c_void,
    child: RawVariant,
    output: *mut BSTR,
) -> HRESULT {
    match target(unsafe { provider(this) }, &child) {
        Some(_) => unsafe { allocate_bstr("", output) },
        None => E_INVALIDARG,
    }
}

unsafe extern "system" fn role(
    this: *mut c_void,
    child: RawVariant,
    output: *mut RawVariant,
) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    let item = unsafe { provider(this) };
    let role = match target(item, &child) {
        Some(None) => container(item).1,
        Some(Some(found)) => found.role,
        None => return E_INVALIDARG,
    };
    unsafe { *output = RawVariant::integer(role as i32) };
    S_OK
}

unsafe extern "system" fn state(
    this: *mut c_void,
    child: RawVariant,
    output: *mut RawVariant,
) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    let item = unsafe { provider(this) };
    let state = match target(item, &child) {
        Some(None) => STATE_FOCUSABLE | if has_focus(item) { STATE_FOCUSED } else { 0 },
        Some(Some(found)) => found.state,
        None => return E_INVALIDARG,
    };
    unsafe { *output = RawVariant::integer(state as i32) };
    S_OK
}

unsafe extern "system" fn focus(this: *mut c_void, output: *mut RawVariant) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    let item = unsafe { provider(this) };
    if !has_focus(item) {
        unsafe { *output = RawVariant::empty() };
        return S_FALSE;
    }
    let id = index_answer(item, Query::Current).map_or(0, |index| index as i32 + 1);
    unsafe { *output = RawVariant::integer(id) };
    S_OK
}

unsafe extern "system" fn selection(this: *mut c_void, output: *mut RawVariant) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    match index_answer(unsafe { provider(this) }, Query::Current) {
        Some(index) => {
            unsafe { *output = RawVariant::integer(index as i32 + 1) };
            S_OK
        }
        None => {
            unsafe { *output = RawVariant::empty() };
            S_FALSE
        }
    }
}

unsafe extern "system" fn default_action_text(
    this: *mut c_void,
    child: RawVariant,
    output: *mut BSTR,
) -> HRESULT {
    match target(unsafe { provider(this) }, &child) {
        Some(Some(found)) => unsafe { allocate_bstr(default_action(&found), output) },
        Some(None) => unsafe { allocate_bstr("", output) },
        None => E_INVALIDARG,
    }
}

/// Posts an action for child `id`, or runs it at once in a window-less test fixture.
fn post_action(item: &SidebarAccessible, id: i32, action: LPARAM) -> HRESULT {
    if id <= 0 || id as usize > count_of(item) {
        return E_INVALIDARG;
    }
    let index = id as usize - 1;
    if item.hwnd.is_null() {
        run_action(item.hwnd, item.source, index, action);
        return S_OK;
    }
    let posted = unsafe { PostMessageW(item.hwnd, WM_FASTPAD_SIDEBAR_ACTION, index, action) };
    if posted != 0 { S_OK } else { E_INVALIDARG }
}

unsafe extern "system" fn select(this: *mut c_void, flags: i32, child: RawVariant) -> HRESULT {
    let flags = flags as u32;
    if flags & (SELFLAG_TAKEFOCUS | SELFLAG_TAKESELECTION) == 0 {
        return E_INVALIDARG;
    }
    let Some(id) = child.child_id() else {
        return E_INVALIDARG;
    };
    let action = if flags & SELFLAG_TAKEFOCUS != 0 {
        ACTION_FOCUS
    } else {
        ACTION_SELECT
    };
    post_action(unsafe { provider(this) }, id, action)
}

unsafe extern "system" fn location(
    this: *mut c_void,
    left: *mut i32,
    top: *mut i32,
    width: *mut i32,
    height: *mut i32,
    child: RawVariant,
) -> HRESULT {
    if left.is_null() || top.is_null() || width.is_null() || height.is_null() {
        return E_INVALIDARG;
    }
    let item = unsafe { provider(this) };
    let rect = match target(item, &child) {
        Some(None) => {
            let mut window = RECT::default();
            if item.hwnd.is_null() || unsafe { GetWindowRect(item.hwnd, &mut window) } == 0 {
                return S_FALSE;
            }
            window
        }
        Some(Some(found)) => {
            let mut origin = POINT { x: 0, y: 0 };
            if !item.hwnd.is_null() && unsafe { ClientToScreen(item.hwnd, &mut origin) } == 0 {
                return S_FALSE;
            }
            RECT {
                left: found.rect.left + origin.x,
                top: found.rect.top + origin.y,
                right: found.rect.right + origin.x,
                bottom: found.rect.bottom + origin.y,
            }
        }
        None => return E_INVALIDARG,
    };
    unsafe {
        *left = rect.left;
        *top = rect.top;
        *width = rect.right - rect.left;
        *height = rect.bottom - rect.top;
    }
    S_OK
}

unsafe extern "system" fn navigate(
    this: *mut c_void,
    direction: i32,
    start: RawVariant,
    output: *mut RawVariant,
) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    let Some(id) = start.child_id() else {
        return E_INVALIDARG;
    };
    let count = i32::try_from(count_of(unsafe { provider(this) })).unwrap_or(i32::MAX);
    if id < 0 || id > count {
        return E_INVALIDARG;
    }
    let target = match (direction as u32, id) {
        (NAVDIR_FIRSTCHILD, 0) if count > 0 => Some(1),
        (NAVDIR_LASTCHILD, 0) if count > 0 => Some(count),
        (NAVDIR_NEXT | NAVDIR_DOWN, value) if value > 0 && value < count => Some(value + 1),
        (NAVDIR_PREVIOUS | NAVDIR_UP, value) if value > 1 => Some(value - 1),
        (
            NAVDIR_FIRSTCHILD | NAVDIR_LASTCHILD | NAVDIR_NEXT | NAVDIR_DOWN | NAVDIR_PREVIOUS
            | NAVDIR_UP,
            _,
        ) => None,
        _ => return E_INVALIDARG,
    };
    unsafe { *output = target.map_or_else(RawVariant::empty, RawVariant::integer) };
    if target.is_some() { S_OK } else { S_FALSE }
}

unsafe extern "system" fn hit_test(
    this: *mut c_void,
    x: i32,
    y: i32,
    output: *mut RawVariant,
) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    let item = unsafe { provider(this) };
    let mut point = POINT { x, y };
    let mut client = RECT::default();
    if item.hwnd.is_null()
        || unsafe { ScreenToClient(item.hwnd, &mut point) } == 0
        || unsafe { GetClientRect(item.hwnd, &mut client) } == 0
        || point.x < client.left
        || point.x >= client.right
        || point.y < client.top
        || point.y >= client.bottom
    {
        unsafe { *output = RawVariant::empty() };
        return S_FALSE;
    }
    let id = index_answer(item, Query::Hit(point)).map_or(0, |index| index as i32 + 1);
    unsafe { *output = RawVariant::integer(id) };
    S_OK
}

unsafe extern "system" fn do_default_action(this: *mut c_void, child: RawVariant) -> HRESULT {
    let Some(id) = child.child_id() else {
        return E_INVALIDARG;
    };
    post_action(unsafe { provider(this) }, id, ACTION_ACTIVATE)
}

unsafe extern "system" fn put_text(
    _this: *mut c_void,
    _child: RawVariant,
    _value: BSTR,
) -> HRESULT {
    E_NOTIMPL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tree::{RowKind, TreeRow};
    use crate::window::row_list::RowListState;
    use std::cell::Cell;
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{SysFreeString, SysStringLen};

    thread_local! {
        static ITEMS_BUILT: Cell<usize> = const { Cell::new(0) };
    }

    const ROW: RECT = RECT {
        left: 0,
        top: 0,
        right: 100,
        bottom: 26,
    };

    fn fake_container(_: HWND) -> (String, u32) {
        ("Fake notes".to_owned(), ROLE_SYSTEM_LIST)
    }
    fn fake_count(_: HWND) -> usize {
        10_000
    }
    fn fake_item(_: HWND, index: usize) -> Option<AccessibleItem> {
        ITEMS_BUILT.set(ITEMS_BUILT.get() + 1);
        (index < 10_000).then(|| list_item(&format!("Note {index}"), index == 3, false, ROW, true))
    }
    fn fake_hit(_: HWND, _: POINT) -> Option<usize> {
        Some(7)
    }
    fn fake_current(_: HWND) -> Option<usize> {
        Some(3)
    }
    fn fake_select(_: HWND, _: usize) {}
    fn fake_activate(_: HWND, _: usize) {}
    fn fake_identity(_: HWND, _: usize) -> Option<u64> {
        None
    }
    fn fake_generation(_: HWND) -> u64 {
        0
    }

    static FAKE: AccessibleSource = AccessibleSource {
        container: fake_container,
        count: fake_count,
        item: fake_item,
        hit: fake_hit,
        current: fake_current,
        select: fake_select,
        activate: fake_activate,
        identity: fake_identity,
        generation: fake_generation,
    };

    fn read_bstr(value: BSTR) -> String {
        let text = unsafe { std::slice::from_raw_parts(value, SysStringLen(value) as usize) };
        let result = String::from_utf16_lossy(text);
        unsafe { SysFreeString(value) };
        result
    }

    fn row(kind: RowKind, name: &str, depth: u16, pinned: bool, expanded: bool) -> TreeRow {
        TreeRow {
            kind,
            depth,
            name: name.to_owned(),
            pinned,
            expanded,
        }
    }

    #[test]
    fn a_ten_thousand_row_list_is_counted_without_building_its_items() {
        // Break caught: accChildCount building every row's name, making each screen-reader call
        // cost O(rows) on a 10,000-note notebook.
        let provider = create_provider(std::ptr::null_mut(), &FAKE);
        ITEMS_BUILT.set(0);
        let mut count = 0;
        unsafe {
            assert_eq!(
                (SIDEBAR_VTABLE.get_acc_child_count)(provider, &mut count),
                S_OK
            );
        }
        assert_eq!(count, 10_000);
        assert_eq!(ITEMS_BUILT.get(), 0);
        let mut name: BSTR = std::ptr::null();
        unsafe {
            assert_eq!(
                (SIDEBAR_VTABLE.get_acc_name)(provider, RawVariant::integer(10_000), &mut name),
                S_OK
            );
        }
        assert_eq!(read_bstr(name), "Note 9999");
        assert_eq!(ITEMS_BUILT.get(), 1);
        unsafe {
            assert_eq!(
                (SIDEBAR_VTABLE.get_acc_name)(provider, RawVariant::integer(10_001), &mut name),
                E_INVALIDARG
            );
            (SIDEBAR_VTABLE.release)(provider);
        }
    }

    #[test]
    fn the_container_and_children_report_their_roles_states_and_selection() {
        // Break caught: the list announced as a generic client area, or the selected row not
        // reported through accSelection.
        let provider = create_provider(std::ptr::null_mut(), &FAKE);
        let table = &SIDEBAR_VTABLE;
        unsafe {
            let mut value = RawVariant::empty();
            assert_eq!(
                (table.get_acc_role)(provider, RawVariant::integer(0), &mut value),
                S_OK
            );
            assert_eq!(value.child_id(), Some(ROLE_SYSTEM_LIST as i32));
            assert_eq!(
                (table.get_acc_role)(provider, RawVariant::integer(4), &mut value),
                S_OK
            );
            assert_eq!(value.child_id(), Some(ROLE_SYSTEM_LISTITEM as i32));
            assert_eq!(
                (table.get_acc_state)(provider, RawVariant::integer(4), &mut value),
                S_OK
            );
            let state = value.child_id().unwrap() as u32;
            assert_ne!(state & STATE_SELECTED, 0);
            assert_ne!(state & STATE_SELECTABLE, 0);
            assert_eq!((table.get_acc_selection)(provider, &mut value), S_OK);
            assert_eq!(value.child_id(), Some(4));
            // Nothing has the focus in a window-less fixture.
            assert_eq!((table.get_acc_focus)(provider, &mut value), S_FALSE);
            let mut name: BSTR = std::ptr::null();
            assert_eq!(
                (table.get_acc_name)(provider, RawVariant::integer(0), &mut name),
                S_OK
            );
            assert_eq!(read_bstr(name), "Fake notes");
            let mut next = RawVariant::empty();
            assert_eq!(
                (table.acc_navigate)(
                    provider,
                    NAVDIR_NEXT as i32,
                    RawVariant::integer(1),
                    &mut next
                ),
                S_OK
            );
            assert_eq!(next.child_id(), Some(2));
            assert_eq!(
                (table.acc_navigate)(
                    provider,
                    NAVDIR_NEXT as i32,
                    RawVariant::integer(10_000),
                    &mut next
                ),
                S_FALSE
            );
            (table.release)(provider);
        }
    }

    #[test]
    fn tree_rows_are_outline_items_with_expansion_pin_and_unsaved_in_their_names() {
        // Break caught: a folder's expanded state missing, a pin conveyed only by the filled
        // icon, or an unsaved tab's row indistinguishable from a saved note.
        let folder = tree_item(
            &row(
                RowKind::Folder(PathBuf::from("Work")),
                "Work",
                0,
                false,
                true,
            ),
            false,
            false,
            ROW,
            true,
        );
        assert_eq!(folder.role, ROLE_SYSTEM_OUTLINEITEM);
        assert_ne!(folder.state & STATE_EXPANDED, 0);
        assert_eq!(folder.state & STATE_COLLAPSED, 0);
        assert_eq!(folder.value, "0");
        let closed = tree_item(
            &row(
                RowKind::Folder(PathBuf::from("Old")),
                "Old",
                1,
                false,
                false,
            ),
            false,
            false,
            ROW,
            true,
        );
        assert_ne!(closed.state & STATE_COLLAPSED, 0);
        assert_eq!(closed.value, "1");

        let pinned = tree_item(
            &row(RowKind::Note(PathBuf::from("a.md")), "a", 1, true, false),
            true,
            true,
            ROW,
            false,
        );
        assert_eq!(pinned.name, "a, pinned");
        assert_eq!(pinned.state & (STATE_EXPANDED | STATE_COLLAPSED), 0);
        assert_ne!(pinned.state & STATE_SELECTED, 0);
        assert_ne!(pinned.state & STATE_FOCUSED, 0);
        assert_ne!(pinned.state & STATE_OFFSCREEN, 0);

        let unsaved = tree_item(
            &row(RowKind::Unsaved(7), "Groceries", 0, false, false),
            false,
            true,
            ROW,
            true,
        );
        assert_eq!(unsaved.name, "Groceries, unsaved");
        assert_eq!(
            unsaved.state & STATE_FOCUSED,
            0,
            "focus follows selection only"
        );
    }

    #[test]
    fn rows_scrolled_out_of_the_list_are_offscreen() {
        // Break caught: a screen reader told that row 5,000 sits at the top of the list.
        let mut list = RowListState::new(26);
        list.set_count(100);
        list.top = 10;
        let area = RECT {
            left: 0,
            top: 38,
            right: 200,
            bottom: 38 + 26 * 5,
        };
        let (rect, visible) = row_rect(area, &list, 10);
        assert_eq!((rect.top, rect.bottom, visible), (38, 64, true));
        assert!(!row_rect(area, &list, 9).1);
        assert!(!row_rect(area, &list, 15).1);
        assert_eq!(row_rect(area, &list, 12).0.top, 38 + 52);
    }

    #[test]
    fn events_announce_selection_focus_state_and_reorders() {
        // Break caught: no event when the selection moves, so a screen reader keeps reading the
        // old row, or no state change when a folder expands or a note is pinned in place.
        let mark = |current, state, name: &str, count| AccessibleMark {
            current,
            state,
            name: name.to_owned(),
            count,
            ..AccessibleMark::default()
        };
        assert_eq!(
            events_between(&mark(Some(1), 0, "a", 5), &mark(Some(2), 0, "b", 5), true),
            vec![(EVENT_OBJECT_SELECTION, 3), (EVENT_OBJECT_FOCUS, 3)]
        );
        assert_eq!(
            events_between(&mark(Some(1), 0, "a", 5), &mark(Some(2), 0, "b", 5), false),
            vec![(EVENT_OBJECT_SELECTION, 3)]
        );
        assert_eq!(
            events_between(
                &mark(Some(0), STATE_COLLAPSED, "Work", 5),
                &mark(Some(0), STATE_EXPANDED, "Work", 9),
                true
            ),
            vec![(EVENT_OBJECT_REORDER, 0), (EVENT_OBJECT_STATECHANGE, 1)]
        );
        assert_eq!(
            events_between(
                &mark(Some(0), 0, "a", 5),
                &mark(Some(0), 0, "a, pinned", 5),
                true
            ),
            vec![(EVENT_OBJECT_STATECHANGE, 1), (EVENT_OBJECT_NAMECHANGE, 1)]
        );
        assert!(events_between(&mark(None, 0, "", 0), &mark(None, 0, "", 0), true).is_empty());
    }

    #[test]
    fn a_pin_that_re_sorts_the_row_raises_reorder_selection_and_state_change() {
        // Break caught: pinning a note that is not first moves it to the top at the same count,
        // announcing only a new selection, with no state change and no reorder of its siblings.
        let before = AccessibleMark {
            current: Some(3),
            name: "b".to_owned(),
            count: 5,
            identity: Some(7),
            generation: 1,
            ..AccessibleMark::default()
        };
        let after = AccessibleMark {
            current: Some(0),
            name: "b, pinned".to_owned(),
            generation: 2,
            ..before.clone()
        };
        assert_eq!(
            events_between(&before, &after, true),
            vec![
                (EVENT_OBJECT_REORDER, 0),
                (EVENT_OBJECT_SELECTION, 1),
                (EVENT_OBJECT_FOCUS, 1),
                (EVENT_OBJECT_STATECHANGE, 1),
                (EVENT_OBJECT_NAMECHANGE, 1),
            ]
        );
        // Another row now at the same index is a new selection, not a state change.
        let other = AccessibleMark {
            identity: Some(8),
            ..before.clone()
        };
        assert_eq!(
            events_between(&before, &other, false),
            vec![(EVENT_OBJECT_SELECTION, 4)]
        );
    }

    #[test]
    fn a_query_message_with_an_unknown_pointer_is_ignored() {
        // Break caught: any process sending WM_APP + 0x60 with a junk lParam crashing FastPad by
        // having it written through as a `Call`.
        assert_eq!(unsafe { answer(std::ptr::null_mut(), 0x10) }, 0);
        assert_eq!(unsafe { answer(std::ptr::null_mut(), -1) }, 0);
    }

    #[test]
    fn default_actions_follow_the_item_kind() {
        // Break caught: a folder offering "Open", or a button offering nothing to a screen
        // reader's default-action command.
        let button = button_item("Search", true, false, ROW);
        assert_eq!(default_action(&button), "Press");
        assert_ne!(button.state & STATE_PRESSED, 0);
        let folder = tree_item(
            &row(RowKind::Folder(PathBuf::from("w")), "w", 0, false, false),
            false,
            false,
            ROW,
            true,
        );
        assert_eq!(default_action(&folder), "Expand");
        let note = list_item("n", false, false, ROW, true);
        assert_eq!(default_action(&note), "Open");
    }
}
