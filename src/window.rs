use crate::{
    bookmarks::{MANAGER_URL, Request as BookmarkRequest},
    chrome::{self, ButtonKind},
    download_guard, i18n,
    keyboard::{self, Shortcut},
    memory_sites::{self, SiteExceptions},
    navigation::{self, HOME, SearchEngine},
    panel_motion::Motion,
    popover::Anchor,
    permissions,
    session::{self, Entry, Session, SplitPlacement, WindowPlacement, WindowState},
    site_setup::{self, SourceBrowser},
    tab_memory, vault_ui,
};
use cef::*;
use crate::exact_model::{self, Action, ActionKind, Item, Section, Surface, SETTINGS_CONTROL};
use crate::exact_ui;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    time::{Duration, Instant, SystemTime},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::{
        COLOR_HIGHLIGHT, CreateFontW, DEFAULT_GUI_FONT, DeleteObject, FillRect, GetMonitorInfoW,
        GetStockObject, GetSysColorBrush, HDC, HFONT, HMONITOR, InvalidateRect,
        MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect, MonitorFromWindow, OffsetRect,
        RDW_ALLCHILDREN, RDW_INVALIDATE, RedrawWindow, ScreenToClient, UpdateWindow,
    },
    System::{
        DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
        LibraryLoader::GetModuleHandleW,
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        SystemServices::SS_CENTERIMAGE,
    },
    UI::{
        Controls::*,
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, GetSystemMetricsForDpi,
            SetThreadDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            EnableWindow, GetActiveWindow, GetCapture, GetFocus, GetKeyState, IsWindowEnabled,
            ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_DELETE, VK_DOWN, VK_ESCAPE,
            VK_LEFT, VK_MENU, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
        },
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const MIN_WINDOW_WIDTH: i32 = 530;
const MIN_WINDOW_HEIGHT: i32 = 340;

pub(crate) const ADDRESS: usize = 101;
const BACK: usize = 102;
const FORWARD: usize = 103;
const RELOAD: usize = 104;
const NEW_TAB: usize = 105;
const CLOSE_TAB: usize = 106;
const GO: usize = 107;
const TABS: usize = 108;
const SIDEBAR: usize = 109;
pub(crate) const FILTER: usize = 110;
const TAB_LABEL: usize = 111;
const EMPTY: usize = 112;
const FOCUS_ADDRESS: usize = 113;
const FOCUS_FILTER: usize = 114;
const NEXT_TAB: usize = 115;
const PREVIOUS_TAB: usize = 116;
const MENU: usize = 117;
const HISTORY: usize = 119;
const BOOKMARKS: usize = 120;
const DOWNLOADS: usize = 121;
const SETTINGS: usize = 122;
const EXTENSIONS: usize = 123;
const FIND: usize = 124;
pub(crate) const FIND_TEXT: usize = 125;
const FIND_STATUS: usize = 126;
const FIND_NEXT: usize = 127;
const FIND_PREVIOUS: usize = 128;
const FIND_CLOSE: usize = 129;
const PRINT: usize = 133;
const RESTORE_TAB: usize = 136;
const PASSWORDS: usize = 137;
const UPDATE: usize = 138;
const MATERIAL: usize = 139;
const HARD_RELOAD: usize = 140;
const FOCUS_SEARCH: usize = 141;
const FULLSCREEN: usize = 142;
const FOCUS_FIND_NEXT: usize = 143;
const FOCUS_FIND_PREV: usize = 144;
const TOGGLE_RESTORE: usize = 145;
const SITE_PERMISSIONS: usize = 146;
const CLEAR_BROWSING: usize = 147;
const ZOOM_IN: usize = 148;
const ZOOM_OUT: usize = 149;
const ZOOM_RESET: usize = 150;
const SELECT_TAB_FIRST: usize = 151;
const SELECT_TAB_LAST: usize = 159;
const SAVE_BOOKMARK: usize = 160;
const NEW_WINDOW: usize = 161;
const PRIVATE_WINDOW: usize = 162;
const DEVTOOLS: usize = 163;
const DEVTOOLS_CONSOLE: usize = 164;
const CLOSE_WINDOW: usize = 165;
const SITE_INFO: usize = 166;
const FREE_MEMORY: usize = 167;
const TOGGLE_DEFER_BACKGROUND: usize = 168;
const RELOAD_SHORTCUT: usize = 169;
const EXIT_FLIT: usize = 172;
const RECOVERY_COPY: usize = 173;
const IMPORT_BOOKMARKS: usize = 174;
const EXPORT_BOOKMARKS: usize = 175;
const SEARCH_ENGINE_FIRST: usize = 176;
const SEARCH_ENGINE_LAST: usize = SEARCH_ENGINE_FIRST + SearchEngine::ALL.len() - 1;
const GO_NEW_TAB: usize = 180;
const IMPORT_PASSWORDS: usize = 181;
const PREPARE_CHROME: usize = 182;
const PREPARE_EDGE: usize = 183;
const PREPARE_CLIPBOARD: usize = 184;
const IMPORT_CHROME_BOOKMARKS: usize = 185;
const IMPORT_EDGE_BOOKMARKS: usize = 186;
const REOPEN_RECENT: usize = 187;
pub(crate) const TAB_STATUS: usize = 188;
const BOOKMARK_ALL_TABS: usize = 189;
const DOWNLOADS_BUTTON: usize = 190;
const FREE_MEMORY_BUTTON: usize = 191;
const DISPLAY_LANGUAGE: usize = 192;
const DOWNLOAD_PREFERENCES: usize = 193;
const BOOKMARK_LIST: usize = 194;
const BOOKMARK_HEADER: usize = 195;
const AGENT_LIST: usize = 196;
const AGENT_HEADER: usize = 197;
const TOGGLE_AGENT_AUTO_CLOSE: usize = 198;
const CHROMIUM_SETTINGS: usize = 199;
const EXTENSIONS_BUTTON: usize = 200;
const MEMORY_SAVER_FIRST: usize = 201;
const MEMORY_SAVER_CHOICES: [u32; 4] = [0, 15, 30, 60];
const PAGE_CHANGED: u32 = WM_APP + 1;
const PAGE_CLOSED: u32 = WM_APP + 2;
const CLOSE_BY_KEY: u32 = WM_APP + 3;
const OPEN_LINK: u32 = WM_APP + 4;
const TAB_MENU_ACTION: u32 = WM_APP + 5;
const TAB_CLOSE_CANCELLED: u32 = WM_APP + 6;
const POPUP_CLOSED: u32 = WM_APP + 7;
const OPEN_SETTINGS_WINDOW: u32 = WM_APP + 8;
const LOAD_STATE_CHANGED: u32 = WM_APP + 9;
const STOP_LOADING: u32 = WM_APP + 10;
const EXIT_NEXT_WINDOW: u32 = WM_APP + 11;
const TAB_DOWNLOAD_ENDED: u32 = WM_APP + 12;
const LOAD_RESTORED_WINDOW: u32 = WM_APP + 13;
const SELECT_POPUP_TAB: u32 = WM_APP + 14;
const DOWNLOADS_CHANGED: u32 = WM_APP + 15;
const VAULT_OFFER: u32 = WM_APP + 16;
const AUDIBLE_CHANGED: u32 = WM_APP + 17;
const MOUNT_START: u32 = WM_APP + 19;
const ADDRESS_SUBMIT: u32 = WM_APP + 20;
const SPLIT_FOCUS: u32 = WM_APP + 21;
const START_CONTROL: usize = 212;
const START_LINK: usize = 5 << 32;
const TAB_PIN: usize = 1;
const TAB_DUPLICATE: usize = 2;
const TAB_MUTE: usize = 3;
const TAB_CLOSE: usize = 4;
const TAB_UNLOAD: usize = 5;
const TAB_KEEP_SITE: usize = 6;
const TAB_MOVE_UP: usize = 7;
const TAB_MOVE_DOWN: usize = 8;
const TAB_CANCEL_DEFERRED_CLOSE: usize = 9;
const TAB_SPLIT_WITH: usize = 11;
const TAB_COPY_LINK: usize = 10;
const WINDOW_CLASS: &str = "FlitBrowserWindow";
const APP_TITLE: &str = "Flit Browser";
const PRIVATE_TITLE: &str = "Flit Browser — Private";
const ICON_ID: usize = 32512;
const SIDEBAR_TIMER: usize = 1;
const RECOVERY_TIMER: usize = 2;
const TAB_MEMORY_TIMER: usize = 3;
const ZOOM_LABEL_TIMER: usize = 4;
const MEMORY_SAVER_TIMER: usize = 5;
const HOVER_TIMER: usize = 6;
const ADDRESS_TIMER: usize = 7;
const DOWNLOAD_MENU_TIMER: usize = 8;
const ADDRESS_LIMIT: usize = 6;
/// Theme choices on the settings page: system, light, dark.
const THEME_FIRST: usize = 206;
const THEME_LAST: usize = 208;
/// The thin scroll thumb at the sidebar's right edge.
const SIDEBAR_THUMB: usize = 209;
pub(crate) const OPEN_COPIED_TABS: usize = 210;
const SPLIT_VIEW: usize = 213;
const SPLIT_DIVIDER: usize = 214;
const VAULT_BUTTON: usize = 215;
const FLIT_SETTINGS: &str = "flit://settings";
const SETTINGS_URL: &str = "chrome://settings/";
const CLEAR_DATA_URL: &str = "chrome://settings/clearBrowserData";
const DOWNLOADS_URL: &str = "chrome://downloads/";

#[derive(Clone, Copy)]
struct Controls {
    window: HWND,
    address: HWND,
    site_info: HWND,
    back: HWND,
    forward: HWND,
    reload: HWND,
    add: HWND,
    go: HWND,
    tabs: HWND,
    sidebar: HWND,
    filter: HWND,
    label: HWND,
    tab_status: HWND,
    free_memory: HWND,
    empty: HWND,
    menu: HWND,
    downloads: HWND,
    extensions: HWND,
    vault: HWND,
    bookmark: HWND,
    find_text: HWND,
    find_status: HWND,
    find_next: HWND,
    find_previous: HWND,
    find_close: HWND,
    bookmarks: HWND,
    bookmark_header: HWND,
    agents: HWND,
    agent_header: HWND,
    scroll_thumb: HWND,
    divider: HWND,
}

impl Controls {
    fn chrome(self) -> [HWND; 23] {
        [
            self.sidebar,
            self.extensions,
            self.back,
            self.forward,
            self.reload,
            self.site_info,
            self.address,
            self.go,
            self.bookmark,
            self.downloads,
            self.vault,
            self.menu,
            self.add,
            self.filter,
            self.tabs,
            self.free_memory,
            self.label,
            self.bookmark_header,
            self.bookmarks,
            self.agent_header,
            self.agents,
            self.scroll_thumb,
            self.divider,
        ]
    }
    fn material_toolbar(self) -> [HWND; 8] {
        [
            self.sidebar, self.back, self.forward, self.reload,
            self.extensions, self.downloads, self.vault, self.menu,
        ]
    }

    fn find(self) -> [HWND; 5] {
        [
            self.find_text,
            self.find_status,
            self.find_previous,
            self.find_next,
            self.find_close,
        ]
    }

    fn tab_stops(self) -> impl Iterator<Item = HWND> {
        let sidebar_visible = unsafe { IsWindowVisible(self.tabs) } != 0;
        [
            self.sidebar,
            self.back,
            self.forward,
            self.reload,
            self.site_info,
            self.address,
            self.bookmark,
            self.go,
            if sidebar_visible { null_mut() } else { self.add },
            self.extensions,
            self.downloads,
            self.menu,
            self.find_text,
            self.find_previous,
            self.find_next,
            self.find_close,
            if sidebar_visible { self.add } else { null_mut() },
            self.filter,
            self.bookmark_header,
            self.bookmarks,
            self.tabs,
            self.agent_header,
            self.agents,
            self.free_memory,
            self.divider,
        ]
        .into_iter()
        .filter(|&item| unsafe { IsWindowVisible(item) != 0 && IsWindowEnabled(item) != 0 })
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Row {
    key: usize,
    label: String,
    /// The tab's site icon (an HBITMAP), 0 for none.
    icon: usize,
    pinned: bool,
    muted: bool,
    unloaded: bool,
}

/// One visible line of the sidebar's bookmark section.
#[derive(Clone, PartialEq, Eq)]
struct BookmarkRow {
    title: String,
    url: String,
    depth: usize,
}

enum AddressTarget {
    Tab(usize),
    Bookmark(String),
}

struct AddressSuggestion {
    id: usize,
    title: String,
    target: AddressTarget,
}

struct AddressSuggestions {
    owner: HWND,
    input: String,
    items: Vec<AddressSuggestion>,
    selected: Option<usize>,
}

#[derive(Clone, Copy)]
struct MemoryPreview {
    key: usize,
    browser_id: Option<i32>,
    pending: Option<(i32, Instant)>,
    mib: Option<i32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FilterCursor {
    Off,
    First,
    Tab(usize),
}

impl FilterCursor {
    fn index(self, rows: &[Row]) -> Option<usize> {
        if rows.is_empty() {
            return None;
        }
        match self {
            Self::Off => None,
            Self::First => Some(0),
            Self::Tab(key) => Some(rows.iter().position(|row| row.key == key).unwrap_or(0)),
        }
    }

    fn highlight(self, rows: &[Row], active: Option<usize>) -> Option<usize> {
        self.index(rows)
            .or_else(|| rows.iter().position(|row| Some(row.key) == active))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TabDrop {
    key: usize,
    after: bool,
}

#[derive(Clone, Copy)]
struct TabDrag {
    key: usize,
    start_y: i32,
    dragging: bool,
    target: Option<TabDrop>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DownloadClose {
    None,
    Asking,
    Waiting,
}

struct Tab {
    key: usize,
    return_tab: Option<usize>,
    background_warm: bool,
    panel: HWND,
    browser: Option<Browser>,
    native_window: Option<Window>,
    native_close_sent: bool,
    close_requested: bool,
    download_close: DownloadClose,
    retiring: bool,
    url: String,
    title: String,
    has_video_access: bool,
    has_audio_access: bool,
    back: bool,
    forward: bool,
    loading: bool,
    pinned: bool,
    muted: bool,
    unloading: bool,
    unloaded: bool,
    bookmark: Option<BookmarkRequest>,
    /// Original saved address for a page hosted by a sidebar bookmark.
    bookmark_url: Option<String>,
    /// Set for a tab an AI (MCP) session opened and drives.
    agent: Option<u32>,
    favicon: Option<chrome::Favicon>,
    start_links: Vec<(usize, String)>,
    /// When the tab last stopped being the selected one (Memory Saver).
    hidden_since: Instant,
    /// The page is playing sound; Memory Saver leaves it alone.
    audible: bool,
}

enum ClosedItem {
    Tab {
        owner: usize,
        index: usize,
        entry: Entry,
        muted: bool,
    },
    Window(WindowState),
}

enum MemoryRelease {
    Manual(usize),
    Automatic(usize),
}

#[derive(Clone, Copy)]
struct PageSplit {
    left: usize,
    right: usize,
    ratio: u16,
}

impl PageSplit {
    fn other(self, key: usize) -> Option<usize> {
        if key == self.left {
            Some(self.right)
        } else if key == self.right {
            Some(self.left)
        } else {
            None
        }
    }

    fn bounds(self, layout: &Layout) -> Option<[RECT; 3]> {
        let page = layout.page_bounds();
        let width = page.right - page.left;
        if width < 3 {
            return None;
        }
        let gap = layout.scale(10).clamp(1, width - 2);
        let remaining = width - gap;
        let minimum = layout.scale(140).clamp(1, remaining / 2);
        let left_width =
            (remaining * i32::from(self.ratio) / 1000).clamp(minimum, remaining - minimum);
        let x = page.left + left_width;
        Some([
            RECT { right: x, ..page },
            RECT {
                left: x,
                right: x + gap,
                ..page
            },
            RECT {
                left: x + gap,
                ..page
            },
        ])
    }
}

struct PendingLogin {
    owner: usize,
    key: usize,
    sequence: u32,
    origin: String,
    user: String,
    password: String,
}

struct ClosedRecord {
    id: usize,
    item: ClosedItem,
}

struct PendingLink {
    url: String,
    disposition: WindowOpenDisposition,
    source: Option<usize>,
}

#[derive(Clone, Copy)]
struct PopupTab {
    opener: usize,
    id: i32,
    foreground: bool,
}

struct PageChanges {
    active: bool,
    save: bool,
}

struct Shell {
    controls: Controls,
    id: usize,
    tabs: Vec<Tab>,
    active: Option<usize>,
    split: Option<PageSplit>,
    next_key: usize,
    sidebar_open: bool,
    sidebar_motion: Motion,
    tooltip: HWND,
    reload_is_stop: bool,
    active_downloads: usize,
    filter: String,
    filter_cursor: FilterCursor,
    rows: Vec<Row>,
    page_changes: Option<PageChanges>,
    page_counts: (usize, usize),
    deferred_restore: bool,
    font: HFONT,
    font_dpi: i32,
    find_open: bool,
    minimized: bool,
    fullscreen: Option<(WINDOWPLACEMENT, Option<WindowPlacement>)>,
    recently_closed: Vec<ClosedRecord>,
    private: bool,
    address_edited: bool,
    updating_address: bool,
    row_close: Option<(usize, u32)>,
    row_unload: Option<usize>,
    hovered_row: Option<usize>,
    /// How far the whole sidebar (bookmarks, tabs, AI tabs) is scrolled, in pixels.
    side_scroll: i32,
    /// The active tab the sidebar last scrolled into view.
    side_active: Option<usize>,
    /// Imported tab to reveal after leaving fullscreen.
    pending_tab_reveal: Option<usize>,
    memory_preview: Option<MemoryPreview>,
    memory_request: i32,
    tab_drag: Option<TabDrag>,
    links: VecDeque<PendingLink>,
    popups: Vec<Browser>,
    pending_popups: BTreeMap<(usize, i32), Option<usize>>,
    settings_window: Option<Browser>,
    pending_settings: Option<(usize, String)>,
    closing: bool,
    closing_session: Option<WindowState>,
    memory_queue: VecDeque<MemoryRelease>,
    /// The sidebar's AI tab group is expanded.
    agent_open: bool,
    bookmarks_open: bool,
    agent_rows: Vec<Row>,
    bookmark_rows: Vec<BookmarkRow>,
}

impl Shell {
    fn bookmark_tab(&self, url: &str) -> Option<&Tab> {
        self.tabs.iter().find(|tab| !tab.retiring && tab.bookmark_url.as_deref() == Some(url))
    }

    fn visible_partner(&self) -> Option<usize> {
        self.split?.other(self.active?)
    }

    fn page_visible(&self, key: usize) -> bool {
        Some(key) == self.active || Some(key) == self.visible_partner()
    }
}

thread_local! {
    static WINDOWS: RefCell<BTreeMap<usize, Shell>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT_WINDOW_ID: Cell<usize> = const { Cell::new(1) };
    static RECENT_NORMAL: RefCell<Vec<ClosedRecord>> = const { RefCell::new(Vec::new()) };
    static NEXT_CLOSED_ID: Cell<usize> = const { Cell::new(1) };
    static CURRENT_WINDOW: Cell<usize> = const { Cell::new(0) };
    static LAST_NORMAL: Cell<usize> = const { Cell::new(0) };
    static PROFILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static PRIVATE_CONTEXT: RefCell<Option<RequestContext>> = const { RefCell::new(None) };
    static RESTORE_TABS: Cell<bool> = const { Cell::new(true) };
    static SIDEBAR_DEFAULT: Cell<bool> = const { Cell::new(true) };
    static DEFER_BACKGROUND: Cell<bool> = const { Cell::new(true) };
    static SEARCH_ENGINE: Cell<SearchEngine> = const { Cell::new(SearchEngine::DuckDuckGo) };
    static MEMORY_SITES: RefCell<SiteExceptions> = RefCell::new(SiteExceptions::default());
    static RECOVERY_WINDOW: Cell<usize> = const { Cell::new(0) };
    static OPENING_SESSION: Cell<bool> = const { Cell::new(false) };
    static EXIT_SESSION: RefCell<Option<ExitSession>> = const { RefCell::new(None) };
    static NATIVE_EXIT: Cell<bool> = const { Cell::new(false) };
    static EXIT_CODE: Cell<i32> = const { Cell::new(0) };
    /// Started by an MCP agent rather than by the user.
    static AGENT_LAUNCH: Cell<bool> = const { Cell::new(false) };
    /// The user activated a Flit window since startup.
    static USER_ACTIVATED: Cell<bool> = const { Cell::new(false) };
    static MEMORY_SAVER_MINUTES: Cell<u32> = const { Cell::new(30) };
    static SIDEBAR_BOOKMARKS: RefCell<Vec<BookmarkRow>> = const { RefCell::new(Vec::new()) };
    static ADDRESS_SUGGESTIONS: RefCell<Option<AddressSuggestions>> = const { RefCell::new(None) };
    static ADDRESS_ACTION_ID: Cell<usize> = const { Cell::new(1) };
    static DOWNLOAD_TIMER_OWNER: Cell<HWND> = const { Cell::new(null_mut()) };
    static BOOKMARKS_STAMP: Cell<Option<std::time::SystemTime>> = const { Cell::new(None) };
    /// A login the user just submitted: (origin, username, password).
    static PENDING_LOGIN: RefCell<Option<PendingLogin>> = const { RefCell::new(None) };
    static LOGIN_SEQUENCE: Cell<u32> = const { Cell::new(0) };
    static START_LINK_SEQUENCE: Cell<u32> = const { Cell::new(0) };
    static PANEL_VAULT_ORIGIN: RefCell<Option<String>> = const { RefCell::new(None) };
    static EXTENSION_CHOICES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

struct ExitSession {
    current: usize,
    remaining: VecDeque<usize>,
}

fn exiting() -> bool {
    NATIVE_EXIT.get() || EXIT_SESSION.with(|slot| slot.borrow().is_some())
}

struct WindowScope(usize);

impl WindowScope {
    fn enter(window: usize) -> Self {
        Self(CURRENT_WINDOW.replace(window))
    }
}

impl Drop for WindowScope {
    fn drop(&mut self) {
        CURRENT_WINDOW.set(self.0);
    }
}

pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn with_shell<T>(f: impl FnOnce(&Shell) -> T) -> Option<T> {
    WINDOWS.with(|windows| windows.borrow().get(&CURRENT_WINDOW.get()).map(f))
}

fn change<T>(f: impl FnOnce(&mut Shell) -> T) -> Option<T> {
    WINDOWS.with(|windows| windows.borrow_mut().get_mut(&CURRENT_WINDOW.get()).map(f))
}

pub(crate) fn private() -> bool {
    with_shell(|s| s.private).unwrap_or(false)
}

fn download_window_private(window: usize) -> bool {
    WINDOWS.with_borrow(|windows| windows.get(&window).is_none_or(|s| s.private))
}

fn download_handler(window: usize, key: usize) -> DownloadHandler {
    download_guard::handler(window, key, download_window_private(window))
}

fn app_title() -> &'static str {
    if private() { i18n::text(PRIVATE_TITLE) } else { APP_TITLE }
}

fn request_context() -> Result<Option<RequestContext>, &'static str> {
    if !private() {
        return Ok(None);
    }
    if let Some(context) = PRIVATE_CONTEXT.with(|slot| slot.borrow().clone()) {
        return Ok(Some(context));
    }
    let context = request_context_create_context(Some(&RequestContextSettings::default()), None)
        .filter(|context| {
            context.is_global() == 0
                && CefString::from(&context.cache_path())
                    .to_string()
                    .is_empty()
        })
        .ok_or("Could not create the private browsing context.")?;
    PRIVATE_CONTEXT.with(|slot| slot.replace(Some(context.clone())));
    Ok(Some(context))
}

fn update_page(key: usize, save: bool, f: impl FnOnce(&mut Tab) -> bool) {
    change(|s| {
        let Some(tab) = s.tabs.iter_mut().find(|tab| tab.key == key) else {
            return;
        };
        if !f(tab) {
            return;
        }
        let active = s.active == Some(key);
        if let Some(pending) = &mut s.page_changes {
            pending.active |= active;
            pending.save |= save;
        } else if unsafe { PostMessageW(s.controls.window, PAGE_CHANGED, 0, 0) } != 0 {
            s.page_changes = Some(PageChanges { active, save });
        }
    });
}

fn browser_closed(window: usize, message: u32, key: usize) {
    unsafe {
        PostMessageW(window as HWND, message, key, 0);
    }
}

fn queue_session_save() {
    if RESTORE_TABS.get() {
        queue_profile_save();
    }
}

fn queue_profile_save() {
    if OPENING_SESSION.get()
        || exiting()
        || RECOVERY_WINDOW.get() != 0
        || with_shell(|s| !s.private && !s.closing) != Some(true)
    {
        return;
    }
    if let Some(window) = normal_window() {
        unsafe {
            if SetTimer(window, RECOVERY_TIMER, 700, None) != 0 {
                RECOVERY_WINDOW.set(window as usize);
            }
        }
    }
}

wrap_client! {
    struct FlitDevToolsClient;
    impl Client {}
}

wrap_client! {
    struct FlitSettingsClient { window: usize }
    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(FlitSettingsLife::new(self.window))
        }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> {
            Some(FlitUnload::new(self.window, 0))
        }
        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(download_handler(self.window, 0))
        }
        fn command_handler(&self) -> Option<CommandHandler> {
            Some(FlitSettingsCommands::new(self.window))
        }
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(FlitSettingsRequest::new(self.window))
        }
    }
}

wrap_request_handler! {
    struct FlitSettingsRequest { window: usize }
    impl RequestHandler {
        fn on_before_browse(
            &self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>,
            request: Option<&mut Request>, _gesture: i32, _redirect: i32,
        ) -> i32 {
            if frame.is_none_or(|frame| frame.is_main() != 0) {
                if let Some(url) = request.map(|request| CefString::from(&request.url())) {
                    if queue_link(self.window, Some(&url), WindowOpenDisposition::NEW_FOREGROUND_TAB, None) {
                        return 1;
                    }
                }
            }
            0
        }
        fn on_open_urlfrom_tab(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            url: Option<&CefString>, disposition: WindowOpenDisposition, _gesture: i32,
        ) -> i32 {
            let disposition = match disposition {
                WindowOpenDisposition::NEW_BACKGROUND_TAB | WindowOpenDisposition::NEW_WINDOW => disposition,
                _ => WindowOpenDisposition::NEW_FOREGROUND_TAB,
            };
            i32::from(queue_link(self.window, url, disposition, None))
        }
    }
}

wrap_command_handler! {
    struct FlitSettingsCommands { window: usize }
    impl CommandHandler {
        fn on_chrome_command(
            &self, _browser: Option<&mut Browser>, command: i32, _disposition: WindowOpenDisposition,
        ) -> i32 {
            let action = if command == chrome_command(c"IDC_NEW_TAB") {
                NEW_TAB
            } else if command == chrome_command(c"IDC_NEW_WINDOW") {
                NEW_WINDOW
            } else if command == chrome_command(c"IDC_NEW_INCOGNITO_WINDOW") {
                PRIVATE_WINDOW
            } else {
                return 0;
            };
            unsafe { PostMessageW(self.window as HWND, WM_COMMAND, action, 0); }
            1
        }
        // Keep Chromium's normal menu and toolbar policies when intercepting
        // the commands that would otherwise create unmanaged browser windows.
        fn is_chrome_app_menu_item_visible(&self, _browser: Option<&mut Browser>, _command: i32) -> i32 { 1 }
        fn is_chrome_app_menu_item_enabled(&self, _browser: Option<&mut Browser>, _command: i32) -> i32 { 1 }
        fn is_chrome_page_action_icon_visible(&self, _icon: ChromePageActionIconType) -> i32 { 1 }
        fn is_chrome_toolbar_button_visible(&self, _button: ChromeToolbarButtonType) -> i32 { 1 }
    }
}

wrap_life_span_handler! {
    struct FlitSettingsLife { window: usize }
    impl LifeSpanHandler {
        fn on_before_popup(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _popup_id: i32,
            url: Option<&CefString>, _name: Option<&CefString>, disposition: WindowOpenDisposition,
            gesture: i32, _features: Option<&PopupFeatures>, _window: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>, _settings: Option<&mut BrowserSettings>,
            _extra: Option<&mut Option<DictionaryValue>>, _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            if gesture != 0 {
                queue_link(self.window, url, disposition, None);
            }
            1
        }
        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let _scope = WindowScope::enter(self.window);
            if let Some(browser) = browser {
                download_guard::clear_browser(self.window, browser.identifier());
                let _ = change(|s| {
                    if s.settings_window.as_ref().is_some_and(|stored| stored.is_same(Some(browser)) != 0) {
                        s.settings_window = None;
                    }
                });
            }
            browser_closed(self.window, POPUP_CLOSED, 0);
        }
    }
}

wrap_client! {
    struct FlitClient { key: usize, window: usize, popup: Option<PopupTab>, agent_tab: Option<u32> }
    impl Client {
        fn on_process_message_received(
            &self, browser: Option<&mut Browser>, frame: Option<&mut Frame>,
            source: ProcessId, message: Option<&mut ProcessMessage>,
        ) -> i32 {
            let Some((request, mib)) = message.and_then(|msg| tab_memory::read_reply(source, msg)) else {
                return 0;
            };
            if let (Some(browser), Some(frame)) = (browser, frame)
                && frame.is_valid() != 0 && frame.is_main() != 0
                && browser.main_frame().is_some_and(|current| {
                    CefString::from(&current.identifier()).to_string() ==
                        CefString::from(&frame.identifier()).to_string()
                })
            {
                let _scope = WindowScope::enter(self.window);
                finish_tab_memory(self.key, browser.identifier(), request, mib);
            }
            1
        }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(FlitLife::new(self.window, self.key, self.popup))
        }
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(FlitDisplay::new(self.window, self.key))
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(FlitLoad::new(self.window, self.key))
        }
        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(FlitKeyboard::new(self.window, self.key))
        }
        fn focus_handler(&self) -> Option<FocusHandler> {
            Some(FlitFocus::new(self.window, self.key))
        }
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(FlitRequest::new(self.window, self.key, self.agent_tab))
        }
        fn find_handler(&self) -> Option<FindHandler> {
            Some(FlitFind::new(self.window, self.key))
        }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> {
            Some(FlitUnload::new(self.window, self.key))
        }
        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(download_handler(self.window, self.key))
        }
        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(permissions::handler(self.window, self.key))
        }
        fn audio_handler(&self) -> Option<AudioHandler> {
            Some(FlitAudio::new(self.window, self.key))
        }
    }
}

wrap_focus_handler! {
    struct FlitFocus { window: usize, key: usize }
    impl FocusHandler {
        fn on_got_focus(&self, _browser: Option<&mut Browser>) {
            unsafe { PostMessageW(self.window as HWND, SPLIT_FOCUS, self.key, 0) };
        }
    }
}

wrap_audio_handler! {
    // Only tracks whether a page plays sound, so Memory Saver keeps it awake.
    struct FlitAudio { window: usize, key: usize }
    impl AudioHandler {
        fn on_audio_stream_started(&self, _browser: Option<&mut Browser>, _params: Option<&AudioParameters>, _channels: i32) {
            unsafe { PostMessageW(self.window as HWND, AUDIBLE_CHANGED, self.key, 1) };
        }
        fn on_audio_stream_stopped(&self, _browser: Option<&mut Browser>) {
            unsafe { PostMessageW(self.window as HWND, AUDIBLE_CHANGED, self.key, 0) };
        }
        fn on_audio_stream_error(&self, _browser: Option<&mut Browser>, _message: Option<&CefString>) {
            unsafe { PostMessageW(self.window as HWND, AUDIBLE_CHANGED, self.key, 0) };
        }
    }
}

fn set_audible(window: usize, key: usize, audible: bool) {
    let _scope = WindowScope::enter(window);
    change(|s| {
        if let Some(tab) = s.tabs.iter_mut().find(|tab| tab.key == key) {
            tab.audible = audible;
            if !audible {
                tab.hidden_since = tab.hidden_since.max(Instant::now());
            }
        }
    });
}

fn automatic_release_delay() -> Option<Duration> {
    let minutes = MEMORY_SAVER_MINUTES.get();
    if minutes == 0 {
        return None;
    }
    let idle = if physical_memory_low() { minutes.min(3) } else { minutes };
    Some(Duration::from_secs(60 * u64::from(idle)))
}

fn can_auto_release(s: &Shell, tab: &Tab, idle: Duration) -> bool {
    !tab.audible && tab.agent.is_none() && !tab.back && !tab.forward
        && tab.hidden_since.elapsed() >= idle && can_release_background(s, tab)
}

/// Put inactive background tabs to sleep in oldest-first order.
fn memory_saver_tick() {
    let Some(idle) = automatic_release_delay() else { return };
    let mut keys = with_shell(|s| {
        if !s.memory_queue.is_empty() || s.tabs.iter().any(|tab| tab.unloading) {
            return Vec::new();
        }
        s.tabs
            .iter()
            .filter(|tab| can_auto_release(s, tab, idle))
            .map(|tab| (tab.hidden_since, tab.key))
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    if keys.is_empty() {
        return;
    }
    keys.sort();
    change(|s| s.memory_queue.extend(keys.into_iter().map(|(_, key)| MemoryRelease::Automatic(key))));
    release_next_background_tab();
}

wrap_jsdialog_handler! {
    struct FlitUnload { window: usize, key: usize }
    impl JsdialogHandler {
        fn on_before_unload_dialog(
            &self, _browser: Option<&mut Browser>, _message: Option<&CefString>,
            is_reload: i32, callback: Option<&mut JsdialogCallback>,
        ) -> i32 {
            let Some(callback) = callback else { return 0 };
            let _scope = WindowScope::enter(self.window);
            if agent_of(self.key).is_some() {
                // Nobody is watching an AI tab's "leave page?" prompt.
                callback.cont(1, None);
                return 1;
            }
            let message = if is_reload != 0 {
                wide(i18n::text("Reload this page? Unsaved changes may be lost."))
            } else {
                wide(i18n::text("Leave this page? Unsaved changes may be lost."))
            };
            let releasing = with_shell(|s| s.tabs.iter().any(|tab| tab.key == self.key && tab.unloading)) == Some(true);
            let proceed = !releasing && unsafe { MessageBoxW(self.window as HWND, message.as_ptr(),
                windows_sys::w!("Flit Browser"), MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) } == IDYES;
            let cancelled_close = !proceed && is_reload == 0 && (
                (!releasing && cancel_native_exit()) || change(|s| {
                    let tab = s.tabs.iter().any(|tab| tab.key == self.key && tab.close_requested);
                    if !releasing {
                        s.closing_session = None;
                        return std::mem::replace(&mut s.closing, false) || tab;
                    }
                    tab
                }) == Some(true));
            callback.cont(i32::from(proceed), None);
            if cancelled_close {
                unsafe { PostMessageW(self.window as HWND, TAB_CLOSE_CANCELLED, self.key, isize::from(releasing)); }
            }
            1
        }
    }
}

wrap_find_handler! {
    struct FlitFind { window: usize, key: usize }
    impl FindHandler {
        fn on_find_result(
            &self, _browser: Option<&mut Browser>, _identifier: i32, count: i32,
            _selection: Option<&Rect>, active_match: i32, _final_update: i32,
        ) {
            let _scope = WindowScope::enter(self.window);
            find_update(self.key, count, active_match);
        }
    }
}

wrap_life_span_handler! {
    struct FlitLife { window: usize, key: usize, popup: Option<PopupTab> }
    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let (Some(popup), Some(browser)) = (self.popup, browser) else { return };
            let _scope = WindowScope::enter(self.window);
            let tracked = change(|s| s.pending_popups.remove(&(popup.opener, popup.id)))
                .flatten().flatten() == Some(self.key);
            if !tracked {
                if let Some(host) = browser.host() {
                    host.close_browser(1);
                }
                return;
            }
            if attach_tab_browser(self.key, browser).is_err() {
                return;
            }
            let window = self.window as HWND;
            if with_shell(|s| s.closing) == Some(true) {
                browser_closed(self.window, POPUP_CLOSED, 0);
            } else {
                if popup.foreground {
                    // The CEF navigation still holds the opener's creation lock here.
                    unsafe { PostMessageW(window, SELECT_POPUP_TAB, self.key, 0); }
                }
                queue_session_save();
            }
        }
        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let _scope = WindowScope::enter(self.window);
            if let Some(id) = agent_of(self.key) {
                change(|s| {
                    if let Some(tab) = s.tabs.iter_mut().find(|tab| tab.key == self.key) {
                        tab.agent = None;
                    }
                });
                crate::agent::closed(id);
            }
            permissions::cancel_tab(self.window, self.key);
            if let Some(browser) = browser {
                download_guard::clear_browser(self.window, browser.identifier());
            }
            discard_aborted_popups(self.key, None);
            let browser = change(|state| {
                let tab = state.tabs.iter_mut().find(|tab| tab.key == self.key)?;
                tab.retiring = true;
                tab.close_requested = false;
                tab.browser.take()
            }).flatten();
            drop(browser);
            // Defer parent HWND destruction until the CEF callback returns.
            browser_closed(self.window, PAGE_CLOSED, self.key);
        }
        fn on_before_popup(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            popup_id: i32, target_url: Option<&CefString>, _target_name: Option<&CefString>,
            disposition: WindowOpenDisposition, gesture: i32, _features: Option<&PopupFeatures>,
            window: Option<&mut WindowInfo>, client: Option<&mut Option<Client>>,
            settings: Option<&mut BrowserSettings>, _extra: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            if gesture == 0 || exiting() {
                return 1;
            }
            let _scope = WindowScope::enter(self.window);
            if with_shell(|s| s.closing) != Some(false) {
                return 1;
            }
            let target = target_url.map(CefString::to_string).unwrap_or_default();
            if !(target.is_empty() || target == HOME || crate::launch_url::literal_http_url(&target).is_some()) {
                return 1;
            }
            let Some(client) = client else { return 1 };
            if matches!(disposition, WindowOpenDisposition::NEW_FOREGROUND_TAB | WindowOpenDisposition::NEW_BACKGROUND_TAB) {
                let Some(window) = window else { return 1 };
                let foreground = disposition == WindowOpenDisposition::NEW_FOREGROUND_TAB;
                let Ok((key, _)) = append_tab(if target.is_empty() { HOME } else { &target }, "", false, foreground.then_some(self.key), None) else {
                    return 1;
                };
                let Some(panel) = with_shell(|s| s.tabs.iter().find(|tab| tab.key == key).map(|t| t.panel)).flatten() else {
                    return 1;
                };
                // Let Chromium create and navigate the child so the original
                // method, body and opener survive the new-tab request.
                *window = tab_window_info(self.window as HWND, key, panel);
                if let Some(settings) = settings {
                    settings.chrome_zoom_bubble = State::DISABLED;
                }
                change(|s| s.pending_popups.insert((self.key, popup_id), Some(key)));
                *client = Some(FlitClient::new(
                    key,
                    self.window,
                    Some(PopupTab { opener: self.key, id: popup_id, foreground }),
                    None,
                ));
                return 0;
            }
            if matches!(disposition, WindowOpenDisposition::NEW_POPUP | WindowOpenDisposition::NEW_WINDOW) {
                // Let an explicit separate window keep its native browsing
                // context and window.opener in the same profile.
                change(|s| s.pending_popups.insert((self.key, popup_id), None));
                *client = Some(FlitPopupClient::new(self.window, self.key, popup_id));
                return 0;
            }
            1
        }
        fn on_before_popup_aborted(&self, _browser: Option<&mut Browser>, popup_id: i32) {
            let _scope = WindowScope::enter(self.window);
            discard_aborted_popups(self.key, Some(popup_id));
            browser_closed(self.window, POPUP_CLOSED, 0);
        }
        fn on_before_dev_tools_popup(
            &self, _browser: Option<&mut Browser>, _window: Option<&mut WindowInfo>,
            client: Option<&mut Option<Client>>, _settings: Option<&mut BrowserSettings>,
            _extra: Option<&mut Option<DictionaryValue>>, use_default_window: Option<&mut i32>,
        ) {
            // DevTools is owned by Chromium. Its callbacks must not update or
            // close the Flit tab whose client would otherwise be inherited.
            if let Some(client) = client {
                *client = Some(FlitDevToolsClient::new());
            }
            if let Some(use_default_window) = use_default_window {
                *use_default_window = 1;
            }
        }
    }
}

fn discard_aborted_popups(opener: usize, id: Option<i32>) {
    let panels = change(|s| {
        let keys: Vec<_> = s
            .pending_popups
            .extract_if(.., |&(key, popup_id), _| {
                key == opener && id.is_none_or(|id| id == popup_id)
            })
            .filter_map(|(_, tab)| tab)
            .collect();
        keys.into_iter()
            .filter_map(|key| {
                let index = s.tabs.iter().position(|tab| tab.key == key)?;
                Some(s.tabs.remove(index).panel)
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    if !panels.is_empty() {
        for panel in panels {
            unsafe { DestroyWindow(panel) };
        }
        refresh_rows(false);
    }
}

wrap_client! {
    struct FlitPopupClient { window: usize, opener_key: usize, popup_id: i32 }
    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(FlitPopupLife::new(self.window, self.opener_key, self.popup_id))
        }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> {
            Some(FlitUnload::new(self.window, 0))
        }
        fn download_handler(&self) -> Option<DownloadHandler> {
            // Popup transfers participate in the owner window's close check.
            Some(download_handler(self.window, 0))
        }
    }
}

wrap_life_span_handler! {
    struct FlitPopupLife { window: usize, opener_key: usize, popup_id: i32 }
    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser else { return; };
            let _scope = WindowScope::enter(self.window);
            let tracked = change(|s| {
                if s.pending_popups.remove(&(self.opener_key, self.popup_id)) != Some(None) {
                    return false;
                }
                s.popups.push(browser.clone());
                true
            }) == Some(true);
            if !tracked || with_shell(|s| s.closing) == Some(true) {
                if let Some(host) = browser.host() {
                    host.close_browser(0);
                }
            }
        }
        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser else { return; };
            let id = browser.identifier();
            download_guard::clear_browser(self.window, id);
            let _scope = WindowScope::enter(self.window);
            let popup = change(|s| {
                let i = s.popups.iter().position(|popup| popup.identifier() == id)?;
                Some(s.popups.remove(i))
            }).flatten();
            drop(popup);
            browser_closed(self.window, POPUP_CLOSED, 0);
        }
        fn on_before_popup(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _popup_id: i32,
            _url: Option<&CefString>, _name: Option<&CefString>, _disposition: WindowOpenDisposition,
            _gesture: i32, _features: Option<&PopupFeatures>, _window: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>, _settings: Option<&mut BrowserSettings>,
            _extra: Option<&mut Option<DictionaryValue>>, _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            1
        }
    }
}

wrap_request_handler! {
    struct FlitRequest { window: usize, key: usize, agent_tab: Option<u32> }
    impl RequestHandler {
        fn on_before_browse(
            &self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>,
            request: Option<&mut Request>, _gesture: i32, redirect: i32,
        ) -> i32 {
            if frame.as_ref().is_none_or(|frame| frame.is_main() != 0) {
                permissions::cancel_tab(self.window, self.key);
                if let Some(id) = self.agent_tab {
                    let (Some(frame), Some(request)) = (frame, request) else { return 1 };
                    let target = CefString::from(&request.url()).to_string();
                    // A Vault sign-in pins the tab to its expected destination.
                    if !crate::agent::vault_allows_navigation(id, &target, redirect != 0) {
                        return 1;
                    }
                    if !crate::agent_view::allows_main_navigation(request, &target) {
                        if redirect != 0 {
                            crate::agent::failed(id, crate::agent_view::REDIRECT_BLOCKED);
                        }
                        return 1;
                    }
                    if redirect == 0 {
                        let current = CefString::from(&frame.url()).to_string();
                        if !crate::agent::same_document_link(&current, &target) {
                            crate::agent::loading(id);
                        }
                    }
                    return 0;
                }
                if let Some(request) = request {
                    let target = CefString::from(&request.url()).to_string();
                    if let Some(target) = settings_target(&target) {
                        let _scope = WindowScope::enter(self.window);
                        if private() || exiting() {
                            return 1;
                        }
                        if change(|s| s.pending_settings = Some((self.key, target))).is_some() {
                            unsafe { PostMessageW(self.window as HWND, OPEN_SETTINGS_WINDOW, 0, 0); }
                        }
                        return 1;
                    }
                }
            }
            0
        }

        fn resource_request_handler(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            _request: Option<&mut Request>, navigation: i32, download: i32,
            initiator: Option<&CefString>, _disable_default: Option<&mut i32>,
        ) -> Option<ResourceRequestHandler> {
            // CEF calls this on its IO thread. Use the tab identity captured at creation.
            self.agent_tab
                .is_some()
                .then(|| crate::agent_view::resource_handler(navigation, download, initiator))
        }

        fn on_open_urlfrom_tab(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            url: Option<&CefString>, disposition: WindowOpenDisposition, gesture: i32,
        ) -> i32 {
            if gesture != 0 && matches!(disposition, WindowOpenDisposition::NEW_FOREGROUND_TAB
                | WindowOpenDisposition::NEW_BACKGROUND_TAB | WindowOpenDisposition::NEW_WINDOW)
                && queue_link(self.window, url, disposition, Some(self.key)) {
                1
            } else { 0 }
        }
    }
}

fn queue_link(
    window: usize,
    url: Option<&CefString>,
    disposition: WindowOpenDisposition,
    source: Option<usize>,
) -> bool {
    if exiting() {
        return true;
    }
    let Some(url) = url.map(CefString::to_string) else {
        return false;
    };
    if !(url.starts_with("https://") || url.starts_with("http://"))
        || session::stored_url(&url).is_none()
    {
        return false;
    }
    let _scope = WindowScope::enter(window);
    change(|s| {
        if s.closing {
            return false;
        }
        s.links.push_back(PendingLink {
            url,
            disposition,
            source,
        });
        if unsafe { PostMessageW(window as HWND, OPEN_LINK, 0, 0) } == 0 {
            s.links.pop_back();
            false
        } else {
            true
        }
    })
    .unwrap_or(false)
}

wrap_display_handler! {
    struct FlitDisplay { window: usize, key: usize }
    impl DisplayHandler {
        fn on_address_change(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            if frame.is_some_and(|frame| frame.is_main() != 0) {
                let _scope = WindowScope::enter(self.window);
                let url = url.map(CefString::to_string).unwrap_or_default();
                if let Some(id) = agent_of(self.key) {
                    crate::agent::address(id, url.clone());
                }
                if with_shell(|s| s.tabs.iter().any(|tab| tab.key == self.key && tab.url != url)) == Some(true) {
                    permissions::cancel_tab(self.window, self.key);
                    if with_shell(|s| s.memory_preview.is_some_and(|p| p.key == self.key)) == Some(true) {
                        clear_tab_memory();
                    }
                }
                update_page(self.key, true, |tab| {
                    if tab.url == url {
                        return false;
                    }
                    tab.url = url;
                    true
                });
            }
        }
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            let _scope = WindowScope::enter(self.window);
            let title = title.map(CefString::to_string).unwrap_or_default();
            if let Some(id) = agent_of(self.key) {
                crate::agent::title(id, title.clone());
            }
            update_page(self.key, true, |tab| {
                if tab.title == title {
                    return false;
                }
                tab.title = title;
                true
            });
        }
        fn on_console_message(
            &self, browser: Option<&mut Browser>, _level: LogSeverity, message: Option<&CefString>,
            _source: Option<&CefString>, _line: i32,
        ) -> i32 {
            let Some(text) = message.map(CefString::to_string) else { return 0 };
            let Some(json) = text.strip_prefix(LOGIN_SIGNAL) else { return 0 };
            let _scope = WindowScope::enter(self.window);
            // The origin comes from the browser's own frame URL, never from the page's message.
            let origin = browser.and_then(|b| b.main_frame())
                .and_then(|frame| crate::vault_store::origin(&CefString::from(&frame.url()).to_string()));
            if let (Some(origin), Ok([user, password])) = (origin, serde_json::from_str::<[String; 2]>(json))
                && !user.trim().is_empty() && !password.is_empty() && !private() && agent_of(self.key).is_none()
            {
                let fresh = PENDING_LOGIN.with(|slot| {
                    let mut slot = slot.borrow_mut();
                    if slot.as_ref().is_some_and(|login| {
                        login.owner == self.window && login.key == self.key
                            && login.origin == origin && login.user == user && login.password == password
                    }) {
                        return false;
                    }
                    let sequence = LOGIN_SEQUENCE.get().wrapping_add(1);
                    LOGIN_SEQUENCE.set(sequence);
                    *slot = Some(PendingLogin {
                        owner: self.window, key: self.key, sequence, origin, user, password,
                    });
                    true
                });
                if fresh {
                    unsafe { PostMessageW(self.window as HWND, VAULT_OFFER, 0, 0) };
                }
            }
            1
        }
        fn on_favicon_urlchange(&self, browser: Option<&mut Browser>, icon_urls: Option<&mut CefStringList>) {
            let url = icon_urls.and_then(first_string);
            if let (Some(url), Some(host)) = (url, browser.and_then(|b| b.host())) {
                // 32 px covers the 16-DIP row icon up to 200% scaling.
                let mut callback = FlitFavicon::new(self.window, self.key);
                host.download_image(Some(&CefString::from(url.as_str())), 1, 32, 0, Some(&mut callback));
            }
        }
        fn on_media_access_change(
            &self, _browser: Option<&mut Browser>, has_video_access: i32, has_audio_access: i32,
        ) {
            let _scope = WindowScope::enter(self.window);
            update_page(self.key, false, |tab| {
                let capture = (has_video_access != 0, has_audio_access != 0);
                if (tab.has_video_access, tab.has_audio_access) == capture {
                    return false;
                }
                (tab.has_video_access, tab.has_audio_access) = capture;
                true
            });
        }
    }
}

wrap_load_handler! {
    struct FlitLoad { window: usize, key: usize }
    impl LoadHandler {
        fn on_loading_state_change(&self, _browser: Option<&mut Browser>, loading: i32, back: i32, forward: i32) {
            let _scope = WindowScope::enter(self.window);
            let active = change(|s| {
                let Some(tab) = s.tabs.iter_mut().find(|tab| tab.key == self.key) else { return false };
                (tab.loading, tab.back, tab.forward) = (loading != 0, back != 0, forward != 0);
                s.active == Some(self.key)
            });
            if active == Some(true) {
                unsafe { PostMessageW(self.window as HWND, LOAD_STATE_CHANGED, self.key, 0); }
            }
        }
        fn on_load_end(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, status: i32) {
            if let Some(frame) = frame {
                let _scope = WindowScope::enter(self.window);
                if frame.is_main() != 0 && let Some(id) = agent_of(self.key) {
                    let url = CefString::from(&frame.url()).to_string();
                    if crate::agent::web_url(&url).is_some() {
                        crate::agent::loaded(id, url, status);
                    }
                } else if frame.is_main() != 0 && !private()
                    && CefString::from(&frame.url()).to_string().starts_with("https://")
                {
                    frame.execute_java_script(Some(&CefString::from(WATCH_LOGIN)), None, 0);
                } else if frame.is_main() != 0
                    && CefString::from(&frame.url()).to_string().starts_with("chrome://")
                {
                    frame.execute_java_script(Some(&CefString::from(CHROME_SKIN)), None, 0);
                }
                finish_bookmark(self.key, frame);
            }
        }
        fn on_load_error(
            &self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>,
            code: Errorcode, _text: Option<&CefString>, _url: Option<&CefString>,
        ) {
            // Rejected navigations that leave the page intact also report ABORTED.
            if code == Errorcode::ABORTED || !frame.is_some_and(|frame| frame.is_main() != 0) {
                return;
            }
            let _scope = WindowScope::enter(self.window);
            if let Some(id) = agent_of(self.key) {
                crate::agent::failed(id, "The page could not be loaded.");
            }
        }
    }
}

/// Restyles Chromium's built-in pages (settings, downloads, history, bookmarks,
/// passwords) with Flit's palette and type. WebUI colors are CSS variables, which
/// reach into each component's shadow tree.
const CHROME_SKIN: &str = r#"(() => {
  if (document.getElementById('flit-skin')) return;
  const style = document.createElement('style');
  style.id = 'flit-skin';
  style.textContent = `
    html {
      --flit-ink: #222d40; --flit-muted: #6e6e73; --flit-accent: #0066cc;
      --flit-bg: #eef2f8; --flit-card: #fbfdff; --flit-line: #dfe6f0; --flit-tint: #d9e8ff;
      --color-sys-primary: var(--flit-accent) !important;
      --color-sys-on-primary: #fff !important;
      --color-sys-base: var(--flit-bg) !important;
      --color-sys-surface: var(--flit-bg) !important;
      --color-sys-base-container: var(--flit-card) !important;
      --color-sys-base-container-elevated: #fff !important;
      --color-sys-surface-container: var(--flit-card) !important;
      --color-sys-header: var(--flit-bg) !important;
      --color-sys-header-container: #e3e9f2 !important;
      --color-sys-on-surface: var(--flit-ink) !important;
      --color-sys-on-surface-subtle: var(--flit-muted) !important;
      --color-sys-tonal-container: var(--flit-tint) !important;
      --color-sys-on-tonal-container: #1c4984 !important;
      --color-sys-neutral-outline: #d0d9e7 !important;
      --color-sys-divider: var(--flit-line) !important;
      --color-sys-state-hover: rgba(0, 102, 204, .08) !important;
      --color-sys-state-focus-ring: var(--flit-accent) !important;
      --cr-primary-text-color: var(--flit-ink) !important;
      --cr-secondary-text-color: var(--flit-muted) !important;
      --cr-card-background-color: var(--flit-card) !important;
      --cr-card-border-radius: 16px !important;
      --cr-link-color: var(--flit-accent) !important;
      --cr-checked-color: var(--flit-accent) !important;
      --cr-separator-color: var(--flit-line) !important;
      --cr-focus-outline-color: var(--flit-accent) !important;
      --cr-fallback-color-primary: var(--flit-accent) !important;
      --cr-fallback-color-surface: var(--flit-bg) !important;
      --cr-fallback-color-on-surface: var(--flit-ink) !important;
      --google-blue-500: #0071e3 !important;
      --google-blue-600: var(--flit-accent) !important;
      --google-blue-700: #0058b0 !important;
      background: var(--flit-bg) !important;
    }
    html, body, button, input, select, textarea {
      font-family: "Segoe UI Variable Text", "Segoe UI", "Malgun Gothic", system-ui, sans-serif !important;
    }
    body { background: var(--flit-bg) !important; color: var(--flit-ink) !important; }
  `;
  document.documentElement.appendChild(style);
})()"#;

const LOGIN_SIGNAL: &str = "\u{1}flit-login";

/// Reports a submitted login (username, password) to the browser process through
/// the console, which Flit intercepts and never logs.
const WATCH_LOGIN: &str = r#"(() => {
  if (window.__flitLoginWatch) return;
  window.__flitLoginWatch = true;
  // Read the fields as the user types: sites such as Naver encrypt and clear the
  // password in their own click handler, which can run before Flit sees the click.
  let last = null;
  const read = scope => {
    const root = scope && scope.querySelectorAll ? scope : document;
    const password = [...root.querySelectorAll('input[type=password]')].find(i => i.value);
    if (!password) return null;
    const inputs = [...(password.form || document).querySelectorAll('input')];
    const before = inputs.slice(0, inputs.indexOf(password)).reverse();
    const user = before.find(i => /^(text|email|tel)$/.test(i.type) && i.value)
      || [...document.querySelectorAll('input[autocomplete~=username]')].find(i => i.value);
    return [user ? user.value : '', password.value];
  };
  const send = scope => {
    const values = read(scope) || last;
    if (values) console.debug('\u0001flit-login' + JSON.stringify(values));
  };
  addEventListener('input', e => {
    const field = e.target;
    if (field && /^(password|text|email|tel)$/.test(field.type)) last = read(field.form) || last;
  }, true);
  addEventListener('submit', e => send(e.target), true);
  addEventListener('keydown', e => {
    if (e.key === 'Enter' && e.target && /^(password|text|email|tel)$/.test(e.target.type)) send(e.target.form);
  }, true);
  const pressed = e => {
    const button = e.target.closest && e.target.closest('button, input[type=submit], [role=button]');
    // Skip helper buttons such as "show password"; script-driven logins use a labelled button.
    if (button && (button.type === 'submit'
        || /log\s*in|sign\s*in|로그인|continue|계속/i.test(button.textContent || button.value || ''))) send(button.form);
  };
  addEventListener('pointerdown', pressed, true);
  addEventListener('click', pressed, true);
})()"#;

/// Offer to keep a login the user just submitted, like Chrome's save bubble.
/// Flit Vault is the one place it is saved; Chrome's own store gets a copy for autofill.
fn offer_vault_login(window: HWND) {
    let model = PENDING_LOGIN.with_borrow(|slot| {
        let login = slot.as_ref().filter(|login| login.owner == window as usize)?;
        if with_shell(|s| s.private || s.closing || s.active != Some(login.key)) != Some(false) {
            return None;
        }
        let existing = crate::vault_store::load(&login.origin).ok().flatten();
        if existing.as_ref().is_some_and(|c| c.username == login.user && c.password == login.password) {
            return None;
        }
        let update = existing.is_some_and(|c| c.username == login.user);
        let t = i18n::choose;
        let title = if update {
            t("Update the saved password?", "저장된 비밀번호를 바꿀까요?")
        } else {
            t("Save this login to Flit Vault?", "이 로그인을 Flit Vault에 저장할까요?")
        };
        let mut surface = popup_surface(title, vec![Item {
            id: "answer".into(),
            title: format!("{} · ••••••••", clean(&login.user, 90)),
            detail: t(
                "Flit can fill this password on this site. AI can use it only here.",
                "Flit이 이 사이트의 비밀번호를 채울 수 있습니다. AI도 이 사이트에서만 사용할 수 있습니다.",
            ).into(),
            actions: vec![
                popup_action(POPUP_VAULT_NO | login.sequence as usize, t("Not now", "나중에")),
                popup_action(POPUP_VAULT_YES | login.sequence as usize,
                    if update { t("Update", "바꾸기") } else { t("Save", "저장") }),
            ],
        }]);
        surface.subtitle = login.origin.clone();
        Some(surface)
    });
    if let Some(surface) = model {
        open_popover(window, "vault", crate::popover::Anchor::Top, 400, 245, &surface);
    }
}

fn answer_pending_login(window: HWND, sequence: u32, save: bool) {
    let active = with_shell(|s| if s.private || s.closing { None } else { s.active }).flatten();
    let login = PENDING_LOGIN.with_borrow_mut(|slot| slot.take_if(|login| {
        login.owner == window as usize && login.sequence == sequence && active == Some(login.key)
    }));
    let Some(login) = login.filter(|_| save) else { return };
    if let Err(error) = crate::vault_store::save(&login.origin, &login.user, &login.password) {
        set_text(window, &format!("{} — {}", app_title(), i18n::text(error)));
        return;
    }
    copy_login_to_chrome(window, &login.origin, &login.user, &login.password);
}

/// Add the login to Chromium's password store through the password manager WebUI,
/// in a page the user never sees.
fn copy_login_to_chrome(window: HWND, origin: &str, user: &str, password: &str) {
    let quote = |value: &str| serde_json::to_string(value).unwrap_or_default();
    let (origin, user, password) = (quote(origin), quote(user), quote(password));
    let script = format!(
        "(async () => {{ try {{ await chrome.passwordsPrivate.addPassword({{url: {origin}, username: {user}, password: {password}, note: '', useAccountStore: false}}); document.title = 'flit-saved'; }} catch (e) {{ document.title = 'flit-failed'; }} }})()"
    );
    // shortcut: the hidden host panel stays until the window closes; fine for an occasional save.
    let panel = child(window, 0, "Static", "", WS_CHILD, 0);
    if panel.is_null() {
        return;
    }
    let info = WindowInfo::default().set_as_child(
        sys::HWND(panel.cast()),
        &Rect { x: 0, y: 0, width: 1, height: 1 },
    );
    let mut client = FlitPasswordClient::new(script);
    let mut context = request_context().ok().flatten();
    browser_host_create_browser(
        Some(&info),
        Some(&mut client),
        Some(&CefString::from("chrome://password-manager/passwords")),
        Some(&BrowserSettings::default()),
        None,
        context.as_mut(),
    );
}

wrap_client! {
    struct FlitPasswordClient { script: String }
    impl Client {
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(FlitPasswordLoad::new(self.script.clone()))
        }
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(FlitPasswordDisplay::new())
        }
    }
}

wrap_load_handler! {
    struct FlitPasswordLoad { script: String }
    impl LoadHandler {
        fn on_load_end(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _status: i32) {
            if let Some(frame) = frame.filter(|f| f.is_main() != 0)
                && CefString::from(&frame.url()).to_string().starts_with("chrome://password-manager/")
            {
                frame.execute_java_script(Some(&CefString::from(self.script.as_str())), None, 0);
            }
        }
    }
}

wrap_display_handler! {
    struct FlitPasswordDisplay;
    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            let title = title.map(CefString::to_string).unwrap_or_default();
            if title.starts_with("flit-") {
                if title == "flit-failed" {
                    eprintln!("Flit Browser: could not copy the login to the browser's passwords.");
                }
                if let Some(host) = browser.and_then(|b| b.host()) {
                    host.close_browser(1);
                }
            }
        }
    }
}

const PANEL_COMMANDS: &[usize] = &[
    NEW_TAB, NEW_WINDOW, PRIVATE_WINDOW, RESTORE_TAB, HISTORY, SAVE_BOOKMARK, BOOKMARKS, FIND, PRINT,
    FREE_MEMORY, SPLIT_VIEW, SETTINGS, DEVTOOLS, UPDATE, EXIT_FLIT, DOWNLOADS, EXTENSIONS_BUTTON, VAULT_BUTTON,
];
const POPUP_VAULT_SAVE: usize = 1000;
const POPUP_EXTENSION_STORE: usize = 1001;
const POPUP_VAULT_REMOVE: usize = 1002;
const POPUP_EXTENSION_FIRST: usize = 1024;
const POPUP_DOWNLOAD_OPEN: usize = 1 << 32;
const POPUP_DOWNLOAD_FOLDER: usize = 2 << 32;
const POPUP_VAULT_YES: usize = 3 << 32;
const POPUP_VAULT_NO: usize = 4 << 32;
const POPUP_DOWNLOAD_PAUSE: usize = 6 << 32;
const POPUP_DOWNLOAD_RESUME: usize = 7 << 32;
const POPUP_DOWNLOAD_CANCEL: usize = 8 << 32;
const POPUP_NAMESPACE: usize = !((1 << 32) - 1);

fn panel_section(id: &str, title: &str, items: Vec<Item>) -> Section {
    Section { id: id.into(), title: title.into(), items }
}

fn popup_surface(title: &str, items: Vec<Item>) -> Surface {
    grouped_panel(title, vec![panel_section("actions", "", items)])
}

fn grouped_panel(title: &str, sections: Vec<Section>) -> Surface {
    Surface {
        title: title.to_owned(),
        subtitle: String::new(),
        dark: chrome::dark(),
        compact: true,
        start: false,
        preserve_input_focus: false,
        sections,
    }
}

fn popup_action(id: usize, label: &str) -> Action {
    Action { id, label: label.to_owned(), selected: false, kind: ActionKind::Link }
}

fn open_popover(window: HWND, kind: &'static str, anchor: Anchor,
    width: i32, height: i32, surface: &Surface) {
    if let Err(error) = crate::popover::show_native(window, kind, anchor, width, height, surface) {
        set_text(window, &format!("{} — {}", app_title(), i18n::text(error)));
    }
}

fn zoom_percent() -> Option<i32> {
    host().map(|host| (100.0 * 1.2_f64.powf(host.zoom_level())).round() as i32)
}

fn menu_surface() -> Surface {
    let t = i18n::choose;
    let private = private();
    let recent = if private {
        with_shell(|s| closed_choices(&s.recently_closed)).unwrap_or_default()
    } else {
        RECENT_NORMAL.with(|items| closed_choices(&items.borrow()))
    };
    let link = |id, label: &str, shortcut: &str| Item::link(id, label.to_owned(), shortcut.to_owned());
    let mut windows = vec![
        link(NEW_TAB, t("New tab", "새 탭"), "Ctrl+T"),
        link(NEW_WINDOW, t("New window", "새 창"), "Ctrl+N"),
        link(PRIVATE_WINDOW, t("Private window", "비공개 창"), "Ctrl+Shift+N"),
    ];
    if let Some((_, title)) = recent.first() {
        windows.push(link(RESTORE_TAB, &format!("{} {}", t("Reopen", "다시 열기:"), title.replace("&&", "&")), "Ctrl+Shift+T"));
    }
    let mut library = vec![link(DOWNLOADS, t("Downloads", "다운로드"), "Ctrl+J")];
    if !private {
        library.extend([
            link(HISTORY, t("History", "방문 기록"), "Ctrl+H"),
            link(BOOKMARKS, t("Bookmarks", "북마크 관리"), ""),
            link(EXTENSIONS_BUTTON, t("Extensions", "확장 프로그램"), ""),
            link(VAULT_BUTTON, t("Flit Vault", "로그인 보관함"), ""),
        ]);
    }
    let mut page = vec![
        link(FIND, t("Find in page", "페이지에서 찾기"), "Ctrl+F"),
        link(PRINT, t("Print", "인쇄"), "Ctrl+P"),
    ];
    if !private {
        page.push(link(SAVE_BOOKMARK, t("Bookmark this page", "이 페이지 북마크"), "Ctrl+D"));
    }
    if let Some(percent) = zoom_percent() {
        page.push(Item {
            id: "zoom".into(), title: t("Zoom", "확대/축소").into(), detail: String::new(),
            actions: vec![popup_action(ZOOM_OUT, "−"), popup_action(ZOOM_RESET, &format!("{percent}%")),
                popup_action(ZOOM_IN, "+")],
        });
    }
    if let Some(label) = with_shell(|s| {
        if s.split.is_some() {
            Some(t("Exit split view", "분할 보기 종료"))
        } else if next_split_partner(s).is_some() {
            Some(t("Split view", "분할 보기"))
        } else {
            None
        }
    }).flatten() {
        page.push(link(SPLIT_VIEW, label, r"Ctrl+Shift+\"));
    }
    let mut app = Vec::new();
    if !private {
        app.push(link(SETTINGS, t("Settings", "설정"), ""));
    }
    if !background_tabs_for_release().is_empty() {
        app.push(link(FREE_MEMORY, t("Free memory from background tabs", "백그라운드 탭 메모리 정리"), ""));
    }
    app.extend([
        link(DEVTOOLS, t("Developer tools", "개발자 도구"), "F12"),
        link(UPDATE, t("About Flit", "Flit 정보 · 업데이트"), ""),
        link(EXIT_FLIT, t("Exit Flit", "Flit 종료"), ""),
    ]);
    let mut model = grouped_panel(t("Menu", "메뉴"), vec![
        panel_section("menu-window", "", windows),
        panel_section("menu-library", "", library),
        panel_section("menu-app", "", app),
        panel_section("menu-page", "", page),
    ]);
    model.subtitle = if private {
        t("Private browsing", "비공개 탐색")
    } else {
        t("Regular browsing", "일반 탐색")
    }.into();
    model
}

fn show_menu_panel(ui: Controls) {
    let model = menu_surface();
    let rows = model.sections.iter().map(|section| section.items.len() as i32).sum::<i32>();
    open_popover(ui.window, "menu", Anchor::Below(ui.menu), 318, (94 + rows * 34).min(620), &model);
}

fn show_vault_panel(window: HWND) {
    let Some(ui) = controls() else { return };
    let t = i18n::choose;
    let site = vault_site();
    let (item, subtitle) = if let Some(site) = site.as_ref() {
        let (detail, actions) = match crate::vault_store::has(&site.origin) {
            Ok(true) => (
                t("Login saved", "로그인 정보가 저장되어 있습니다"),
                vec![popup_action(POPUP_VAULT_REMOVE, t("Remove login…", "로그인 정보 삭제…"))],
            ),
            Ok(false) => (
                t("No saved login", "저장된 로그인 정보가 없습니다"),
                vec![popup_action(POPUP_VAULT_SAVE, t("Save login…", "로그인 정보 저장…"))],
            ),
            Err(_) => (t("Could not read the Vault", "보관함을 읽을 수 없습니다"), Vec::new()),
        };
        (Item { id: "vault-current".into(), title: site.origin.clone(), detail: detail.into(), actions },
         t("Current site", "현재 사이트"))
    } else {
        (Item {
            id: "vault-empty".into(),
            title: t("Secure site required", "보안 연결이 필요합니다").into(),
            detail: t("Open an HTTPS site to manage its login.", "HTTPS 사이트를 열면 로그인 정보를 관리할 수 있습니다.").into(),
            actions: Vec::new(),
        }, "")
    };
    PANEL_VAULT_ORIGIN.with_borrow_mut(|origin| *origin = site.map(|site| site.origin));
    let height = if item.actions.is_empty() { 126 } else { 162 };
    let model = grouped_panel(t("Flit Vault", "로그인 보관함"), vec![
        panel_section("vault-site", subtitle, vec![item]),
    ]);
    open_popover(window, "vault-site", Anchor::Below(ui.vault), 332, height, &model);
}

fn completed_download_detail(download: &download_guard::Recent) -> String {
    let t = i18n::choose;
    let size = match download.received_bytes {
        ..0 => None,
        bytes @ 0..1024 => Some(format!("{bytes} B")),
        bytes => {
            let value = bytes as f64;
            let (divisor, unit) = if bytes >= 1 << 30 { (1 << 30, "GiB") }
                else if bytes >= 1 << 20 { (1 << 20, "MiB") }
                else { (1 << 10, "KiB") };
            Some(format!("{:.1} {unit}", value / f64::from(divisor)))
        }
    };
    let age = download.completed_at
        .and_then(|at| SystemTime::now().duration_since(at).ok())
        .map(|age| match age.as_secs() / 60 {
            0 => t("Just now", "방금").into(),
            minutes @ 1..60 => format!("{}{}", minutes, t(" min ago", "분 전")),
            minutes @ 60..1440 => format!("{}{}", minutes / 60, t(" hr ago", "시간 전")),
            minutes => {
                let days = minutes / 1440;
                if i18n::korean() { format!("{days}일 전") }
                else if days == 1 { "1 day ago".into() }
                else { format!("{days} days ago") }
            }
        });
    let parts: Vec<_> = [size, age].into_iter().flatten().collect();
    if parts.is_empty() { t("Done", "완료").into() } else { parts.join(" · ") }
}

fn downloads_surface(window: HWND) -> Surface {
    let t = i18n::choose;
    let recent = download_guard::recent(window as usize, private());
    let mut items = Vec::new();
    if recent.is_empty() {
        items.push(Item {
            id: "empty".into(), title: t("Nothing downloaded in this session", "이번 실행에서 받은 파일이 없습니다").into(),
            detail: String::new(), actions: Vec::new(),
        });
    }
    for download in recent {
        let status = if download.complete {
            completed_download_detail(&download)
        } else if download.paused {
            t("Paused", "일시 중지됨").to_owned()
        } else if download.in_progress && download.percent >= 0 {
            format!("{}%", download.percent)
        } else if download.in_progress {
            t("Downloading", "다운로드 중").to_owned()
        } else {
            t("Stopped", "중지됨").to_owned()
        };
        let id = download.id as usize;
        let actions = if download.complete && !download.path.is_empty() {
            vec![
                popup_action(POPUP_DOWNLOAD_OPEN | id, t("Open", "열기")),
                popup_action(POPUP_DOWNLOAD_FOLDER | id, t("Show in folder", "폴더에서 보기")),
            ]
        } else if download.controllable {
            let (command, label) = if download.paused {
                (POPUP_DOWNLOAD_RESUME, t("Resume", "재개"))
            } else {
                (POPUP_DOWNLOAD_PAUSE, t("Pause", "일시 중지"))
            };
            vec![popup_action(command | id, label),
                 popup_action(POPUP_DOWNLOAD_CANCEL | id, t("Cancel", "취소"))]
        } else {
            Vec::new()
        };
        let name = clean(&download.name, 200);
        items.push(Item {
            id: format!("download-{id}"),
            title: if name.trim().is_empty() { t("Download", "다운로드 파일").into() } else { name },
            detail: status,
            actions,
        });
    }
    grouped_panel(t("Downloads", "다운로드"), vec![
        panel_section("downloads-recent", "", items),
        panel_section("downloads-history", "", vec![
            Item::link(DOWNLOADS, t("Show all downloads", "모든 다운로드 보기").into(), "Ctrl+J".into()),
        ]),
    ])
}

fn show_downloads_panel(window: HWND) {
    let Some(ui) = controls() else { return };
    let downloads = downloads_surface(window);
    let transfers = downloads.sections[0].items.iter()
        .filter(|item| item.id.starts_with("download-"))
        .count().min(6) as i32;
    let height = (118 + transfers * 79).max(144);
    open_popover(window, "downloads", Anchor::Below(ui.downloads), 356, height, &downloads);
}

fn downloads_progress() {
    if let Some(owner) = crate::popover::native_owner("downloads") {
        let _scope = WindowScope::enter(owner as usize);
        crate::popover::update_native(owner, "downloads", &downloads_surface(owner));
    }
}

struct Extension {
    id: String,
    name: String,
    popup: Option<String>,
    enabled: bool,
}

fn read_extension_manifest(path: &std::path::Path) -> Option<serde_json::Value> {
    use std::io::Read;

    const MAX_BYTES: u64 = 1024 * 1024;
    let mut limited = std::fs::File::open(path).ok()?.take(MAX_BYTES + 1);
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

/// Read the live Chromium inventory, including changes still waiting to be written to disk.
fn installed_extensions() -> Vec<Extension> {
    let Some(profile) = PROFILE.with(|slot| slot.borrow().clone()) else {
        return Vec::new();
    };
    let pref = CefString::from("extensions.settings");
    let Some(mut value) = request_context_get_global_context().and_then(|ctx| ctx.preference(Some(&pref))) else {
        return Vec::new();
    };
    let data = CefString::from(&write_json(Some(&mut value), JsonWriterOptions::DEFAULT)).to_string();
    let Ok(serde_json::Value::Object(settings)) = serde_json::from_str(&data) else {
        return Vec::new();
    };
    let default = profile.join("Default");
    let mut list = Vec::with_capacity(settings.len());
    for (id, entry) in &settings {
        // 5 and 10 are Chromium's own component extensions.
        let location = entry.get("location").and_then(|v| v.as_i64()).unwrap_or(0);
        if matches!(location, 5 | 10) || id.len() != 32
            || !id.bytes().all(|byte| (b'a'..=b'p').contains(&byte))
        {
            continue;
        }
        let directory = entry.get("path").and_then(|value| value.as_str()).map(|path| {
            let path = std::path::PathBuf::from(path);
            if path.is_absolute() { path } else { default.join("Extensions").join(path) }
        });
        let stored = entry.get("manifest").filter(|manifest| manifest.is_object());
        // Chromium persists the unpacked extension path without embedding its manifest.
        let unpacked = (location == 4 && stored.is_none()).then(|| {
            directory.as_ref().and_then(|dir| read_extension_manifest(&dir.join("manifest.json")))
        }).flatten();
        let Some(manifest) = stored.or(unpacked.as_ref()) else { continue };
        let mut name = manifest.get("name").and_then(|v| v.as_str()).unwrap_or(id).to_owned();
        if let Some(key) = name.strip_prefix("__MSG_").and_then(|k| k.strip_suffix("__")) {
            let locale = manifest.get("default_locale").and_then(|v| v.as_str()).unwrap_or("en");
            let lookup = |lang: &str| -> Option<String> {
                let file = directory.as_ref()?.join("_locales").join(lang).join("messages.json");
                let messages = read_extension_manifest(&file)?;
                let object = messages.as_object()?;
                let (_, value) = object.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))?;
                value.get("message")?.as_str().map(str::to_owned)
            };
            if let Some(local) = (if i18n::korean() { lookup("ko") } else { None }).or_else(|| lookup(locale)) {
                name = local;
            }
        }
        let popup = ["action", "browser_action"]
            .iter()
            .find_map(|key| manifest.pointer(&format!("/{key}/default_popup"))?.as_str())
            .filter(|p| !p.is_empty())
            .and_then(|path| {
                let base = url::Url::parse(&format!("chrome-extension://{id}/")).ok()?;
                let target = base.join(path.trim_start_matches('/')).ok()?;
                (target.scheme() == base.scheme() && target.host_str() == base.host_str())
                    .then(|| String::from(target))
            });
        let enabled = entry.get("state").and_then(|v| v.as_i64()) != Some(0)
            && entry.get("disable_reasons").is_none_or(|v| v.as_i64() == Some(0) || v.as_array().is_some_and(|a| a.is_empty()));
        list.push(Extension { id: id.clone(), name, popup, enabled });
    }
    list.sort_by(|a, b| (!a.enabled, a.name.to_lowercase()).cmp(&(!b.enabled, b.name.to_lowercase())));
    list
}

fn show_extensions_panel(window: HWND) {
    let Some(ui) = controls() else { return };
    let t = i18n::choose;
    let extensions = installed_extensions();
    EXTENSION_CHOICES.with_borrow_mut(|ids| {
        *ids = extensions.iter().take(12).map(|extension| extension.id.clone()).collect();
    });
    let mut items: Vec<_> = extensions.iter().take(12).enumerate().map(|(index, extension)| {
        Item::link(POPUP_EXTENSION_FIRST + index, clean(&extension.name, 160),
            if extension.enabled { String::new() } else { t("Off", "꺼짐").into() })
    }).collect();
    if items.is_empty() {
        items.push(Item {
            id: "empty".into(), title: t("No extensions yet", "설치된 확장 프로그램이 없습니다").into(),
            detail: String::new(), actions: Vec::new(),
        });
    }
    let height = (158 + items.len() as i32 * 34).min(536);
    let model = grouped_panel(t("Extensions", "확장 프로그램"), vec![
        panel_section("extensions-manage", "", vec![
            Item::link(EXTENSIONS, t("Manage extensions", "확장 프로그램 관리").into(), String::new()),
            Item::link(POPUP_EXTENSION_STORE, t("Chrome Web Store", "Chrome 웹 스토어").into(), String::new()),
        ]),
        panel_section("extensions-installed", t("Installed", "설치된 항목"), items),
    ]);
    open_popover(window, "extensions", Anchor::Below(ui.extensions), 324, height, &model);
}

/// Dispatch a command after the native panel identity has been checked.
fn native_popover_action(owner: HWND, kind: &str, id: usize) {
    if with_shell(|s| s.closing) != Some(false) {
        return;
    }
    match kind {
        crate::popover::ADDRESS_KIND => {
            accept_address_suggestion(owner, Some(id));
        }
        "menu" if PANEL_COMMANDS.contains(&id) => {
            crate::popover::hide();
            unsafe { PostMessageW(owner, WM_COMMAND, id, 0) };
        }
        "menu" if matches!(id, ZOOM_IN | ZOOM_OUT | ZOOM_RESET) => {
            if let Some(host) = host() {
                zoom_page(&host, id);
                unsafe { SetTimer(owner, ZOOM_LABEL_TIMER, 120, None) };
            }
        }
        "vault-site" if matches!(id, POPUP_VAULT_SAVE | POPUP_VAULT_REMOVE) && !private() => {
            crate::popover::hide();
            let removal = id == POPUP_VAULT_REMOVE;
            if let Some(site) = vault_site().filter(|site| {
                PANEL_VAULT_ORIGIN.with_borrow(|origin| origin.as_ref() == Some(&site.origin))
            }) && crate::vault_store::has(&site.origin).ok() == Some(removal) {
                vault_ui::manage(owner, &site, removal, vault_site);
            }
        }
        "downloads" if id == DOWNLOADS => {
            crate::popover::hide();
            unsafe { PostMessageW(owner, WM_COMMAND, DOWNLOADS, 0) };
        }
        "downloads" => {
            let step = id & POPUP_NAMESPACE;
            let control = match step {
                POPUP_DOWNLOAD_PAUSE => Some(download_guard::Control::Pause),
                POPUP_DOWNLOAD_RESUME => Some(download_guard::Control::Resume),
                POPUP_DOWNLOAD_CANCEL => Some(download_guard::Control::Cancel),
                _ => None,
            };
            if let Some(control) = control {
                download_guard::control(owner as usize, private(), id as u32, control);
                return;
            }
            if !matches!(step, POPUP_DOWNLOAD_OPEN | POPUP_DOWNLOAD_FOLDER) {
                return;
            }
            let item = download_guard::recent(owner as usize, private()).into_iter()
                .find(|item| item.id == id as u32 && item.complete);
            if let Some(item) = item {
                crate::popover::hide();
                let (file, args) = if step == POPUP_DOWNLOAD_OPEN {
                    (item.path, String::new())
                } else {
                    ("explorer.exe".into(), format!("/select,\"{}\"", item.path))
                };
                let file = wide(&file);
                let args = wide(&args);
                unsafe {
                    windows_sys::Win32::UI::Shell::ShellExecuteW(
                        owner, windows_sys::w!("open"), file.as_ptr(),
                        if args.len() > 1 { args.as_ptr() } else { null() }, null(), SW_SHOWNORMAL,
                    );
                }
            }
        }
        "extensions" if !private() && id == EXTENSIONS => {
            crate::popover::hide();
            open_builtin("chrome://extensions/");
        }
        "extensions" if !private() && id == POPUP_EXTENSION_STORE => {
            crate::popover::hide();
            let _ = open_tab("https://chromewebstore.google.com/", true, None);
        }
        "extensions" if !private() && id >= POPUP_EXTENSION_FIRST => {
            let choice = EXTENSION_CHOICES.with_borrow(|ids| ids.get(id - POPUP_EXTENSION_FIRST).cloned());
            let extension = choice.and_then(|id| installed_extensions().into_iter().find(|e| e.id == id));
            match extension {
                Some(Extension { popup: Some(popup), enabled: true, .. }) =>
                    crate::popover::show_page(&popup, 360, 520),
                Some(extension) => {
                    crate::popover::hide();
                    open_builtin(&format!("chrome://extensions/?id={}", extension.id));
                }
                None => crate::popover::hide(),
            }
        }
        "vault" if matches!(id & POPUP_NAMESPACE, POPUP_VAULT_YES | POPUP_VAULT_NO) => {
            crate::popover::hide();
            answer_pending_login(owner, id as u32, id & POPUP_NAMESPACE == POPUP_VAULT_YES);
        }
        _ => {}
    }
}

/// The first entry of a CEF string list that CEF lends to a callback.
fn first_string(list: &mut CefStringList) -> Option<String> {
    // CefStringList's own iterator only reads lists it owns, not borrowed ones.
    let list: *mut sys::_cef_string_list_t = list.into();
    if list.is_null() {
        return None;
    }
    unsafe {
        if sys::cef_string_list_size(list) == 0 {
            return None;
        }
        let mut value = std::mem::zeroed();
        if sys::cef_string_list_value(list, 0, &mut value) == 0 {
            return None;
        }
        let text = CefString::from(std::ptr::from_ref(&value)).to_string();
        sys::cef_string_utf16_clear(&mut value);
        Some(text)
    }
}

wrap_download_image_callback! {
    struct FlitFavicon { window: usize, key: usize }
    impl DownloadImageCallback {
        fn on_download_image_finished(
            &self, _url: Option<&CefString>, _status: i32, image: Option<&mut Image>,
        ) {
            let Some(icon) = image.and_then(|image| chrome::Favicon::from_image(image)) else { return };
            let _scope = WindowScope::enter(self.window);
            let changed = change(|s| {
                let tab = s.tabs.iter_mut().find(|tab| tab.key == self.key)?;
                tab.favicon = Some(icon);
                Some(())
            }).flatten().is_some();
            if changed {
                refresh_rows(false);
            }
        }
    }
}

/// The AI session tab ID of a tab in the current window, if an agent drives it.
fn agent_of(key: usize) -> Option<u32> {
    with_shell(|s| s.tabs.iter().find(|tab| tab.key == key)?.agent).flatten()
}

wrap_keyboard_handler! {
    struct FlitKeyboard { window: usize, key: usize }
    impl KeyboardHandler {
        fn on_pre_key_event(
            &self, _browser: Option<&mut Browser>, event: Option<&KeyEvent>,
            _os_event: Option<&mut sys::MSG>, _shortcut: Option<&mut i32>,
        ) -> i32 {
            let Some(event) = event else { return 0 };
            if !matches!(event.type_, KeyEventType::RAWKEYDOWN | KeyEventType::KEYDOWN)
                || event.modifiers & sys::cef_event_flags_t::EVENTFLAG_ALTGR_DOWN.0 as u32 != 0
            {
                return 0;
            }
            if event.windows_key_code == i32::from(VK_ESCAPE)
                && crate::popover::escape(self.window as HWND)
            {
                return 1;
            }
            let Some(command) = keyboard::shortcut(
                event.windows_key_code as u16,
                event.modifiers & sys::cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0 as u32 != 0,
                event.modifiers & sys::cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0 as u32 != 0,
                event.modifiers & sys::cef_event_flags_t::EVENTFLAG_ALT_DOWN.0 as u32 != 0,
            ) else { return 0 };
            unsafe { PostMessageW(self.window as HWND, WM_COMMAND, shortcut_command(command), 0); }
            1
        }
        fn on_key_event(
            &self, browser: Option<&mut Browser>, event: Option<&KeyEvent>, _os_event: Option<&mut sys::MSG>,
        ) -> i32 {
            let modifiers = sys::cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0
                | sys::cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0
                | sys::cef_event_flags_t::EVENTFLAG_ALT_DOWN.0
                | sys::cef_event_flags_t::EVENTFLAG_ALTGR_DOWN.0;
            if event.is_some_and(|e| {
                matches!(e.type_, KeyEventType::RAWKEYDOWN | KeyEventType::KEYDOWN)
                    && e.windows_key_code == i32::from(VK_ESCAPE)
                    && e.modifiers & modifiers as u32 == 0
            }) && browser.is_some_and(|b| b.is_popup() == 0 && b.is_loading() != 0) {
                unsafe { PostMessageW(self.window as HWND, STOP_LOADING, self.key, 0) }
            } else {
                0
            }
        }
    }
}

fn controls() -> Option<Controls> {
    with_shell(|state| state.controls)
}

/// Own DWM surface for a GDI control, so it stays opaque over the window backdrop.
const OPAQUE: u32 = WS_EX_LAYERED;

fn child(parent: HWND, id: usize, class: &str, caption: &str, style: u32, ex_style: u32) -> HWND {
    let class = wide(class);
    let caption = wide(caption);
    unsafe {
        let hwnd = CreateWindowExW(
            ex_style,
            class.as_ptr(),
            caption.as_ptr(),
            style,
            0,
            0,
            1,
            1,
            parent,
            id as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        if !hwnd.is_null() {
            if ex_style & OPAQUE != 0 {
                SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
            }
            SendMessageW(
                hwnd,
                WM_SETFONT,
                GetStockObject(DEFAULT_GUI_FONT) as usize,
                1,
            );
        }
        hwnd
    }
}

fn button(parent: HWND, id: usize, caption: &str) -> HWND {
    child(
        parent,
        id,
        "Button",
        i18n::text(caption),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        OPAQUE,
    )
}

fn edit(parent: HWND, id: usize) -> HWND {
    child(
        parent,
        id,
        "Edit",
        "",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
        OPAQUE,
    )
}

fn edit_cue(id: usize) -> &'static str {
    match id {
        ADDRESS => i18n::text("Search or enter address"),
        FILTER => i18n::choose("Search tabs…", "탭 검색…"),
        FIND_TEXT => i18n::text("Find in page"),
        _ => "",
    }
}

fn side_list(parent: HWND, id: usize) -> HWND {
    child(
        parent,
        id,
        "ListBox",
        "",
        WS_CHILD
            | WS_TABSTOP
            | (LBS_NOTIFY | LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOINTEGRALHEIGHT) as u32,
        OPAQUE,
    )
}

fn static_label(parent: HWND, id: usize, title: &str) -> HWND {
    child(
        parent,
        id,
        "Static",
        i18n::text(title),
        WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE as u32,
        OPAQUE,
    )
}

fn tooltips(ui: Controls) -> HWND {
    use windows_sys::w;
    let label = |en, ko| if i18n::korean() { ko } else { en };
    let class = wide("tooltips_class32");
    let tip = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST,
            class.as_ptr(),
            null(),
            WS_POPUP | TTS_NOPREFIX | TTS_ALWAYSTIP,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            ui.window,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    };
    if tip.is_null() {
        return tip;
    }
    for (hwnd, text) in [
        (
            ui.sidebar,
            label(
                w!("Show or hide vertical tabs"),
                w!("세로 탭 접기 또는 펼치기"),
            ),
        ),
        (ui.back, label(w!("Back (Alt+Left)"), w!("뒤로 (Alt+←)"))),
        (
            ui.forward,
            label(w!("Forward (Alt+Right)"), w!("앞으로 (Alt+→)")),
        ),
        (
            ui.reload,
            label(w!("Reload (Ctrl+R)"), w!("새로고침 (Ctrl+R)")),
        ),
        (
            ui.go,
            label(
                w!("Open (Enter), or open in a new tab (Alt+Enter)"),
                w!("열기 (Enter), 새 탭에서 열기 (Alt+Enter)"),
            ),
        ),
        (ui.add, label(w!("New tab (Ctrl+T)"), w!("새 탭 (Ctrl+T)"))),
        (ui.menu, label(w!("Flit menu"), w!("Flit 메뉴"))),
        (
            ui.downloads,
            label(w!("Downloads (Ctrl+J)"), w!("다운로드 (Ctrl+J)")),
        ),
        (
            ui.vault,
            label(w!("Flit Vault"), w!("로그인 보관함")),
        ),
        (
            ui.free_memory,
            label(
                w!("Free memory from eligible background tabs"),
                w!("백그라운드 탭 메모리 정리"),
            ),
        ),
        (
            ui.tab_status,
            label(
                w!("Hover a tab for its main renderer’s RAM. Tabs can share a renderer."),
                w!(
                    "탭에 마우스를 올리면 렌더러 RAM이 표시됩니다. 여러 탭이 렌더러를 공유할 수 있습니다."
                ),
            ),
        ),
        (
            ui.bookmark,
            label(
                w!("Bookmark this page (Ctrl+D)"),
                w!("현재 페이지 북마크에 추가 (Ctrl+D)"),
            ),
        ),
        (
            ui.find_previous,
            label(w!("Previous match"), w!("이전 일치 항목")),
        ),
        (ui.find_next, label(w!("Next match"), w!("다음 일치 항목"))),
        (
            ui.find_close,
            label(w!("Close find (Esc)"), w!("찾기 닫기 (Esc)")),
        ),
    ] {
        let info = TTTOOLINFOW {
            cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_IDISHWND | TTF_SUBCLASS,
            hwnd: ui.window,
            uId: hwnd as usize,
            lpszText: text.cast_mut(),
            ..Default::default()
        };
        unsafe {
            SendMessageW(tip, TTM_ADDTOOLW, 0, (&info as *const TTTOOLINFOW) as isize);
        }
    }
    tip
}

fn icon(exe: HINSTANCE, width: i32, height: i32) -> HICON {
    let embedded = unsafe {
        LoadImageW(
            exe,
            ICON_ID as *const u16,
            IMAGE_ICON,
            width,
            height,
            LR_SHARED,
        )
    };
    if !embedded.is_null() {
        return embedded.cast();
    }
    let file_icon = std::env::current_exe()
        .ok()
        .and_then(|path| {
            let path = path.with_file_name("Flit.ico");
            path.is_file().then(|| {
                path.as_os_str()
                    .encode_wide()
                    .chain([0])
                    .collect::<Vec<_>>()
            })
        })
        .map_or(null_mut(), |path| unsafe {
            LoadImageW(
                null_mut(),
                path.as_ptr(),
                IMAGE_ICON,
                width,
                height,
                LR_LOADFROMFILE,
            )
        });
    file_icon.cast()
}

pub(crate) fn open(
    profile: &Path,
    initial_url: Option<&str>,
    skip_restore: bool,
) -> Result<(), &'static str> {
    PROFILE.with(|slot| slot.replace(Some(profile.to_owned())));
    let saved = session::load(profile);
    RESTORE_TABS.set(saved.as_ref().is_none_or(|state| state.restore));
    SIDEBAR_DEFAULT.set(saved.as_ref().is_none_or(|state| state.sidebar_open));
    DEFER_BACKGROUND.set(saved.as_ref().is_none_or(|state| state.defer_background));
    MEMORY_SAVER_MINUTES.set(saved.as_ref().map_or(30, |state| state.memory_saver_minutes));
    crate::agent_host::AUTO_CLOSE.store(
        saved.as_ref().is_none_or(|state| state.agent_auto_close),
        std::sync::atomic::Ordering::Relaxed,
    );
    SEARCH_ENGINE.set(
        saved
            .as_ref()
            .map_or_else(SearchEngine::default, |state| state.search_engine),
    );
    chrome::set_acrylic(saved.as_ref().is_some_and(|state| state.acrylic));
    chrome::set_theme(saved.as_ref().map_or(0, |state| state.theme));
    match SiteExceptions::load(profile) {
        Ok(sites) => MEMORY_SITES.with(|slot| *slot.borrow_mut() = sites),
        Err(error) => eprintln!("Flit Browser: Could not read memory exceptions: {error}"),
    }
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let classes = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_WIN95_CLASSES,
    };
    let name = wide(WINDOW_CLASS);
    unsafe {
        if InitCommonControlsEx(&classes) == 0 {
            return Err("Could not initialize Windows browser controls.");
        }
        let exe = GetModuleHandleW(null());
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: exe,
            hIcon: icon(
                exe,
                GetSystemMetrics(SM_CXICON),
                GetSystemMetrics(SM_CYICON),
            ),
            hIconSm: icon(
                exe,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
            ),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszClassName: name.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassExW(&class) == 0 {
            return Err("Could not register the Flit window.");
        }
    }
    if !skip_restore && RESTORE_TABS.get() {
        if let Err(error) = session::backup_before_startup(profile) {
            eprintln!("Flit Browser: Could not keep the previous session copy: {error}");
        }
    }
    // Flit asks to save logins itself (Flit Vault), so Chrome's own offer stays quiet.
    // Saved browser passwords still autofill.
    if let (Some(context), Some(mut off)) = (request_context_get_global_context(), cef::value_create()) {
        off.set_bool(0);
        context.set_preference(
            Some(&CefString::from("credentials_enable_service")),
            Some(&mut off),
            Some(&mut CefString::from("")),
        );
    }
    let opened = match saved.filter(|s| !skip_restore && s.restore) {
        Some(saved) => open_saved_windows(saved, initial_url),
        None => open_window(false, initial_url, None).map(|_| ()),
    };
    if opened.is_ok() {
        crate::agent::host_mode();
        crate::agent_host::serve(profile);
    }
    opened
}

fn open_window(
    private: bool,
    initial_url: Option<&str>,
    saved: Option<WindowState>,
) -> Result<HWND, &'static str> {
    // A Flit the agent starts shows up behind the user's work instead of taking focus.
    let show = if AGENT_LAUNCH.get() && !USER_ACTIVATED.get() {
        SW_SHOWNOACTIVATE
    } else {
        SW_SHOW
    };
    create_window(private, initial_url, saved, show)
}

fn open_saved_windows(saved: Session, initial_url: Option<&str>) -> Result<(), &'static str> {
    OPENING_SESSION.set(true);
    // Start each additional window behind the previously selected one.
    let mut incomplete = false;
    for window in saved.additional_windows {
        if create_window(false, None, Some(window), SW_SHOWNOACTIVATE).is_err() {
            incomplete = true;
        }
    }
    let primary = open_window(false, initial_url, Some(saved.primary));
    OPENING_SESSION.set(false);
    if let Some(window) = primary.as_ref().ok().copied().or_else(normal_window) {
        let _scope = WindowScope::enter(window as usize);
        if primary.is_err() {
            load_waiting_window(window);
        }
        queue_session_save();
        if incomplete || primary.is_err() {
            set_text(
                window,
                &format!("{} — {}", app_title(), i18n::text("Some saved windows could not be opened.")),
            );
        }
        return Ok(());
    }
    primary.map(|_| ())
}

fn monitor_info(monitor: HMONITOR) -> Option<MONITORINFO> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    (unsafe { GetMonitorInfoW(monitor, &mut info) } != 0).then_some(info)
}

fn fitted_placement(saved: WindowPlacement) -> Option<WindowPlacement> {
    let [left, top, right, bottom] = saved.edges()?;
    let rect = RECT {
        left,
        top,
        right,
        bottom,
    };
    let monitor = monitor_info(unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) })?;
    let work = monitor.rcWork;
    saved.fit(
        [work.left, work.top, work.right, work.bottom],
        [MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT],
    )
}

fn create_window(
    private: bool,
    initial_url: Option<&str>,
    saved: Option<WindowState>,
    show: SHOW_WINDOW_CMD,
) -> Result<HWND, &'static str> {
    if exiting() {
        return Err("Flit is exiting.");
    }
    let name = wide(WINDOW_CLASS);
    let caption = wide(if private { i18n::text(PRIVATE_TITLE) } else { APP_TITLE });
    let placement = saved
        .as_ref()
        .filter(|_| !private)
        .and_then(|state| state.placement)
        .and_then(fitted_placement);
    let (x, y, width, height) = placement.map_or((CW_USEDEFAULT, CW_USEDEFAULT, 1160, 790), |p| {
        (p.x, p.y, p.width, p.height)
    });
    let maximized = placement.is_some_and(|p| p.maximized);
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | if maximized { WS_MAXIMIZE } else { 0 };
    let window = unsafe {
        CreateWindowExW(
            0,
            name.as_ptr(),
            caption.as_ptr(),
            style,
            x,
            y,
            width,
            height,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    };
    if window.is_null() {
        return Err("Could not create the Flit window.");
    }
    let _scope = WindowScope::enter(window as usize);
    let ui = Controls {
        window,
        address: edit(window, ADDRESS),
        site_info: button(window, SITE_INFO, "Site information"),
        back: button(window, BACK, "Back"),
        forward: button(window, FORWARD, "Forward"),
        reload: button(window, RELOAD, "Reload"),
        add: button(window, NEW_TAB, "New tab"),
        go: button(window, GO, "Go to address"),
        sidebar: button(window, SIDEBAR, "Show or hide vertical tabs"),
        filter: edit(window, FILTER),
        label: static_label(window, TAB_LABEL, "Tabs"),
        tab_status: static_label(window, TAB_STATUS, "No pages loaded"),
        free_memory: button(window, FREE_MEMORY_BUTTON, "Free memory…"),
        empty: static_label(window, EMPTY, "No matching tabs"),
        menu: button(window, MENU, "Flit menu"),
        downloads: button(window, DOWNLOADS_BUTTON, "Downloads"),
        extensions: button(window, EXTENSIONS_BUTTON, "Extensions"),
        vault: button(window, VAULT_BUTTON, i18n::choose("Flit Vault", "로그인 보관함")),
        bookmark: button(window, SAVE_BOOKMARK, "Bookmark this page"),
        find_text: edit(window, FIND_TEXT),
        find_status: static_label(window, FIND_STATUS, ""),
        find_next: button(window, FIND_NEXT, i18n::choose("Next match", "다음 일치 항목")),
        find_previous: button(window, FIND_PREVIOUS, i18n::choose("Previous match", "이전 일치 항목")),
        find_close: button(window, FIND_CLOSE, i18n::choose("Close find bar", "페이지 찾기 닫기")),
        tabs: child(
            window,
            TABS,
            "ListBox",
            "",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | (LBS_NOTIFY | LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOINTEGRALHEIGHT) as u32,
            OPAQUE,
        ),
        bookmarks: side_list(window, BOOKMARK_LIST),
        bookmark_header: button(window, BOOKMARK_HEADER, "Bookmarks"),
        agents: side_list(window, AGENT_LIST),
        agent_header: button(window, AGENT_HEADER, "AI tabs"),
        // SS_NOTIFY (0x100) so the thumb gets mouse input for dragging.
        // Layered like the other controls, so GDI colors survive on the glass window.
        scroll_thumb: child(window, SIDEBAR_THUMB, "Static", "", WS_CHILD | 0x0100, OPAQUE),
        divider: child(window, SPLIT_DIVIDER, "Button",
            i18n::choose("Resize split view", "분할 너비 조절"),
            WS_CHILD | WS_TABSTOP | BS_OWNERDRAW as u32, OPAQUE),
    };
    if ui.chrome().iter().any(|p| p.is_null())
        || ui.find().iter().any(|p| p.is_null())
        || ui.empty.is_null()
        || ui.tab_status.is_null()
        || ui.chrome().into_iter().chain(ui.find()).any(|hwnd| unsafe {
            SetWindowSubclass(hwnd, Some(chrome_proc), 1, window as usize) == 0
        })
    {
        unsafe {
            DestroyWindow(window);
        }
        return Err("Could not create the Flit navigation controls.");
    }
    unsafe {
        SendMessageW(ui.address, EM_SETLIMITTEXT, 16_384, 0);
        SendMessageW(ui.filter, EM_SETLIMITTEXT, 256, 0);
        SendMessageW(ui.find_text, EM_SETLIMITTEXT, 1024, 0);
        for (field, id) in [(ui.address, ADDRESS), (ui.filter, FILTER), (ui.find_text, FIND_TEXT)] {
            SendMessageW(field, EM_SETCUEBANNER, 0, wide(edit_cue(id)).as_ptr() as isize);
        }
    }
    let tooltip = tooltips(ui);
    let sidebar_open = SIDEBAR_DEFAULT.get();
    WINDOWS.with(|windows| {
        let id = NEXT_WINDOW_ID.get();
        NEXT_WINDOW_ID.set(id + 1);
        windows.borrow_mut().insert(
            window as usize,
            Shell {
                controls: ui,
                id,
                tabs: Vec::new(),
                active: None,
                split: None,
                next_key: 1,
                sidebar_open,
                sidebar_motion: Motion::new(sidebar_open),
                tooltip,
                reload_is_stop: false,
                active_downloads: 0,
                filter: String::new(),
                filter_cursor: FilterCursor::Off,
                rows: Vec::new(),
                page_changes: None,
                page_counts: (0, 0),
                deferred_restore: false,
                font: null_mut(),
                font_dpi: 0,
                find_open: false,
                minimized: false,
                fullscreen: None,
                recently_closed: Vec::new(),
                private,
                address_edited: false,
                updating_address: false,
                row_close: None,
                row_unload: None,
                hovered_row: None,
                side_scroll: 0,
                side_active: None,
                pending_tab_reveal: None,
                memory_preview: None,
                memory_request: 0,
                tab_drag: None,
                links: VecDeque::new(),
                popups: Vec::new(),
                pending_popups: BTreeMap::new(),
                settings_window: None,
                pending_settings: None,
                closing: false,
                closing_session: None,
                memory_queue: VecDeque::new(),
                agent_open: true,
                bookmarks_open: true,
                agent_rows: Vec::new(),
                bookmark_rows: Vec::new(),
            },
        );
    });
    if private {
        unsafe {
            EnableWindow(ui.bookmark, 0);
        }
    } else if show == SW_SHOW {
        LAST_NORMAL.set(window as usize);
    }
    let dpi = unsafe { GetDpiForWindow(window) }.max(96) as i32;
    chrome::apply_backdrop(window, 8 * dpi / 96);
    unsafe { SetTimer(window, MEMORY_SAVER_TIMER, 60_000, None) };
    layout();
    unsafe {
        // Keep the initial maximized state without activating a background window.
        ShowWindow(
            window,
            if maximized && show == SW_SHOWNOACTIVATE {
                SW_SHOWNA
            } else {
                show
            },
        );
        UpdateWindow(window);
    }
    let restored = if let Some(state) = saved.filter(|state| !private && !state.entries.is_empty())
    {
        for entry in state.entries {
            append_tab(&entry.url, &entry.title, entry.pinned, None, entry.bookmark_url)?;
        }
        if let Some(SplitPlacement { left, right, ratio }) = state.split
            && initial_url.is_none()
        {
            change(|s| {
                let left = s.tabs.get(left)?.key;
                let right = s.tabs.get(right)?.key;
                s.split = Some(PageSplit { left, right, ratio });
                Some(())
            });
        }
        Some(state.selected)
    } else {
        None
    };
    if let Some(url) = initial_url {
        // Opening a link from another app should keep the saved tabs. Load only
        // the link, leaving the restored tabs idle until selected.
        add_tab(url)?;
    } else if let Some(selected) = restored {
        let defer = show == SW_SHOWNOACTIVATE && unsafe { GetActiveWindow() } != window;
        select_tab(selected, !defer);
    } else {
        add_tab(HOME)?;
    }
    if show == SW_SHOW && active().is_none() {
        unsafe {
            SetFocus(ui.address);
            SendMessageW(ui.address, EM_SETSEL, 0, -1);
        }
    }
    Ok(window)
}

pub(crate) fn startup_failed(message: &'static str) {
    eprintln!("Flit Browser: {message}");
    EXIT_CODE.set(1);
    if let Some(window) = normal_window() {
        set_text(window, &format!("{APP_TITLE} — {}", i18n::text(message)));
        unsafe {
            PostMessageW(window, WM_CLOSE, 0, 0);
        }
    }
}

pub(crate) fn initialization_failed() -> bool {
    EXIT_CODE.get() != 0 && WINDOWS.with(|windows| windows.borrow().is_empty())
}

pub(crate) fn finish() -> i32 {
    EXIT_CODE.get()
}

fn normal_window() -> Option<HWND> {
    WINDOWS.with(|windows| {
        let windows = windows.borrow();
        windows
            .get(&LAST_NORMAL.get())
            .filter(|s| !s.private && !s.closing)
            .or_else(|| {
                windows
                    .values()
                    .filter(|s| !s.private && !s.closing)
                    .max_by_key(|s| s.id)
            })
            .map(|s| s.controls.window)
    })
}

pub(crate) fn show_existing(url: Option<&str>) {
    if exiting() {
        return;
    }
    if let Some(window) = normal_window() {
        let _scope = WindowScope::enter(window as usize);
        if let Some(url) = url {
            let _ = add_tab(url);
            focus_page();
        }
        unsafe {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            }
            SetForegroundWindow(window);
        }
    } else {
        let _ = open_window(false, url, None);
    }
}

pub(crate) struct Layout {
    pub width: i32,
    pub height: i32,
    pub dpi: i32,
    pub top: i32,
    pub side: i32,
    pub side_full: i32,
    pub side_offset: i32,
    pub fullscreen: bool,
    pub toolbar_y: i32,
    pub toolbar_bottom: i32,
    pub toolbar_right: i32,
    pub focused_field: usize,
    pub address_frame: RECT,
    pub filter_frame: RECT,
    pub find_frame: RECT,
}

impl Layout {
    fn read(
        window: HWND,
        sidebar_open: bool,
        sidebar_position: f64,
        find_open: bool,
        fullscreen: bool,
    ) -> Self {
        let mut rect = RECT::default();
        unsafe {
            GetClientRect(window, &mut rect);
        }
        let width = rect.right.max(1);
        let height = rect.bottom.max(1);
        let dpi = unsafe { GetDpiForWindow(window) }.max(96) as i32;
        let d = |n| n * dpi / 96;
        let caption = if fullscreen { 0 } else { chrome::caption_buttons(window) };
        // Leave enough room for the same toolbar when the tab panel is opened or closed.
        let stacked = caption > 0 && width - caption < d(154 + 193 + 210);
        let toolbar_y = if stacked { d(34) } else { 0 };
        let toolbar_bottom = if fullscreen { 0 } else { toolbar_y + d(43) };
        let toolbar_right = width - if stacked { 0 } else { caption };
        let top = toolbar_bottom + if find_open { d(46) } else { 0 };
        let side_full = (width / 4).clamp(d(184), d(228));
        let side = if fullscreen {
            0
        } else {
            (f64::from(side_full) * sidebar_position).round() as i32
        };
        let side_offset = side - side_full;
        let focused_field = unsafe {
            let focus = GetFocus();
            if IsChild(window, focus) != 0 { GetDlgCtrlID(focus) as usize } else { 0 }
        };
        Self {
            width,
            height,
            dpi,
            top,
            side,
            side_full,
            side_offset,
            fullscreen,
            toolbar_y,
            toolbar_bottom,
            toolbar_right,
            focused_field,
            address_frame: RECT {
                left: d(154),
                top: toolbar_y + d(3),
                right: toolbar_right - d(if sidebar_open || stacked { 157 } else { 193 }),
                bottom: toolbar_y + d(37),
            },
            filter_frame: RECT {
                left: side_offset + d(17),
                top: top + d(47),
                right: side_offset + side_full - d(14),
                bottom: top + d(79),
            },
            find_frame: if find_open {
                RECT {
                    left: (width - d(402)).max(d(12)),
                    top: toolbar_bottom + d(4),
                    right: width - d(10),
                    bottom: toolbar_bottom + d(38),
                }
            } else {
                RECT::default()
            },
        }
    }

    fn scale(&self, dp: i32) -> i32 {
        dp * self.dpi / 96
    }

    pub(crate) fn page_bounds(&self) -> RECT {
        let gap = if self.fullscreen { 0 } else { self.scale(8) };
        RECT {
            left: self.side + gap,
            top: self.top,
            right: (self.width - gap).max(self.side + gap + 1),
            bottom: (self.height - gap).max(self.top + 1),
        }
    }

    pub(crate) fn sidebar_bounds(&self) -> RECT {
        RECT {
            left: self.side_offset + self.scale(9),
            top: self.top,
            right: self.side - self.scale(4),
            bottom: self.height - self.scale(9),
        }
    }
}

fn current_layout(window: HWND) -> Layout {
    let (open, position, find, fullscreen) = with_shell(|s| {
        (
            s.sidebar_open,
            s.sidebar_motion.position(),
            s.find_open,
            s.fullscreen.is_some(),
        )
    })
    .unwrap_or((true, 1.0, false, false));
    Layout::read(window, open, position, find, fullscreen)
}

fn set_bounds(window: HWND, x: i32, y: i32, width: i32, height: i32) {
    if !window.is_null() {
        unsafe {
            let mut old = RECT::default();
            GetWindowRect(window, &mut old);
            SetWindowPos(
                window,
                null_mut(),
                x,
                y,
                width.max(1),
                height.max(1),
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
            // Controls lack CS_HREDRAW: a wider list repaints only the new strip and
            // leaves right-aligned glyphs (row X, memory chip) drawn at the old edge.
            if old.right - old.left != width.max(1) {
                InvalidateRect(window, null(), 0);
            }
        }
    }
}

fn fit_browser(panel: HWND, host: &BrowserHost) {
    let child: HWND = host.window_handle().0.cast();
    let mut client = RECT::default();
    unsafe {
        if child.is_null() || GetParent(child) != panel || GetClientRect(panel, &mut client) == 0 {
            return;
        }
    }
    set_bounds(child, 0, 0, client.right, client.bottom);
}

fn layout() {
    let Some((ui, find_open)) = with_shell(|s| (s.controls, s.find_open)) else {
        return;
    };
    let l = current_layout(ui.window);
    let d = |n| l.scale(n);
    update_fonts(ui, l.dpi);
    apply_chrome_material(ui);
    for hwnd in [
        ui.sidebar,
        ui.back,
        ui.forward,
        ui.reload,
        ui.site_info,
        ui.address,
        ui.go,
        ui.bookmark,
        ui.downloads,
        ui.menu,
        ui.extensions,
        ui.add,
    ] {
        set_visible(hwnd, !l.fullscreen);
    }
    set_visible(ui.vault, !l.fullscreen && !private());
    let y = l.toolbar_y + d(5);
    for (hwnd, left) in [(ui.sidebar, 10), (ui.back, 46), (ui.forward, 81), (ui.reload, 116)] {
        set_bounds(hwnd, d(left), y, d(32), d(30));
    }
    let a = l.address_frame;
    set_bounds(ui.site_info, a.left + d(5), y, d(26), d(30));
    set_bounds(
        ui.address,
        a.left + d(35),
        a.top + d(7),
        a.right - a.left - d(95),
        d(21),
    );
    set_bounds(ui.bookmark, a.right - d(58), y, d(26), d(30));
    set_bounds(ui.go, a.right - d(30), y, d(25), d(30));
    for (hwnd, offset) in [(ui.extensions, 149), (ui.vault, 113), (ui.downloads, 77), (ui.menu, 41)] {
        set_bounds(hwnd, l.toolbar_right - d(offset), y, d(32), d(30));
    }
    for hwnd in ui.find() {
        unsafe {
            ShowWindow(hwnd, if find_open { SW_SHOWNA } else { SW_HIDE });
        }
    }
    if find_open {
        let f = l.find_frame;
        set_bounds(
            ui.find_text,
            f.left + d(13),
            f.top + d(7),
            f.right - f.left - d(164),
            d(21),
        );
        set_bounds(ui.find_status, f.right - d(144), f.top + d(3), d(56), d(28));
        for (hwnd, offset) in [(ui.find_previous, 87), (ui.find_next, 60), (ui.find_close, 32)] {
            set_bounds(hwnd, f.right - d(offset), f.top + d(3), d(26), d(28));
        }
    }
    position_sidebar(ui, &l, false);
    position_active_page(ui.window, &l);
}

fn position_sidebar(ui: Controls, l: &Layout, animating: bool) {
    let d = |n| l.scale(n);
    let visible = !l.fullscreen && (l.side > 0 || with_shell(|s| s.sidebar_open) == Some(true));
    if !visible
        && with_shell(|s| s.memory_preview.is_some() || s.hovered_row.is_some()) == Some(true)
    {
        clear_tab_memory();
    }
    for hwnd in [ui.label, ui.filter, ui.tabs, ui.tab_status, ui.free_memory] {
        set_visible(hwnd, visible);
    }
    if !visible {
        set_visible(ui.scroll_thumb, false);
    }
    // Bookmarks sit above the tabs and AI tabs below them; each section folds.
    let (marks, marks_open, agents, agents_open, filtering) = with_shell(|s| {
        (
            s.bookmark_rows.len(),
            s.bookmarks_open,
            s.agent_rows.len(),
            s.agent_open,
            !s.filter.is_empty(),
        )
    })
    .unwrap_or_default();
    let show_marks = visible && !filtering && marks > 0;
    let show_agents = visible && agents > 0;
    set_visible(ui.bookmark_header, show_marks);
    set_visible(ui.bookmarks, show_marks && marks_open && marks > 0);
    set_visible(ui.agent_header, show_agents);
    set_visible(ui.agents, show_agents && agents_open);
    let show_empty =
        visible && with_shell(|s| !s.filter.is_empty() && s.rows.is_empty()) == Some(true);
    set_visible(ui.empty, show_empty);
    if visible {
        let x = l.side_offset;
        set_bounds(
            ui.label,
            x + d(18),
            l.top + d(11),
            l.side_full - d(128),
            d(30),
        );
        set_bounds(
            ui.add,
            x + l.side_full - d(105),
            l.top + d(10),
            d(89),
            d(31),
        );
        let f = l.filter_frame;
        set_bounds(
            ui.filter,
            f.left + d(27),
            f.top + d(6),
            f.right - f.left - d(35),
            d(21),
        );
        // One column scrolls as a whole: bookmarks, then tabs, then AI tabs, each at its
        // full height. Whatever falls outside the band is clipped by a window region.
        let width = l.side_full - d(29);
        let header = d(26);
        let band = (l.top + d(88), l.height - d(69));
        let item = |list: HWND| unsafe { SendMessageW(list, LB_GETITEMHEIGHT, 0, 0) }.max(1) as i32;
        let tab_count = unsafe { SendMessageW(ui.tabs, LB_GETCOUNT, 0, 0) }.max(0) as i32;
        let marks_height = if marks_open { item(ui.bookmarks) * marks as i32 } else { 0 };
        let tabs_height = (item(ui.tabs) * tab_count).max(d(34));
        let agents_height = if agents_open { item(ui.agents) * agents as i32 } else { 0 };
        let mut content = tabs_height;
        if show_marks {
            content += header + d(2) + if marks_height > 0 { marks_height + d(6) } else { 0 };
        }
        if show_agents {
            content += d(6) + header + d(2) + agents_height;
        }
        let room = (band.1 - band.0).max(0);
        let scroll = change(|s| {
            s.side_scroll = s.side_scroll.clamp(0, (content - room).max(0));
            s.side_scroll
        })
        .unwrap_or(0);
        // A thin rounded thumb in the margin right of the rows, only when there is more to see.
        let scrollable = content > room && room > 0;
        set_visible(ui.scroll_thumb, scrollable);
        if scrollable {
            let length = (room * room / content).max(d(28)).min(room);
            let offset = (room - length) * scroll / (content - room);
            let thickness = d(4);
            set_bounds(ui.scroll_thumb, x + l.side_full - d(10), band.0 + offset, thickness, length);
            round_thumb(ui.scroll_thumb, thickness, length);
            SIDE_METRICS.set((content, room, length));
        }
        let mut y = band.0 - scroll;
        let mut place = |hwnd: HWND, height: i32, after: i32| {
            set_bounds(hwnd, x + d(16), y, width, height);
            clip_to_band(hwnd, y, height, band);
            y += height + after;
        };
        if show_marks {
            place(ui.bookmark_header, header, d(2));
            if marks_open && marks > 0 {
                place(ui.bookmarks, marks_height, d(6));
            }
        }
        place(ui.tabs, tabs_height, d(6));
        if show_agents {
            place(ui.agent_header, header, d(2));
            if agents_open {
                place(ui.agents, agents_height, 0);
            }
        }
        set_bounds(
            ui.tab_status,
            x + d(18),
            l.height - d(65),
            l.side_full - d(34),
            d(22),
        );
        set_bounds(
            ui.free_memory,
            x + d(16),
            l.height - d(42),
            l.side_full - d(30),
            d(29),
        );
        set_bounds(
            ui.empty,
            x + d(22),
            l.top + d(88),
            l.side_full - d(41),
            d(34),
        );
    } else {
        if l.toolbar_y != 0 {
            set_bounds(ui.add, d(10), d(2), d(32), d(28));
        } else {
            set_bounds(ui.add, l.toolbar_right - d(185), l.toolbar_y + d(5), d(32), d(30));
        }
    }
    if !animating {
        unsafe {
            InvalidateRect(ui.add, null(), 0);
        }
    }
}

thread_local! {
    /// Last sidebar layout: (content height, visible band height, thumb length).
    static SIDE_METRICS: Cell<(i32, i32, i32)> = const { Cell::new((0, 0, 0)) };
    /// A thumb drag in progress: (screen y where it started, scroll at that moment).
    static THUMB_DRAG: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
}

/// Give the thumb fully rounded ends; skipped when its size has not changed.
fn round_thumb(hwnd: HWND, width: i32, length: i32) {
    use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, GetWindowRgnBox, SetWindowRgn};
    let mut current = RECT::default();
    if unsafe { GetWindowRgnBox(hwnd, &mut current) } > 1
        && (current.right, current.bottom) == (width, length)
    {
        return;
    }
    unsafe {
        let region = CreateRoundRectRgn(0, 0, width + 1, length + 1, width, width);
        if !region.is_null() {
            SetWindowRgn(hwnd, region, 1);
        }
    }
}

/// Drag the thumb: the column moves in proportion, like a regular scrollbar.
fn drag_thumb(hwnd: HWND, message: u32) -> bool {
    let at = unsafe { GetMessagePos() };
    let screen_y = ((at >> 16) & 0xffff) as i16 as i32;
    match message {
        WM_LBUTTONDOWN => {
            let scroll = with_shell(|s| s.side_scroll).unwrap_or(0);
            THUMB_DRAG.set(Some((screen_y, scroll)));
            unsafe { SetCapture(hwnd) };
        }
        WM_MOUSEMOVE => {
            let Some((start, scroll)) = THUMB_DRAG.get() else {
                return false;
            };
            let (content, room, length) = SIDE_METRICS.get();
            let travel = (room - length).max(1);
            let next = scroll + (screen_y - start) * (content - room) / travel;
            change(|s| s.side_scroll = next);
            if let Some(ui) = controls() {
                position_sidebar(ui, &current_layout(ui.window), false);
            }
        }
        WM_LBUTTONUP | WM_CAPTURECHANGED => {
            if THUMB_DRAG.take().is_some() && message == WM_LBUTTONUP {
                unsafe { ReleaseCapture() };
            }
        }
        _ => return false,
    }
    true
}

/// Show only the part of a sidebar control that lies inside the scrolling band.
fn clip_to_band(hwnd: HWND, y: i32, height: i32, band: (i32, i32)) {
    use windows_sys::Win32::Graphics::Gdi::{CreateRectRgn, GetWindowRgnBox, NULLREGION, SetWindowRgn};
    let mut window = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut window) };
    let top = (band.0 - y).clamp(0, height);
    let bottom = (band.1 - y).clamp(top, height);
    let want = if top == bottom {
        RECT::default()
    } else {
        RECT { left: 0, top, right: (window.right - window.left).max(1), bottom }
    };
    let mut current = RECT::default();
    let kind = unsafe { GetWindowRgnBox(hwnd, &mut current) };
    let same = if top == bottom {
        kind == NULLREGION
    } else {
        kind > NULLREGION && (current.left, current.top, current.right, current.bottom) == (want.left, want.top, want.right, want.bottom)
    };
    if !same {
        unsafe {
            let region = CreateRectRgn(want.left, want.top, want.right, want.bottom);
            if !region.is_null() {
                SetWindowRgn(hwnd, region, 1);
            }
        }
    }
}

/// Screen rectangle of the part of a control its window region leaves visible.
fn shown_rect(hwnd: HWND) -> Option<RECT> {
    use windows_sys::Win32::Graphics::Gdi::{GetWindowRgnBox, NULLREGION};
    let mut r = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut r) } == 0 {
        return None;
    }
    let mut b = RECT::default();
    match unsafe { GetWindowRgnBox(hwnd, &mut b) } {
        NULLREGION => None,
        0 => Some(r), // no region: the whole window shows
        _ => Some(RECT { left: r.left + b.left, top: r.top + b.top, right: r.left + b.right, bottom: r.top + b.bottom }),
    }
}

/// Whether a wheel message's screen point is over the open sidebar's scrolling band.
fn over_sidebar(window: HWND, lp: LPARAM) -> bool {
    if with_shell(|s| s.sidebar_open && s.fullscreen.is_none()) != Some(true) {
        return false;
    }
    let mut p = POINT { x: (lp & 0xffff) as i16 as i32, y: ((lp >> 16) & 0xffff) as i16 as i32 };
    unsafe { ScreenToClient(window, &mut p) };
    let l = current_layout(window);
    p.x >= l.side_offset + l.scale(9)
        && p.x < l.side - l.scale(4)
        && p.y >= l.top + l.scale(88)
        && p.y < l.height - l.scale(69)
}

/// Wheel over the sidebar: move the whole column, three tab rows per notch.
fn scroll_sidebar(delta: i32) {
    let Some(ui) = controls() else {
        return;
    };
    let dpi = unsafe { GetDpiForWindow(ui.window) }.max(96) as i32;
    let step = 34 * 3 * dpi / 96 * delta / 120;
    change(|s| s.side_scroll -= step);
    clear_tab_memory();
    position_sidebar(ui, &current_layout(ui.window), false);
    refresh_hover();
}

/// Scroll the sidebar so the active tab's row is in view, once per activation.
fn reveal_active_tab(ui: Controls, active_bookmark: Option<usize>) {
    let (list, index) = active_bookmark
        .map(|index| (ui.bookmarks, index as isize))
        .unwrap_or_else(|| (ui.tabs, unsafe { SendMessageW(ui.tabs, LB_GETCURSEL, 0, 0) }));
    let Some(key) = with_shell(|s| s.active).flatten() else {
        return;
    };
    if index < 0 || unsafe { IsWindowVisible(list) } == 0
        || change(|s| s.side_active.replace(key) == Some(key)) != Some(false)
    {
        return;
    }
    reveal_sidebar_row(ui, list, index as usize);
}

fn reveal_tab_key(key: usize) {
    if let Some((ui, index)) = with_shell(|s| {
        s.rows
            .iter()
            .position(|row| row.key == key)
            .map(|index| (s.controls, index))
    })
    .flatten()
    {
        reveal_sidebar_row(ui, ui.tabs, index);
    }
}

fn reveal_sidebar_row(ui: Controls, list: HWND, index: usize) {
    let Some(row) = tab_rect(list, index) else {
        return;
    };
    let l = current_layout(ui.window);
    let band = (l.top + l.scale(88), l.height - l.scale(69));
    let mut origin = POINT::default();
    unsafe {
        windows_sys::Win32::Graphics::Gdi::ClientToScreen(list, &mut origin);
        ScreenToClient(ui.window, &mut origin);
    }
    let (top, bottom) = (origin.y + row.top, origin.y + row.bottom);
    let shift = if top < band.0 {
        top - band.0
    } else if bottom > band.1 {
        bottom - band.1
    } else {
        return;
    };
    change(|s| s.side_scroll += shift);
    position_sidebar(ui, &l, false);
}

fn set_visible(hwnd: HWND, show: bool) {
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_VISIBLE != 0;
        if current != show {
            ShowWindow(hwnd, if show { SW_SHOWNA } else { SW_HIDE });
        }
    }
}

fn position_page(panel: HWND, browser: Option<&Browser>, p: RECT) {
    let (width, height) = (p.right - p.left, p.bottom - p.top);
    set_bounds(panel, p.left, p.top, width, height);
    if let Some(host) = browser.and_then(|browser| browser.host()) {
        fit_browser(panel, &host);
    }
}

fn position_active_page(window: HWND, l: &Layout) {
    if let Some((divider, gap, pages)) = with_shell(|s| {
        let page = l.page_bounds();
        let split = s.split.filter(|_| !s.deferred_restore && s.visible_partner().is_some())
            .and_then(|split| split.bounds(l).map(|rects| (split, rects)));
        let get = |key: Option<usize>, rect| {
            let tab = s.tabs.iter().find(|tab| Some(tab.key) == key)?;
            Some((tab.panel, tab.browser.clone(), rect))
        };
        if let Some((split, [left, gap, right])) = split {
            (s.controls.divider, Some(gap), [get(Some(split.left), left), get(Some(split.right), right)])
        } else {
            (s.controls.divider, None, [get(s.active, page), None])
        }
    }) {
        if let Some(gap) = gap {
            set_bounds(divider, gap.left, gap.top, gap.right - gap.left, gap.bottom - gap.top);
        }
        set_visible(divider, gap.is_some());
        for (panel, browser, bounds) in pages.into_iter().flatten() {
            position_page(panel, browser.as_ref(), bounds);
        }
    }
    unsafe {
        InvalidateRect(window, null(), 0);
    }
}

fn split_eligible(tab: &Tab) -> bool {
    tab.agent.is_none()
        && !tab.retiring
        && !tab.unloading
        && !tab.close_requested
        && tab.download_close == DownloadClose::None
        && (tab.url.starts_with("https://") || tab.url.starts_with("http://"))
}

fn next_split_partner(s: &Shell) -> Option<usize> {
    let active = s.active?;
    let index = s
        .tabs
        .iter()
        .position(|tab| tab.key == active && split_eligible(tab))?;
    s.tabs[index + 1..]
        .iter()
        .chain(&s.tabs[..index])
        .find(|tab| split_eligible(tab))
        .map(|tab| tab.key)
}

fn set_native_page_visible(native: Option<&Window>, visible: bool) {
    let Some(native) = native.filter(|native| native.is_closed() == 0) else {
        return;
    };
    if visible {
        native.show();
    } else {
        native.hide();
    }
}

fn hide_page(panel: HWND, native: Option<Window>) {
    close_start(panel);
    // An ancestor HWND hide alone leaves Chromium's WebContents visible.
    set_native_page_visible(native.as_ref(), false);
    set_visible(panel, false);
}

fn sync_minimized_pages(minimized: bool) {
    let Some(pages) = change(|s| {
        if std::mem::replace(&mut s.minimized, minimized) == minimized || s.closing {
            return None;
        }
        Some([s.active, s.visible_partner()].map(|key| {
            s.tabs
                .iter()
                .find(|tab| {
                    Some(tab.key) == key && !tab.retiring && !tab.unloading && !tab.close_requested
                })
                .and_then(|tab| tab.native_window.clone())
        }))
    })
    .flatten() else {
        return;
    };
    // CEF's child widget does not receive the ancestor's minimize transition.
    for page in &pages {
        set_native_page_visible(page.as_ref(), !minimized);
    }
}

fn end_split(reflow: bool) -> bool {
    let Some((ui, partner)) = change(|s| {
        let other = s.split.take()?.other(s.active?);
        let partner = s
            .tabs
            .iter_mut()
            .find(|tab| Some(tab.key) == other)
            .map(|tab| {
                tab.hidden_since = Instant::now();
                (tab.panel, tab.native_window.clone())
            });
        Some((s.controls, partner))
    })
    .flatten() else {
        return false;
    };
    if unsafe { GetCapture() } == ui.divider {
        unsafe { ReleaseCapture() };
    }
    set_visible(ui.divider, false);
    if let Some((panel, native)) = partner {
        hide_page(panel, native);
    }
    if reflow {
        position_active_page(ui.window, &current_layout(ui.window));
        refresh_rows(false);
        queue_session_save();
    }
    true
}

fn reveal_split_partner() {
    let Some((ui, key, panel, native)) = change(|s| {
        if s.closing || s.deferred_restore {
            return None;
        }
        let key = s.visible_partner()?;
        let tab = s.tabs.iter_mut().find(|tab| {
            tab.key == key && !tab.retiring && !tab.unloading && !tab.close_requested
        })?;
        tab.background_warm = false;
        Some((s.controls, key, tab.panel, tab.native_window.clone()))
    })
    .flatten() else {
        return;
    };
    if unsafe { GetWindowLongPtrW(panel, GWL_STYLE) } as u32 & WS_VISIBLE == 0 {
        set_visible(panel, true);
        set_native_page_visible(native.as_ref(), unsafe { IsIconic(ui.window) } == 0);
    }
    if let Err(error) = load_deferred(key) {
        end_split(true);
        set_text(
            ui.window,
            &format!("{} — {}", app_title(), i18n::text(error)),
        );
    }
}

fn can_split_with(s: &Shell, key: usize) -> bool {
    s.active.is_some_and(|active| {
        active != key
            && !s.closing
            && s.tabs
                .iter()
                .any(|tab| tab.key == active && split_eligible(tab))
            && s.tabs
                .iter()
                .any(|tab| tab.key == key && split_eligible(tab))
    })
}

fn split_with(key: usize) {
    if with_shell(|s| s.visible_partner()) == Some(Some(key)) {
        end_split(true);
        focus_page();
        return;
    }
    if with_shell(|s| can_split_with(s, key)) != Some(true) {
        return;
    }
    end_split(false);
    if let Some(window) = change(|s| {
        s.split = Some(PageSplit {
            left: s.active?,
            right: key,
            ratio: 500,
        });
        Some(s.controls.window)
    })
    .flatten()
    {
        position_active_page(window, &current_layout(window));
        reveal_split_partner();
        focus_page();
        refresh_rows(false);
        queue_session_save();
    }
}

fn toggle_split() {
    if end_split(true) {
        focus_page();
    } else if let Some(key) = with_shell(next_split_partner).flatten() {
        split_with(key);
    }
}

fn focus_split_page(key: usize) {
    if let Some(panel) = with_shell(|s| {
        if s.closing || s.visible_partner() != Some(key) {
            return None;
        }
        s.tabs
            .iter()
            .find(|tab| tab.key == key && !tab.close_requested)
            .map(|tab| tab.panel)
    })
    .flatten()
    {
        let focus = unsafe { GetFocus() };
        if focus == panel || unsafe { IsChild(panel, focus) } != 0 {
            select_key(key);
        }
    }
}

fn resize_split(window: HWND, ratio: u16) {
    let changed = change(|s| {
        let Some(split) = s.split.as_mut() else {
            return false;
        };
        let ratio = ratio.clamp(200, 800);
        if split.ratio == ratio {
            return false;
        }
        split.ratio = ratio;
        true
    });
    if changed == Some(true) {
        position_active_page(window, &current_layout(window));
        queue_session_save();
    }
}

fn split_divider_message(
    hwnd: HWND,
    owner: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> Option<LRESULT> {
    match msg {
        WM_SETCURSOR => {
            unsafe { SetCursor(LoadCursorW(null_mut(), IDC_SIZEWE)) };
            return Some(1);
        }
        WM_LBUTTONDOWN => unsafe {
            SetFocus(hwnd);
            SetCapture(hwnd);
        },
        WM_LBUTTONUP if unsafe { GetCapture() } == hwnd => unsafe {
            ReleaseCapture();
        },
        WM_LBUTTONDBLCLK => resize_split(owner, 500),
        WM_MOUSEMOVE if unsafe { GetCapture() } == hwnd => {
            let mut point = POINT {
                x: (lp as u16 as i16).into(),
                y: ((lp >> 16) as u16 as i16).into(),
            };
            if unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut point) } != 0
                && unsafe { ScreenToClient(owner, &mut point) } != 0
            {
                let layout = current_layout(owner);
                let page = layout.page_bounds();
                let gap = layout.scale(10);
                let width = (page.right - page.left - gap).max(1);
                let offset = (point.x - page.left - gap / 2).clamp(0, width);
                resize_split(owner, (offset * 1000 / width) as u16);
            }
        }
        WM_KEYDOWN if matches!(wp as u16, VK_LEFT | VK_RIGHT) => {
            let step = if wp as u16 == VK_LEFT { -50 } else { 50 };
            if let Some(ratio) = with_shell(|s| s.split.map(|split| split.ratio)).flatten() {
                resize_split(owner, (i32::from(ratio) + step).clamp(200, 800) as u16);
            }
        }
        _ => return None,
    }
    Some(0)
}

/// Height of a bookmark or AI tab row, in DIPs.
const SECTION_ROW: i32 = 30;

/// Color-keyed pixels let clicks through to the parent. Use the full visible
/// bounds of each control so its target is larger than the painted glyph.
fn material_target(point: POINT, children: impl IntoIterator<Item = HWND>) -> Option<HWND> {
    children.into_iter().find(|&hwnd| {
        (unsafe { IsWindowVisible(hwnd) } != 0)
            && shown_rect(hwnd).is_some_and(|r| {
                (r.left..r.right).contains(&point.x) && (r.top..r.bottom).contains(&point.y)
            })
    })
}

fn material_control_at() -> Option<(HWND, LPARAM)> {
    chrome::material_key()?;
    let ui = controls()?;
    // Where the message happened on screen; the window's own client
    // coordinates are not reliable across its merged caption frame.
    let at = unsafe { GetMessagePos() };
    let point = POINT {
        x: (at & 0xffff) as i16 as i32,
        y: ((at >> 16) & 0xffff) as i16 as i32,
    };
    let children = ui.material_toolbar().into_iter().chain([
        ui.add, ui.tabs, ui.bookmarks, ui.agents, ui.bookmark_header, ui.agent_header, ui.free_memory,
    ]);
    let hwnd = material_target(point, children)?;
    let mut local = point;
    unsafe { ScreenToClient(hwnd, &mut local) };
    Some((hwnd, ((local.y as u16 as isize) << 16) | local.x as u16 as isize))
}

fn apply_chrome_material(ui: Controls) {
    let key = chrome::material_key();
    for hwnd in ui.material_toolbar().into_iter().chain([
        ui.tabs,
        ui.label,
        ui.tab_status,
        ui.empty,
        ui.add,
        ui.free_memory,
        ui.bookmarks,
        ui.bookmark_header,
        ui.agents,
        ui.agent_header,
    ]) {
        unsafe {
            match key {
                Some(key) => SetLayeredWindowAttributes(hwnd, key, 255, LWA_COLORKEY),
                None => SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA),
            };
        }
    }
}

fn update_fonts(ui: Controls, dpi: i32) {
    if with_shell(|s| s.font_dpi) == Some(dpi) {
        return;
    }
    let font = unsafe {
        CreateFontW(
            -(14 * dpi / 96),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            windows_sys::w!("Segoe UI"),
        )
    };
    let previous = change(|s| {
        s.font_dpi = dpi;
        std::mem::replace(&mut s.font, font)
    })
    .unwrap_or(null_mut());
    unsafe {
        let applied = if font.is_null() {
            GetStockObject(DEFAULT_GUI_FONT)
        } else {
            font
        };
        for hwnd in ui
            .chrome()
            .into_iter()
            .chain(ui.find())
            .chain([ui.empty, ui.tab_status])
        {
            SendMessageW(hwnd, WM_SETFONT, applied as usize, 1);
        }
        SendMessageW(ui.tabs, LB_SETITEMHEIGHT, 0, (34 * dpi / 96) as isize);
        for list in [ui.bookmarks, ui.agents] {
            SendMessageW(list, LB_SETITEMHEIGHT, 0, (SECTION_ROW * dpi / 96) as isize);
        }
        if !previous.is_null() {
            DeleteObject(previous);
        }
    }
}

fn set_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    unsafe {
        SetWindowTextW(hwnd, text.as_ptr());
    }
}

fn show_address(hwnd: HWND, url: &str) {
    if let Some(ui) = controls() {
        dismiss_address_suggestions(ui.window);
    }
    change(|s| s.updating_address = true);
    let shown = if url == HOME {
        ""
    } else {
        url
    };
    set_text(hwnd, shown);
    change(|s| {
        s.updating_address = false;
        s.address_edited = false;
    });
}

fn dismiss_address_suggestions(owner: HWND) -> bool {
    unsafe { KillTimer(owner, ADDRESS_TIMER) };
    ADDRESS_SUGGESTIONS.with_borrow_mut(|current| {
        if current.as_ref().is_some_and(|s| s.owner == owner) {
            *current = None;
        }
    });
    crate::popover::dismiss_address(owner)
}

fn schedule_address_suggestions(ui: Controls) {
    if unsafe { GetFocus() } == ui.address && !keyboard::edit_composing(ui.address) {
        unsafe { SetTimer(ui.window, ADDRESS_TIMER, 90, None) };
    }
}

fn address_match(title: &str, url: &str, query: &str) -> Option<u8> {
    let title = title.to_lowercase();
    let url = url.to_lowercase();
    let address = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
    let address = address.strip_prefix("www.").unwrap_or(address);
    if title == query || address == query || url == query {
        Some(0)
    } else if title.starts_with(query) || address.starts_with(query) || url.starts_with(query) {
        Some(1)
    } else if query
        .split_whitespace()
        .all(|word| title.contains(word) || url.contains(word))
    {
        Some(2)
    } else {
        None
    }
}

fn suggestible_tab(tab: &Tab, active: Option<usize>) -> bool {
    tab.agent.is_none()
        && !tab.close_requested
        && !tab.retiring
        && !tab.unloading
        && tab.download_close == DownloadClose::None
        && Some(tab.key) != active
}

fn address_matches(query: &str) -> Vec<AddressSuggestion> {
    let mut ranked = Vec::with_capacity(ADDRESS_LIMIT + 1);
    let mut offer = |title: &str, url: &str, rank: u8, tab: Option<usize>| {
        let index =
            ranked.partition_point(|(r, _, _): &(u8, String, AddressSuggestion)| *r <= rank);
        if index >= ADDRESS_LIMIT || ranked.iter().any(|(_, address, _)| address == url) {
            return;
        }
        let target = if let Some(key) = tab {
            AddressTarget::Tab(key)
        } else {
            let Some(url) = session::stored_url(url)
                .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
            else {
                return;
            };
            AddressTarget::Bookmark(url)
        };
        let id = ADDRESS_ACTION_ID.get();
        ADDRESS_ACTION_ID.set(id.wrapping_add(1).max(1));
        let title = clean(title.trim(), 56);
        let title = if title.is_empty() {
            clean(url, 56)
        } else {
            title
        };
        ranked.insert(
            index,
            (
                rank,
                url.to_owned(),
                AddressSuggestion { id, title, target },
            ),
        );
        ranked.truncate(ADDRESS_LIMIT);
    };
    let normal = with_shell(|s| {
        for tab in s.tabs.iter().filter(|tab| suggestible_tab(tab, s.active)) {
            if let Some(rank) = address_match(&tab.title, &tab.url, query) {
                offer(&tab_label(tab), &tab.url, rank * 2, Some(tab.key));
            }
        }
        !s.private
    });
    if normal == Some(true) {
        // Refresh only if the engine's bookmark file changed since the last read.
        reload_bookmarks();
        SIDEBAR_BOOKMARKS.with_borrow(|marks| {
            for row in marks.iter() {
                if let Some(rank) = address_match(&row.title, &row.url, query) {
                    offer(&row.title, &row.url, rank * 2 + 1, None);
                }
            }
        });
    }
    ranked.into_iter().map(|(_, _, item)| item).collect()
}

fn refresh_address_suggestions(owner: HWND) {
    let Some(ui) = controls() else { return };
    if exiting()
        || unsafe { GetFocus() } != ui.address
        || unsafe { IsWindowVisible(ui.address) } == 0
        || keyboard::edit_composing(ui.address)
        || with_shell(|s| s.address_edited && !s.closing) != Some(true)
    {
        return;
    }
    let input = text(ui.address, 16_384);
    let query = input.trim();
    let items = if !query.is_empty() && query.len() <= 512 && !query.starts_with('?') {
        address_matches(&query.to_lowercase())
    } else {
        Vec::new()
    };
    if items.is_empty() {
        dismiss_address_suggestions(owner);
        return;
    }
    ADDRESS_SUGGESTIONS.with_borrow_mut(|current| {
        *current = Some(AddressSuggestions {
            owner,
            input,
            items,
            selected: None,
        });
    });
    render_address_suggestions(owner);
}

fn render_address_suggestions(owner: HWND) -> bool {
    let list = ADDRESS_SUGGESTIONS.with_borrow(|current| {
        let suggestions = current.as_ref().filter(|s| s.owner == owner)?;
        Some(
            suggestions
                .items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let detail = match item.target {
                        AddressTarget::Tab(_) => i18n::choose("Switch to tab", "탭으로 이동"),
                        AddressTarget::Bookmark(_) => i18n::choose("Bookmark", "북마크"),
                    };
                    let mut row = Item::link(item.id, item.title.clone(), detail.into());
                    row.actions[0].selected = suggestions.selected == Some(index);
                    row
                })
                .collect::<Vec<_>>(),
        )
    });
    let Some(list) = list else { return false };
    let frame = current_layout(owner).address_frame;
    let mut first = POINT {
        x: frame.left,
        y: frame.top,
    };
    let mut last = POINT {
        x: frame.right,
        y: frame.bottom,
    };
    let screen = |point: &mut POINT| unsafe {
        windows_sys::Win32::Graphics::Gdi::ClientToScreen(owner, point) != 0
    };
    let height = 30 + list.len() as i32 * 46;
    let mut surface = popup_surface("", list);
    surface.preserve_input_focus = true;
    if screen(&mut first)
        && screen(&mut last)
        && crate::popover::show_address(
            owner,
            RECT {
                left: first.x,
                top: first.y,
                right: last.x,
                bottom: last.y,
            },
            height,
            &surface,
        )
    {
        return true;
    }
    dismiss_address_suggestions(owner);
    false
}

fn move_address_suggestion(ui: Controls, down: bool) -> bool {
    if crate::popover::native_owner(crate::popover::ADDRESS_KIND) != Some(ui.window) {
        return false;
    }
    let input = text(ui.address, 16_384);
    let moved = ADDRESS_SUGGESTIONS.with_borrow_mut(|current| {
        let Some(s) = current
            .as_mut()
            .filter(|s| s.owner == ui.window && s.input == input)
        else {
            return false;
        };
        s.selected = if down {
            s.selected
                .map_or(Some(0), |i| (i + 1 < s.items.len()).then_some(i + 1))
        } else {
            s.selected
                .map_or(Some(s.items.len() - 1), |i| i.checked_sub(1))
        };
        true
    });
    moved && render_address_suggestions(ui.window)
}

fn accept_address_suggestion(owner: HWND, action: Option<usize>) -> bool {
    let Some(ui) = controls() else { return false };
    if exiting()
        || unsafe { GetFocus() } != ui.address
        || keyboard::edit_composing(ui.address)
        || crate::popover::native_owner(crate::popover::ADDRESS_KIND) != Some(owner)
    {
        return false;
    }
    let input = text(ui.address, 16_384);
    let target = ADDRESS_SUGGESTIONS.with_borrow_mut(|current| {
        let s = current
            .as_mut()
            .filter(|s| s.owner == owner && s.input == input)?;
        let index = match action {
            Some(action) => s.items.iter().position(|item| item.id == action)?,
            None => s.selected?,
        };
        Some(s.items.swap_remove(index).target)
    });
    let Some(target) = target else { return false };
    dismiss_address_suggestions(owner);
    match target {
        AddressTarget::Tab(key) => {
            if with_shell(|s| {
                s.tabs
                    .iter()
                    .any(|tab| tab.key == key && suggestible_tab(tab, s.active))
            }) != Some(true)
            {
                return false;
            }
            select_key(key);
            focus_page();
        }
        AddressTarget::Bookmark(url) => submit_address(ui, false, &url),
    }
    true
}

fn active() -> Option<Browser> {
    with_shell(|s| {
        s.tabs
            .iter()
            .find(|t| Some(t.key) == s.active)?
            .browser
            .clone()
    })
    .flatten()
}

fn text(hwnd: HWND, limit: i32) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) }.clamp(0, limit);
    let mut text = vec![0; length as usize + 1];
    let read = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), length + 1) }.max(0) as usize;
    String::from_utf16_lossy(&text[..read])
}

fn clean(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}

fn capture_labels(tab: &Tab) -> Option<(&'static str, &'static str)> {
    match (tab.has_video_access, tab.has_audio_access) {
        (true, true) => Some((i18n::text("Cam/Mic on"), i18n::text("Camera + mic in use"))),
        (true, false) => Some((i18n::text("Camera on"), i18n::text("Camera in use"))),
        (false, true) => Some((i18n::text("Mic on"), i18n::text("Microphone in use"))),
        (false, false) => None,
    }
}

fn tab_label(tab: &Tab) -> String {
    let title = clean(tab.title.trim(), 240);
    if !title.is_empty() {
        title
    } else if tab.url == HOME || tab.url.is_empty() {
        i18n::text("New tab").into()
    } else if let Ok(url) = url::Url::parse(&tab.url) {
        url.host_str()
            .map_or_else(|| i18n::text("Untitled page").into(), str::to_owned)
    } else {
        i18n::text("Untitled page").into()
    }
}

fn update_empty(ui: Controls) {
    let visible = with_shell(|s| {
        s.fullscreen.is_none()
            && (s.sidebar_open || s.sidebar_motion.running())
            && !s.filter.is_empty()
            && s.rows.is_empty()
    });
    unsafe {
        ShowWindow(
            ui.empty,
            if visible == Some(true) {
                SW_SHOWNA
            } else {
                SW_HIDE
            },
        );
    }
}

fn refresh_rows(ensure_visible: bool) {
    reload_bookmarks();
    let bookmarks = if private() { Vec::new() } else { visible_bookmarks() };
    let Some((ui, changed, index, matches, total, status, detached_bookmark)) = change(|s| {
        let mut detached = false;
        if bookmarks.is_empty() || bookmarks != s.bookmark_rows {
            let saved: BTreeSet<_> = bookmarks.iter().map(|row| row.url.as_str()).collect();
            for tab in &mut s.tabs {
                if tab.bookmark_url.as_deref().is_some_and(|url| !saved.contains(url)) {
                    // A deleted bookmark must not hide the page the user already has open.
                    tab.bookmark_url = None;
                    detached = true;
                }
            }
            if detached {
                s.side_active = None;
            }
        }
        let mut counts = (0, 0);
        let words = s.filter.split_whitespace();
        let phrase = !words
            .clone()
            .all(|word| word.chars().any(char::is_alphanumeric));
        let rows: Vec<_> = s
            .tabs
            .iter()
            .filter_map(|tab| {
                if tab.browser.is_some() {
                    counts.0 += 1;
                } else if tab.unloaded {
                    counts.1 += 1;
                }
                if tab.agent.is_some() || tab.bookmark_url.is_some() {
                    return None;
                }
                let label = tab_label(tab);
                if !s.filter.is_empty() {
                    let title = label.to_lowercase();
                    let mut url = None;
                    let mut contains = |word: &str| {
                        title.contains(word)
                            || url
                                .get_or_insert_with(|| tab.url.to_lowercase())
                                .contains(word)
                    };
                    // A standalone symbol (e.g. ": docs") is an intentional phrase.
                    let matches = if phrase {
                        contains(&s.filter)
                    } else {
                        words.clone().all(contains)
                    };
                    if !matches {
                        return None;
                    }
                }
                Some(Row {
                    key: tab.key,
                    label: if tab.download_close == DownloadClose::Waiting {
                        format!("{} — {label}", i18n::text("After download"))
                    } else if let Some((capture, _)) = capture_labels(tab) {
                        format!("{capture} — {label}")
                    } else {
                        label
                    },
                    icon: tab.favicon.as_ref().map_or(0, |icon| icon.handle()),
                    pinned: tab.pinned,
                    muted: tab.muted,
                    unloaded: tab.unloaded,
                })
            })
            .collect();
        let index = s.filter_cursor.highlight(&rows, s.active);
        let matches = rows.len();
        let changed = if rows == s.rows {
            None
        } else {
            if let Some(p) = s.memory_preview
                && rows.iter().position(|row| row.key == p.key)
                    != s.rows.iter().position(|row| row.key == p.key)
            {
                s.memory_preview = None;
                s.hovered_row = None;
            }
            // Same tabs in the same order: only labels or icons changed.
            let same_keys = rows.len() == s.rows.len() && rows.iter().zip(&s.rows).all(|(a, b)| a.key == b.key);
            let relabel: Vec<_> = rows
                .iter()
                .enumerate()
                .filter(|(i, row)| s.rows.get(*i).is_none_or(|old| old.label != row.label))
                .map(|(i, row)| (i, row.label.clone()))
                .collect();
            s.rows = rows;
            Some((same_keys, relabel, s.rows.clone()))
        };
        let status = std::mem::replace(&mut s.page_counts, counts) != counts || changed.is_some();
        let total = s.tabs.iter().filter(|tab| tab.agent.is_none() && tab.bookmark_url.is_none()).count();
        (s.controls, changed, index, matches, total, status, detached)
    }) else {
        return;
    };
    unsafe {
        // Rebuilding the list releases the mouse capture of a click in progress, so a
        // close click would be lost while pages keep updating their titles and icons.
        let (rebuild, changed) = match changed {
            Some((true, relabel, _)) => {
                for (i, label) in relabel {
                    SendMessageW(ui.tabs, LB_DELETESTRING, i, 0);
                    SendMessageW(ui.tabs, LB_INSERTSTRING, i, wide(&label).as_ptr() as isize);
                }
                InvalidateRect(ui.tabs, null(), 0);
                (false, None)
            }
            Some((false, _, rows)) => (true, Some(rows)),
            None => (false, None),
        };
        let previous = SendMessageW(ui.tabs, LB_GETCURSEL, 0, 0);
        let top = SendMessageW(ui.tabs, LB_GETTOPINDEX, 0, 0).max(0) as usize;
        if let Some(rows) = changed {
            let empty = rows.is_empty();
            SendMessageW(ui.tabs, WM_SETREDRAW, 0, 0);
            SendMessageW(ui.tabs, LB_RESETCONTENT, 0, 0);
            for row in &rows {
                SendMessageW(ui.tabs, LB_ADDSTRING, 0, wide(&row.label).as_ptr() as isize);
            }
            if !empty && top != 0 && !ensure_visible {
                SendMessageW(ui.tabs, LB_SETTOPINDEX, top.min(rows.len() - 1), 0);
            }
            SendMessageW(ui.tabs, WM_SETREDRAW, 1, 0);
            // WM_SETREDRAW can change WS_VISIBLE on a hidden control.
            if with_shell(|s| {
                s.fullscreen.is_some() || (!s.sidebar_open && !s.sidebar_motion.running())
            }) == Some(true)
            {
                ShowWindow(ui.tabs, SW_HIDE);
            }
        }
        let selected = index.map_or(usize::MAX, |index| index);
        if rebuild || previous as usize != selected {
            SendMessageW(ui.tabs, LB_SETCURSEL, selected, 0);
        }
        if rebuild {
            InvalidateRect(ui.tabs, null(), 1);
        }
    }
    let heading = if matches != total {
        format!("{matches}/{total}")
    } else if i18n::korean() {
        format!("탭 {total}개")
    } else if total == 1 {
        "1 tab".into()
    } else {
        format!("{total} tabs")
    };
    if text(ui.label, 32) != heading {
        set_text(ui.label, &heading);
    }
    if status {
        update_sidebar_status();
    }
    update_empty(ui);
    refresh_sections(bookmarks, ensure_visible);
    if detached_bookmark {
        queue_session_save();
    }
}

fn reload_bookmarks() {
    let Some(file) = PROFILE.with(|slot| slot.borrow().as_ref().map(|p| p.join("Default").join("Bookmarks"))) else {
        return;
    };
    let stamp = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
    if private() || BOOKMARKS_STAMP.replace(stamp) == stamp {
        return;
    }
    let rows = site_setup::sidebar_bookmarks(&file)
        .into_iter()
        .filter_map(|(title, url, depth)| {
            Some(BookmarkRow { title, url: session::stored_url(&url)?, depth })
        })
        .collect();
    SIDEBAR_BOOKMARKS.with(|slot| *slot.borrow_mut() = rows);
}

/// The sidebar shows bookmarks flat, like pinned tabs: every link from every
/// folder, once each, and no folder rows.
fn visible_bookmarks() -> Vec<BookmarkRow> {
    SIDEBAR_BOOKMARKS.with(|all| {
        let mut seen = std::collections::HashSet::new();
        all.borrow()
            .iter()
            .filter(|row| seen.insert(row.url.as_str()))
            .map(|row| BookmarkRow { depth: 0, ..row.clone() })
            .collect()
    })
}

/// Fill the bookmark and AI tab sections; re-lay the sidebar when a section grows or shrinks.
fn refresh_sections(marks: Vec<BookmarkRow>, ensure_visible: bool) {
    let Some((ui, agents, marks, active_mark, agent_count)) = change(|s| {
        let agents: Vec<Row> = s
            .tabs
            .iter()
            .filter(|tab| tab.agent.is_some())
            .map(|tab| Row {
                key: tab.key,
                label: tab_label(tab),
                icon: tab.favicon.as_ref().map_or(0, |icon| icon.handle()),
                pinned: false,
                muted: tab.muted,
                unloaded: tab.unloaded,
            })
            .collect();
        let count = agents.len();
        let agents = (agents != s.agent_rows).then(|| {
            s.agent_rows = agents;
            s.agent_rows.iter().map(|row| row.label.clone()).collect::<Vec<_>>()
        });
        let marks = (marks != s.bookmark_rows).then(|| {
            s.bookmark_rows = marks;
            s.bookmark_rows.iter().map(|row| row.title.clone()).collect::<Vec<_>>()
        });
        let active_mark = s.tabs.iter().find(|tab| Some(tab.key) == s.active)
            .and_then(|tab| tab.bookmark_url.as_deref())
            .and_then(|url| s.bookmark_rows.iter().position(|row| row.url == url));
        (s.controls, agents, marks, active_mark, count)
    }) else {
        return;
    };
    let update_shortcuts = marks.is_some();
    for (list, labels) in [(ui.agents, agents), (ui.bookmarks, marks)] {
        let Some(labels) = labels else { continue };
        unsafe {
            SendMessageW(list, WM_SETREDRAW, 0, 0);
            SendMessageW(list, LB_RESETCONTENT, 0, 0);
            for label in &labels {
                SendMessageW(list, LB_ADDSTRING, 0, wide(label).as_ptr() as isize);
            }
            SendMessageW(list, WM_SETREDRAW, 1, 0);
            InvalidateRect(list, null(), 1);
        }
    }
    if update_shortcuts || ensure_visible || unsafe { GetFocus() } != ui.bookmarks {
        let selected = active_mark.map_or(usize::MAX, |index| index);
        unsafe {
            if SendMessageW(ui.bookmarks, LB_GETCURSEL, 0, 0) as usize != selected {
                SendMessageW(ui.bookmarks, LB_SETCURSEL, selected, 0);
            }
        }
    }
    if update_shortcuts {
        update_start();
    }
    let marks_heading = i18n::choose("Bookmarks", "북마크");
    if text(ui.bookmark_header, 32) != marks_heading {
        set_text(ui.bookmark_header, marks_heading);
    }
    let heading = if i18n::korean() {
        format!("AI 탭 {agent_count}")
    } else {
        format!("AI tabs ({agent_count})")
    };
    if text(ui.agent_header, 32) != heading {
        set_text(ui.agent_header, &heading);
    }
    // Section heights follow their row counts (tabs included), so re-stack the column.
    position_sidebar(ui, &current_layout(ui.window), false);
    unsafe {
        InvalidateRect(ui.bookmarks, null(), 0);
        InvalidateRect(ui.agents, null(), 0);
    }
    reveal_active_tab(ui, active_mark);
}

/// Bookmark shortcuts for the new-tab page: (title, url), top-level links first.
fn start_shortcuts() -> Vec<(String, String)> {
    if private() {
        return Vec::new();
    }
    reload_bookmarks();
    SIDEBAR_BOOKMARKS.with(|all| {
        let all = all.borrow();
        let mut links: Vec<_> = all.iter().collect();
        links.sort_by_key(|row| row.depth);
        links
            .into_iter()
            .take(8)
            .map(|row| (row.title.clone(), row.url.clone()))
            .collect()
    })
}

fn start_surface(key: usize) -> Option<Surface> {
    let bookmarks = start_shortcuts();
    let actions = change(|s| {
        let tab = s.tabs.iter_mut().find(|t| t.key == key)?;
        let mut retained = Vec::with_capacity(bookmarks.len());
        let mut actions = Vec::with_capacity(bookmarks.len());
        for (title, url) in bookmarks {
            let id = tab.start_links.iter()
                .find_map(|&(id, ref previous)| (previous == &url).then_some(id))
                .or_else(|| retained.iter().find_map(|&(id, ref previous)| (previous == &url).then_some(id)))
                .unwrap_or_else(|| {
                    let sequence = START_LINK_SEQUENCE.get().wrapping_add(1);
                    START_LINK_SEQUENCE.set(sequence);
                    START_LINK | sequence as usize
                });
            if retained.iter().any(|(previous, _)| *previous == id) {
                continue;
            }
            retained.push((id, url));
            actions.push(popup_action(id, &clean(&title, 110)));
        }
        tab.start_links = retained;
        Some(actions)
    }).flatten()?;
    let t = i18n::choose;
    let (title, subtitle) = if private() {
        (t("Private browsing", "비공개 탐색"), t(
            "Flit clears private browsing data when the last private window closes. Downloaded files stay on this device.",
            "마지막 비공개 창을 닫으면 비공개 탐색 데이터가 지워집니다. 다운로드한 파일은 기기에 남습니다.",
        ))
    } else {
        ("Flit", t("Search or enter a website in the address bar.", "주소창에서 검색하거나 웹사이트 주소를 입력하세요."))
    };
    let sections = if actions.is_empty() { Vec::new() } else {
        vec![Section {
            id: "bookmarks".into(), title: t("Bookmarks", "북마크").into(),
            items: vec![Item { id: "shortcuts".into(), title: String::new(),
                detail: String::new(), actions }],
        }]
    };
    Some(Surface { title: title.into(), subtitle: subtitle.into(), dark: chrome::dark(),
        compact: false, start: true, preserve_input_focus: false, sections })
}

fn start_view(panel: HWND) -> HWND {
    unsafe { GetDlgItem(panel, START_CONTROL as i32) }
}

fn active_start() -> Option<(usize, HWND)> {
    with_shell(|s| {
        let tab = s.tabs.iter().find(|tab| Some(tab.key) == s.active && tab.url == HOME && tab.browser.is_none())?;
        (!s.closing && !tab.retiring && !tab.close_requested).then_some((tab.key, tab.panel))
    }).flatten()
}

fn update_start() {
    if let Some((key, panel)) = active_start() {
        let view = start_view(panel);
        if !view.is_null() && let Some(surface) = start_surface(key) {
            exact_ui::update(view, &surface);
        }
    }
}

fn close_start(panel: HWND) {
    let view = start_view(panel);
    if !view.is_null() {
        unsafe { DestroyWindow(view) };
    }
}

fn mount_start(key: usize) {
    let Some((active, _)) = active_start() else { return };
    if active != key {
        return;
    }
    let Some(surface) = start_surface(key) else { return };
    if let Err(error) = mount_exact_tab(key, HOME, START_CONTROL, &surface)
        && active_start().is_some_and(|(active, _)| active == key)
        && let Some(ui) = controls()
    {
        set_text(ui.window, &format!("{} — {}", app_title(), i18n::text(error)));
    }
}

fn activate_start_link(owner: HWND, view: HWND, id: usize) {
    let Some((key, panel)) = active_start() else { return };
    if view.is_null() || start_view(panel) != view {
        return;
    }
    let url = with_shell(|s| {
        let tab = s.tabs.iter().find(|t| t.key == key)?;
        tab.start_links.iter().find_map(|(link, url)| (*link == id).then(|| url.clone()))
    }).flatten();
    if let Some(url) = url {
        match navigate_tab(key, &url) {
            Ok(browser) => {
                if let Some(host) = browser.host() {
                    host.set_focus(1);
                }
            }
            Err(error) => set_text(owner, &format!("{} — {}", app_title(), i18n::text(error))),
        }
    }
}

fn toggle_section(agents: bool) {
    change(|s| {
        let open = if agents { &mut s.agent_open } else { &mut s.bookmarks_open };
        *open = !*open;
    });
    if let Some(ui) = controls() {
        position_sidebar(ui, &current_layout(ui.window), false);
        for hwnd in [ui.agent_header, ui.bookmark_header] {
            unsafe { InvalidateRect(hwnd, null(), 0) };
        }
    }
}

/// Click or Enter on a bookmark or AI tab row, or close its open page.
fn activate_section_row(list: usize, index: usize, close: bool) {
    if list == BOOKMARK_LIST {
        let Some((row, attached)) = with_shell(|s| {
            let row = s.bookmark_rows.get(index)?;
            Some((row.clone(), s.bookmark_tab(&row.url).map(|tab| tab.key)))
        }).flatten() else { return };
        if let Some(key) = attached {
            if close {
                close_tab(Some(key));
            } else {
                select_key(key);
                focus_page();
            }
            return;
        }
        let Some((matching, return_tab)) = change(|s| {
            let key = s.tabs.iter_mut().find(|tab| {
                !tab.retiring && !tab.unloading && !tab.close_requested && tab.agent.is_none()
                    && tab.bookmark_url.is_none() && tab.url == row.url
            }).map(|tab| {
                tab.bookmark_url = Some(row.url.clone());
                tab.key
            });
            if key.is_some() {
                s.side_active = None;
            }
            (key, s.active)
        }) else { return };
        let result = matching.map_or_else(
            || append_tab(&row.url, &row.title, false, return_tab, Some(row.url.clone())).map(|(key, _)| key),
            Ok,
        );
        match result {
            Ok(key) => {
                select_key(key);
                focus_page();
            }
            Err(message) => {
                if let Some(ui) = controls() {
                    set_text(ui.window, &format!("{} — {}", app_title(), i18n::text(message)));
                }
            }
        }
        return;
    }
    let Some(key) = with_shell(|s| s.agent_rows.get(index).map(|row| row.key)).flatten() else {
        return;
    };
    if close {
        close_tab(Some(key));
    } else {
        select_key(key);
    }
}

fn refresh(force_address: bool) {
    refresh_rows(force_address);
    refresh_active_page(force_address);
}

fn refresh_active_page(force_address: bool) {
    refresh_load_controls();
    let Some((ui, url, title, capture, web)) = with_shell(|s| {
        let tab = s.tabs.iter().find(|t| Some(t.key) == s.active)?;
        Some((
            s.controls,
            tab.url.clone(),
            clean(&tab.title, 100),
            capture_labels(tab).map(|(_, caption)| caption),
            tab.browser.is_some()
                && (tab.url.starts_with("https://") || tab.url.starts_with("http://")),
        ))
    })
    .flatten() else {
        return;
    };
    if force_address
        || (with_shell(|s| !s.address_edited) == Some(true)
            && text(ui.address, 16_384) != if url == HOME { "" } else { &url })
    {
        show_address(ui.address, &url);
    }
    let caption = if title.is_empty() {
        app_title().into()
    } else {
        format!("{} — {title}", app_title())
    };
    let caption = if let Some(capture) = capture {
        format!("{capture} — {caption}")
    } else {
        caption
    };
    if text(ui.window, 256) != caption {
        set_text(ui.window, &caption);
    }
    unsafe {
        EnableWindow(ui.site_info, i32::from(web));
    }
}

fn refresh_load_controls() {
    let Some((ui, tooltip, back, forward, loading, changed)) = change(|s| {
        let (back, forward, loading) = s
            .tabs
            .iter()
            .find(|t| Some(t.key) == s.active)
            .map_or((false, false, false), |tab| {
                (tab.back, tab.forward, tab.loading)
            });
        let changed = std::mem::replace(&mut s.reload_is_stop, loading) != loading;
        (s.controls, s.tooltip, back, forward, loading, changed)
    }) else {
        return;
    };
    unsafe {
        EnableWindow(ui.back, i32::from(back));
        EnableWindow(ui.forward, i32::from(forward));
    }
    if changed {
        set_text(
            ui.reload,
            i18n::text(if loading { "Stop loading" } else { "Reload" }),
        );
        unsafe {
            InvalidateRect(ui.reload, null(), 0);
            if !tooltip.is_null() {
                let info = TTTOOLINFOW {
                    cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                    uFlags: TTF_IDISHWND,
                    hwnd: ui.window,
                    uId: ui.reload as usize,
                    lpszText: (if i18n::korean() && loading {
                        windows_sys::w!("로드 중지 (Esc)")
                    } else if i18n::korean() {
                        windows_sys::w!("새로고침 (Ctrl+R)")
                    } else if loading {
                        windows_sys::w!("Stop loading (Esc)")
                    } else {
                        windows_sys::w!("Reload (Ctrl+R)")
                    })
                    .cast_mut(),
                    ..Default::default()
                };
                SendMessageW(
                    tooltip,
                    TTM_UPDATETIPTEXTW,
                    0,
                    (&info as *const TTTOOLINFOW) as isize,
                );
            }
        }
    }
}

fn row_key(index: usize) -> Option<usize> {
    with_shell(|s| s.rows.get(index).map(|row| row.key)).flatten()
}

fn update_sidebar_status() {
    let Some((status, label)) = with_shell(|s| {
        let (live, on_demand) = s.page_counts;
        let preview = s.memory_preview.and_then(|p| {
            let tab = s.tabs.iter().find(|tab| tab.key == p.key)?;
            let Some(browser) = tab
                .browser
                .as_ref()
                .filter(|_| !tab.unloading && !tab.retiring)
            else {
                return Some(i18n::text("Not loaded · no renderer").into());
            };
            if p.browser_id != Some(browser.identifier()) {
                return None;
            }
            Some(if let Some(mib) = p.mib {
                format!("{mib} MiB · {}", i18n::choose("renderer", "렌더러"))
            } else if p.pending.is_some() {
                i18n::text("Measuring renderer…").into()
            } else {
                i18n::text("Renderer RAM unavailable").into()
            })
        });
        let label = preview.unwrap_or_else(|| {
            if live == 0 && on_demand == 0 {
                i18n::text("No pages loaded").into()
            } else if i18n::korean() {
                format!("로드 {live} · 대기 {on_demand}")
            } else {
                format!("{live} live · {on_demand} on demand")
            }
        });
        (s.controls.tab_status, label)
    }) else {
        return;
    };
    if text(status, 96) != label {
        set_text(status, &label);
    }
}

fn clear_tab_memory() {
    if let Some((window, had_preview)) = change(|s| {
        s.hovered_row = None;
        (s.controls.window, s.memory_preview.take().is_some())
    }) && had_preview
    {
        unsafe { KillTimer(window, TAB_MEMORY_TIMER) };
        update_sidebar_status();
    }
}

const MEMORY_TIMEOUT_MS: u32 = 1200;

fn show_tab_memory(key: usize) {
    let Some((window, browser, id)) = change(|s| {
        if !s.sidebar_open || s.fullscreen.is_some() || s.closing {
            return None;
        }
        let tab = s.tabs.iter().find(|tab| tab.key == key)?;
        let browser = tab
            .browser
            .clone()
            .filter(|_| !tab.unloading && !tab.retiring);
        let browser_id = browser.as_ref().map(ImplBrowser::identifier);
        if s.memory_preview
            .is_some_and(|p| p.key == key && p.browser_id == browser_id)
        {
            return None;
        }
        s.memory_request = s.memory_request.wrapping_add(1);
        let id = s.memory_request;
        s.memory_preview = Some(MemoryPreview {
            key,
            browser_id,
            pending: browser_id.map(|_| {
                (
                    id,
                    Instant::now() + Duration::from_millis(u64::from(MEMORY_TIMEOUT_MS)),
                )
            }),
            mib: None,
        });
        Some((s.controls.window, browser, id))
    })
    .flatten() else {
        return;
    };
    unsafe { KillTimer(window, TAB_MEMORY_TIMER) };
    if let Some(browser) = browser
        && (!browser
            .main_frame()
            .is_some_and(|frame| tab_memory::request(&frame, id))
            || unsafe { SetTimer(window, TAB_MEMORY_TIMER, MEMORY_TIMEOUT_MS, None) } == 0)
    {
        finish_tab_memory(key, browser.identifier(), id, 0);
    }
    update_sidebar_status();
}

fn finish_tab_memory(key: usize, browser_id: i32, request: i32, mib: i32) {
    if let Some(window) = change(|s| {
        let p = s.memory_preview.as_mut()?;
        if p.key != key
            || p.browser_id != Some(browser_id)
            || p.pending.map(|(id, _)| id) != Some(request)
        {
            return None;
        }
        p.pending = None;
        p.mib = (mib > 0).then_some(mib);
        Some(s.controls.window)
    })
    .flatten()
    {
        unsafe { KillTimer(window, TAB_MEMORY_TIMER) };
        update_sidebar_status();
    }
}

fn expire_tab_memory(window: HWND) {
    if change(|s| {
        let Some(p) = s.memory_preview.as_mut() else {
            return true;
        };
        if p.pending
            .is_some_and(|(_, deadline)| Instant::now() < deadline)
        {
            return false;
        }
        p.pending = None;
        true
    }) == Some(true)
    {
        unsafe { KillTimer(window, TAB_MEMORY_TIMER) };
        update_sidebar_status();
    }
}

fn select_key(key: usize) {
    if let Some(index) = with_shell(|s| s.tabs.iter().position(|t| t.key == key)).flatten() {
        select(index);
    }
}

fn select(index: usize) {
    select_tab(index, true);
}

fn select_tab(index: usize, load_page: bool) {
    if with_shell(|s| {
        let Some(tab) = s.tabs.get(index).filter(|tab| !tab.unloading && !tab.retiring) else {
            return false;
        };
        s.split.is_some_and(|split| split.other(tab.key).is_none())
    }) == Some(true) {
        end_split(false);
    }
    if with_shell(|s| {
        s.tabs
            .get(index)
            .is_some_and(|tab| !tab.unloading && !tab.retiring && Some(tab.key) != s.active)
    }) == Some(true)
    {
        if let Some((window, Some(active))) = with_shell(|s| (s.controls.window, s.active)) {
            permissions::cancel_tab(window as usize, active);
        }
        close_find(false);
    }
    let Some((window, key, panel, native, close_requested, previous, closing)) =
        change(|s| {
            let tab = s.tabs.get_mut(index)?;
            if tab.unloading || tab.retiring {
                return None;
            }
            let key = tab.key;
            tab.background_warm = false;
            s.deferred_restore = !load_page && tab.browser.is_none() && tab.url != HOME;
            let previous = s.active.replace(key).filter(|&previous| !s.page_visible(previous));
            if let Some(old) = previous.and_then(|key| s.tabs.iter_mut().find(|tab| tab.key == key)) {
                old.hidden_since = Instant::now();
            }
            let tab = s.tabs.get_mut(index)?;
            let (panel, native, close_requested) = (
                tab.panel,
                tab.native_window.clone(),
                tab.close_requested,
            );
            let previous = previous.and_then(|key| {
                let tab = s.tabs.iter().find(|tab| tab.key == key)?;
                Some((tab.panel, tab.native_window.clone()))
            });
            Some((
                s.controls.window,
                key,
                panel,
                native,
                close_requested,
                previous,
                s.closing,
            ))
        })
        .flatten()
    else {
        return;
    };
    if let Some((previous, native)) = previous {
        hide_page(previous, native);
    }
    // Hidden views keep their old bounds until they become visible again.
    position_active_page(window, &current_layout(window));
    set_visible(panel, true);
    if !close_requested {
        set_native_page_visible(native.as_ref(), unsafe { IsIconic(window) } == 0);
    }
    refresh(true);
    if !closing && load_page {
        if let Err(message) = load_deferred(key) {
            end_split(true);
            if let Some(ui) = controls() {
                set_text(ui.window, &format!("{} — {}", app_title(), i18n::text(&message)));
            }
        } else {
            reveal_split_partner();
        }
        queue_session_save();
    }
}

fn add_tab(url: &str) -> Result<(), &'static str> {
    open_tab(url, true, None)
}

/// Open a background tab for an AI session in the window the user last used.
/// It shares the normal profile, so the AI sees the user's signed-in sites.
pub(crate) fn open_agent_tab(id: u32, url: &str) -> Result<Browser, &'static str> {
    let usable = |s: &Shell| !s.private && !s.closing;
    let window = WINDOWS.with(|windows| {
        let windows = windows.borrow();
        let last = LAST_NORMAL.get();
        if windows.get(&last).is_some_and(usable) {
            Some(last)
        } else {
            windows.iter().find(|(_, s)| usable(s)).map(|(&w, _)| w)
        }
    });
    let window = window.ok_or("Open a Flit window first.")?;
    let _scope = WindowScope::enter(window);
    let (key, _) = append_tab(url, "", false, None, None)?;
    change(|s| {
        s.agent_open = true;
        if let Some(tab) = s.tabs.iter_mut().find(|tab| tab.key == key) {
            tab.agent = Some(id);
        }
    });
    match navigate_tab(key, url) {
        Ok(browser) => {
            refresh_rows(false);
            Ok(browser)
        }
        Err(message) => {
            let panel = change(|s| {
                let index = s.tabs.iter().position(|tab| tab.key == key)?;
                Some(s.tabs.remove(index).panel)
            })
            .flatten();
            if let Some(panel) = panel {
                unsafe { DestroyWindow(panel) };
            }
            refresh_rows(false);
            Err(message)
        }
    }
}

/// Close an AI session's tab through the normal tab-close path.
pub(crate) fn close_agent_tab(id: u32) -> bool {
    let found = WINDOWS.with(|windows| {
        windows.borrow().iter().find_map(|(&window, s)| {
            let tab = s.tabs.iter().find(|tab| tab.agent == Some(id))?;
            Some((window, tab.key))
        })
    });
    let Some((window, key)) = found else {
        return false;
    };
    let _scope = WindowScope::enter(window);
    close_tab(Some(key));
    true
}

/// An AI session ended. A Flit the agent started for itself goes away again
/// unless the user picked it up in the meantime.
pub(crate) fn agent_session_done(auto_close: bool) {
    if !(auto_close && AGENT_LAUNCH.get() && !USER_ACTIVATED.get()) {
        return;
    }
    let window = WINDOWS.with(|windows| {
        windows
            .borrow()
            .iter()
            .find(|(_, s)| !s.private && !s.closing)
            .map(|(&w, _)| w)
    });
    if let Some(window) = window {
        unsafe { PostMessageW(window as HWND, WM_COMMAND, EXIT_FLIT, 0) };
    }
}

pub(crate) fn set_agent_launch() {
    AGENT_LAUNCH.set(true);
}

fn open_tab(url: &str, foreground: bool, source: Option<usize>) -> Result<(), &'static str> {
    let return_tab = if foreground { source } else { None };
    let (key, index) = append_tab(url, "", false, return_tab, None)?;
    if foreground || with_shell(|s| s.active.is_none()) == Some(true) {
        select(index);
    } else {
        refresh_rows(false);
        warm_background_tab(key)?;
    }
    queue_session_save();
    Ok(())
}

fn warm_background_tab(key: usize) -> Result<(), &'static str> {
    const LIMIT: usize = 2;
    if DEFER_BACKGROUND.get() {
        return Ok(());
    }
    let budget_full = WINDOWS.with(|windows| {
        windows
            .borrow()
            .values()
            .flat_map(|s| &s.tabs)
            .filter(|tab| tab.background_warm)
            .take(LIMIT)
            .count()
            == LIMIT
    });
    if budget_full || physical_memory_low() {
        return Ok(());
    }
    let reserved = change(|s| {
        if s.active == Some(key) {
            return false;
        }
        if let Some(tab) = s
            .tabs
            .iter_mut()
            .find(|t| t.key == key && t.browser.is_none())
        {
            // Count the slot before CEF creates the page; selecting it releases the slot.
            tab.background_warm = true;
            return true;
        }
        false
    }) == Some(true);
    if reserved && let Err(error) = load_deferred(key) {
        change(|s| {
            if let Some(tab) = s.tabs.iter_mut().find(|t| t.key == key) {
                tab.background_warm = false;
            }
        });
        return Err(error);
    }
    Ok(())
}

fn physical_memory_low() -> bool {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // Check only when opening a background link; never evict a loaded tab here.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) } != 0;
    ok && status.ullAvailPhys < (status.ullTotalPhys / 16).clamp(512 << 20, 3 << 30)
}

fn append_tab(
    url: &str,
    title: &str,
    pinned: bool,
    return_tab: Option<usize>,
    bookmark_url: Option<String>,
) -> Result<(usize, usize), &'static str> {
    let (ui, key) = change(|s| {
        if s.closing {
            return None;
        }
        let key = s.next_key;
        s.next_key += 1;
        Some((s.controls, key))
    })
    .flatten()
    .ok_or("The Flit window is not available.")?;
    let panel = child(
        ui.window,
        0,
        "Static",
        "",
        WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        0,
    );
    if panel.is_null() {
        return Err("Could not create the tab panel.");
    }
    if unsafe { SetWindowSubclass(panel, Some(start_proc), 2, key) } == 0 {
        unsafe {
            DestroyWindow(panel);
        }
        return Err("Could not initialize the new-tab page.");
    }
    let index = change(|s| {
        s.tabs.push(Tab {
            key,
            return_tab,
            background_warm: false,
            panel,
            browser: None,
            native_window: None,
            native_close_sent: false,
            close_requested: false,
            download_close: DownloadClose::None,
            retiring: false,
            url: url.to_owned(),
            title: title.to_owned(),
            has_video_access: false,
            has_audio_access: false,
            back: false,
            forward: false,
            loading: false,
            pinned,
            muted: false,
            unloading: false,
            unloaded: url != HOME && url != FLIT_SETTINGS,
            bookmark: None,
            bookmark_url,
            agent: None,
            favicon: None,
            start_links: Vec::new(),
            hidden_since: Instant::now(),
            audible: false,
        });
        s.tabs.len() - 1
    })
    .ok_or("The Flit window closed during tab creation.")?;
    Ok((key, index))
}

fn load_deferred(key: usize) -> Result<(), &'static str> {
    let url = with_shell(|s| {
        s.tabs
            .iter()
            .find(|t| {
                t.key == key && t.browser.is_none() && !t.unloading && !t.retiring
            })
            .map(|t| t.url.clone())
    })
    .flatten();
    if let Some(url) = url {
        if url == FLIT_SETTINGS {
            mount_exact_settings(key)?;
        } else if url == HOME {
            if let Some(ui) = controls() {
                unsafe { PostMessageW(ui.window, MOUNT_START, key, 0) };
            }
        } else {
            navigate_tab(key, &url)?;
        }
    }
    Ok(())
}

fn load_waiting_window(window: HWND) {
    if exiting() || unsafe { IsIconic(window) } != 0 {
        return;
    }
    let key = change(|s| {
        if s.closing || !s.deferred_restore {
            return None;
        }
        s.deferred_restore = false;
        s.active
    })
    .flatten();
    if let Some(key) = key {
        position_active_page(window, &current_layout(window));
        match load_deferred(key) {
            Ok(()) => reveal_split_partner(),
            Err(message) => {
                end_split(true);
                set_text(window, &format!("{} — {}", app_title(), i18n::text(&message)));
            }
        }
    }
}

fn tab_window_info(window: HWND, key: usize, panel: HWND) -> WindowInfo {
    let layout = current_layout(window);
    let bounds = with_shell(|s| s.split.and_then(|split| {
        split.other(key)?;
        let [left, _, right] = split.bounds(&layout)?;
        Some(if split.left == key { left } else { right })
    })).flatten().unwrap_or_else(|| layout.page_bounds());
    position_page(panel, None, bounds);
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(panel, &mut rect);
    }
    let (width, height) = (rect.right.max(1), rect.bottom.max(1));
    // Chrome-style CEF creates a Views widget; its initial bounds are DIPs.
    // The Win32 panel and any later SetWindowPos bounds use physical pixels.
    let dpi = unsafe { GetDpiForWindow(panel) }.max(96) as i32;
    let dip = |px: i32| ((px * 96 + dpi / 2) / dpi).max(1);
    let mut win = WindowInfo {
        runtime_style: RuntimeStyle::CHROME,
        ..Default::default()
    }
    .set_as_child(
        sys::HWND(panel.cast()),
        &Rect {
            x: 0,
            y: 0,
            width: dip(width),
            height: dip(height),
        },
    );
    win.ex_style |= WS_EX_NOACTIVATE;
    win
}

fn navigate_tab(key: usize, url: &str) -> Result<Browser, &'static str> {
    let (window, panel, existing, was_settings, agent_tab) = with_shell(|s| {
        let tab = s.tabs.iter().find(|tab| tab.key == key && !tab.retiring)?;
        Some((
            s.controls.window,
            tab.panel,
            tab.browser.clone(),
            tab.url == FLIT_SETTINGS,
            tab.agent,
        ))
    })
    .flatten()
    .ok_or("The tab is no longer available.")?;
    let url = CefString::from(url);
    if let Some(browser) = existing {
        let frame = browser
            .main_frame()
            .ok_or("The page is no longer available.")?;
        frame.load_url(Some(&url));
        return Ok(browser);
    }
    let mut context = request_context()?;
    let win = tab_window_info(window, key, panel);
    let mut client = FlitClient::new(key, window as usize, None, agent_tab);
    let settings = BrowserSettings {
        // The embedded host has no Chromium toolbar to anchor a zoom bubble.
        chrome_zoom_bubble: State::DISABLED,
        ..Default::default()
    };
    let Some(browser) = browser_host_create_browser_sync(
        Some(&win),
        Some(&mut client),
        // Go straight to the requested page so Chrome can assign its renderer
        // without first opening an about:blank document in a separate process.
        Some(&url),
        Some(&settings),
        None,
        context.as_mut(),
    ) else {
        unsafe {
            InvalidateRect(panel, null(), 1);
        }
        return Err("CEF could not create the browser tab.");
    };
    attach_tab_browser(key, &browser)?;
    close_start(panel);
    if was_settings {
        close_exact_settings(panel);
    }
    Ok(browser)
}

fn attach_tab_browser(key: usize, browser: &Browser) -> Result<(), &'static str> {
    let Some((window, panel, muted, foreground)) = with_shell(|s| {
        let tab = s.tabs.iter().find(|tab| tab.key == key && !tab.retiring)?;
        Some((
            s.controls.window,
            tab.panel,
            tab.muted,
            s.page_visible(key),
        ))
    })
    .flatten() else {
        if let Some(host) = browser.host() {
            host.close_browser(1);
        }
        return Err("The tab is no longer available.");
    };
    let native_window =
        browser_view_get_for_browser(Some(&mut browser.clone())).and_then(|view| view.window());
    let usable = native_window.is_some()
        && browser.host().is_some_and(|host| {
            let child: HWND = host.window_handle().0.cast();
            if host.runtime_style() != RuntimeStyle::CHROME
                || child.is_null()
                || unsafe { GetParent(child) } != panel
            {
                return false;
            }
            if unsafe { SetWindowSubclass(child, Some(page_widget_proc), key, window as usize) }
                == 0
            {
                return false;
            }
            fit_browser(panel, &host);
            if muted {
                host.set_audio_muted(1);
            }
            true
        });
    if usable && (!foreground || unsafe { IsIconic(window) } != 0) {
        set_native_page_visible(native_window.as_ref(), false);
    }
    change(|s| {
        if let Some(tab) = s.tabs.iter_mut().find(|tab| tab.key == key) {
            tab.browser = Some(browser.clone());
            tab.native_window = native_window;
            tab.native_close_sent = false;
            tab.unloaded = false;
        }
    });
    if !usable {
        if let Some(host) = browser.host() {
            host.close_browser(0);
        }
        return Err("CEF did not provide a windowed Chrome-style browser.");
    }
    refresh_rows(false);
    Ok(())
}

fn navigate(new_tab: bool) {
    let Some(ui) = controls() else {
        return;
    };
    dismiss_address_suggestions(ui.window);
    submit_address(ui, new_tab, &text(ui.address, 16_384));
}

fn submit_address(ui: Controls, new_tab: bool, raw: &str) {
    if raw.trim().eq_ignore_ascii_case(FLIT_SETTINGS) {
        if !private() {
            open_flit_settings();
        }
        return;
    }
    let url = match navigation::address_bar(raw, SEARCH_ENGINE.get()) {
        Ok(url) => url,
        Err(message) => {
            let label = wide(i18n::text(message));
            let title = wide(app_title());
            unsafe {
                SendMessageW(
                    ui.address,
                    EM_SHOWBALLOONTIP,
                    0,
                    (&EDITBALLOONTIP {
                        cbStruct: std::mem::size_of::<EDITBALLOONTIP>() as u32,
                        pszTitle: title.as_ptr(),
                        pszText: label.as_ptr(),
                        ttiIcon: TTI_WARNING as i32,
                    } as *const EDITBALLOONTIP) as isize,
                );
                SetFocus(ui.address);
            }
            return;
        }
    };
    if settings_target(&url).is_some() {
        refresh(true);
        open_settings_window(&url);
        return;
    }
    if new_tab {
        let source = with_shell(|s| s.active).flatten();
        match open_tab(&url, true, source) {
            Ok(()) => focus_page(),
            Err(message) => set_text(ui.window, &format!("{} — {}", app_title(), i18n::text(&message))),
        }
        return;
    }
    let Some(key) = with_shell(|s| s.active).flatten() else {
        return;
    };
    permissions::cancel_tab(ui.window as usize, key);
    if url == HOME && active().is_none() {
        let settings = with_shell(|s| s.tabs.iter().any(|tab| tab.key == key && tab.url == FLIT_SETTINGS))
            == Some(true);
        if settings {
            if let Some(index) = with_shell(|s| s.tabs.iter().position(|tab| tab.key == key)).flatten() {
                let panel = change(|s| {
                    let tab = &mut s.tabs[index];
                    tab.url = HOME.to_owned();
                    tab.title.clear();
                    tab.panel
                });
                if let Some(panel) = panel {
                    close_exact_settings(panel);
                }
                unsafe { PostMessageW(ui.window, MOUNT_START, key, 0) };
                refresh(true);
                queue_session_save();
            }
        }
        return;
    }
    close_find(false);
    show_address(ui.address, &url);
    let browser = match navigate_tab(key, &url) {
        Ok(browser) => browser,
        Err(message) => {
            set_text(ui.window, &format!("{} — {}", app_title(), i18n::text(&message)));
            return;
        }
    };
    if let Some(host) = browser.host() {
        host.set_focus(1);
    }
}

fn finish_closed_browser(key: usize) {
    let native = change(|s| {
        let tab = s
            .tabs
            .iter_mut()
            .find(|tab| tab.key == key && tab.retiring)?;
        let child = unsafe { GetWindow(tab.panel, GW_CHILD) };
        if !child.is_null() {
            let native = if tab.native_close_sent {
                None
            } else {
                tab.native_close_sent = true;
                tab.native_window.clone()
            };
            Some((true, native))
        } else {
            tab.native_window = None;
            tab.retiring = false;
            Some((false, None))
        }
    })
    .flatten();
    match native {
        Some((false, _)) => closed(key),
        Some((true, Some(window))) => {
            // The Views widget can outlive OnBeforeClose. Close it through CEF
            // and wait for the subclass's WM_NCDESTROY signal.
            if window.is_closed() == 0 {
                window.close();
            }
        }
        _ => {}
    }
}

fn closed(key: usize) {
    if let Some((window, closing)) = change(|s| {
        let Some(tab) = s
            .tabs
            .iter_mut()
            .find(|tab| tab.key == key && (tab.unloading || s.closing))
        else {
            return None;
        };
        tab.unloading = false;
        tab.background_warm = false;
        // Keep tab entries until the entire window closes. If a later page
        // vetoes closing, earlier tabs can still be selected and reloaded.
        tab.unloaded = tab.url != HOME;
        tab.has_video_access = false;
        tab.has_audio_access = false;
        tab.back = false;
        tab.forward = false;
        tab.loading = false;
        Some((s.controls.window, s.closing))
    })
    .flatten()
    {
        if closing {
            continue_window_close(window);
        } else {
            refresh_rows(true);
            release_next_background_tab();
        }
        return;
    }
    let Some((ui, panel, next, was_active, was_bookmark, split_ended, closing)) = change(|s| {
        let index = s.tabs.iter().position(|tab| tab.key == key)?;
        let tab = s.tabs.remove(index);
        let was_bookmark = tab.bookmark_url.is_some();
        let former_partner = s.split.and_then(|split| split.other(key));
        if former_partner.is_some() {
            s.split = None;
        }
        if (tab.url.starts_with("https://") || tab.url.starts_with("http://"))
            && let Some(url) = session::stored_url(&tab.url)
        {
            let mut title = tab.title;
            title.truncate(title.floor_char_boundary(1024));
            let item = ClosedItem::Tab {
                owner: s.id,
                index,
                entry: Entry {
                    url,
                    title,
                    pinned: tab.pinned,
                    bookmark_url: tab.bookmark_url,
                },
                muted: tab.muted,
            };
            if s.private {
                remember_closed(&mut s.recently_closed, item);
            } else {
                RECENT_NORMAL.with(|items| remember_closed(&mut items.borrow_mut(), item));
            }
        }
        let was_active = s.active == Some(key);
        let next = (!s.tabs.is_empty()).then(|| {
            if was_active
                && let Some(source) = former_partner.or(tab.return_tab)
                && let Some(source_index) = s.tabs.iter().position(|t| {
                    t.key == source && !t.unloading && !t.retiring && !t.close_requested
                })
            {
                return source_index;
            }
            index.min(s.tabs.len() - 1)
        });
        Some((s.controls, tab.panel, next, was_active, was_bookmark, former_partner.is_some(), s.closing))
    })
    .flatten() else {
        return;
    };
    close_start(panel);
    close_exact_settings(panel);
    // A Chrome-style HWND can still finish platform teardown after its
    // lifespan callback. It owns the child; the outer panel can wait for
    // top-level destruction if the engine is still attached.
    unsafe {
        ShowWindow(panel, SW_HIDE);
        if GetWindow(panel, GW_CHILD).is_null() {
            DestroyWindow(panel);
        }
    }
    if closing {
        continue_window_close(ui.window);
    } else if let Some(next) = next {
        if split_ended && unsafe { GetCapture() } == ui.divider {
            unsafe { ReleaseCapture() };
        }
        if was_active {
            select(next);
            if split_ended {
                focus_page();
            }
        } else {
            if split_ended {
                position_active_page(ui.window, &current_layout(ui.window));
            }
            refresh_rows(false);
        }
        queue_session_save();
    } else if !was_bookmark || add_tab(HOME).is_err() {
        close_window(ui.window);
    }
}

unsafe extern "system" fn page_widget_proc(
    hwnd: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
    key: usize,
    parent: usize,
) -> LRESULT {
    let result = unsafe { DefSubclassProc(hwnd, message, wp, lp) };
    match message {
        WM_NCDESTROY => unsafe { PostMessageW(parent as HWND, PAGE_CLOSED, key, 0) },
        WM_SETFOCUS => unsafe { PostMessageW(parent as HWND, SPLIT_FOCUS, key, 0) },
        _ => 0,
    };
    result
}

wrap_task! {
    struct CloseEmbeddedPage { window: usize, key: usize, popups: Vec<usize>, deadline: Instant }
    impl Task {
        fn execute(&self) {
            let _scope = WindowScope::enter(self.window);
            let host = with_shell(|s| {
                let tab = s.tabs.iter().find(|tab| tab.key == self.key && tab.close_requested)?;
                tab.browser.clone()
            }).flatten().and_then(|browser| browser.host());
            if let Some(host) = host {
                if Instant::now() < self.deadline && self.popups.iter().any(|hwnd| unsafe {
                    GetWindow(*hwnd as HWND, GW_OWNER) == self.window as HWND
                }) {
                    let mut retry = CloseEmbeddedPage::new(self.window, self.key, self.popups.clone(), self.deadline);
                    if post_delayed_task(ThreadId::UI, Some(&mut retry), 32) != 0 {
                        return;
                    }
                }
                host.close_browser(0);
            }
        }
    }
}

fn owned_transient_windows(owner: HWND) -> Vec<usize> {
    struct Scan {
        owner: HWND,
        found: Vec<usize>,
    }
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> i32 {
        let scan = unsafe { &mut *(data as *mut Scan) };
        if unsafe {
            GetWindow(hwnd, GW_OWNER) == scan.owner
                && GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_POPUP != 0
                && IsWindowVisible(hwnd) != 0
        } {
            scan.found.push(hwnd as usize);
        }
        1
    }
    let mut scan = Scan {
        owner,
        found: Vec::new(),
    };
    unsafe {
        let thread = GetWindowThreadProcessId(owner, null_mut());
        if thread != 0 {
            EnumThreadWindows(thread, Some(collect), (&mut scan as *mut Scan) as LPARAM);
        }
    }
    scan.found
}

fn close_embedded_page(window: HWND, key: usize) {
    let Some(native) = change(|s| {
        let tab = s
            .tabs
            .iter_mut()
            .find(|tab| tab.key == key && !tab.retiring)?;
        if tab.browser.is_none() || std::mem::replace(&mut tab.close_requested, true) {
            return None;
        }
        Some(tab.native_window.clone())
    })
    .flatten() else {
        return;
    };
    let mut popups = Vec::new();
    if let Some(native) = native.filter(|native| native.is_closed() == 0) {
        // The outer Flit HWND can own a Chromium Views bubble. Hide the
        // BrowserView while it is live and let Chromium retire its own UI.
        popups = owned_transient_windows(window);
        native.hide();
    }
    // Observe only windows present at hide time. A popup that stays open
    // cannot block the user's close indefinitely.
    CloseEmbeddedPage::new(
        window as usize,
        key,
        popups,
        Instant::now() + Duration::from_millis(1500),
    )
    .execute();
}

fn tab_download_close(window: HWND, key: usize, count: usize) {
    change(|s| {
        if let Some(tab) = s.tabs.iter_mut().find(|t| t.key == key) {
            tab.download_close = DownloadClose::Asking;
        }
    });
    let message = wide(&if i18n::korean() {
        format!(
            "진행 중인 다운로드: {count}개\n지금 탭을 닫으면 다운로드가 중단될 수 있습니다.\n\n다운로드가 끝난 뒤에 이 탭을 닫을까요?"
        )
    } else {
        format!(
            "Downloads in progress: {count}\nClosing this tab now may interrupt them.\n\nClose this tab after the downloads finish?"
        )
    });
    let confirmed = unsafe {
        MessageBoxW(
            window,
            message.as_ptr(),
            wide(i18n::text("Download in progress")).as_ptr(),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        )
    } == IDYES;
    let confirmed = confirmed && !exiting();
    let scheduled = change(|s| {
        let Some(tab) = s
            .tabs
            .iter_mut()
            .find(|t| t.key == key && t.download_close == DownloadClose::Asking)
        else {
            return false;
        };
        let schedule = confirmed && !s.closing && !tab.retiring && !tab.close_requested;
        tab.download_close = if schedule {
            DownloadClose::Waiting
        } else {
            DownloadClose::None
        };
        schedule
    }) == Some(true);
    if scheduled {
        complete_deferred_close(window, key);
    }
}

pub(crate) fn downloads_changed(window: usize) {
    unsafe { PostMessageW(window as HWND, DOWNLOADS_CHANGED, 0, 0) };
}

pub(crate) fn download_progress_changed(window: usize) {
    let Some(owner) = crate::popover::native_owner("downloads") else {
        return;
    };
    if DOWNLOAD_TIMER_OWNER.get() == owner
        || (owner as usize != window
            && (download_window_private(window) || download_window_private(owner as usize)))
    {
        return;
    }
    let old = DOWNLOAD_TIMER_OWNER.replace(null_mut());
    unsafe {
        if !old.is_null() {
            KillTimer(old, DOWNLOAD_MENU_TIMER);
        }
        if SetTimer(owner, DOWNLOAD_MENU_TIMER, 180, None) != 0 {
            DOWNLOAD_TIMER_OWNER.set(owner);
        }
    }
}

fn refresh_downloads_button(window: HWND) {
    let count = download_guard::count(window as usize, None);
    let button = change(|s| {
        (std::mem::replace(&mut s.active_downloads, count) != count).then_some(s.controls.downloads)
    })
    .flatten();
    if let Some(button) = button {
        if count == 0 {
            set_text(button, i18n::text("Downloads"));
        } else if i18n::korean() {
            set_text(button, &format!("다운로드 ({count}개 진행 중)"));
        } else {
            set_text(button, &format!("Downloads ({count} active)"));
        }
        unsafe { InvalidateRect(button, null(), 0) };
    }
}

pub(crate) fn download_ended(window: usize, key: usize, complete: bool) {
    let _scope = WindowScope::enter(window);
    let waiting = change(|s| {
        let Some(tab) = s.tabs.iter_mut().find(|t| t.key == key) else {
            return false;
        };
        let waiting = tab.download_close == DownloadClose::Waiting;
        if !complete {
            // Leave unsuccessful or canceled downloads' tabs available for retry.
            tab.download_close = DownloadClose::None;
        }
        waiting
    }) == Some(true);
    if waiting {
        // Closing the source must happen after the CEF download callback returns.
        unsafe { PostMessageW(window as HWND, TAB_DOWNLOAD_ENDED, key, 0) };
    }
}

fn complete_deferred_close(window: HWND, key: usize) {
    let done = download_guard::count(window as usize, Some(key)) == 0;
    let close = done
        && change(|s| {
            let Some(tab) = s
                .tabs
                .iter_mut()
                .find(|t| t.key == key && t.download_close == DownloadClose::Waiting)
            else {
                return false;
            };
            tab.download_close = DownloadClose::None;
            !s.closing
        }) == Some(true);
    refresh_rows(false);
    if close && !exiting() {
        close_tab(Some(key));
    }
}

fn close_tab(key: Option<usize>) {
    let Some((window, key, loaded)) = with_shell(|s| {
        if s.closing {
            return None;
        }
        let key = key.or(s.active)?;
        let tab = s.tabs.iter().find(|t| {
            t.key == key
                && !t.unloading
                && !t.retiring
                && !t.close_requested
                && t.download_close == DownloadClose::None
        })?;
        Some((s.controls.window, key, tab.browser.is_some()))
    })
    .flatten() else {
        return;
    };
    let count = download_guard::count(window as usize, Some(key));
    if count != 0 {
        tab_download_close(window, key, count);
        return;
    }
    permissions::cancel_tab(window as usize, key);
    if loaded {
        close_embedded_page(window, key);
    } else {
        closed(key);
    }
}

fn close_window(window: HWND) {
    if exiting() || with_shell(|s| s.closing) != Some(false) {
        return;
    }
    // Chromium's shutdown command applies to the entire process, not one
    // Flit window among several. It checks every beforeunload first.
    if WINDOWS.with(|windows| windows.borrow().len() == 1) && try_native_exit() {
        return;
    }
    if confirm_window_download_close(window) && !exiting() {
        begin_window_close(window);
    }
}

fn begin_window_close(window: HWND) {
    permissions::cancel_window(window as usize);
    close_find(false);
    if RECOVERY_WINDOW.get() == window as usize {
        unsafe {
            KillTimer(window, RECOVERY_TIMER);
        }
        RECOVERY_WINDOW.set(0);
    }
    if change(|s| {
        if s.closing {
            return false;
        }
        s.closing = true;
        s.pending_settings = None;
        s.memory_queue.clear();
        s.closing_session = (!s.private).then(|| window_snapshot(s));
        true
    }) == Some(true)
    {
        continue_window_close(window);
    }
}

fn exit_flit(window: HWND) {
    if exiting()
        || OPENING_SESSION.get()
        || WINDOWS.with(|windows| windows.borrow().values().any(|s| s.closing))
    {
        return;
    }
    if try_native_exit() {
        return;
    }
    let count = WINDOWS.with(|windows| {
        windows
            .borrow()
            .keys()
            .map(|&window| download_guard::count(window, None))
            .sum::<usize>()
    });
    if count != 0 {
        let message = wide(&if i18n::korean() {
            format!(
                "진행 중인 다운로드: {count}개\nFlit을 종료하면 다운로드가 중단될 수 있습니다. 그래도 종료할까요?"
            )
        } else {
            format!("Downloads in progress: {count}\nExiting Flit may interrupt them. Exit anyway?")
        });
        if unsafe {
            MessageBoxW(
                window,
                message.as_ptr(),
                wide(i18n::text("Download in progress")).as_ptr(),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            )
        } != IDYES
        {
            return;
        }
    }
    if exiting() || WINDOWS.with(|windows| windows.borrow().values().any(|s| s.closing)) {
        return;
    }
    let mut order = WINDOWS.with(|windows| {
        let windows = windows.borrow();
        let mut order = windows
            .iter()
            .map(|(&hwnd, s)| (hwnd, s.id))
            .collect::<Vec<_>>();
        order.sort_unstable_by_key(|&(hwnd, id)| (hwnd != window as usize, id));
        order
            .into_iter()
            .map(|(hwnd, _)| hwnd)
            .collect::<VecDeque<_>>()
    });
    let Some(current) = order.pop_front() else {
        return;
    };
    save_before_exit();
    EXIT_SESSION.with(|slot| {
        slot.replace(Some(ExitSession {
            current,
            remaining: order,
        }))
    });
    let _scope = WindowScope::enter(current);
    begin_window_close(current as HWND);
}

fn save_before_exit() {
    // Mid-exit writes would lose the windows that already closed.
    if let Some(state) = profile_snapshot() {
        store_session(state);
    }
    let timer = RECOVERY_WINDOW.replace(0);
    if timer != 0 {
        unsafe { KillTimer(timer as HWND, RECOVERY_TIMER) };
    }
}

fn try_native_exit() -> bool {
    let command = chrome_command(c"IDC_EXIT");
    if command <= 0 {
        return false;
    }
    let browser = WINDOWS.with(|windows| {
        let windows = windows.borrow();
        if windows.values().any(|s| {
            s.closing || !s.pending_popups.is_empty()
                || s.tabs.iter().any(|t| t.close_requested || t.unloading || t.retiring)
                // Let the per-tab close path dismiss Chromium's owned bubbles
                // before the Flit owner window is destroyed.
                || !owned_transient_windows(s.controls.window).is_empty()
                // Keep the existing owned download confirmation.
                || download_guard::count(s.controls.window as usize, None) != 0
        }) {
            return None;
        }
        windows
            .values()
            .flat_map(|s| &s.tabs)
            .filter_map(|tab| tab.browser.as_ref())
            .next()
            .cloned()
    });
    let Some(host) = browser
        .and_then(|browser| browser.host())
        .filter(|host| host.can_execute_chrome_command(command) != 0)
    else {
        return false;
    };
    save_before_exit();
    NATIVE_EXIT.set(true);
    WINDOWS.with(|windows| {
        for s in windows.borrow_mut().values_mut() {
            s.closing = true;
        }
    });
    host.execute_chrome_command(command, WindowOpenDisposition::CURRENT_TAB);
    true
}

fn cancel_native_exit() -> bool {
    if !NATIVE_EXIT.replace(false) {
        return false;
    }
    WINDOWS.with(|windows| {
        for s in windows.borrow_mut().values_mut() {
            s.closing = false;
        }
    });
    true
}

fn finish_native_exit() {
    let ready = WINDOWS.with(|windows| {
        let windows = windows.borrow();
        if windows.values().any(|s| {
            s.settings_window.is_some()
                || !s.popups.is_empty()
                || !s.pending_popups.is_empty()
                || s.tabs
                    .iter()
                    .any(|t| t.browser.is_some() || t.retiring || t.unloading)
        }) {
            return Vec::new();
        }
        windows.keys().copied().collect::<Vec<_>>()
    });
    for window in ready {
        unsafe { DestroyWindow(window as HWND) };
    }
}

fn advance_exit(closed: HWND) {
    let next = EXIT_SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let exit = slot
            .as_mut()
            .filter(|exit| exit.current == closed as usize)?;
        let next = WINDOWS.with(|windows| {
            let windows = windows.borrow();
            exit.remaining
                .iter()
                .position(|window| windows.contains_key(window))
        });
        if let Some(next) = next {
            exit.remaining.drain(..next);
            exit.current = exit.remaining.pop_front()?;
            Some(exit.current as HWND)
        } else {
            *slot = None;
            None
        }
    });
    if let Some(next) = next {
        // Run outside the destroyed window's CEF/Win32 teardown.
        if unsafe { PostMessageW(next, EXIT_NEXT_WINDOW, 0, 0) } == 0 {
            EXIT_SESSION.with(|slot| slot.borrow_mut().take());
            save_session();
        }
    }
}

fn confirm_window_download_close(window: HWND) -> bool {
    let count = download_guard::count(window as usize, None);
    if count == 0 {
        return true;
    }
    let message = wide(&if i18n::korean() {
        format!(
            "진행 중인 다운로드: {count}개\n창을 닫으면 다운로드가 중단될 수 있습니다. 그래도 닫을까요?"
        )
    } else {
        format!(
            "Downloads in progress: {count}\nClosing this window may interrupt them. Close anyway?"
        )
    });
    unsafe {
        MessageBoxW(
            window,
            message.as_ptr(),
            wide(i18n::text("Download in progress")).as_ptr(),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        ) == IDYES
    }
}

fn cancel_active_permission(window: HWND) {
    if let Some(key) = with_shell(|s| s.active).flatten() {
        permissions::cancel_tab(window as usize, key);
    }
}

fn continue_window_close(window: HWND) {
    if NATIVE_EXIT.get() {
        finish_native_exit();
        return;
    }
    let Some((aux, tab, waiting)) = with_shell(|s| {
        let waiting =
            !s.pending_popups.is_empty() || s.tabs.iter().any(|tab| tab.unloading || tab.retiring);
        if let Some(settings) = &s.settings_window {
            return (Some(settings.clone()), None, waiting);
        }
        if let Some(popup) = s.popups.last() {
            return (Some(popup.clone()), None, waiting);
        }
        let key = s
            .tabs
            .iter()
            .find(|t| Some(t.key) != s.active && t.browser.is_some() && !t.unloading)
            .or_else(|| s.tabs.iter().find(|t| t.browser.is_some() && !t.unloading))
            .map(|tab| tab.key);
        (None, key, waiting)
    }) else {
        return;
    };
    if let Some(aux) = aux {
        if let Some(host) = aux.host() {
            host.close_browser(0);
        }
    } else if let Some(key) = tab {
        close_embedded_page(window, key);
    } else if !waiting {
        if let Some(state) = change(|s| s.closing_session.take()).flatten() {
            if !exiting() {
                if last_normal_window() {
                    store_session(session_from(state.clone(), vec![]));
                } else if let Some(remaining) = profile_snapshot_without(window as usize) {
                    store_session(remaining);
                }
            }
            if state.entries.iter().any(|entry| entry.url != HOME) && normal_window().is_some() {
                // If Exit is canceled in a later window, Ctrl+Shift+T can reopen
                // the normal windows that already closed.
                let state = session::validated_window(&state);
                RECENT_NORMAL.with(|items| {
                    remember_closed(&mut items.borrow_mut(), ClosedItem::Window(state))
                });
            }
        }
        unsafe {
            DestroyWindow(window);
        }
    }
}

fn native_placement(window: HWND) -> Option<WINDOWPLACEMENT> {
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    (unsafe { GetWindowPlacement(window, &mut placement) } != 0).then_some(placement)
}

fn session_placement(window: HWND, native: WINDOWPLACEMENT) -> Option<WindowPlacement> {
    let mut rect = native.rcNormalPosition;
    if native.showCmd == SW_SHOWNORMAL as u32 {
        // An arranged (snapped) window's live bounds can differ from rcNormalPosition.
        if unsafe { GetWindowRect(window, &mut rect) } == 0 {
            return None;
        }
    } else {
        let monitor = monitor_info(unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) })?;
        // GetWindowPlacement reports workspace coordinates; creation uses screen coordinates.
        unsafe {
            OffsetRect(
                &mut rect,
                monitor.rcWork.left - monitor.rcMonitor.left,
                monitor.rcWork.top - monitor.rcMonitor.top,
            );
        }
    }
    Some(WindowPlacement {
        x: rect.left,
        y: rect.top,
        width: rect.right.checked_sub(rect.left)?,
        height: rect.bottom.checked_sub(rect.top)?,
        maximized: native.showCmd == SW_SHOWMAXIMIZED as u32,
    })
}

fn window_snapshot(s: &Shell) -> WindowState {
    let mut entries = Vec::new();
    let mut selected = 0;
    let mut left = None;
    let mut right = None;
    for tab in s.tabs.iter().filter(|tab| tab.agent.is_none()) {
        let Some(url) = session::stored_url(&tab.url) else {
            continue;
        };
        if settings_target(&url).is_some() {
            continue;
        }
        if Some(tab.key) == s.active {
            selected = entries.len();
        }
        if let Some(split) = s.split {
            if tab.key == split.left { left = Some(entries.len()); }
            if tab.key == split.right { right = Some(entries.len()); }
        }
        let title = &tab.title[..tab.title.floor_char_boundary(1024)];
        entries.push(Entry {
            url,
            title: title.to_owned(),
            pinned: tab.pinned,
            bookmark_url: tab.bookmark_url.clone(),
        });
    }
    let window = s.controls.window;
    let placement = match s.fullscreen {
        Some((_, placement)) => placement,
        None => native_placement(window).and_then(|native| session_placement(window, native)),
    };
    WindowState {
        entries,
        selected,
        placement,
        split: s.split.and_then(|split| Some(SplitPlacement {
            left: left?,
            right: right?,
            ratio: split.ratio,
        })),
    }
}

fn last_normal_window() -> bool {
    !private()
        && WINDOWS.with(|windows| windows.borrow().values().filter(|s| !s.private).count() == 1)
}

fn session_from(mut primary: WindowState, mut additional_windows: Vec<WindowState>) -> Session {
    let restore = RESTORE_TABS.get();
    if !restore {
        primary.entries.clear();
        primary.selected = 0;
        primary.placement = None;
        additional_windows.clear();
    }
    Session {
        primary,
        additional_windows,
        restore,
        sidebar_open: SIDEBAR_DEFAULT.get(),
        defer_background: DEFER_BACKGROUND.get(),
        agent_auto_close: crate::agent_host::AUTO_CLOSE.load(std::sync::atomic::Ordering::Relaxed),
        memory_saver_minutes: MEMORY_SAVER_MINUTES.get(),
        search_engine: SEARCH_ENGINE.get(),
        acrylic: chrome::acrylic(),
        theme: chrome::theme(),
    }
}

fn profile_snapshot() -> Option<Session> {
    profile_snapshot_without(0)
}

fn profile_snapshot_without(excluded: usize) -> Option<Session> {
    WINDOWS.with(|windows| {
        let windows = windows.borrow();
        let mut normal = windows
            .iter()
            .filter(|&(&hwnd, s)| !s.private && hwnd != excluded)
            .collect::<Vec<_>>();
        normal.sort_unstable_by_key(|&(&hwnd, s)| {
            (hwnd != LAST_NORMAL.get(), std::cmp::Reverse(s.id))
        });
        let mut states = normal.into_iter().take(session::MAX_WINDOWS).map(|(_, s)| {
            s.closing_session
                .clone()
                .unwrap_or_else(|| window_snapshot(s))
        });
        Some(session_from(states.next()?, states.collect()))
    })
}

fn store_session(state: Session) {
    PROFILE.with(|slot| {
        if let Some(profile) = slot.borrow().as_deref() {
            if let Err(error) = session::save(profile, &state) {
                eprintln!("Flit Browser: Could not save tabs: {error}");
            }
        }
    });
}

fn save_session() {
    if !exiting() && !OPENING_SESSION.get() {
        if let Some(state) = profile_snapshot() {
            store_session(state);
        }
    }
}

fn restore_recovery_copy(window: HWND) {
    if !RESTORE_TABS.get() {
        return;
    }
    let saved = PROFILE.with(|slot| slot.borrow().as_deref().and_then(session::read_backup));
    let Some(saved) = saved else {
        set_text(
            window,
            &format!("{} — {}", app_title(), i18n::text("No previous session backup is available.")),
        );
        return;
    };
    let mut open = WINDOWS.with(|windows| {
        windows
            .borrow()
            .values()
            .filter(|s| !s.private && !s.closing)
            .map(window_snapshot)
            .collect::<Vec<_>>()
    });
    let mut opened = 0;
    let mut failed = false;
    for previous in std::iter::once(saved.primary).chain(saved.additional_windows) {
        // Match each current window at most once, keeping duplicates the user
        // deliberately had open in separate windows.
        let same = open.iter().position(|other| {
            other
                .entries
                .iter()
                .map(|e| (&e.url, e.pinned))
                .eq(previous.entries.iter().map(|e| (&e.url, e.pinned)))
        });
        if let Some(index) = same {
            open.swap_remove(index);
        } else if create_window(false, None, Some(previous), SW_SHOWNOACTIVATE).is_ok() {
            opened += 1;
        } else {
            failed = true;
        }
    }
    if failed {
        set_text(
            window,
            &format!("{} — {}", app_title(), i18n::text("Some windows in the previous session could not be opened.")),
        );
    } else if opened == 0 {
        set_text(
            window,
            &format!("{} — {}", app_title(), i18n::text("Windows from the previous session are already open.")),
        );
    }
}

fn remember_closed(items: &mut Vec<ClosedRecord>, item: ClosedItem) {
    if items.len() >= 20 {
        items.remove(0);
    }
    let id = NEXT_CLOSED_ID.get();
    NEXT_CLOSED_ID.set(id.wrapping_add(1).max(1));
    items.push(ClosedRecord { id, item });
}

fn take_closed(items: &mut Vec<ClosedRecord>, choice: Option<usize>) -> Option<ClosedItem> {
    let record = match choice {
        Some(id) => items.remove(items.iter().position(|record| record.id == id)?),
        None => items.pop()?,
    };
    Some(record.item)
}

fn reopen_closed(choice: Option<usize>) {
    if with_shell(|s| !s.closing) != Some(true) {
        return;
    }
    let private = private();
    let item = if private {
        change(|s| take_closed(&mut s.recently_closed, choice)).flatten()
    } else {
        RECENT_NORMAL.with(|items| take_closed(&mut items.borrow_mut(), choice))
    };
    let Some(item) = item else { return };
    match item {
        ClosedItem::Window(state) => {
            if open_window(false, None, Some(state.clone())).is_err() {
                RECENT_NORMAL.with(|items| {
                    remember_closed(&mut items.borrow_mut(), ClosedItem::Window(state))
                });
            }
        }
        ClosedItem::Tab {
            owner,
            index,
            entry,
            muted,
        } => {
            let target = if private {
                CURRENT_WINDOW.get()
            } else {
                WINDOWS.with(|windows| {
                    windows
                        .borrow()
                        .iter()
                        .find(|(_, s)| s.id == owner && !s.private && !s.closing)
                        .map_or(CURRENT_WINDOW.get(), |(&window, _)| window)
                })
            };
            let _scope = WindowScope::enter(target);
            if let Some(key) = entry.bookmark_url.as_deref().and_then(|url| {
                with_shell(|s| s.bookmark_tab(url).map(|tab| tab.key)).flatten()
            }) {
                select_key(key);
                focus_page();
                return;
            }
            let Ok((key, appended)) = append_tab(
                &entry.url, &entry.title, entry.pinned, None, entry.bookmark_url.clone(),
            ) else {
                let item = ClosedItem::Tab {
                    owner,
                    index,
                    entry,
                    muted,
                };
                if private {
                    change(|s| remember_closed(&mut s.recently_closed, item));
                } else {
                    RECENT_NORMAL.with(|items| remember_closed(&mut items.borrow_mut(), item));
                }
                return;
            };
            let placed = change(|s| {
                let mut tab = s.tabs.remove(appended);
                tab.muted = muted;
                let pinned = s.tabs.iter().take_while(|tab| tab.pinned).count();
                let index = if entry.pinned {
                    index.min(pinned)
                } else {
                    index.clamp(pinned, s.tabs.len())
                };
                s.tabs.insert(index, tab);
                (s.controls.window, key)
            });
            if let Some((window, key)) = placed {
                select_key(key);
                unsafe {
                    SetForegroundWindow(window);
                }
                focus_page();
            }
        }
    }
}

unsafe extern "system" fn start_proc(
    hwnd: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    key: usize,
) -> LRESULT {
    let _scope = WindowScope::enter(unsafe { GetParent(hwnd) } as usize);
    match message {
        WM_SIZE if !exact_settings_control(hwnd).is_null() || !start_view(hwnd).is_null() => {
            let mut bounds = RECT::default();
            unsafe { GetClientRect(hwnd, &mut bounds) };
            let moving = with_shell(|s| s.sidebar_motion.running()) == Some(true);
            for view in [exact_settings_control(hwnd), start_view(hwnd)] {
                if !view.is_null() {
                    set_bounds(view, 0, 0, bounds.right, bounds.bottom);
                    // Present the resized Exact view within the same animation step.
                    if moving {
                        unsafe { UpdateWindow(view) };
                    }
                }
            }
            0
        }
        WM_PAINT
            if with_shell(|s| s.tabs.iter().any(|t| t.key == key && t.browser.is_none() && t.url != FLIT_SETTINGS))
                == Some(true) =>
        {
            let (font, waiting) =
                with_shell(|s| (s.font, s.deferred_restore && s.active == Some(key)))
                    .unwrap_or((null_mut(), false));
            chrome::paint_page_placeholder(hwnd, font, waiting);
            0
        }
        WM_LBUTTONUP => {
            if let Some(ui) = controls() {
                focus_control(ui.address, true);
            }
            0
        }
        _ => unsafe { DefSubclassProc(hwnd, message, wp, lp) },
    }
}

fn toggle_sidebar() {
    if let Some((open, private)) = with_shell(|s| (s.sidebar_open, s.private)) {
        set_sidebar(!open, true);
        if !private {
            SIDEBAR_DEFAULT.set(!open);
            queue_profile_save();
        }
    }
}

fn toggle_fullscreen(window: HWND) {
    if let Some((placement, _)) = change(|s| s.fullscreen.take()).flatten() {
        unsafe {
            SetWindowLongPtrW(
                window,
                GWL_STYLE,
                GetWindowLongPtrW(window, GWL_STYLE) | WS_OVERLAPPEDWINDOW as isize,
            );
            SetWindowPlacement(window, &placement);
            SetWindowPos(
                window,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    } else {
        let Some(saved) = native_placement(window) else {
            return;
        };
        let Some(monitor) =
            monitor_info(unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) })
        else {
            return;
        };
        let placement = if private() {
            None
        } else {
            session_placement(window, saved)
        };
        close_find(false);
        unsafe {
            KillTimer(window, SIDEBAR_TIMER);
        }
        change(|s| {
            s.fullscreen = Some((saved, placement));
            s.sidebar_motion.to(s.sidebar_open, false);
        });
        let r = monitor.rcMonitor;
        unsafe {
            SetWindowLongPtrW(
                window,
                GWL_STYLE,
                GetWindowLongPtrW(window, GWL_STYLE) & !(WS_OVERLAPPEDWINDOW as isize),
            );
            SetWindowPos(
                window,
                null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_FRAMECHANGED | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    layout();
    if let Some(key) = change(|s| {
        if s.fullscreen.is_none() {
            s.pending_tab_reveal.take()
        } else {
            None
        }
    })
    .flatten()
    {
        reveal_tab_key(key);
    }
    focus_page();
}

fn reveal_toolbar(window: HWND) {
    if with_shell(|s| s.fullscreen.is_some()) == Some(true) {
        toggle_fullscreen(window);
    }
}

fn set_sidebar(open: bool, animate: bool) {
    let allowed = animate && crate::panel_motion::enabled();
    let Some((ui, moving)) = change(|s| {
        s.sidebar_open = open;
        s.sidebar_motion.to(open, allowed);
        (s.controls, s.sidebar_motion.running())
    }) else {
        return;
    };
    if !open {
        let focus = unsafe { GetFocus() };
        if focus == ui.filter || focus == ui.tabs || focus == ui.add {
            focus_control(ui.sidebar, false);
        }
    }
    if moving && unsafe { SetTimer(ui.window, SIDEBAR_TIMER, 16, None) } == 0 {
        change(|s| s.sidebar_motion.to(open, false));
    } else if !moving {
        unsafe {
            KillTimer(ui.window, SIDEBAR_TIMER);
        }
    }
    layout();
}

fn tick_sidebar(window: HWND) {
    let Some((ui, moving)) = with_shell(|s| (s.controls, s.sidebar_motion.running())) else {
        return;
    };
    if !moving {
        unsafe {
            KillTimer(window, SIDEBAR_TIMER);
        }
        layout();
        return;
    }
    let l = current_layout(window);
    position_sidebar(ui, &l, true);
    position_active_page(window, &l);
}

fn cycle_tab(previous: bool) {
    if let Some(index) = with_shell(|s| {
        let current = s.tabs.iter().position(|t| Some(t.key) == s.active)?;
        Some(if previous {
            (current + s.tabs.len() - 1) % s.tabs.len()
        } else {
            (current + 1) % s.tabs.len()
        })
    })
    .flatten()
    {
        select(index);
        focus_page();
    }
}

fn host() -> Option<BrowserHost> {
    active().and_then(|b| b.host())
}

fn chrome_command(name: &std::ffi::CStr) -> i32 {
    unsafe { sys::cef_id_for_command_id_name(name.as_ptr()) }
}

fn zoom_page(host: &BrowserHost, id: usize) {
    let command = match id {
        ZOOM_IN => ZoomCommand::IN,
        ZOOM_OUT => ZoomCommand::OUT,
        _ => ZoomCommand::RESET,
    };
    if host.can_zoom(command) != 0 {
        host.zoom(command);
    }
}

fn settings_target(address: &str) -> Option<String> {
    let url = url::Url::parse(address).ok()?;
    // Only clearing browsing data needs Chromium's own browser window; every other
    // settings page opens as a Flit tab with the Flit skin.
    (url.scheme() == "chrome"
        && url.host_str() == Some("settings")
        && url.path().starts_with("/clearBrowserData")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none())
    .then(|| url.into())
}

fn open_settings_window(url: &str) {
    let Some((window, existing)) = with_shell(|s| {
        (!s.private && !s.closing).then(|| (s.controls.window, s.settings_window.clone()))
    })
    .flatten() else {
        return;
    };
    if let Some(browser) = existing.filter(|b| b.is_valid() != 0) {
        if let Some(frame) = browser.main_frame() {
            frame.load_url(Some(&CefString::from(url)));
        }
        if let Some(host) = browser.host() {
            let hwnd: HWND = host.window_handle().0.cast();
            if !hwnd.is_null() {
                unsafe {
                    if IsIconic(hwnd) != 0 {
                        ShowWindow(hwnd, SW_RESTORE);
                    }
                    SetForegroundWindow(hwnd);
                }
            }
            host.set_focus(1);
        }
        return;
    }
    let info = WindowInfo {
        runtime_style: RuntimeStyle::CHROME,
        ..Default::default()
    };
    let mut client = FlitSettingsClient::new(window as usize);
    let browser = browser_host_create_browser_sync(
        Some(&info),
        Some(&mut client),
        Some(&CefString::from(url)),
        Some(&BrowserSettings::default()),
        None,
        None,
    );
    if let Some(browser) = browser {
        change(|s| s.settings_window = Some(browser));
    } else {
        unsafe {
            MessageBoxW(
                window,
                wide(i18n::text("Could not open settings.")).as_ptr(),
                windows_sys::w!("Flit Browser"),
                MB_ICONERROR | MB_OK,
            );
        }
    }
}

fn open_builtin(url: &str) {
    if let Some(url) = settings_target(url) {
        // The history deletion completion handler needs Chromium's complete
        // BrowserView. Flit's lightweight embedded tab omits that UI.
        open_settings_window(&url);
        return;
    }
    let existing = with_shell(|s| {
        s.tabs
            .iter()
            .find(|tab| !tab.retiring && !tab.unloading && tab.url == url)
            .map(|tab| tab.key)
    })
    .flatten();
    if let Some(key) = existing {
        select_key(key);
    } else if let Err(message) = add_tab(url) {
        if let Some(ui) = controls() {
            set_text(ui.window, &format!("{} — {}", app_title(), i18n::text(&message)));
        }
    }
}

fn site_info() -> Option<(String, &'static str)> {
    let browser = active()?;
    let frame = browser.main_frame()?;
    let url = url::Url::parse(&CefString::from(&frame.url()).to_string()).ok()?;
    let origin = url.origin().ascii_serialization();
    let connection = match url.scheme() {
        "http" => "This site uses HTTP. The connection is not encrypted.",
        "https" => {
            let ssl = browser
                .host()?
                .visible_navigation_entry()
                .filter(|entry| entry.is_valid() != 0)
                .filter(|entry| {
                    url::Url::parse(&CefString::from(&entry.url()).to_string())
                        .is_ok_and(|entry_url| entry_url == url)
                })
                .and_then(|entry| entry.sslstatus());
            if ssl.is_some_and(|status| {
                status.is_secure_connection() != 0
                    && is_cert_status_error(status.cert_status()) == 0
                    && status.content_status() == SslContentStatus::NORMAL_CONTENT
            }) {
                "The connection uses verified HTTPS."
            } else {
                "The browser cannot confirm a fully secure connection to this site."
            }
        }
        _ => return None,
    };
    Some((origin, i18n::text(connection)))
}

fn show_site_info(window: HWND) {
    let Some((origin, connection)) = site_info() else {
        return;
    };
    let is_private = private();
    let question = if is_private {
        ""
    } else {
        i18n::choose("\n\nOpen this site's settings?", "\n\n이 사이트의 설정을 열까요?")
    };
    let content = wide(&format!("{origin}\n\n{connection}{question}"));
    let selected = unsafe {
        MessageBoxW(
            window,
            content.as_ptr(),
            wide(i18n::text("Site information")).as_ptr(),
            MB_ICONINFORMATION
                | if is_private {
                    MB_OK
                } else {
                    MB_YESNO | MB_DEFBUTTON2
                },
        )
    };
    if !is_private && selected == IDYES && site_info().is_some_and(|(current, _)| current == origin)
    {
        let mut url =
            url::Url::parse("chrome://settings/content/siteDetails").expect("static settings URL");
        url.query_pairs_mut().append_pair("site", &origin);
        open_builtin(url.as_str());
    }
}

fn bookmark_page() {
    if private() {
        return;
    }
    let request = with_shell(|s| {
        let tab = s.tabs.iter().find(|tab| Some(tab.key) == s.active)?;
        BookmarkRequest::save(&session::stored_url(&tab.url)?, &clean(&tab.title, 240))
    })
    .flatten();
    let Some(request) = request else {
        open_builtin(MANAGER_URL);
        return;
    };
    run_bookmark_request(request);
}

fn run_bookmark_request(request: BookmarkRequest) {
    if exiting() || with_shell(|s| !s.private && !s.closing) != Some(true) {
        return;
    }
    let existing = with_shell(|s| {
        s.tabs
            .iter()
            .find(|tab| {
                tab.url == MANAGER_URL && !tab.retiring && !tab.unloading && !tab.close_requested
            })
            .map(|t| t.key)
    })
    .flatten();
    let key = match existing
        .map(Ok)
        .unwrap_or_else(|| append_tab(MANAGER_URL, "", false, None, None).map(|(key, _)| key))
    {
        Ok(key) => key,
        Err(message) => {
            if let Some(ui) = controls() {
                set_text(ui.window, &format!("{} — {}", app_title(), i18n::text(&message)));
            }
            return;
        }
    };
    change(|s| {
        if let Some(tab) = s.tabs.iter_mut().find(|tab| tab.key == key) {
            tab.bookmark = Some(request);
        }
    });
    select_key(key);
    if let Some(browser) = active().filter(|browser| browser.is_loading() == 0) {
        if let Some(frame) = browser.main_frame() {
            finish_bookmark(key, &frame);
        }
    }
}

fn bookmark_all_tabs(window: HWND) {
    let Some(tabs) = with_shell(|s| {
        let mut seen = BTreeSet::new();
        s.tabs
            .iter()
            .filter(|tab| !tab.close_requested && !tab.retiring)
            .filter_map(|tab| {
                let url = session::stored_url(&tab.url)?;
                if !(url.starts_with("http://") || url.starts_with("https://"))
                    || !seen.insert(url.clone())
                {
                    return None;
                }
                Some(site_setup::BookmarkNode {
                    title: tab_label(tab),
                    url: Some(url),
                    children: Vec::new(),
                })
            })
            .collect::<Vec<_>>()
    }) else {
        return;
    };
    let title = wide(i18n::text("Bookmark all tabs"));
    let title = title.as_ptr();
    let count = tabs.len();
    let Some(request) = BookmarkRequest::save_tabs(&tabs) else {
        let body = wide(i18n::text(if count == 0 {
            "Open a web page to save its tab."
        } else {
            "This window has too much bookmark data to save at once."
        }));
        unsafe { MessageBoxW(window, body.as_ptr(), title, MB_OK | MB_ICONINFORMATION) };
        return;
    };
    let body = wide(&if i18n::korean() {
        format!(
            "웹 탭 {count}개를 기타 북마크의 새 폴더에 저장할까요?\n\n탭은 그대로 열려 있고, 로드 대기 중인 탭은 계속 대기합니다."
        )
    } else {
        format!(
            "Save {count} web {} to a new folder under Other bookmarks?\n\nTabs will stay open, and tabs waiting to load will stay on demand.",
            if count == 1 { "tab" } else { "tabs" }
        )
    });
    if unsafe {
        MessageBoxW(
            window,
            body.as_ptr(),
            title,
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        )
    } == IDYES
    {
        run_bookmark_request(request);
    }
}

fn finish_bookmark(key: usize, frame: &Frame) {
    if frame.is_main() == 0 || CefString::from(&frame.url()).to_string() != MANAGER_URL {
        return;
    }
    let pending = change(|s| {
        s.tabs
            .iter_mut()
            .find(|tab| tab.key == key)?
            .bookmark
            .take()
    })
    .flatten();
    if let Some(request) = pending {
        request.dispatch_on_load_end(frame);
    }
}

fn open_find() {
    if let Some(host) = host() {
        host.set_focus(1);
    }
    let Some(ui) = change(|s| {
        s.find_open = true;
        s.controls
    }) else {
        return;
    };
    layout();
    focus_control(ui.find_text, true);
}

fn close_find(refocus: bool) {
    let Some(ui) =
        change(|s| std::mem::replace(&mut s.find_open, false).then_some(s.controls)).flatten()
    else {
        return;
    };
    if let Some(host) = host() {
        host.stop_finding(1);
    }
    set_text(ui.find_text, "");
    set_text(ui.find_status, "");
    layout();
    if refocus {
        focus_page();
    }
}

fn find_in_page(forward: bool, again: bool) {
    let Some((ui, true)) = with_shell(|s| (s.controls, s.find_open)) else {
        return;
    };
    let Some(host) = host() else {
        return;
    };
    let query = text(ui.find_text, 1024);
    if query.is_empty() {
        host.stop_finding(1);
        set_text(ui.find_status, "");
        return;
    }
    host.find(
        Some(&CefString::from(query.as_str())),
        i32::from(forward),
        0,
        i32::from(again),
    );
}

pub(crate) fn find_update(key: usize, count: i32, active_match: i32) {
    if let Some(ui) =
        with_shell(|s| (s.active == Some(key) && s.find_open).then_some(s.controls)).flatten()
    {
        if text(ui.find_text, 1024).is_empty() {
            return;
        }
        let current = text(ui.find_status, 32);
        let old = current.split_once('/').unwrap_or(("0", "0"));
        let count = if count < 0 {
            old.1.parse().unwrap_or(0)
        } else {
            count
        };
        let active_match = if active_match < 0 {
            old.0.parse().unwrap_or(0)
        } else {
            active_match
        };
        set_text(ui.find_status, &format!("{active_match}/{count}"));
    }
}

fn vault_site() -> Option<vault_ui::Site> {
    let browser = with_shell(|s| {
        if s.private || s.closing {
            return None;
        }
        let tab = s.tabs.iter().find(|t| Some(t.key) == s.active)?;
        if tab.unloaded || tab.unloading || tab.close_requested || tab.retiring {
            return None;
        }
        tab.browser.clone()
    })??;
    vault_ui::site(&browser)
}

/// Commands available from the native Settings tab.
const PAGE_COMMANDS: &[usize] = &[
    TOGGLE_RESTORE, TOGGLE_DEFER_BACKGROUND, TOGGLE_AGENT_AUTO_CLOSE, MATERIAL, DISPLAY_LANGUAGE,
    IMPORT_CHROME_BOOKMARKS, IMPORT_EDGE_BOOKMARKS, IMPORT_BOOKMARKS, EXPORT_BOOKMARKS, BOOKMARKS,
    PASSWORDS, IMPORT_PASSWORDS, SITE_PERMISSIONS, CLEAR_BROWSING, DOWNLOAD_PREFERENCES, EXTENSIONS,
    CHROMIUM_SETTINGS, UPDATE, PREPARE_CHROME, PREPARE_EDGE, PREPARE_CLIPBOARD, OPEN_COPIED_TABS,
    RECOVERY_COPY,
];

fn settings_surface() -> Surface {
    let ko = i18n::korean();
    let t = |en: &str, k: &str| if ko { k.to_owned() } else { en.to_owned() };
    let toggle = |id: usize, title: String, detail: String, on| Item {
        id: id.to_string(),
        title,
        detail,
        actions: vec![Action { id, label: if on { t("On", "켜짐") } else { t("Off", "꺼짐") },
            selected: on, kind: ActionKind::Toggle }],
    };
    let link = |id: usize, title: String, detail: String| Item::link(id, title, detail);
    let choice = |id: &str, title: String, options: Vec<(usize, String, bool)>| Item {
        id: id.to_owned(),
        title,
        detail: String::new(),
        actions: options.into_iter().map(|(id, label, selected)| Action {
            id, label, selected, kind: ActionKind::Choice,
        }).collect(),
    };
    let engine = SEARCH_ENGINE.get();
    let sections = vec![
        Section {
            id: "general".into(),
            title: t("General", "일반"),
            items: vec![
                choice("search", t("Search engine", "검색 엔진"),
                    SearchEngine::ALL
                        .iter()
                        .enumerate()
                        .map(|(i, (e, label))| (SEARCH_ENGINE_FIRST + i, label.to_string(), *e == engine))
                        .collect(),
                ),
                toggle(TOGGLE_RESTORE, t("Restore windows and tabs on startup", "시작할 때 창과 탭 복원"),
                    t("Reopen what you had open last time.", "지난번에 열어 둔 창과 탭을 다시 엽니다."), RESTORE_TABS.get()),
                choice("memory", t("Memory Saver: sleep tabs you haven't looked at for", "메모리 절약: 안 본 탭을 잠재우기까지"),
                    MEMORY_SAVER_CHOICES
                        .iter()
                        .enumerate()
                        .map(|(i, &minutes)| {
                            let label = match (minutes, ko) {
                                (0, true) => "끄기".to_owned(),
                                (0, false) => "Off".to_owned(),
                                (60, true) => "1시간".to_owned(),
                                (60, false) => "1 hour".to_owned(),
                                (m, true) => format!("{m}분"),
                                (m, false) => format!("{m} min"),
                            };
                            (MEMORY_SAVER_FIRST + i, label, minutes == MEMORY_SAVER_MINUTES.get())
                        })
                        .collect(),
                ),
                toggle(TOGGLE_DEFER_BACKGROUND, t("Load background links when selected", "백그라운드 링크는 선택할 때 불러오기"),
                    t("Saves memory until you open the tab.", "탭을 열기 전까지 메모리를 아낍니다."), DEFER_BACKGROUND.get()),
                link(DISPLAY_LANGUAGE, t("Display language", "표시 언어"), t("Applies after restarting Flit.", "Flit을 다시 시작하면 적용됩니다.")),
                link(RECOVERY_COPY, t("Open previous session backup", "이전 세션 백업 열기"),
                    if PROFILE.with(|slot| slot.borrow().as_deref().is_some_and(session::has_backup)) {
                        t("Brings back the windows and tabs from before this start.", "이번 실행 전에 열려 있던 창과 탭을 되살립니다.")
                    } else {
                        t("No backup yet.", "아직 백업이 없습니다.")
                    }),
            ],
        },
        Section {
            id: "appearance".into(),
            title: t("Appearance", "화면"),
            items: vec![
                choice("theme", t("Theme", "테마"),
                    [t("Follow Windows", "Windows 설정 따르기"), t("Light", "라이트"), t("Dark", "다크")]
                        .into_iter()
                        .enumerate()
                        .map(|(i, label)| (THEME_FIRST + i, label, chrome::theme() as usize == i))
                        .collect(),
                ),
                toggle(MATERIAL, t("Acrylic window material", "아크릴 창 재질"),
                    t("Off uses Mica, the calmer Windows material.", "끄면 차분한 Mica 재질을 씁니다."), chrome::acrylic()),
            ],
        },
        Section {
            id: "ai".into(),
            title: t("AI", "AI"),
            items: vec![toggle(TOGGLE_AGENT_AUTO_CLOSE, t("Close AI tabs when the AI finishes", "AI 작업이 끝나면 AI 탭 닫기"),
                t("AI tabs open under \"AI tabs\" in the sidebar.", "AI 탭은 사이드바의 \"AI 탭\" 아래에 열립니다."),
                crate::agent_host::AUTO_CLOSE.load(std::sync::atomic::Ordering::Relaxed))],
        },
        Section {
            id: "import".into(),
            title: t("Bookmarks and moving in", "북마크와 가져오기"),
            items: vec![
                link(IMPORT_CHROME_BOOKMARKS, t("Import Chrome bookmarks", "Chrome 북마크 가져오기"), t("Shows up in the sidebar's Bookmarks.", "사이드바의 북마크에 나타납니다.")),
                link(IMPORT_EDGE_BOOKMARKS, t("Import Edge bookmarks", "Edge 북마크 가져오기"), String::new()),
                link(IMPORT_BOOKMARKS, t("Import bookmarks from an HTML file", "HTML 파일에서 북마크 가져오기"), String::new()),
                link(PREPARE_CHROME, t("Open sites from Chrome to sign in", "Chrome에서 쓰던 사이트 열어 로그인하기"), String::new()),
                link(PREPARE_EDGE, t("Open sites from Edge to sign in", "Edge에서 쓰던 사이트 열어 로그인하기"), String::new()),
                link(PREPARE_CLIPBOARD, t("Open copied site links", "복사한 사이트 링크 열기"), String::new()),
                link(OPEN_COPIED_TABS, t("Open copied tabs", "복사한 탭 열기"),
                    t("Add copied addresses. Tabs load when selected.", "복사한 주소를 추가합니다. 탭은 선택할 때 불러옵니다.")),
                link(EXPORT_BOOKMARKS, t("Export bookmarks", "북마크 내보내기"), String::new()),
                link(BOOKMARKS, t("Manage bookmarks", "북마크 관리"), String::new()),
            ],
        },
        Section {
            id: "privacy".into(),
            title: t("Privacy and security", "개인정보와 보안"),
            items: vec![
                link(PASSWORDS, t("Passwords", "비밀번호"), t("Logins you save go to Flit Vault and here.", "저장한 로그인은 Flit Vault와 여기에 함께 들어갑니다.")),
                link(IMPORT_PASSWORDS, t("Import passwords", "비밀번호 가져오기"), String::new()),
                link(SITE_PERMISSIONS, t("Site permissions", "사이트 권한"), t("Camera, microphone, location, notifications", "카메라, 마이크, 위치, 알림")),
                link(CLEAR_BROWSING, t("Clear browsing data", "인터넷 사용 기록 삭제"), String::new()),
            ],
        },
        Section {
            id: "downloads".into(),
            title: t("Downloads and more", "다운로드와 기타"),
            items: vec![
                link(DOWNLOAD_PREFERENCES, t("Download location", "다운로드 위치"), String::new()),
                link(EXTENSIONS, t("Extensions", "확장 프로그램"), String::new()),
                link(UPDATE, t("Check for updates", "업데이트 확인"), String::new()),
                link(CHROMIUM_SETTINGS, t("Advanced engine settings", "고급 엔진 설정"), t("Chromium's full settings", "Chromium 전체 설정")),
            ],
        },
    ];
    Surface {
        title: t("Settings", "설정"),
        subtitle: t("Changes take effect immediately unless noted.", "별도 안내가 없으면 변경 사항이 바로 적용됩니다."),
        dark: chrome::dark(),
        compact: false,
        start: false,
        preserve_input_focus: false,
        sections,
    }
}

fn exact_settings_control(panel: HWND) -> HWND {
    unsafe { GetDlgItem(panel, SETTINGS_CONTROL as i32) }
}

fn close_exact_settings(panel: HWND) {
    let view = exact_settings_control(panel);
    if !view.is_null() {
        unsafe { DestroyWindow(view) };
    }
}

fn active_exact_settings() -> Option<HWND> {
    let panel = with_shell(|s| s.tabs.iter()
        .find(|tab| Some(tab.key) == s.active && tab.url == FLIT_SETTINGS && !tab.retiring)
        .map(|tab| tab.panel)).flatten()?;
    let view = exact_settings_control(panel);
    (!view.is_null()).then_some(view)
}

fn refresh_exact_settings() {
    if let Some(view) = active_exact_settings() {
        exact_ui::update(view, &settings_surface());
    }
}

fn mount_exact_settings(key: usize) -> Result<HWND, &'static str> {
    mount_exact_tab(key, FLIT_SETTINGS, SETTINGS_CONTROL, &settings_surface())
}

fn mount_exact_tab(key: usize, address: &str, control: usize, surface: &Surface) -> Result<HWND, &'static str> {
    let (owner, panel) = with_shell(|s| {
        let tab = s.tabs.iter().find(|tab| tab.key == key && tab.url == address
            && !tab.retiring && tab.browser.is_none())?;
        Some((s.controls.window, tab.panel))
    }).flatten().ok_or("The tab is no longer available.")?;
    let existing = unsafe { GetDlgItem(panel, control as i32) };
    if !existing.is_null() {
        exact_ui::update(existing, surface);
        return Ok(existing);
    }
    let view = exact_ui::create(panel, owner, control, surface)?;
    // A synchronous UI Automation call can deliver a navigation before create returns.
    if with_shell(|s| s.tabs.iter().any(|tab| tab.key == key && tab.panel == panel
        && tab.url == address && tab.browser.is_none() && !tab.retiring))
        != Some(true)
        || unsafe { GetParent(view) } != panel
    {
        if unsafe { GetParent(view) } == panel {
            unsafe { DestroyWindow(view) };
        }
        return Err("The tab is no longer available.");
    }
    if unsafe { SetWindowSubclass(view, Some(chrome_proc), control, owner as usize) } == 0 {
        unsafe { DestroyWindow(view) };
        return Err("Could not initialize the tab interface.");
    }
    Ok(view)
}

fn restyle_windows() {
    let windows = WINDOWS.with(|windows| windows.borrow().keys().copied().collect::<Vec<_>>());
    for hwnd in windows {
        unsafe { SendMessageW(hwnd as HWND, WM_SETTINGCHANGE, 0, 0) };
    }
}

fn open_flit_settings() {
    let result = (|| {
        let existing = with_shell(|s| {
            s.tabs.iter().find(|tab| !tab.retiring && tab.url == FLIT_SETTINGS).map(|tab| tab.key)
        }).flatten();
        let key = if let Some(key) = existing {
            key
        } else {
            let title = i18n::choose("Settings", "설정");
            append_tab(FLIT_SETTINGS, title, false, None, None)?.0
        };
        let view = match mount_exact_settings(key) {
            Ok(view) => view,
            Err(error) => {
                if existing.is_none() {
                    close_tab(Some(key));
                }
                return Err(error);
            }
        };
        select_key(key);
        unsafe { SetFocus(view) };
        Ok(())
    })();
    if let (Err(message), Some(ui)) = (result, controls()) {
        let label = wide(i18n::text(message));
        unsafe { MessageBoxW(ui.window, label.as_ptr(), wide(app_title()).as_ptr(), MB_OK | MB_ICONERROR) };
    }
}

fn closed_title(entry: &Entry) -> String {
    let title = clean(entry.title.trim(), 64);
    let host = url::Url::parse(&entry.url)
        .ok()
        .and_then(|url| url.host_str().map(|host| clean(host, 40)));
    match (title.is_empty(), host) {
        (true, Some(host)) => host,
        (true, None) => "Untitled tab".into(),
        (false, Some(host)) if title != host => format!("{title} — {host}"),
        (false, _) => title,
    }
}

fn closed_choices(items: &[ClosedRecord]) -> Vec<(usize, String)> {
    items
        .iter()
        .rev()
        .map(|record| {
            let label = match &record.item {
                ClosedItem::Tab { entry, .. } => closed_title(entry),
                ClosedItem::Window(state) => {
                    let title = state
                        .entries
                        .get(state.selected)
                        .filter(|entry| entry.url != HOME)
                        .or_else(|| state.entries.iter().find(|entry| entry.url != HOME))
                        .map_or_else(|| i18n::text("Untitled window").into(), closed_title);
                    let count = state.entries.len();
                    let noun = if count == 1 { "tab" } else { "tabs" };
                    if i18n::korean() {
                        format!("창: {title} (탭 {count}개)")
                    } else {
                        format!("Window: {title} ({count} {noun})")
                    }
                }
            };
            (record.id, label.replace('&', "&&"))
        })
        .collect()
}

fn site_setup_info(window: HWND, message: &str) {
    let message = wide(i18n::text(message));
    unsafe {
        MessageBoxW(
            window,
            message.as_ptr(),
            wide(i18n::text("Prepare sites in Flit")).as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

fn choose_browser_profile(
    ui: Controls,
    browser: SourceBrowser,
) -> Option<site_setup::SourceProfile> {
    let Some(local) = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    else {
        site_setup_info(ui.window, "Flit could not find the local browser profiles.");
        return None;
    };
    let mut profiles = site_setup::discover(&local, browser);
    let profile = match profiles.len() {
        0 => {
            let name = browser.label();
            let message = if i18n::korean() {
                format!("{name} 북마크가 저장된 프로필을 찾지 못했습니다.")
            } else {
                format!("No standard {name} bookmarks profile was found.")
            };
            site_setup_info(ui.window, &message);
            return None;
        }
        1 => profiles.pop().unwrap(),
        _ => {
            let menu = unsafe { CreatePopupMenu() };
            if menu.is_null() {
                return None;
            }
            let selection = unsafe {
                for (index, profile) in profiles.iter().enumerate() {
                    AppendMenuW(menu, MF_STRING, index + 1, wide(&profile.label).as_ptr());
                }
                let mut rect = RECT::default();
                GetWindowRect(ui.menu, &mut rect);
                let selection = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTALIGN,
                    rect.right,
                    rect.bottom,
                    0,
                    ui.window,
                    null(),
                );
                DestroyMenu(menu);
                selection as usize
            };
            if selection == 0 || selection > profiles.len() || exiting() {
                return None;
            }
            profiles.swap_remove(selection - 1)
        }
    };
    (with_shell(|s| !s.private && !s.closing) == Some(true)).then_some(profile)
}

fn browser_site_setup(ui: Controls, browser: SourceBrowser) {
    let Some(profile) = choose_browser_profile(ui, browser) else {
        return;
    };
    match site_setup::bookmarked_sites(&profile, &open_site_origins()) {
        Ok(sites) => prepare_site_tabs(
            ui.window,
            &format!("{} · {}", browser.label(), profile.label),
            sites,
        ),
        Err(message) => site_setup_info(ui.window, message),
    }
}

fn import_browser_bookmarks(ui: Controls, browser: SourceBrowser) {
    let Some(profile) = choose_browser_profile(ui, browser) else {
        return;
    };
    let (nodes, count) = match site_setup::profile_bookmarks(&profile) {
        Ok(result) => result,
        Err(message) => {
            site_setup_info(ui.window, message);
            return;
        }
    };
    if count == 0 {
        site_setup_info(
            ui.window,
            i18n::choose(
                "There are no website bookmarks in this profile to import.",
                "이 프로필에는 가져올 웹사이트 북마크가 없습니다.",
            ),
        );
        return;
    }
    let title = format!("From {} - {}", browser.label(), profile.label);
    let Some(request) = BookmarkRequest::import_profile(&title, &nodes) else {
        site_setup_info(
            ui.window,
            i18n::choose(
                "This profile contains too much bookmark data to import at once.",
                "이 프로필에는 한 번에 가져올 수 있는 양보다 많은 북마크가 있습니다.",
            ),
        );
        return;
    };
    let detail = wide(&if i18n::korean() {
        format!(
            "북마크 {count}개와 폴더를 기타 북마크 → {title}에 추가하시겠습니까?\n\n기존 북마크는 그대로 유지됩니다."
        )
    } else {
        format!(
            "Import {count} bookmarks and their folders into Other bookmarks → {title}?\n\nExisting bookmarks will stay in place."
        )
    });
    if unsafe {
        MessageBoxW(
            ui.window,
            detail.as_ptr(),
            wide(i18n::text("Import bookmarks to Flit")).as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        )
    } == IDYES
    {
        run_bookmark_request(request);
    }
}

fn copied_site_text(window: HWND) -> Result<String, &'static str> {
    const UNICODE_TEXT: u32 = 13; // CF_UNICODETEXT.
    const MAX_COPY_BYTES: usize = 256 * 1024;
    if unsafe { OpenClipboard(window) } == 0 {
        return Err("The clipboard is busy. Try again after copying the site links.");
    }
    let result = (|| {
        let data = unsafe { GetClipboardData(UNICODE_TEXT) };
        if data.is_null() {
            return Err("Copy the site links as text, then try again.");
        }
        let size = unsafe { GlobalSize(data) };
        if !(2..=MAX_COPY_BYTES).contains(&size) || size % 2 != 0 {
            return Err("Copy a smaller list of site links, then try again.");
        }
        let buffer = unsafe { GlobalLock(data) }.cast::<u16>();
        if buffer.is_null() {
            return Err("Flit could not read the copied text.");
        }
        let units = unsafe { std::slice::from_raw_parts(buffer, size / 2) };
        let result = units
            .iter()
            .position(|&unit| unit == 0)
            .map(|end| String::from_utf16_lossy(&units[..end]))
            .ok_or("Flit could not read the copied text.");
        unsafe { GlobalUnlock(data) };
        result
    })();
    unsafe { CloseClipboard() };
    result
}

fn open_site_origins() -> BTreeSet<String> {
    with_shell(|s| {
        s.tabs
            .iter()
            .filter_map(|t| site_setup::site_origin(&t.url))
            .collect()
    })
    .unwrap_or_default()
}

fn copied_tabs_for_window(urls: &[String]) -> Option<(Vec<String>, usize)> {
    with_shell(|s| {
        if s.private || s.closing || s.active.is_none() {
            return None;
        }
        let open: BTreeSet<_> = s
            .tabs
            .iter()
            .filter_map(|tab| url::Url::parse(&tab.url).ok().map(String::from))
            .collect();
        let mut snapshot = window_snapshot(s);
        let existing = snapshot.entries.len();
        snapshot.entries.extend(
            urls.iter()
                .filter(|url| !open.contains(*url))
                .map(|url| Entry {
                    url: url.clone(),
                    title: String::new(),
                    pinned: false,
                    bookmark_url: None,
                }),
        );
        let requested = snapshot.entries.len() - existing;
        // Bound the import to the tabs and address lengths the session can restore.
        let urls = session::validated_window(&snapshot)
            .entries
            .into_iter()
            .skip(existing)
            .map(|entry| entry.url)
            .collect();
        Some((urls, requested))
    })
    .flatten()
}

fn open_copied_tabs(window: HWND) {
    if with_shell(|s| !s.private && !s.closing && s.active.is_some()) != Some(true) {
        return;
    }
    let caption = wide(i18n::choose("Open copied tabs", "복사한 탭 열기"));
    let show = |message: &str, flags| unsafe {
        MessageBoxW(window, wide(message).as_ptr(), caption.as_ptr(), flags)
    };
    let info = |message: &str| show(message, MB_OK | MB_ICONINFORMATION);
    let text = match copied_site_text(window) {
        Ok(text) => text,
        Err(message) => {
            info(i18n::text(message));
            return;
        }
    };
    let parsed = site_setup::pasted_tab_urls(&text, usize::MAX);
    if parsed.is_empty() {
        info(i18n::choose(
            "No HTTP(S) tab addresses found. Copy the tab addresses and try again.",
            "열 수 있는 탭 주소가 없습니다. 탭 주소를 복사한 뒤 다시 시도하세요.",
        ));
        return;
    }
    let Some((urls, requested)) = copied_tabs_for_window(&parsed) else {
        return;
    };
    let count = urls.len();
    if count == 0 {
        info(if requested == 0 {
            i18n::choose(
                "The copied tabs are already open in this window.",
                "복사한 탭이 이미 이 창에 열려 있습니다.",
            )
        } else {
            i18n::choose(
                "The copied tabs exceed this window’s session limits. Close some tabs or copy shorter addresses.",
                "복사한 탭이 이 창의 저장 한도를 넘습니다. 탭을 몇 개 닫거나 더 짧은 주소를 복사하세요.",
            )
        });
        return;
    }
    let prompt = if i18n::korean() {
        format!("이 창에 탭 {count}개를 추가할까요?\n\n주소만 가져옵니다. 탭은 선택할 때 불러옵니다.")
    } else {
        let noun = if count == 1 { "tab" } else { "tabs" };
        format!("Add {count} {noun} to this window?\n\nOnly addresses are imported. Tabs load when selected.")
    };
    if show(&prompt, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES || exiting() {
        return;
    }
    let Some((urls, _)) = copied_tabs_for_window(&urls) else {
        return;
    };
    let mut first = None;
    let mut added = 0;
    for url in urls {
        let address = url.split_once("://").map_or(url.as_str(), |(_, address)| address);
        let label = address.split(['?', '#']).next().unwrap_or(address);
        let Ok((key, _)) = append_tab(&url, label.trim_end_matches('/'), false, None, None) else {
            break;
        };
        first.get_or_insert(key);
        added += 1;
    }
    if let Some(key) = first {
        set_filter_cursor(FilterCursor::Off);
        show_prepared_tabs();
        if with_shell(|s| s.fullscreen.is_some()) == Some(true) {
            change(|s| s.pending_tab_reveal = Some(key));
        } else {
            reveal_tab_key(key);
        }
    }
    let outcome = if added == 0 {
        i18n::choose("No tabs were added.", "추가된 탭이 없습니다.").to_owned()
    } else if added < requested {
        if i18n::korean() {
            format!("새 탭 {requested}개 중 {added}개를 추가했습니다.")
        } else {
            format!("Added {added} of {requested} new tabs.")
        }
    } else if i18n::korean() {
        format!("탭 {added}개를 추가했습니다.")
    } else {
        let noun = if added == 1 { "tab" } else { "tabs" };
        format!("Added {added} {noun}.")
    };
    info(&outcome);
}

fn prepare_site_tabs(window: HWND, label: &str, sites: Vec<String>) {
    if sites.is_empty() {
        site_setup_info(window, "There are no new site links to prepare.");
        return;
    }
    let body = wide(&if i18n::korean() {
        format!(
            "{label}\n\n이 창에 사이트 탭 {}개를 준비할까요?\n\n{}\n\n탭은 선택할 때 로드됩니다. 필요한 사이트에는 직접 로그인하세요. 비밀번호와 로그인 상태는 옮겨지지 않습니다.",
            sites.len(),
            sites.join("\n")
        )
    } else {
        format!(
            "{label}\n\nPrepare {} site tabs in this window?\n\n{}\n\nTabs load when selected. Sign in on each site as needed. Passwords and login sessions are not transferred.",
            sites.len(),
            sites.join("\n")
        )
    });
    if unsafe {
        MessageBoxW(
            window,
            body.as_ptr(),
            wide(i18n::text("Prepare sites in Flit")).as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        )
    } != IDYES
        || exiting()
    {
        return;
    }
    if with_shell(|s| s.private || s.closing || s.active.is_none()) != Some(false) {
        return;
    }
    let open = open_site_origins();
    let mut added = false;
    let mut failure = None;
    for url in sites.into_iter().filter(|site| !open.contains(site)) {
        if let Err(message) = append_tab(&url, url.trim_end_matches('/'), false, None, None) {
            failure = Some(message);
            break;
        }
        added = true;
    }
    if added {
        show_prepared_tabs();
    }
    if let Some(message) = failure {
        site_setup_info(window, message);
    }
}

fn show_prepared_tabs() {
    if let Some(ui) = controls()
        && !text(ui.filter, 256).is_empty()
    {
        set_text(ui.filter, "");
    }
    if with_shell(|s| s.sidebar_open) == Some(false) {
        set_sidebar(true, true);
    }
    refresh_rows(false);
    queue_session_save();
}

fn shortcut_command(action: Shortcut) -> usize {
    match action {
        Shortcut::FocusAddress => FOCUS_ADDRESS,
        Shortcut::FocusSearch => FOCUS_SEARCH,
        Shortcut::Bookmark => SAVE_BOOKMARK,
        Shortcut::BookmarkAllTabs => BOOKMARK_ALL_TABS,
        Shortcut::ManageBookmarks => BOOKMARKS,
        Shortcut::NewTab => NEW_TAB,
        Shortcut::NewWindow => NEW_WINDOW,
        Shortcut::PrivateWindow => PRIVATE_WINDOW,
        Shortcut::RestoreTab => RESTORE_TAB,
        Shortcut::CloseTab => CLOSE_TAB,
        Shortcut::CloseWindow => CLOSE_WINDOW,
        Shortcut::Reload => RELOAD_SHORTCUT,
        Shortcut::HardReload => HARD_RELOAD,
        Shortcut::Back => BACK,
        Shortcut::Forward => FORWARD,
        Shortcut::NextTab => NEXT_TAB,
        Shortcut::PreviousTab => PREVIOUS_TAB,
        Shortcut::SelectTab(n) => SELECT_TAB_FIRST + n.saturating_sub(1).min(7),
        Shortcut::LastTab => SELECT_TAB_LAST,
        Shortcut::FocusFilter => FOCUS_FILTER,
        Shortcut::ToggleSidebar => SIDEBAR,
        Shortcut::ToggleSplit => SPLIT_VIEW,
        Shortcut::Find => FIND,
        Shortcut::FindNext => FOCUS_FIND_NEXT,
        Shortcut::FindPrevious => FOCUS_FIND_PREV,
        Shortcut::History => HISTORY,
        Shortcut::ClearBrowsingData => CLEAR_BROWSING,
        Shortcut::Downloads => DOWNLOADS,
        Shortcut::Print => PRINT,
        Shortcut::DevTools => DEVTOOLS,
        Shortcut::DevToolsConsole => DEVTOOLS_CONSOLE,
        Shortcut::ToggleFullscreen => FULLSCREEN,
        Shortcut::ZoomIn => ZOOM_IN,
        Shortcut::ZoomOut => ZOOM_OUT,
        Shortcut::ZoomReset => ZOOM_RESET,
    }
}

fn focus_control(hwnd: HWND, select_text: bool) {
    unsafe {
        SetFocus(hwnd);
        if select_text {
            SendMessageW(hwnd, EM_SETSEL, 0, -1);
        }
    }
}

fn focus_page() {
    if let Some(host) = active().and_then(|browser| browser.host()) {
        host.set_focus(1);
    } else if let Some(view) = active_exact_settings()
        .or_else(|| active_start().map(|(_, panel)| start_view(panel)).filter(|view| !view.is_null()))
    {
        unsafe { SetFocus(view) };
    } else if let Some(ui) = controls() {
        if with_shell(|s| s.fullscreen.is_some()) == Some(true) {
            unsafe { SetFocus(ui.window) };
        } else {
            focus_control(ui.address, false);
        }
    }
}

fn modifiers() -> (bool, bool, bool) {
    let down = |key: u16| unsafe { GetKeyState(i32::from(key)) < 0 };
    (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU))
}

fn tab_rect(hwnd: HWND, index: usize) -> Option<RECT> {
    let mut row = RECT::default();
    (unsafe {
        SendMessageW(
            hwnd,
            LB_GETITEMRECT,
            index,
            (&mut row as *mut RECT) as isize,
        )
    } != LB_ERR as isize)
        .then_some(row)
}

fn tab_hit(hwnd: HWND, lp: LPARAM) -> Option<(usize, RECT)> {
    let hit = unsafe { SendMessageW(hwnd, LB_ITEMFROMPOINT, 0, lp) } as u32;
    if hit >> 16 != 0 {
        return None;
    }
    let index = (hit & 0xffff) as usize;
    let row = tab_rect(hwnd, index)?;
    let x = (lp as u16 as i16) as i32;
    let y = ((lp >> 16) as u16 as i16) as i32;
    (x >= row.left && x < row.right && y >= row.top && y < row.bottom).then_some((index, row))
}

fn tab_point(hwnd: HWND, lp: LPARAM, close_only: bool) -> Option<usize> {
    let (index, row) = tab_hit(hwnd, lp)?;
    let target = chrome::TAB_CLOSE_WIDTH * unsafe { GetDpiForWindow(hwnd) }.max(96) as i32 / 96;
    if close_only && ((lp as u16 as i16) as i32) < row.right - target {
        return None;
    }
    row_key(index)
}

fn tab_unload_point(hwnd: HWND, lp: LPARAM) -> Option<usize> {
    let (index, row) = tab_hit(hwnd, lp)?;
    let x = (lp as u16 as i16) as i32;
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
    let right = row.right - chrome::TAB_CLOSE_WIDTH * dpi / 96;
    let left = row.right - (chrome::TAB_CLOSE_WIDTH + chrome::TAB_UNLOAD_WIDTH) * dpi / 96;
    if !(left..right).contains(&x) {
        return None;
    }
    with_shell(|s| {
        let key = s.rows.get(index)?.key;
        let tab = s.tabs.iter().find(|tab| tab.key == key)?;
        can_unload(s, tab).then_some(key)
    })
    .flatten()
}

thread_local! {
    /// When the hovered tab row became hot, and whether its memory preview is up.
    static HOVER_SINCE: Cell<(Option<Instant>, bool)> = const { Cell::new((None, false)) };
}

/// Hover from the cursor position, for every toolbar/sidebar control at once.
/// TrackMouseEvent cannot work here: color-keyed sidebar pixels belong to the
/// main window, so a list "leaves" the moment the cursor is over its background.
fn refresh_hover() {
    let Some(ui) = controls() else {
        return;
    };
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) };
    let under = unsafe { WindowFromPoint(point) };
    let mine = !under.is_null()
        && unsafe { GetAncestor(under, GA_ROOT) } == ui.window
        && unsafe { GetCapture() }.is_null()
        && !exiting();
    let lists = [ui.tabs, ui.bookmarks, ui.agents];
    let mut next: (usize, isize) = (0, -1);
    if mine {
        for hwnd in [
            ui.sidebar, ui.back, ui.forward, ui.reload, ui.site_info, ui.bookmark, ui.go,
            ui.extensions, ui.vault, ui.downloads, ui.menu, ui.add, ui.free_memory, ui.bookmark_header,
            ui.agent_header, ui.tabs, ui.bookmarks, ui.agents,
        ] {
            if unsafe { IsWindowVisible(hwnd) } == 0 {
                continue;
            }
            let Some(r) = shown_rect(hwnd) else {
                continue;
            };
            if point.x < r.left || point.x >= r.right || point.y < r.top || point.y >= r.bottom {
                continue;
            }
            if lists.contains(&hwnd) {
                let mut local = point;
                unsafe { ScreenToClient(hwnd, &mut local) };
                let lp = ((local.y as u16 as isize) << 16) | local.x as u16 as isize;
                let hit = unsafe { SendMessageW(hwnd, LB_ITEMFROMPOINT, 0, lp) } as u32;
                if hit >> 16 == 0 {
                    next = (hwnd as usize, (hit & 0xffff) as isize);
                }
            } else if unsafe { IsWindowEnabled(hwnd) } != 0 {
                next = (hwnd as usize, -1);
            }
            break;
        }
    }
    let previous = chrome::set_hot(next);
    if previous != next {
        for (hwnd, index) in [previous, next] {
            let hwnd = hwnd as HWND;
            if hwnd.is_null() {
                continue;
            }
            let mut r = RECT::default();
            if index < 0 || unsafe { SendMessageW(hwnd, LB_GETITEMRECT, index as usize, &mut r as *mut RECT as LPARAM) } < 0 {
                unsafe { InvalidateRect(hwnd, null(), 0) };
            } else {
                unsafe { InvalidateRect(hwnd, &r, 0) };
            }
        }
        if previous.0 == ui.tabs as usize {
            clear_tab_memory();
        }
        HOVER_SINCE.set((Some(Instant::now()), false));
    }
    // A tab row held under the cursor shows its memory use in the status line.
    if next.0 == ui.tabs as usize
        && let (Some(since), false) = HOVER_SINCE.get()
        && since.elapsed() >= Duration::from_millis(360)
        && let Some(key) = row_key(next.1 as usize)
    {
        HOVER_SINCE.set((Some(since), true));
        change(|s| s.hovered_row = Some(key));
        show_tab_memory(key);
    }
    unsafe {
        if next.0 != 0 {
            SetTimer(ui.window, HOVER_TIMER, 60, None);
        } else {
            KillTimer(ui.window, HOVER_TIMER);
        }
    }
}

fn tab_move_indices(s: &Shell, key: usize, drop: TabDrop) -> Option<(usize, usize)> {
    let from = s.tabs.iter().position(|tab| tab.key == key)?;
    let hit = s.tabs.iter().position(|tab| tab.key == drop.key)?;
    if s.closing || from == hit || s.tabs[from].pinned != s.tabs[hit].pinned {
        return None;
    }
    let to = hit + usize::from(drop.after) - usize::from(from < hit);
    (from != to).then_some((from, to))
}

fn adjacent_tab(s: &Shell, key: usize, down: bool) -> Option<TabDrop> {
    let index = s.tabs.iter().position(|t| t.key == key)?;
    let next = if down {
        index + 1
    } else {
        index.checked_sub(1)?
    };
    let drop = TabDrop {
        key: s.tabs.get(next)?.key,
        after: down,
    };
    tab_move_indices(s, key, drop).map(|_| drop)
}

fn move_tab(key: usize, drop: TabDrop) {
    if change(|s| {
        let (from, to) = tab_move_indices(s, key, drop)?;
        let tab = s.tabs.remove(from);
        s.tabs.insert(to, tab);
        Some(())
    })
    .flatten()
    .is_some()
    {
        refresh_rows(false);
        queue_session_save();
    }
}

fn update_tab_drag(list: HWND, lp: LPARAM) -> Option<TabDrag> {
    let mut drag = with_shell(|s| s.tab_drag).flatten()?;
    let y = ((lp >> 16) as u16 as i16) as i32;
    let threshold = 5 * unsafe { GetDpiForWindow(list) }.max(96) as i32 / 96;
    drag.dragging |= (y - drag.start_y).abs() >= threshold;
    let target = drag
        .dragging
        .then(|| {
            let (row, rect) = tab_hit(list, lp)?;
            let drop = TabDrop {
                key: row_key(row)?,
                after: y >= (rect.top + rect.bottom) / 2,
            };
            with_shell(|s| tab_move_indices(s, drag.key, drop))
                .flatten()
                .map(|_| drop)
        })
        .flatten();
    if target != drag.target {
        drag.target = target;
        unsafe { InvalidateRect(list, null(), 0) };
    }
    change(|s| s.tab_drag = Some(drag));
    Some(drag)
}

fn show_tab_menu(list: HWND, parent: HWND, coordinates: LPARAM) {
    let mut point = POINT {
        x: (coordinates as u16 as i16) as i32,
        y: ((coordinates >> 16) as u16 as i16) as i32,
    };
    let key = if coordinates == -1 {
        let index = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) };
        if index < 0 {
            return;
        }
        let Some(row) = tab_rect(list, index as usize) else {
            return;
        };
        point = POINT {
            x: row.left,
            y: row.bottom,
        };
        unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(list, &mut point) };
        row_key(index as usize)
    } else {
        let mut client = point;
        unsafe { ScreenToClient(list, &mut client) };
        tab_point(
            list,
            (client.x as u16 as u32 | (client.y as u16 as u32) << 16) as LPARAM,
            false,
        )
    };
    let Some(key) = key else {
        return;
    };
    let split_label = with_shell(|s| {
        if s.visible_partner() == Some(key) {
            Some(i18n::choose("Exit split view", "분할 보기 종료"))
        } else if can_split_with(s, key) {
            Some(i18n::choose("Split with current tab", "현재 탭과 분할 보기"))
        } else {
            None
        }
    }).flatten();
    let Some((pinned, muted, duplicable, copyable, unloadable, up, down, site, close_later)) =
        with_shell(|s| {
            let tab = s.tabs.iter().find(|t| t.key == key)?;
            let url = session::stored_url(&tab.url);
            let copyable = !s.private
                && !s.closing
                && url.as_deref().is_some_and(|url| {
                    url.starts_with("http://") || url.starts_with("https://")
                });
            Some((
                tab.pinned,
                tab.muted,
                url.is_some(),
                copyable,
                can_unload(s, tab),
                adjacent_tab(s, key, false).is_some(),
                adjacent_tab(s, key, true).is_some(),
                if s.private {
                    None
                } else {
                    memory_sites::host(&tab.url)
                },
                tab.download_close == DownloadClose::Waiting,
            ))
        })
        .flatten()
    else {
        return;
    };
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    if let Some(label) = split_label {
        unsafe { AppendMenuW(menu, MF_STRING, TAB_SPLIT_WITH, wide(label).as_ptr()) };
    }
    for (id, label, enabled) in [
        (TAB_PIN, if pinned { "Unpin tab" } else { "Pin tab" }, true),
        (TAB_DUPLICATE, i18n::choose("Duplicate tab", "탭 복제"), duplicable),
        (TAB_COPY_LINK, i18n::choose("Copy link", "링크 복사"), copyable),
        (
            TAB_MUTE,
            if muted { "Unmute tab" } else { "Mute tab" },
            true,
        ),
        (TAB_UNLOAD, "Unload tab to free memory…", unloadable),
        (TAB_MOVE_UP, "Move tab up", up),
        (TAB_MOVE_DOWN, "Move tab down", down),
        if close_later {
            (
                TAB_CANCEL_DEFERRED_CLOSE,
                "Keep tab open after downloads",
                true,
            )
        } else {
            (TAB_CLOSE, "Close tab", true)
        },
    ] {
        unsafe {
            AppendMenuW(
                menu,
                MF_STRING | if enabled { 0 } else { MF_GRAYED },
                id,
                wide(label).as_ptr(),
            );
        }
    }
    if let Some(site) = &site {
        let flags = MEMORY_SITES.with(|slot| {
            if slot.borrow().contains(site) {
                MF_CHECKED
            } else {
                0
            }
        });
        let label = wide(&if i18n::korean() {
            format!("메모리 정리에서 {site} 제외")
        } else {
            format!("Keep {site} active when freeing memory")
        });
        unsafe {
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
            AppendMenuW(menu, MF_STRING | flags, TAB_KEEP_SITE, label.as_ptr());
        }
    }
    unsafe {
        let action = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            parent,
            null(),
        );
        DestroyMenu(menu);
        if action as usize == TAB_KEEP_SITE {
            // The page can navigate while the native menu is open.
            if let Some(site) = site.as_deref()
                && tab_memory_site(key).as_deref() == Some(site)
                && !exiting()
            {
                tab_action(key, TAB_KEEP_SITE);
            }
        } else if action != 0 {
            PostMessageW(parent, TAB_MENU_ACTION, key, action as LPARAM);
        }
    }
}

fn can_unload(s: &Shell, tab: &Tab) -> bool {
    !s.closing
        && !tab.unloading
        && !tab.close_requested
        && tab.download_close == DownloadClose::None
        && !s.page_visible(tab.key)
        && tab.browser.is_some()
        && (tab.url.starts_with("http://") || tab.url.starts_with("https://"))
        && session::stored_url(&tab.url).is_some()
        && download_guard::count(s.controls.window as usize, Some(tab.key)) == 0
}

fn can_release_background(s: &Shell, tab: &Tab) -> bool {
    !tab.pinned
        && !tab.has_video_access
        && !tab.has_audio_access
        && !tab.retiring
        && s.popups.is_empty()
        && s.pending_popups.is_empty()
        && can_unload(s, tab)
        && (s.private
            || !MEMORY_SITES.with(|slot| {
                let sites = slot.borrow();
                !sites.is_empty()
                    && memory_sites::host(&tab.url).is_some_and(|site| sites.contains(&site))
            }))
        && tab.browser.as_ref().is_some_and(|browser| {
            browser.is_loading() == 0
                && browser.host().is_some_and(|host| host.has_dev_tools() == 0)
        })
}

fn tab_memory_site(key: usize) -> Option<String> {
    with_shell(|s| {
        if s.private || s.closing {
            return None;
        }
        memory_sites::host(&s.tabs.iter().find(|tab| tab.key == key)?.url)
    })
    .flatten()
}

fn toggle_memory_site(window: HWND, site: &str) {
    let result = PROFILE.with(|slot| {
        let profile = slot.borrow();
        profile
            .as_deref()
            .map(|profile| MEMORY_SITES.with(|sites| sites.borrow_mut().toggle(profile, site)))
    });
    if let Some(Err(error)) = result {
        eprintln!("Flit Browser: Could not save memory exception: {error}");
        unsafe {
            MessageBoxW(
                window,
                wide(i18n::text("Could not save this site's memory setting.")).as_ptr(),
                windows_sys::w!("Flit Browser"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

fn background_tabs_for_release() -> Vec<usize> {
    with_shell(|s| {
        if !s.memory_queue.is_empty() || s.tabs.iter().any(|tab| tab.unloading) {
            return Vec::new();
        }
        s.tabs
            .iter()
            .filter(|tab| can_release_background(s, tab))
            .map(|tab| tab.key)
            .collect()
    })
    .unwrap_or_default()
}

fn free_background_memory(window: HWND) {
    let keys = background_tabs_for_release();
    if keys.is_empty() {
        unsafe {
            MessageBoxW(
                window,
                wide(i18n::text("There are no eligible background tabs to unload right now.")).as_ptr(),
                wide(i18n::text("Free tab memory")).as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
        return;
    }
    let message = wide(&if i18n::korean() {
        format!(
            "백그라운드 탭을 최대 {}개 정리할까요? 탭은 사이드바에 남고, 선택하면 다시 로드됩니다.\n\n제외한 사이트, 고정 탭, 다운로드 중인 탭, 카메라·마이크 사용 중인 탭, 개발자 도구로 검사 중인 탭은 건너뜁니다. 정리한 탭의 뒤로·앞으로 기록은 지워집니다. 재생 중인 소리가 멈추거나 저장하지 않은 작업이 사라질 수 있습니다.",
            keys.len()
        )
    } else {
        format!(
            "Unload up to {} background tabs? They stay in the sidebar and reload when selected.\n\nKept sites, pinned tabs, downloads, camera or microphone use, and inspected tabs are skipped. Back and Forward history is cleared in the unloaded tabs. Other audio and unsaved work can stop or be lost.",
            keys.len()
        )
    });
    if unsafe {
        MessageBoxW(
            window,
            message.as_ptr(),
            wide(i18n::text("Free tab memory?")).as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        )
    } != IDYES
    {
        return;
    }
    change(|s| s.memory_queue.extend(keys.into_iter().map(MemoryRelease::Manual)));
    release_next_background_tab();
}

fn release_next_background_tab() {
    let next = change(|s| {
        if s.closing || s.tabs.iter().any(|tab| tab.unloading) {
            return None;
        }
        let automatic_delay = s.memory_queue.iter().any(|request| matches!(request, MemoryRelease::Automatic(_)))
            .then(automatic_release_delay).flatten();
        while let Some(request) = s.memory_queue.pop_front() {
            let key = match request {
                MemoryRelease::Manual(key) | MemoryRelease::Automatic(key) => key,
            };
            let Some(index) = s.tabs.iter().position(|tab| tab.key == key) else {
                continue;
            };
            let tab = &s.tabs[index];
            let eligible = match request {
                MemoryRelease::Manual(_) => can_release_background(s, tab),
                MemoryRelease::Automatic(_) => automatic_delay.is_some_and(|idle| can_auto_release(s, tab, idle)),
            };
            if eligible {
                s.tabs[index].unloading = true;
                return Some((s.controls.window, key));
            }
        }
        None
    })
    .flatten();
    if let Some((window, key)) = next {
        permissions::cancel_tab(window as usize, key);
        close_embedded_page(window, key);
    }
}

fn unload_tab(key: usize) {
    let window = with_shell(|s| {
        let tab = s.tabs.iter().find(|t| t.key == key)?;
        can_unload(s, tab).then_some(s.controls.window)
    })
    .flatten();
    let Some(window) = window else {
        return;
    };
    let confirmed = unsafe {
        MessageBoxW(
            window,
            wide(i18n::text(
                "This tab will reload when you select it. Back and Forward history will be cleared, audio and calls will stop, and unsaved changes may be lost. Unload the tab?",
            ))
            .as_ptr(),
            wide(i18n::text("Unload tab?")).as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        )
    } == IDYES;
    if !confirmed {
        return;
    }
    let host = with_shell(|s| {
        let tab = s
            .tabs
            .iter()
            .find(|tab| tab.key == key && can_unload(s, tab))?;
        tab.browser.clone()
    })
    .flatten()
    .and_then(|browser| browser.host());
    if let Some(host) = host {
        permissions::cancel_tab(window as usize, key);
        change(|s| {
            if let Some(tab) = s.tabs.iter_mut().find(|tab| tab.key == key) {
                tab.unloading = true;
            }
        });
        // The user already confirmed losing page state. Wait for CEF to finish closing.
        host.close_browser(1);
    }
}

fn tab_action(key: usize, action: usize) {
    match action {
        TAB_SPLIT_WITH => split_with(key),
        TAB_KEEP_SITE => {
            if let Some(site) = tab_memory_site(key)
                && let Some(ui) = controls()
            {
                toggle_memory_site(ui.window, &site);
            }
        }
        TAB_MOVE_UP | TAB_MOVE_DOWN => {
            if let Some(drop) =
                with_shell(|s| adjacent_tab(s, key, action == TAB_MOVE_DOWN)).flatten()
            {
                move_tab(key, drop);
            }
        }
        TAB_PIN => {
            change(|s| {
                let i = s.tabs.iter().position(|t| t.key == key)?;
                let mut tab = s.tabs.remove(i);
                tab.pinned = !tab.pinned;
                let boundary = s.tabs.iter().take_while(|t| t.pinned).count();
                s.tabs.insert(boundary, tab);
                Some(())
            });
            refresh_rows(false);
            queue_session_save();
        }
        TAB_DUPLICATE => {
            let Some((url, title, pinned)) = with_shell(|s| {
                let tab = s.tabs.iter().find(|tab| tab.key == key)?;
                Some((session::stored_url(&tab.url)?, tab.title.clone(), tab.pinned))
            })
            .flatten() else {
                return;
            };
            if let Ok((duplicate, _)) = append_tab(&url, &title, pinned, Some(key), None) {
                move_tab(duplicate, TabDrop { key, after: true });
                select_key(duplicate);
                focus_page();
            }
        }
        TAB_COPY_LINK => {
            use windows_sys::Win32::System::{
                DataExchange::{EmptyClipboard, SetClipboardData},
                Memory::{GMEM_MOVEABLE, GlobalAlloc},
            };

            let Some((window, url)) = with_shell(|s| {
                if s.private || s.closing {
                    return None;
                }
                let tab = s.tabs.iter().find(|tab| tab.key == key)?;
                let url = session::stored_url(&tab.url)?;
                (url.starts_with("http://") || url.starts_with("https://"))
                    .then_some((s.controls.window, url))
            })
            .flatten() else {
                return;
            };
            let units = wide(&url);
            unsafe {
                let memory = GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of_val(units.as_slice()));
                if memory.is_null() {
                    return;
                }
                let buffer = GlobalLock(memory).cast::<u16>();
                if buffer.is_null() {
                    GlobalFree(memory);
                    return;
                }
                std::ptr::copy_nonoverlapping(units.as_ptr(), buffer, units.len());
                GlobalUnlock(memory);
                const CF_UNICODETEXT: u32 = 13;
                let transferred = if OpenClipboard(window) != 0 {
                    let set = EmptyClipboard() != 0
                        && !SetClipboardData(CF_UNICODETEXT, memory).is_null();
                    CloseClipboard();
                    set
                } else {
                    false
                };
                // Windows owns the allocation after SetClipboardData succeeds.
                if !transferred {
                    GlobalFree(memory);
                }
            }
        }
        TAB_MUTE => {
            if let Some((muted, browser)) = change(|s| {
                let tab = s.tabs.iter_mut().find(|t| t.key == key)?;
                tab.muted = !tab.muted;
                Some((tab.muted, tab.browser.clone()))
            })
            .flatten()
            {
                if let Some(host) = browser.and_then(|b| b.host()) {
                    host.set_audio_muted(i32::from(muted));
                }
                refresh_rows(false);
            }
        }
        TAB_CLOSE => close_tab(Some(key)),
        TAB_CANCEL_DEFERRED_CLOSE => {
            if change(|s| {
                let Some(tab) = s
                    .tabs
                    .iter_mut()
                    .find(|t| t.key == key && t.download_close == DownloadClose::Waiting)
                else {
                    return false;
                };
                tab.download_close = DownloadClose::None;
                true
            }) == Some(true)
            {
                refresh_rows(false);
            }
        }
        TAB_UNLOAD => unload_tab(key),
        _ => {}
    }
}

fn set_filter_cursor(cursor: FilterCursor) {
    let Some((tabs, index)) = change(|s| {
        if s.filter_cursor == cursor {
            return None;
        }
        s.filter_cursor = cursor;
        Some((s.controls.tabs, cursor.highlight(&s.rows, s.active)))
    })
    .flatten() else {
        return;
    };
    unsafe { SendMessageW(tabs, LB_SETCURSEL, index.unwrap_or(usize::MAX), 0) };
}

fn filter_input(command: u16) {
    let Some(ui) = controls() else {
        return;
    };
    let selection = match command {
        VK_UP | VK_DOWN => {
            let target = with_shell(|s| {
                let end = s.rows.len().checked_sub(1)?;
                let index = s.filter_cursor.index(&s.rows).unwrap_or(0);
                let next = if command == VK_UP {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(end)
                };
                Some(s.rows[next].key)
            })
            .flatten();
            if let Some(key) = target {
                set_filter_cursor(FilterCursor::Tab(key));
            }
            return;
        }
        VK_RETURN => {
            let Some(key) = with_shell(|s| {
                s.rows
                    .get(s.filter_cursor.index(&s.rows).unwrap_or(0))
                    .map(|row| row.key)
            })
            .flatten() else {
                return;
            };
            Some(key)
        }
        VK_ESCAPE => None,
        _ => return,
    };
    set_filter_cursor(FilterCursor::Off);
    set_text(ui.filter, "");
    if let Some(key) = selection {
        select_key(key);
        focus_page();
    } else {
        focus_control(ui.tabs, false);
    }
}

unsafe extern "system" fn chrome_proc(
    hwnd: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    parent: usize,
) -> LRESULT {
    const ALT_CONTEXT: LPARAM = 1 << 29;
    const PREVIOUS_KEY_STATE: LPARAM = 1 << 30;
    let _scope = WindowScope::enter(parent);
    let id = unsafe { GetDlgCtrlID(hwnd) } as usize;
    if id == SPLIT_DIVIDER
        && let Some(result) = split_divider_message(hwnd, parent as HWND, message, wp, lp)
    {
        return result;
    }
    if message == WM_MOUSEMOVE {
        refresh_hover();
    }
    // The sidebar scrolls as one column; no section scrolls on its own.
    if message == WM_MOUSEWHEEL
        && matches!(id, TABS | BOOKMARK_LIST | AGENT_LIST | BOOKMARK_HEADER | AGENT_HEADER | SIDEBAR_THUMB)
    {
        scroll_sidebar(((wp >> 16) & 0xffff) as i16 as i32);
        return 0;
    }
    if id == SIDEBAR_THUMB && drag_thumb(hwnd, message) {
        return 0;
    }
    if matches!(id, ADDRESS | FILTER | FIND_TEXT) {
        if matches!(message, WM_PAINT | WM_PRINTCLIENT)
            && chrome::paint_dark_edit_cue(hwnd, id, edit_cue(id), wp as HDC)
        {
            return 0;
        }
        let composing = (matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN)
            || (matches!(message, WM_CHAR | WM_SYSCHAR) && wp == VK_RETURN as usize))
            && keyboard::edit_composing(hwnd);
        if let Some(result) = keyboard::handle_edit_message(hwnd, message, wp, lp, composing) {
            return result;
        }
        if id == ADDRESS {
            match message {
                WM_KILLFOCUS | WM_IME_STARTCOMPOSITION => {
                    dismiss_address_suggestions(parent as HWND);
                }
                WM_IME_ENDCOMPOSITION => {
                    if let Some(ui) = controls() {
                        schedule_address_suggestions(ui);
                    }
                }
                _ => {}
            }
        }
        if composing {
            return unsafe { DefSubclassProc(hwnd, message, wp, lp) };
        }
    }
    if id == FILTER {
        match message {
            WM_SETFOCUS => set_filter_cursor(FilterCursor::First),
            WM_KILLFOCUS => set_filter_cursor(FilterCursor::Off),
            _ => {}
        }
    }
    if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN) {
        let (ctrl, shift, alt) = modifiers();
        if id == ADDRESS && !ctrl && !shift && !alt {
            if wp == VK_ESCAPE as usize && dismiss_address_suggestions(parent as HWND) {
                return 0;
            }
            if matches!(wp as u16, VK_DOWN | VK_UP)
                && let Some(ui) = controls()
                && move_address_suggestion(ui, wp == VK_DOWN as usize)
            {
                return 0;
            }
        }
        if wp == VK_ESCAPE as usize && !ctrl && !shift && !alt && crate::popover::escape(parent as HWND) {
            return 0;
        }
        if id == ADDRESS
            && wp == VK_RETURN as usize
            && message == WM_SYSKEYDOWN
            && lp & ALT_CONTEXT != 0
            && !ctrl
            && !shift
        {
            if lp & PREVIOUS_KEY_STATE == 0 {
                unsafe { PostMessageW(parent as HWND, WM_COMMAND, GO_NEW_TAB, 0) };
            }
            return 0;
        }
        if let Some(action) = keyboard::shortcut(wp as u16, ctrl, shift, alt) {
            unsafe {
                PostMessageW(parent as HWND, WM_COMMAND, shortcut_command(action), 0);
            }
            return 0;
        }
        match (id, wp as u16) {
            (TABS, VK_ESCAPE) if change(|s| s.tab_drag.take().is_some()) == Some(true) => {
                unsafe {
                    if GetCapture() == hwnd {
                        ReleaseCapture();
                    }
                    InvalidateRect(hwnd, null(), 0);
                }
                return 0;
            }
            (ADDRESS, VK_ESCAPE) => {
                refresh(true);
                return 0;
            }
            (FILTER, VK_ESCAPE) => {
                filter_input(VK_ESCAPE);
                return 0;
            }
            (FILTER, key @ (VK_UP | VK_DOWN)) if !ctrl && !shift && !alt => {
                filter_input(key);
                return 0;
            }
            (FIND_TEXT, VK_ESCAPE) => {
                close_find(true);
                return 0;
            }
            (_, VK_ESCAPE) if !ctrl && !shift && !alt => {
                if let Some((key, true)) =
                    with_shell(|s| Some((s.active?, s.reload_is_stop))).flatten()
                {
                    unsafe {
                        PostMessageW(parent as HWND, STOP_LOADING, key, 0);
                    }
                    return 0;
                }
            }
            (TABS, VK_DELETE) => {
                let index = unsafe { SendMessageW(hwnd, LB_GETCURSEL, 0, 0) };
                if index >= 0 {
                    close_tab(row_key(index as usize));
                }
                return 0;
            }
            (TABS, VK_RETURN) if !ctrl && !shift && !alt => {
                focus_page();
                return 0;
            }
            (BOOKMARK_LIST | AGENT_LIST, VK_RETURN) if !ctrl && !shift && !alt => {
                let index = unsafe { SendMessageW(hwnd, LB_GETCURSEL, 0, 0) };
                if index >= 0 {
                    activate_section_row(id, index as usize, false);
                }
                return 0;
            }
            (_, VK_TAB) if !ctrl && !alt => {
                if let Some(ui) = controls() {
                    let chain: Vec<_> = ui.tab_stops().collect();
                    if let Some(i) = chain.iter().position(|item| *item == hwnd) {
                        let next = if shift { i.checked_sub(1) } else { Some(i + 1) };
                        if let Some(item) = next.and_then(|j| chain.get(j)) {
                            focus_control(*item, false);
                        } else {
                            focus_page();
                        }
                        return 0;
                    }
                }
            }
            _ => {}
        }
    }
    // TranslateMessage can queue the corresponding character before the keydown
    // handler posts the command. Never submit it a second time in the old tab.
    if id == ADDRESS
        && wp == VK_RETURN as usize
        && (message == WM_SYSCHAR || (message == WM_CHAR && lp & ALT_CONTEXT != 0))
    {
        return 0;
    }
    if message == WM_CHAR && wp == VK_RETURN as usize {
        match id {
            ADDRESS => unsafe {
                PostMessageW(parent as HWND, ADDRESS_SUBMIT, 0, 0);
            },
            FILTER => filter_input(VK_RETURN),
            FIND_TEXT => find_in_page(true, true),
            TABS => (),
            _ => return unsafe { DefSubclassProc(hwnd, message, wp, lp) },
        }
        return 0;
    }
    if matches!(id, BOOKMARK_LIST | AGENT_LIST) && message == WM_LBUTTONUP {
        let hit = unsafe { SendMessageW(hwnd, LB_ITEMFROMPOINT, 0, lp) } as usize;
        if hit >> 16 == 0 {
            let mut client = RECT::default();
            unsafe { GetClientRect(hwnd, &mut client) };
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
            let x = (lp & 0xffff) as i16 as i32;
            let close = x >= client.right - chrome::TAB_CLOSE_WIDTH * dpi / 96;
            let value = unsafe { DefSubclassProc(hwnd, message, wp, lp) };
            activate_section_row(id, hit & 0xffff, close);
            return value;
        }
    }
    if id == TABS {
        match message {
            WM_SETFOCUS if with_shell(|s| s.hovered_row.is_none()) == Some(true) => {
                let index = unsafe { SendMessageW(hwnd, LB_GETCURSEL, 0, 0) };
                if index >= 0
                    && let Some(key) = row_key(index as usize)
                {
                    show_tab_memory(key);
                }
            }
            WM_KILLFOCUS => {
                clear_tab_memory();
            }
            WM_CONTEXTMENU => {
                show_tab_menu(hwnd, parent as HWND, lp);
                return 0;
            }
            WM_LBUTTONDOWN | WM_MBUTTONDOWN => {
                if message == WM_LBUTTONDOWN
                    && !exiting()
                    && let Some(key) = tab_unload_point(hwnd, lp)
                {
                    change(|s| s.row_unload = Some(key));
                    unsafe { SetCapture(hwnd) };
                    return 0;
                }
                if let Some(key) = tab_point(hwnd, lp, message == WM_LBUTTONDOWN) {
                    change(|s| s.row_close = Some((key, message)));
                    unsafe {
                        SetCapture(hwnd);
                    }
                    return 0;
                }
                if message == WM_LBUTTONDOWN
                    && !exiting()
                    && with_shell(|s| !s.closing) == Some(true)
                    && let Some(key) = tab_point(hwnd, lp, false)
                {
                    unsafe { SetFocus(hwnd) };
                    change(|s| {
                        s.tab_drag = Some(TabDrag {
                            key,
                            start_y: ((lp >> 16) as u16 as i16) as i32,
                            dragging: false,
                            target: None,
                        });
                    });
                    unsafe { SetCapture(hwnd) };
                    return 0;
                }
            }
            WM_MOUSEMOVE => {
                if update_tab_drag(hwnd, lp).is_some() {
                    return 0;
                }
            }
            // Real hover goes through refresh_hover; these remain for scripted callers.
            WM_MOUSEHOVER => {
                if let Some(key) = tab_point(hwnd, lp, false) {
                    show_tab_memory(key);
                }
                return 0;
            }
            WM_MOUSELEAVE | WM_MOUSEWHEEL | WM_VSCROLL => {
                clear_tab_memory();
            }
            WM_LBUTTONUP | WM_MBUTTONUP => {
                if message == WM_LBUTTONUP
                    && let Some(key) = change(|s| s.row_unload.take()).flatten()
                {
                    unsafe { ReleaseCapture() };
                    if tab_unload_point(hwnd, lp) == Some(key) {
                        unsafe {
                            PostMessageW(parent as HWND, TAB_MENU_ACTION, key, TAB_UNLOAD as LPARAM)
                        };
                    }
                    return 0;
                }
                if let Some((key, button)) = change(|s| s.row_close.take()).flatten() {
                    unsafe {
                        ReleaseCapture();
                    }
                    if message == button + 1
                        && tab_point(hwnd, lp, button == WM_LBUTTONDOWN) == Some(key)
                    {
                        unsafe {
                            PostMessageW(parent as HWND, CLOSE_BY_KEY, key, 0);
                        }
                    }
                    return 0;
                }
                if message == WM_LBUTTONUP
                    && let Some(drag) = update_tab_drag(hwnd, lp)
                {
                    change(|s| s.tab_drag = None);
                    unsafe {
                        if GetCapture() == hwnd {
                            ReleaseCapture();
                        }
                        InvalidateRect(hwnd, null(), 0);
                    }
                    if !exiting() {
                        if let Some(drop) = drag.target {
                            move_tab(drag.key, drop);
                        } else if !drag.dragging
                            && tab_point(hwnd, lp, false) == Some(drag.key)
                            && with_shell(|s| !s.closing) == Some(true)
                        {
                            if with_shell(|s| s.active != Some(drag.key)) == Some(true) {
                                select_key(drag.key);
                            }
                            focus_page();
                        }
                    }
                    return 0;
                }
            }
            WM_CAPTURECHANGED | WM_CANCELMODE => {
                // Keep a pending close press: the release over the same X still closes it.
                if change(|s| {
                    s.row_unload = None;
                    s.tab_drag.take().is_some()
                }) == Some(true)
                {
                    unsafe { InvalidateRect(hwnd, null(), 0) };
                }
            }
            _ => {}
        }
    }
    let paint = matches!(message, WM_SETFOCUS | WM_KILLFOCUS);
    let value = unsafe { DefSubclassProc(hwnd, message, wp, lp) };
    if paint && matches!(id, ADDRESS | FILTER | FIND_TEXT) {
        unsafe {
            InvalidateRect(parent as HWND, null(), 0);
        }
    }
    value
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    let _scope = WindowScope::enter(window as usize);
    if chrome::glass() {
        // DWM draws and hit-tests the min/max/close buttons, including Snap Layouts.
        let mut handled: LRESULT = 0;
        if unsafe {
            windows_sys::Win32::Graphics::Dwm::DwmDefWindowProc(window, message, wp, lp, &mut handled)
        } != 0
        {
            return handled;
        }
    }
    match message {
        WM_NCCALCSIZE if wp != 0 && lp != 0 && chrome::glass() => {
            // Keep the side and bottom resize frame, but give the caption to the toolbar.
            let params = unsafe { &mut *(lp as *mut NCCALCSIZE_PARAMS) };
            let top = params.rgrc[0].top;
            unsafe { DefWindowProcW(window, message, wp, lp) };
            // A maximized window hangs its frame off screen on every side.
            let border = if unsafe { IsZoomed(window) } != 0 {
                let dpi = unsafe { GetDpiForWindow(window) };
                unsafe {
                    GetSystemMetricsForDpi(SM_CYFRAME, dpi)
                        + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
                }
            } else {
                0
            };
            params.rgrc[0].top = top + border;
            0
        }
        WM_NCHITTEST if chrome::glass() => {
            let hit = unsafe { DefWindowProcW(window, message, wp, lp) };
            if hit != HTCLIENT as isize || with_shell(|s| s.fullscreen.is_some()) == Some(true) {
                return hit;
            }
            let screen = POINT {
                x: (lp & 0xffff) as i16 as i32,
                y: ((lp >> 16) & 0xffff) as i16 as i32,
            };
            let mut point = screen;
            unsafe { ScreenToClient(window, &mut point) };
            let dpi = unsafe { GetDpiForWindow(window) }.max(96);
            let resize = unsafe {
                GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
            };
            let zoomed = unsafe { IsZoomed(window) } != 0;
            let over_toolbar = point.y < 80 * dpi as i32 / 96 && point.y < current_layout(window).toolbar_bottom;
            if over_toolbar
                && controls().is_some_and(|ui| {
                    material_target(screen, ui.material_toolbar().into_iter().chain([ui.add])).is_some()
                })
            {
                return HTCLIENT as isize;
            }
            if point.y < resize && !zoomed {
                return HTTOP as isize;
            }
            if zoomed {
                let (x, y) = ((lp & 0xffff) as i16 as i32, ((lp >> 16) & 0xffff) as i16 as i32);
                if let Some(button) = chrome::caption_button_at(window, x, y) {
                    return button as isize;
                }
            }
            // Child controls hit-test themselves; the bare toolbar strip moves the window.
            if over_toolbar {
                return HTCAPTION as isize;
            }
            hit
        }
        // Our own caption-button hits (maximized only, see WM_NCHITTEST): act on release,
        // and keep DefWindowProc from drawing classic buttons on press.
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK
            if matches!(wp as u32, HTMINBUTTON | HTMAXBUTTON | HTCLOSE) =>
        {
            0
        }
        WM_NCLBUTTONUP if matches!(wp as u32, HTMINBUTTON | HTMAXBUTTON | HTCLOSE) => {
            let command = match wp as u32 {
                HTMINBUTTON => SC_MINIMIZE,
                HTMAXBUTTON if unsafe { IsZoomed(window) } != 0 => SC_RESTORE,
                HTMAXBUTTON => SC_MAXIMIZE,
                _ => SC_CLOSE,
            };
            unsafe { PostMessageW(window, WM_SYSCOMMAND, command as usize, 0) };
            0
        }
        // Wheel over the sidebar's see-through background lands here.
        WM_MOUSEWHEEL if over_sidebar(window, lp) => {
            scroll_sidebar(((wp >> 16) & 0xffff) as i16 as i32);
            0
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_MBUTTONDOWN | WM_MBUTTONUP
        | WM_RBUTTONDOWN | WM_RBUTTONUP
            if let Some((child, point)) = material_control_at() =>
        {
            // Color-keyed pixels are click-through, so the click lands here;
            // hand it to the control that is drawn at that spot.
            unsafe { SendMessageW(child, message, wp, point) }
        }
        WM_ACTIVATE => {
            if wp & 0xffff == WA_INACTIVE as usize {
                dismiss_address_suggestions(window);
                clear_tab_memory();
            } else if with_shell(|s| !s.private && !s.closing) == Some(true) {
                // A CEF child can activate its owner while another app stays in front.
                if unsafe { GetForegroundWindow() } == window {
                    USER_ACTIVATED.set(true);
                }
                if LAST_NORMAL.replace(window as usize) != window as usize {
                    queue_session_save();
                }
                if (wp >> 16) & 0xffff == 0 && with_shell(|s| s.deferred_restore) == Some(true) {
                    unsafe { PostMessageW(window, LOAD_RESTORED_WINDOW, 0, 0) };
                }
            }
            unsafe { DefWindowProcW(window, message, wp, lp) }
        }
        LOAD_RESTORED_WINDOW => {
            load_waiting_window(window);
            0
        }
        VAULT_OFFER => {
            offer_vault_login(window);
            0
        }
        MOUNT_START => {
            mount_start(wp);
            0
        }
        AUDIBLE_CHANGED => {
            set_audible(window as usize, wp, lp != 0);
            0
        }
        WM_KEYDOWN if wp == VK_ESCAPE as usize && crate::popover::escape(window) => 0,
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let (ctrl, shift, alt) = modifiers();
            if let Some(action) = keyboard::shortcut(wp as u16, ctrl, shift, alt) {
                unsafe { PostMessageW(window, WM_COMMAND, shortcut_command(action), 0) };
                0
            } else {
                unsafe { DefWindowProcW(window, message, wp, lp) }
            }
        }
        WM_MOVE => {
            dismiss_address_suggestions(window);
            crate::popover::follow(window);
            queue_session_save();
            0
        }
        WM_SIZE => {
            dismiss_address_suggestions(window);
            crate::popover::follow(window);
            let minimized = unsafe { IsIconic(window) } != 0;
            if !minimized {
                layout();
            }
            sync_minimized_pages(minimized);
            queue_session_save();
            0
        }
        WM_GETMINMAXINFO if lp != 0 => {
            let dpi = unsafe { GetDpiForWindow(window) }.max(96) as i32;
            unsafe {
                let info = &mut *(lp as *mut MINMAXINFO);
                info.ptMinTrackSize.x = MIN_WINDOW_WIDTH * dpi / 96;
                info.ptMinTrackSize.y = MIN_WINDOW_HEIGHT * dpi / 96;
            }
            0
        }
        WM_DPICHANGED if lp != 0 => {
            let r = unsafe { *(lp as *const RECT) };
            chrome::apply_backdrop(window, 8 * (wp as u16 as i32).max(96) / 96);
            set_bounds(window, r.left, r.top, r.right - r.left, r.bottom - r.top);
            layout();
            0
        }
        WM_PAINT if controls().is_some() => {
            chrome::paint_shell(window, current_layout(window));
            0
        }
        WM_TIMER if wp == HOVER_TIMER => {
            refresh_hover();
            0
        }
        WM_TIMER if wp == ADDRESS_TIMER => {
            unsafe { KillTimer(window, ADDRESS_TIMER) };
            refresh_address_suggestions(window);
            0
        }
        WM_TIMER if wp == DOWNLOAD_MENU_TIMER => {
            unsafe { KillTimer(window, DOWNLOAD_MENU_TIMER) };
            if DOWNLOAD_TIMER_OWNER.get() == window {
                DOWNLOAD_TIMER_OWNER.set(null_mut());
                if crate::popover::native_owner("downloads") == Some(window) {
                    downloads_progress();
                }
            }
            0
        }
        // Children pass WM_SETCURSOR up on every mouse move, so this sees them all.
        WM_SETCURSOR => {
            refresh_hover();
            unsafe { DefWindowProcW(window, message, wp, lp) }
        }
        WM_TIMER if wp == SIDEBAR_TIMER => {
            tick_sidebar(window);
            0
        }
        WM_TIMER if wp == MEMORY_SAVER_TIMER => {
            if !exiting() {
                memory_saver_tick();
            }
            0
        }
        WM_TIMER if wp == ZOOM_LABEL_TIMER => {
            unsafe { KillTimer(window, ZOOM_LABEL_TIMER) };
            if crate::popover::native_owner("menu") == Some(window) {
                crate::popover::update_native(window, "menu", &menu_surface());
            }
            0
        }
        WM_TIMER if wp == TAB_MEMORY_TIMER => {
            expire_tab_memory(window);
            0
        }
        WM_TIMER if wp == RECOVERY_TIMER => {
            unsafe {
                KillTimer(window, RECOVERY_TIMER);
            }
            if RECOVERY_WINDOW.get() == window as usize {
                RECOVERY_WINDOW.set(0);
                if with_shell(|s| !s.private && !s.closing) == Some(true) {
                    save_session();
                }
            }
            0
        }
        WM_SYSCOLORCHANGE | WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DWMCOMPOSITIONCHANGED
            if controls().is_some() =>
        {
            chrome::refresh_system_colors();
            let dpi = unsafe { GetDpiForWindow(window) }.max(96) as i32;
            chrome::apply_backdrop(window, 8 * dpi / 96);
            layout(); // the color-keyed sidebar key follows the palette
            refresh_exact_settings();
            update_start();
            // Layered sidebar controls are skipped by RDW_ALLCHILDREN; repaint each one.
            if let Some(ui) = controls() {
                for hwnd in ui.chrome().into_iter().chain(ui.find()).chain([ui.tab_status, ui.empty]) {
                    unsafe { InvalidateRect(hwnd, null(), 1) };
                }
            }
            unsafe {
                RedrawWindow(window, null(), null_mut(), RDW_INVALIDATE
                    | windows_sys::Win32::Graphics::Gdi::RDW_ERASE
                    | windows_sys::Win32::Graphics::Gdi::RDW_FRAME
                    | RDW_ALLCHILDREN);
            }
            unsafe { DefWindowProcW(window, message, wp, lp) }
        }
        WM_DRAWITEM if lp != 0 => {
            let item = unsafe { &*(lp as *const DRAWITEMSTRUCT) };
            let dpi = unsafe { GetDpiForWindow(window) }.max(96) as i32;
            if wp == SPLIT_DIVIDER {
                chrome::draw_splitter(item, dpi);
                return 1;
            }
            if wp == TABS {
                let Some((row, active, unloadable)) = with_shell(|s| {
                    let row = s.rows.get(item.itemID as usize);
                    let unloadable = row.is_some_and(|row| {
                        s.tabs
                            .iter()
                            .find(|tab| tab.key == row.key)
                            .is_some_and(|tab| can_unload(s, tab))
                    });
                    (row.cloned(), s.active, unloadable)
                }) else {
                    return 0;
                };
                let (label, icon, pinned, muted, unloaded) =
                    row.as_ref().map_or(("", 0, false, false, false), |row| {
                        (row.label.as_str(), row.icon, row.pinned, row.muted, row.unloaded)
                    });
                chrome::draw_tab(
                    item,
                    label,
                    icon,
                    pinned,
                    muted,
                    unloaded,
                    unloadable,
                    row.as_ref().is_some_and(|row| Some(row.key) == active),
                    dpi,
                );
                if let Some((row, drop)) = row
                    .as_ref()
                    .zip(with_shell(|s| s.tab_drag.and_then(|drag| drag.target)).flatten())
                    && row.key == drop.key
                {
                    let height = (2 * dpi / 96).max(1);
                    let y = if drop.after {
                        item.rcItem.bottom - height
                    } else {
                        item.rcItem.top
                    };
                    let line = RECT {
                        top: y,
                        bottom: y + height,
                        ..item.rcItem
                    };
                    unsafe { FillRect(item.hDC, &line, GetSysColorBrush(COLOR_HIGHLIGHT)) };
                }
                return 1;
            }
            if wp == BOOKMARK_LIST {
                if let Some((title, page)) = with_shell(|s| {
                    let row = s.bookmark_rows.get(item.itemID as usize)?;
                    Some((row.title.clone(), s.bookmark_tab(&row.url).map(|tab| {
                        let icon = tab.favicon.as_ref().map_or(0, |icon| icon.handle());
                        (icon, tab.muted, tab.unloaded, Some(tab.key) == s.active)
                    })))
                }).flatten() {
                    if let Some((icon, muted, unloaded, active)) = page {
                        chrome::draw_tab(item, &title, icon, false, muted, unloaded, false, active, dpi);
                    } else {
                        chrome::draw_bookmark(item, &title, dpi);
                    }
                }
                return 1;
            }
            if wp == AGENT_LIST {
                if let Some((row, active)) = with_shell(|s| Some((s.agent_rows.get(item.itemID as usize)?.clone(), s.active))).flatten() {
                    chrome::draw_tab(item, &row.label, row.icon, false, row.muted, row.unloaded, false, Some(row.key) == active, dpi);
                }
                return 1;
            }
            let kind = match wp {
                BOOKMARK_HEADER => ButtonKind::Section(with_shell(|s| s.bookmarks_open) == Some(true)),
                AGENT_HEADER => ButtonKind::Section(with_shell(|s| s.agent_open) == Some(true)),
                SIDEBAR => ButtonKind::Sidebar,
                BACK => ButtonKind::Back,
                FORWARD => ButtonKind::Forward,
                RELOAD if with_shell(|s| s.reload_is_stop) == Some(true) => ButtonKind::Stop,
                RELOAD => ButtonKind::Reload,
                GO => ButtonKind::Go,
                FIND_PREVIOUS => ButtonKind::FindPrevious,
                FIND_NEXT => ButtonKind::FindNext,
                FIND_CLOSE => ButtonKind::FindClose,
                SAVE_BOOKMARK => ButtonKind::Bookmark,
                SITE_INFO => ButtonKind::SiteInfo,
                DOWNLOADS_BUTTON => {
                    ButtonKind::Downloads(with_shell(|s| s.active_downloads != 0) == Some(true))
                }
                MENU => ButtonKind::Menu,
                EXTENSIONS_BUTTON => ButtonKind::Extensions,
                VAULT_BUTTON => ButtonKind::Vault,
                FREE_MEMORY_BUTTON => ButtonKind::FreeMemory,
                NEW_TAB
                    if with_shell(|s| !s.sidebar_open && !s.sidebar_motion.running())
                        == Some(true) =>
                {
                    ButtonKind::NewTabCompact
                }
                NEW_TAB => ButtonKind::NewTab,
                _ => return unsafe { DefWindowProcW(window, message, wp, lp) },
            };
            chrome::draw_button(item, kind, dpi);
            1
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC => {
            chrome::field_colors(wp as HDC, unsafe { GetDlgCtrlID(lp as HWND) } as usize) as isize
        }
        exact_model::ACTION_MESSAGE => {
            let view = lp as HWND;
            if !private()
                && active_exact_settings() == Some(view)
                && (PAGE_COMMANDS.contains(&wp)
                    || (SEARCH_ENGINE_FIRST..=SEARCH_ENGINE_LAST).contains(&wp)
                    || (MEMORY_SAVER_FIRST..MEMORY_SAVER_FIRST + MEMORY_SAVER_CHOICES.len()).contains(&wp)
                    || (THEME_FIRST..=THEME_LAST).contains(&wp))
            {
                unsafe { SendMessageW(window, WM_COMMAND, wp, 0) };
                refresh_exact_settings();
            } else if wp & POPUP_NAMESPACE == START_LINK {
                activate_start_link(window, view, wp);
            } else if let Some(kind) = crate::popover::native_kind(window, view) {
                native_popover_action(window, kind, wp);
            }
            0
        }
        ADDRESS_SUBMIT => {
            if !exiting() && with_shell(|s| !s.closing) == Some(true)
                && !accept_address_suggestion(window, None)
            {
                navigate(false);
            }
            0
        }
        SPLIT_FOCUS => {
            if !exiting() {
                focus_split_page(wp);
            }
            0
        }
        WM_COMMAND => {
            if exiting() {
                return 0;
            }
            let id = wp & 0xffff;
            let notification = ((wp >> 16) & 0xffff) as u32;
            match id {
                GO | GO_NEW_TAB => navigate(id == GO_NEW_TAB),
                NEW_TAB => {
                    reveal_toolbar(window);
                    if let Some(ui) = controls() {
                        if !text(ui.filter, 256).is_empty() {
                            set_text(ui.filter, "");
                        }
                    }
                    if let Err(message) = add_tab(HOME) {
                        set_text(window, &format!("{} — {}", app_title(), i18n::text(&message)));
                    } else if let Some(ui) = controls() {
                        focus_control(ui.address, true);
                    }
                }
                NEW_WINDOW | PRIVATE_WINDOW => {
                    if let Err(message) = open_window(id == PRIVATE_WINDOW, None, None) {
                        set_text(window, &format!("{} — {}", app_title(), i18n::text(&message)));
                    }
                }
                BACK => {
                    cancel_active_permission(window);
                    close_find(false);
                    refresh(true);
                    if let Some(b) = active() {
                        b.go_back();
                    }
                }
                FORWARD => {
                    cancel_active_permission(window);
                    close_find(false);
                    refresh(true);
                    if let Some(b) = active() {
                        b.go_forward();
                    }
                }
                RELOAD if with_shell(|s| s.reload_is_stop) == Some(true) => {
                    if let Some(b) = active() {
                        b.stop_load();
                    }
                }
                RELOAD | RELOAD_SHORTCUT | HARD_RELOAD => {
                    cancel_active_permission(window);
                    refresh(true);
                    if let Some(b) = active() {
                        if id == HARD_RELOAD {
                            b.reload_ignore_cache();
                        } else {
                            b.reload();
                        }
                    }
                }
                CLOSE_TAB => close_tab(None),
                MEMORY_SAVER_FIRST..=204 if !private() => {
                    MEMORY_SAVER_MINUTES.set(MEMORY_SAVER_CHOICES[id - MEMORY_SAVER_FIRST]);
                    save_session();
                }
                BOOKMARK_HEADER => toggle_section(false),
                AGENT_HEADER => toggle_section(true),
                TOGGLE_AGENT_AUTO_CLOSE if !private() => {
                    let on = !crate::agent_host::AUTO_CLOSE.load(std::sync::atomic::Ordering::Relaxed);
                    crate::agent_host::AUTO_CLOSE.store(on, std::sync::atomic::Ordering::Relaxed);
                    save_session();
                }
                RESTORE_TAB => reopen_closed(None),
                REOPEN_RECENT if lp != 0 => reopen_closed(Some(lp as usize)),
                FREE_MEMORY | FREE_MEMORY_BUTTON => free_background_memory(window),
                TOGGLE_DEFER_BACKGROUND if !private() => {
                    DEFER_BACKGROUND.set(!DEFER_BACKGROUND.get());
                    queue_profile_save();
                }
                SIDEBAR => {
                    reveal_toolbar(window);
                    toggle_sidebar();
                }
                FOCUS_ADDRESS | FOCUS_SEARCH => {
                    reveal_toolbar(window);
                    if let Some(ui) = controls() {
                        if id == FOCUS_SEARCH {
                            // Treat leading '?' as visible search mode, even for a dotted term.
                            set_text(ui.address, "? ");
                            focus_control(ui.address, false);
                            unsafe { SendMessageW(ui.address, EM_SETSEL, 2, 2) };
                        } else {
                            if text(ui.address, 16_384).trim_start().starts_with('?') {
                                refresh(true);
                            }
                            focus_control(ui.address, true);
                        }
                    }
                }
                SEARCH_ENGINE_FIRST..=SEARCH_ENGINE_LAST if !private() => {
                    let (engine, _) = SearchEngine::ALL[id - SEARCH_ENGINE_FIRST];
                    if SEARCH_ENGINE.replace(engine) != engine {
                        save_session();
                    }
                }
                FOCUS_FILTER => {
                    reveal_toolbar(window);
                    if with_shell(|s| !s.sidebar_open || s.sidebar_motion.running()) == Some(true) {
                        set_sidebar(true, false);
                    }
                    if let Some(ui) = controls() {
                        focus_control(ui.filter, true);
                        set_filter_cursor(FilterCursor::First);
                    }
                }
                NEXT_TAB | PREVIOUS_TAB => cycle_tab(id == PREVIOUS_TAB),
                SELECT_TAB_FIRST..=SELECT_TAB_LAST => {
                    let key = with_shell(|s| {
                        let mut ordinary = s.tabs.iter().filter(|tab| {
                            tab.agent.is_none() && tab.bookmark_url.is_none()
                        });
                        let tab = if id == SELECT_TAB_LAST {
                            ordinary.next_back()
                        } else {
                            ordinary.nth(id - SELECT_TAB_FIRST)
                        };
                        tab.map(|tab| tab.key)
                    }).flatten();
                    if let Some(key) = key {
                        select_key(key);
                        focus_page();
                    }
                }
                FULLSCREEN => toggle_fullscreen(window),
                SPLIT_VIEW => toggle_split(),
                SPLIT_DIVIDER => resize_split(window, 500),
                ZOOM_IN | ZOOM_OUT | ZOOM_RESET => {
                    if let Some(host) = host() {
                        zoom_page(&host, id);
                    }
                }
                EXTENSIONS_BUTTON if !private() => show_extensions_panel(window),
                VAULT_BUTTON if !private() => show_vault_panel(window),
                MENU => {
                    if let Some(ui) = controls() {
                        show_menu_panel(ui);
                    }
                }
                HISTORY if !private() => open_builtin("chrome://history/"),
                BOOKMARKS if !private() => open_builtin(MANAGER_URL),
                SAVE_BOOKMARK => bookmark_page(),
                BOOKMARK_ALL_TABS if !private() => bookmark_all_tabs(window),
                IMPORT_BOOKMARKS => run_bookmark_request(BookmarkRequest::Import),
                EXPORT_BOOKMARKS => run_bookmark_request(BookmarkRequest::Export),
                SITE_INFO => show_site_info(window),
                DOWNLOADS_BUTTON => {
                    reveal_toolbar(window);
                    show_downloads_panel(window);
                }
                DOWNLOADS => {
                    reveal_toolbar(window);
                    open_builtin(DOWNLOADS_URL);
                }
                SETTINGS if !private() => open_flit_settings(),
                CHROMIUM_SETTINGS if !private() => open_builtin(SETTINGS_URL),
                DISPLAY_LANGUAGE if !private() => open_builtin("chrome://settings/languages"),
                DOWNLOAD_PREFERENCES if !private() => open_builtin("chrome://settings/downloads"),
                SITE_PERMISSIONS if !private() => open_builtin("chrome://settings/content"),
                CLEAR_BROWSING if !private() => open_builtin(CLEAR_DATA_URL),
                TOGGLE_RESTORE if !private() => {
                    RESTORE_TABS.set(!RESTORE_TABS.get());
                    save_session();
                    if !RESTORE_TABS.get() {
                        PROFILE.with(|slot| {
                            if let Some(profile) = slot.borrow().as_deref()
                                && let Err(error) = session::remove_backup(profile)
                            {
                                eprintln!("Flit Browser: Could not remove the previous session copy: {error}");
                            }
                        });
                    }
                }
                RECOVERY_COPY if !private() => restore_recovery_copy(window),
                EXTENSIONS if !private() => open_builtin("chrome://extensions/"),
                PASSWORDS if !private() => open_builtin("chrome://password-manager/"),
                IMPORT_PASSWORDS if !private() => {
                    open_builtin("chrome://password-manager/settings")
                }
                PREPARE_CHROME | PREPARE_EDGE if !private() => {
                    if let Some(ui) = controls() {
                        browser_site_setup(
                            ui,
                            if id == PREPARE_CHROME {
                                SourceBrowser::Chrome
                            } else {
                                SourceBrowser::Edge
                            },
                        );
                    }
                }
                PREPARE_CLIPBOARD if !private() => match copied_site_text(window) {
                    Ok(text) => prepare_site_tabs(
                        window,
                        "Copied site links",
                        site_setup::pasted_sites(&text, &open_site_origins()),
                    ),
                    Err(message) => site_setup_info(window, message),
                },
                OPEN_COPIED_TABS if !private() => open_copied_tabs(window),
                IMPORT_CHROME_BOOKMARKS | IMPORT_EDGE_BOOKMARKS if !private() => {
                    if let Some(ui) = controls() {
                        import_browser_bookmarks(
                            ui,
                            if id == IMPORT_CHROME_BOOKMARKS {
                                SourceBrowser::Chrome
                            } else {
                                SourceBrowser::Edge
                            },
                        );
                    }
                }
                MATERIAL => {
                    let dpi = unsafe { GetDpiForWindow(window) }.max(96) as i32;
                    chrome::toggle_acrylic(window, 8 * dpi / 96);
                    if !private() {
                        save_session();
                    }
                }
                THEME_FIRST..=THEME_LAST if !private() => {
                    chrome::set_theme((id - THEME_FIRST) as u8);
                    save_session();
                    restyle_windows();
                }
                UPDATE => {
                    let message = wide(&crate::updater::check_manual());
                    unsafe {
                        MessageBoxW(
                            window,
                            message.as_ptr(),
                            windows_sys::w!("Flit Browser"),
                            MB_OK | MB_ICONINFORMATION,
                        );
                    }
                }
                FIND => {
                    reveal_toolbar(window);
                    open_find();
                }
                FOCUS_FIND_PREV | FOCUS_FIND_NEXT => {
                    if with_shell(|s| s.find_open) == Some(true) {
                        find_in_page(id == FOCUS_FIND_NEXT, true);
                    } else {
                        reveal_toolbar(window);
                        open_find();
                    }
                }
                FIND_PREVIOUS | FIND_NEXT => find_in_page(id == FIND_NEXT, true),
                FIND_CLOSE => close_find(true),
                PRINT => {
                    if let Some(host) = host() {
                        host.print();
                    }
                }
                DEVTOOLS | DEVTOOLS_CONSOLE => {
                    let command = if id == DEVTOOLS {
                        chrome_command(c"IDC_DEV_TOOLS_TOGGLE")
                    } else {
                        chrome_command(c"IDC_DEV_TOOLS_CONSOLE")
                    };
                    if let Some(host) = host() {
                        if id == DEVTOOLS && host.has_dev_tools() != 0 {
                            host.close_dev_tools();
                        } else if host.can_execute_chrome_command(command) != 0 {
                            host.execute_chrome_command(
                                command,
                                WindowOpenDisposition::CURRENT_TAB,
                            );
                        }
                    }
                }
                CLOSE_WINDOW => close_window(window),
                EXIT_FLIT => exit_flit(window),
                ADDRESS if notification == EN_CHANGE => {
                    if change(|s| {
                        if s.updating_address { return false; }
                        s.address_edited = true;
                        true
                    }) == Some(true)
                        && let Some(ui) = controls()
                    {
                        schedule_address_suggestions(ui);
                    }
                }
                FILTER if notification == EN_CHANGE => {
                    if let Some(ui) = controls() {
                        let value = text(ui.filter, 256);
                        change(|s| {
                            s.filter = value.trim().to_lowercase();
                            if s.filter_cursor != FilterCursor::Off {
                                s.filter_cursor = FilterCursor::First;
                            }
                        });
                        refresh_rows(false);
                    }
                }
                FIND_TEXT if notification == EN_CHANGE => find_in_page(true, false),
                TABS if notification == LBN_SELCHANGE => {
                    if let Some(ui) = controls() {
                        let index = unsafe { SendMessageW(ui.tabs, LB_GETCURSEL, 0, 0) };
                        if index >= 0 {
                            if let Some(key) = row_key(index as usize) {
                                select_key(key);
                                if unsafe { GetFocus() } == ui.tabs {
                                    change(|s| s.hovered_row = None);
                                    show_tab_memory(key);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            0
        }
        CLOSE_BY_KEY => {
            if !exiting() {
                close_tab(Some(wp));
            }
            0
        }
        TAB_MENU_ACTION => {
            if !exiting() {
                tab_action(wp, lp as usize);
            }
            0
        }
        TAB_DOWNLOAD_ENDED => {
            complete_deferred_close(window, wp);
            0
        }
        DOWNLOADS_CHANGED => {
            refresh_downloads_button(window);
            downloads_progress();
            if let Some(ui) = controls() {
                unsafe { InvalidateRect(ui.tabs, null(), 0) };
            }
            0
        }
        TAB_CLOSE_CANCELLED => {
            if lp != 0 {
                change(|s| {
                    if let Some(tab) = s.tabs.iter_mut().find(|tab| tab.key == wp) {
                        tab.unloading = false;
                        tab.close_requested = false;
                    }
                });
                if with_shell(|s| s.closing) == Some(true) {
                    continue_window_close(window);
                } else {
                    release_next_background_tab();
                }
                return 0;
            }
            EXIT_SESSION.with(|slot| {
                let mut exit = slot.borrow_mut();
                if exit
                    .as_ref()
                    .is_some_and(|exit| exit.current == window as usize)
                {
                    *exit = None;
                }
            });
            let native = change(|s| {
                let tab = s.tabs.iter_mut().find(|tab| tab.key == wp)?;
                if std::mem::take(&mut tab.close_requested) {
                    tab.native_window.clone()
                } else {
                    None
                }
            })
            .flatten();
            select_key(wp);
            set_native_page_visible(native.as_ref(), unsafe { IsIconic(window) } == 0);
            focus_page();
            queue_session_save();
            0
        }
        POPUP_CLOSED => {
            if with_shell(|s| s.closing) == Some(true) {
                continue_window_close(window);
            }
            0
        }
        OPEN_SETTINGS_WINDOW => {
            if let Some((key, url)) = change(|s| s.pending_settings.take()).flatten()
                && !exiting()
            {
                let restore = change(|s| {
                    let tab = s.tabs.iter_mut().find(|tab| tab.key == key)?;
                    if settings_target(&tab.url).is_some() && tab.browser.is_some() {
                        tab.url = HOME.into();
                        tab.title.clear();
                    }
                    Some(s.active == Some(key))
                })
                .flatten();
                if restore == Some(true) {
                    refresh(true);
                }
                open_settings_window(&url);
            }
            0
        }
        OPEN_LINK => {
            let link = change(|s| s.links.pop_front()).flatten();
            if let Some(PendingLink {
                url,
                disposition,
                source,
            }) = link
                && !exiting()
                && with_shell(|s| s.closing) == Some(false)
            {
                if disposition == WindowOpenDisposition::NEW_WINDOW {
                    let _ = open_window(private(), Some(&url), None);
                } else {
                    let _ = open_tab(
                        &url,
                        disposition != WindowOpenDisposition::NEW_BACKGROUND_TAB,
                        source,
                    );
                }
            }
            0
        }
        SELECT_POPUP_TAB => {
            if !exiting() && with_shell(|s| !s.closing) == Some(true) {
                select_key(wp);
                focus_page();
            }
            0
        }
        PAGE_CHANGED => {
            if let Some(pending) = change(|s| s.page_changes.take()).flatten() {
                refresh_rows(false);
                if pending.active {
                    refresh_active_page(false);
                }
                if pending.save {
                    queue_session_save();
                }
            }
            0
        }
        LOAD_STATE_CHANGED => {
            if with_shell(|s| s.active == Some(wp)) == Some(true) {
                refresh_load_controls();
            }
            0
        }
        STOP_LOADING => {
            if with_shell(|s| s.active == Some(wp) && !s.closing) == Some(true) {
                if let Some(browser) = active().filter(|b| b.is_loading() != 0) {
                    browser.stop_load();
                }
            }
            0
        }
        PAGE_CLOSED => {
            finish_closed_browser(wp);
            0
        }
        WM_CLOSE => {
            close_window(window);
            0
        }
        EXIT_NEXT_WINDOW => {
            if EXIT_SESSION.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .is_some_and(|exit| exit.current == window as usize)
            }) {
                begin_window_close(window);
            }
            0
        }
        WM_DESTROY => 0,
        WM_NCDESTROY => {
            dismiss_address_suggestions(window);
            if DOWNLOAD_TIMER_OWNER.get() == window {
                DOWNLOAD_TIMER_OWNER.set(null_mut());
                unsafe { KillTimer(window, DOWNLOAD_MENU_TIMER) };
            }
            unsafe {
                KillTimer(window, SIDEBAR_TIMER);
                KillTimer(window, RECOVERY_TIMER);
                KillTimer(window, TAB_MEMORY_TIMER);
            }
            if RECOVERY_WINDOW.get() == window as usize {
                RECOVERY_WINDOW.set(0);
            }
            let (state, empty, any_private) = WINDOWS.with(|windows| {
                let mut windows = windows.borrow_mut();
                let state = windows.remove(&(window as usize));
                (
                    state,
                    windows.is_empty(),
                    windows.values().any(|s| s.private),
                )
            });
            if let Some(state) = state {
                permissions::cancel_window(window as usize);
                download_guard::clear_window(window as usize);
                if !state.font.is_null() {
                    unsafe {
                        DeleteObject(state.font);
                    }
                }
                if state.private && !any_private {
                    PRIVATE_CONTEXT.with(|slot| drop(slot.take()));
                }
                advance_exit(window);
                // The floating panel's page must close before CEF shuts down.
                if empty && !crate::popover::shutdown() {
                    quit_message_loop();
                }
            }
            unsafe { DefWindowProcW(window, message, wp, lp) }
        }
        _ => unsafe { DefWindowProcW(window, message, wp, lp) },
    }
}
