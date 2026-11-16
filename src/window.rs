use crate::{
    bookmarks::{MANAGER_URL, Request as BookmarkRequest},
    chrome::{self, ButtonKind},
    download_guard,
    keyboard::{self, Shortcut},
    memory_sites::{self, SiteExceptions},
    navigation::{self, HOME, SearchEngine},
    panel_motion::Motion,
    permissions,
    session::{self, Entry, Session, WindowPlacement, WindowState},
    site_setup::{self, SourceBrowser},
    vault_ui,
};
use cef::*;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    time::{Duration, Instant},
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
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
            SetThreadDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            EnableWindow, GetActiveWindow, GetCapture, GetFocus, GetKeyState, IsWindowEnabled,
            ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_DELETE, VK_DOWN, VK_ESCAPE,
            VK_MENU, VK_RETURN, VK_SHIFT, VK_TAB, VK_UP,
        },
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const MIN_WINDOW_WIDTH: i32 = 530;
const MIN_WINDOW_HEIGHT: i32 = 340;

const ADDRESS: usize = 101;
const BACK: usize = 102;
const FORWARD: usize = 103;
const RELOAD: usize = 104;
const NEW_TAB: usize = 105;
const CLOSE_TAB: usize = 106;
const GO: usize = 107;
const TABS: usize = 108;
const SIDEBAR: usize = 109;
const FILTER: usize = 110;
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
const FIND_TEXT: usize = 125;
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
const VAULT_SAVE: usize = 170;
const VAULT_REMOVE: usize = 171;
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
const RECENT_MENU_FIRST: usize = 0x400;
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
const TAB_PIN: usize = 1;
const TAB_DUPLICATE: usize = 2;
const TAB_MUTE: usize = 3;
const TAB_CLOSE: usize = 4;
const TAB_UNLOAD: usize = 5;
const TAB_KEEP_SITE: usize = 6;
const TAB_MOVE_UP: usize = 7;
const TAB_MOVE_DOWN: usize = 8;
const TAB_CANCEL_DEFERRED_CLOSE: usize = 9;
const WINDOW_CLASS: &str = "FlitBrowserWindow";
const APP_TITLE: &str = "Flit Browser";
const PRIVATE_TITLE: &str = "Flit Browser — Private";
const ICON_ID: usize = 32512;
const SIDEBAR_TIMER: usize = 1;
const RECOVERY_TIMER: usize = 2;
const SETTINGS_URL: &str = "chrome://settings/";
const CLEAR_DATA_URL: &str = "chrome://settings/clearBrowserData";

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
    empty: HWND,
    menu: HWND,
    bookmark: HWND,
    find_text: HWND,
    find_status: HWND,
    find_next: HWND,
    find_previous: HWND,
    find_close: HWND,
}

impl Controls {
    fn chrome(self) -> [HWND; 13] {
        [
            self.sidebar,
            self.back,
            self.forward,
            self.reload,
            self.site_info,
            self.address,
            self.go,
            self.bookmark,
            self.menu,
            self.add,
            self.filter,
            self.tabs,
            self.label,
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
}

#[derive(Clone, PartialEq, Eq)]
struct Row {
    key: usize,
    label: String,
    pinned: bool,
    muted: bool,
    unloaded: bool,
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

struct ClosedRecord {
    id: usize,
    item: ClosedItem,
}

struct PendingLink {
    url: String,
    disposition: WindowOpenDisposition,
    source: Option<usize>,
}

struct Shell {
    controls: Controls,
    id: usize,
    tabs: Vec<Tab>,
    active: Option<usize>,
    next_key: usize,
    sidebar_open: bool,
    sidebar_motion: Motion,
    tooltip: HWND,
    reload_is_stop: bool,
    filter: String,
    filter_cursor: FilterCursor,
    rows: Vec<Row>,
    page_counts: (usize, usize),
    deferred_restore: bool,
    font: HFONT,
    font_dpi: i32,
    find_open: bool,
    fullscreen: Option<(WINDOWPLACEMENT, Option<WindowPlacement>)>,
    recently_closed: Vec<ClosedRecord>,
    private: bool,
    address_edited: bool,
    updating_address: bool,
    row_close: Option<(usize, u32)>,
    tab_drag: Option<TabDrag>,
    links: VecDeque<PendingLink>,
    popups: Vec<Browser>,
    pending_popups: BTreeSet<(usize, i32)>,
    settings_window: Option<Browser>,
    pending_settings: Option<(usize, String)>,
    closing: bool,
    closing_session: Option<WindowState>,
    memory_queue: VecDeque<usize>,
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
    static DEFER_BACKGROUND: Cell<bool> = const { Cell::new(false) };
    static SEARCH_ENGINE: Cell<SearchEngine> = const { Cell::new(SearchEngine::DuckDuckGo) };
    static MEMORY_SITES: RefCell<SiteExceptions> = RefCell::new(SiteExceptions::default());
    static RECOVERY_WINDOW: Cell<usize> = const { Cell::new(0) };
    static OPENING_SESSION: Cell<bool> = const { Cell::new(false) };
    static EXIT_SESSION: RefCell<Option<ExitSession>> = const { RefCell::new(None) };
    static EXIT_CODE: Cell<i32> = const { Cell::new(0) };
}

struct ExitSession {
    current: usize,
    remaining: VecDeque<usize>,
}

fn exiting() -> bool {
    EXIT_SESSION.with(|slot| slot.borrow().is_some())
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

fn app_title() -> &'static str {
    if private() { PRIVATE_TITLE } else { APP_TITLE }
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

fn update_page(key: usize, f: impl FnOnce(&mut Tab)) {
    let window = change(|state| {
        let index = state.tabs.iter().position(|tab| tab.key == key)?;
        f(&mut state.tabs[index]);
        Some(state.controls.window)
    })
    .flatten();
    if let Some(window) = window {
        unsafe {
            PostMessageW(window, PAGE_CHANGED, key, 0);
        }
    }
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
            Some(download_guard::handler(self.window, 0))
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
    struct FlitClient { key: usize, window: usize }
    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(FlitLife::new(self.window, self.key))
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
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(FlitRequest::new(self.window, self.key))
        }
        fn find_handler(&self) -> Option<FindHandler> {
            Some(FlitFind::new(self.window, self.key))
        }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> {
            Some(FlitUnload::new(self.window, self.key))
        }
        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(download_guard::handler(self.window, self.key))
        }
        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(permissions::handler(self.window, self.key))
        }
    }
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
            let message = if is_reload != 0 {
                windows_sys::w!("Reload this page? Unsaved changes may be lost.")
            } else {
                windows_sys::w!("Leave this page? Unsaved changes may be lost.")
            };
            let releasing = with_shell(|s| s.tabs.iter().any(|tab| tab.key == self.key && tab.unloading)) == Some(true);
            let proceed = !releasing && unsafe { MessageBoxW(self.window as HWND, message,
                windows_sys::w!("Flit Browser"), MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) } == IDYES;
            let cancelled_close = !proceed && is_reload == 0
                && change(|s| {
                    let tab = s.tabs.iter().any(|tab| tab.key == self.key && tab.close_requested);
                    if !releasing {
                        s.closing_session = None;
                        return std::mem::replace(&mut s.closing, false) || tab;
                    }
                    tab
                }) == Some(true);
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
    struct FlitLife { window: usize, key: usize }
    impl LifeSpanHandler {
        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let _scope = WindowScope::enter(self.window);
            permissions::cancel_tab(self.window, self.key);
            if let Some(browser) = browser {
                download_guard::clear_browser(self.window, browser.identifier());
            }
            let browser = change(|state| {
                state.pending_popups.retain(|(key, _)| *key != self.key);
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
            _window: Option<&mut WindowInfo>, client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>, _extra: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            if gesture == 0 || exiting() {
                return 1;
            }
            let _scope = WindowScope::enter(self.window);
            if with_shell(|s| s.closing) != Some(false) {
                return 1;
            }
            if disposition == WindowOpenDisposition::NEW_POPUP {
                let target = target_url.map(CefString::to_string).unwrap_or_default();
                if let Some(client) = client {
                    if target.is_empty() || target == HOME || crate::launch_url::literal_http_url(&target).is_some() {
                        // Let CEF create the real child browsing context so OAuth
                        // callbacks can reach window.opener in the same profile.
                        change(|s| s.pending_popups.insert((self.key, popup_id)));
                        *client = Some(FlitPopupClient::new(self.window, self.key, popup_id));
                        return 0;
                    }
                }
            } else if matches!(disposition, WindowOpenDisposition::NEW_FOREGROUND_TAB
                | WindowOpenDisposition::NEW_BACKGROUND_TAB | WindowOpenDisposition::NEW_WINDOW) {
                queue_link(self.window, target_url, disposition, Some(self.key));
            }
            1
        }
        fn on_before_popup_aborted(&self, _browser: Option<&mut Browser>, popup_id: i32) {
            let _scope = WindowScope::enter(self.window);
            change(|s| s.pending_popups.remove(&(self.key, popup_id)));
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
            Some(download_guard::handler(self.window, 0))
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
                s.pending_popups.remove(&(self.opener_key, self.popup_id));
                s.popups.push(browser.clone());
            }).is_some();
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
    struct FlitRequest { window: usize, key: usize }
    impl RequestHandler {
        fn on_before_browse(
            &self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>,
            request: Option<&mut Request>, _gesture: i32, _redirect: i32,
        ) -> i32 {
            if frame.is_none_or(|frame| frame.is_main() != 0) {
                permissions::cancel_tab(self.window, self.key);
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
                if with_shell(|s| s.tabs.iter().any(|tab| tab.key == self.key && tab.url != url)) == Some(true) {
                    permissions::cancel_tab(self.window, self.key);
                }
                update_page(self.key, |tab| tab.url = url);
            }
        }
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            let _scope = WindowScope::enter(self.window);
            let title = title.map(CefString::to_string).unwrap_or_default();
            update_page(self.key, |tab| tab.title = title);
        }
        fn on_media_access_change(
            &self, _browser: Option<&mut Browser>, has_video_access: i32, has_audio_access: i32,
        ) {
            let _scope = WindowScope::enter(self.window);
            update_page(self.key, |tab| {
                tab.has_video_access = has_video_access != 0;
                tab.has_audio_access = has_audio_access != 0;
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
        fn on_load_end(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _status: i32) {
            if let Some(frame) = frame {
                let _scope = WindowScope::enter(self.window);
                finish_bookmark(self.key, frame);
            }
        }
    }
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
        caption,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
    )
}

fn edit(parent: HWND, id: usize) -> HWND {
    child(
        parent,
        id,
        "Edit",
        "",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
        0,
    )
}

fn standard_button(parent: HWND, id: usize, title: &str) -> HWND {
    child(
        parent,
        id,
        "Button",
        title,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        0,
    )
}

fn static_label(parent: HWND, id: usize, title: &str) -> HWND {
    child(
        parent,
        id,
        "Static",
        title,
        WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE as u32,
        0,
    )
}

fn tooltips(ui: Controls) -> HWND {
    use windows_sys::w;
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
        (ui.sidebar, w!("Show or hide vertical tabs")),
        (ui.back, w!("Back (Alt+Left)")),
        (ui.forward, w!("Forward (Alt+Right)")),
        (ui.reload, w!("Reload (Ctrl+R)")),
        (ui.go, w!("Open (Enter), or open in a new tab (Alt+Enter)")),
        (ui.add, w!("New tab (Ctrl+T)")),
        (ui.menu, w!("Flit menu")),
        (ui.bookmark, w!("Bookmark this page (Ctrl+D)")),
        (ui.find_previous, w!("Previous match")),
        (ui.find_next, w!("Next match")),
        (ui.find_close, w!("Close find (Esc)")),
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
    DEFER_BACKGROUND.set(saved.as_ref().is_some_and(|state| state.defer_background));
    SEARCH_ENGINE.set(
        saved
            .as_ref()
            .map_or_else(SearchEngine::default, |state| state.search_engine),
    );
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
    match saved.filter(|s| !skip_restore && s.restore) {
        Some(saved) => open_saved_windows(saved, initial_url),
        None => open_window(false, initial_url, None).map(|_| ()),
    }
}

fn open_window(
    private: bool,
    initial_url: Option<&str>,
    saved: Option<WindowState>,
) -> Result<HWND, &'static str> {
    create_window(private, initial_url, saved, SW_SHOW)
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
                "Flit Browser — Some saved windows could not be opened.",
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
    let caption = wide(if private { PRIVATE_TITLE } else { APP_TITLE });
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
        empty: static_label(window, EMPTY, "No matching tabs"),
        menu: button(window, MENU, "Flit menu"),
        bookmark: button(window, SAVE_BOOKMARK, "Bookmark this page"),
        find_text: edit(window, FIND_TEXT),
        find_status: static_label(window, FIND_STATUS, ""),
        find_next: standard_button(window, FIND_NEXT, "↓"),
        find_previous: standard_button(window, FIND_PREVIOUS, "↑"),
        find_close: standard_button(window, FIND_CLOSE, "×"),
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
            0,
        ),
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
        SendMessageW(
            ui.address,
            EM_SETCUEBANNER,
            0,
            windows_sys::w!("Search or enter address") as isize,
        );
        SendMessageW(
            ui.filter,
            EM_SETCUEBANNER,
            1,
            windows_sys::w!("Search tabs…") as isize,
        );
        SendMessageW(
            ui.find_text,
            EM_SETCUEBANNER,
            1,
            windows_sys::w!("Find in page") as isize,
        );
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
                next_key: 1,
                sidebar_open,
                sidebar_motion: Motion::new(sidebar_open),
                tooltip,
                reload_is_stop: false,
                filter: String::new(),
                filter_cursor: FilterCursor::Off,
                rows: Vec::new(),
                page_counts: (0, 0),
                deferred_restore: false,
                font: null_mut(),
                font_dpi: 0,
                find_open: false,
                fullscreen: None,
                recently_closed: Vec::new(),
                private,
                address_edited: false,
                updating_address: false,
                row_close: None,
                tab_drag: None,
                links: VecDeque::new(),
                popups: Vec::new(),
                pending_popups: BTreeSet::new(),
                settings_window: None,
                pending_settings: None,
                closing: false,
                closing_session: None,
                memory_queue: VecDeque::new(),
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
            append_tab(&entry.url, &entry.title, entry.pinned, None)?;
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
        set_text(window, &format!("{APP_TITLE} — {message}"));
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
        let top = d(if find_open {
            108
        } else if fullscreen {
            0
        } else {
            62
        });
        let side_full = (width / 5).clamp(d(184), d(228));
        let side = if fullscreen {
            0
        } else {
            (f64::from(side_full) * sidebar_position).round() as i32
        };
        let side_offset = side - side_full;
        Self {
            width,
            height,
            dpi,
            top,
            side,
            side_full,
            side_offset,
            fullscreen,
            address_frame: RECT {
                left: d(170),
                top: d(10),
                right: (width - d(if sidebar_open { 62 } else { 101 })).max(d(245)),
                bottom: d(50),
            },
            filter_frame: RECT {
                left: side_offset + d(17),
                top: top + d(47),
                right: side_offset + side_full - d(14),
                bottom: top + d(79),
            },
            find_frame: if find_open {
                RECT {
                    left: (width - d(340)).max(d(12)),
                    top: d(65),
                    right: width - d(155),
                    bottom: d(98),
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
            SetWindowPos(
                window,
                null_mut(),
                x,
                y,
                width.max(1),
                height.max(1),
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
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
    let Some((ui, sidebar_open, find_open)) =
        with_shell(|s| (s.controls, s.sidebar_open, s.find_open))
    else {
        return;
    };
    let l = current_layout(ui.window);
    let d = |n| l.scale(n);
    update_fonts(ui, l.dpi);
    for hwnd in [
        ui.sidebar,
        ui.back,
        ui.forward,
        ui.reload,
        ui.site_info,
        ui.address,
        ui.go,
        ui.bookmark,
        ui.menu,
        ui.add,
    ] {
        set_visible(hwnd, !l.fullscreen);
    }
    set_bounds(ui.sidebar, d(18), d(15), d(30), d(30));
    set_bounds(ui.back, d(64), d(15), d(27), d(30));
    set_bounds(ui.forward, d(94), d(15), d(27), d(30));
    set_bounds(ui.reload, d(124), d(15), d(33), d(30));
    let a = l.address_frame;
    set_bounds(ui.site_info, a.left + d(7), a.top + d(5), d(25), d(30));
    set_bounds(
        ui.address,
        a.left + d(35),
        a.top + d(10),
        a.right - a.left - d(100),
        d(21),
    );
    set_bounds(ui.bookmark, a.right - d(61), a.top + d(5), d(26), d(30));
    set_bounds(ui.go, a.right - d(33), a.top + d(5), d(24), d(30));
    set_bounds(ui.menu, l.width - d(47), d(15), d(30), d(30));
    if !sidebar_open && l.side == 0 {
        set_bounds(ui.add, l.width - d(93), d(10), d(34), d(40));
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
            f.left + d(8),
            f.top + d(5),
            f.right - f.left - d(16),
            d(21),
        );
        set_bounds(ui.find_status, l.width - d(147), f.top, d(54), d(29));
        set_bounds(ui.find_previous, l.width - d(91), f.top, d(24), d(29));
        set_bounds(ui.find_next, l.width - d(65), f.top, d(24), d(29));
        set_bounds(ui.find_close, l.width - d(38), f.top, d(24), d(29));
    }
    position_sidebar(ui, &l, false);
    position_pages(&l, false);
}

fn position_sidebar(ui: Controls, l: &Layout, animating: bool) {
    let d = |n| l.scale(n);
    let visible = !l.fullscreen && (l.side > 0 || with_shell(|s| s.sidebar_open) == Some(true));
    for hwnd in [ui.label, ui.filter, ui.tabs, ui.tab_status] {
        set_visible(hwnd, visible);
    }
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
        set_bounds(
            ui.tabs,
            x + d(16),
            l.top + d(88),
            l.side_full - d(29),
            l.height - l.top - d(134),
        );
        set_bounds(
            ui.tab_status,
            x + d(18),
            l.height - d(40),
            l.side_full - d(34),
            d(25),
        );
        set_bounds(
            ui.empty,
            x + d(22),
            l.top + d(88),
            l.side_full - d(41),
            d(34),
        );
    } else {
        set_bounds(ui.add, l.width - d(93), d(10), d(34), d(40));
    }
    if !animating {
        unsafe {
            InvalidateRect(ui.add, null(), 0);
        }
    }
}

fn set_visible(hwnd: HWND, show: bool) {
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_VISIBLE != 0;
        if current != show {
            ShowWindow(hwnd, if show { SW_SHOWNA } else { SW_HIDE });
        }
    }
}

fn position_pages(l: &Layout, active_only: bool) {
    let Some((window, panels)) = with_shell(|s| {
        let panels = s
            .tabs
            .iter()
            .filter(|t| !active_only || Some(t.key) == s.active)
            .map(|t| (t.panel, t.browser.clone()))
            .collect::<Vec<_>>();
        (s.controls.window, panels)
    }) else {
        return;
    };
    let p = l.page_bounds();
    let (width, height) = (p.right - p.left, p.bottom - p.top);
    for (panel, browser) in panels {
        set_bounds(panel, p.left, p.top, width, height);
        if let Some(host) = browser.and_then(|browser| browser.host()) {
            fit_browser(panel, &host);
        }
    }
    unsafe {
        InvalidateRect(window, null(), 0);
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
    change(|s| s.updating_address = true);
    set_text(hwnd, if url == HOME { "" } else { url });
    change(|s| {
        s.updating_address = false;
        s.address_edited = false;
    });
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
        (true, true) => Some(("Cam/Mic on", "Camera + mic in use")),
        (true, false) => Some(("Camera on", "Camera in use")),
        (false, true) => Some(("Mic on", "Microphone in use")),
        (false, false) => None,
    }
}

fn tab_label(tab: &Tab) -> String {
    let title = clean(tab.title.trim(), 240);
    if !title.is_empty() {
        title
    } else if tab.url == HOME || tab.url.is_empty() {
        "New tab".into()
    } else if let Ok(url) = url::Url::parse(&tab.url) {
        url.host_str()
            .map_or_else(|| "Untitled page".into(), str::to_owned)
    } else {
        "Untitled page".into()
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
    let Some((ui, changed, index, matches, total, status)) = change(|s| {
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
                        format!("After download — {label}")
                    } else if let Some((capture, _)) = capture_labels(tab) {
                        format!("{capture} — {label}")
                    } else {
                        label
                    },
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
            s.rows = rows;
            Some(s.rows.clone())
        };
        let status = (std::mem::replace(&mut s.page_counts, counts) != counts).then_some(counts);
        (s.controls, changed, index, matches, s.tabs.len(), status)
    }) else {
        return;
    };
    unsafe {
        let previous = SendMessageW(ui.tabs, LB_GETCURSEL, 0, 0);
        let rebuild = changed.is_some();
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
    } else if total == 1 {
        "1 tab".into()
    } else {
        format!("{total} tabs")
    };
    if text(ui.label, 32) != heading {
        set_text(ui.label, &heading);
    }
    if let Some((live, on_demand)) = status {
        if live == 0 && on_demand == 0 {
            set_text(ui.tab_status, "No pages loaded");
        } else {
            set_text(
                ui.tab_status,
                &format!("{live} live · {on_demand} on demand"),
            );
        }
    }
    update_empty(ui);
}

fn refresh(force_address: bool) {
    refresh_rows(force_address);
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
        set_text(ui.reload, if loading { "Stop loading" } else { "Reload" });
        unsafe {
            InvalidateRect(ui.reload, null(), 0);
            if !tooltip.is_null() {
                let info = TTTOOLINFOW {
                    cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                    uFlags: TTF_IDISHWND,
                    hwnd: ui.window,
                    uId: ui.reload as usize,
                    lpszText: (if loading {
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
    let Some((key, panels, closing)) = change(|s| {
        let tab = s.tabs.get_mut(index)?;
        if tab.unloading || tab.retiring {
            return None;
        }
        let key = tab.key;
        tab.background_warm = false;
        s.deferred_restore = !load_page && tab.browser.is_none() && tab.url != HOME;
        s.active = Some(key);
        Some((
            key,
            s.tabs
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    (
                        t.panel,
                        t.native_window.clone(),
                        i == index,
                        t.close_requested,
                    )
                })
                .collect::<Vec<_>>(),
            s.closing,
        ))
    })
    .flatten() else {
        return;
    };
    for (panel, native, visible, closing) in panels {
        if !visible {
            if let Some(native) = native.as_ref().filter(|native| native.is_closed() == 0) {
                // An ancestor HWND hide alone leaves Chrome's WebContents visible.
                native.hide();
            }
        }
        unsafe {
            ShowWindow(panel, if visible { SW_SHOWNA } else { SW_HIDE });
        }
        if visible && !closing {
            if let Some(native) = native.filter(|native| native.is_closed() == 0) {
                native.show();
            }
        }
    }
    layout();
    refresh(true);
    if !closing && load_page {
        if let Err(message) = load_deferred(key) {
            if let Some(ui) = controls() {
                set_text(ui.window, &format!("{} — {message}", app_title()));
            }
        }
        queue_session_save();
    }
}

fn add_tab(url: &str) -> Result<(), &'static str> {
    open_tab(url, true, None)
}

fn open_tab(url: &str, foreground: bool, source: Option<usize>) -> Result<(), &'static str> {
    let return_tab = if foreground { source } else { None };
    let (key, index) = append_tab(url, "", false, return_tab)?;
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
    if DEFER_BACKGROUND.get() || physical_memory_low() {
        return Ok(());
    }
    let reserved = change(|s| {
        if s.active == Some(key)
            || s.tabs
                .iter()
                .filter(|t| t.background_warm)
                .take(LIMIT)
                .count()
                == LIMIT
        {
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
    let l = current_layout(ui.window);
    let p = l.page_bounds();
    let (width, height) = (p.right - p.left, p.bottom - p.top);
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
    set_bounds(panel, p.left, p.top, width, height);
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
            unloaded: url != HOME,
            bookmark: None,
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
                t.key == key && t.browser.is_none() && !t.unloading && !t.retiring && t.url != HOME
            })
            .map(|t| t.url.clone())
    })
    .flatten();
    if let Some(url) = url {
        if let Some(frame) = materialize(key)?.main_frame() {
            frame.load_url(Some(&CefString::from(url.as_str())));
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
    if let Some(key) = key
        && let Err(message) = load_deferred(key)
    {
        set_text(window, &format!("{} — {message}", app_title()));
    }
}

fn materialize(key: usize) -> Result<Browser, &'static str> {
    let (ui, panel, existing, muted, foreground) = with_shell(|s| {
        let tab = s.tabs.iter().find(|tab| tab.key == key && !tab.retiring)?;
        Some((
            s.controls,
            tab.panel,
            tab.browser.clone(),
            tab.muted,
            s.active == Some(key),
        ))
    })
    .flatten()
    .ok_or("The tab is no longer available.")?;
    if let Some(browser) = existing {
        return Ok(browser);
    }
    let mut context = request_context()?;
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
    let mut client = FlitClient::new(key, ui.window as usize);
    let settings = BrowserSettings {
        // The embedded host has no Chromium toolbar to anchor a zoom bubble.
        chrome_zoom_bubble: State::DISABLED,
        ..Default::default()
    };
    let Some(browser) = browser_host_create_browser_sync(
        Some(&win),
        Some(&mut client),
        Some(&CefString::from(HOME)),
        Some(&settings),
        None,
        context.as_mut(),
    ) else {
        unsafe {
            InvalidateRect(panel, null(), 1);
        }
        return Err("CEF could not create the browser tab.");
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
            if unsafe { SetWindowSubclass(child, Some(page_widget_proc), key, ui.window as usize) }
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
    if !foreground && usable {
        if let Some(native) = &native_window {
            native.hide();
        }
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
    Ok(browser)
}

fn navigate(new_tab: bool) {
    let Some(ui) = controls() else {
        return;
    };
    let raw = text(ui.address, 16_384);
    let url = match navigation::address_bar(&raw, SEARCH_ENGINE.get()) {
        Ok(url) => url,
        Err(message) => {
            let label = wide(message);
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
            Err(message) => set_text(ui.window, &format!("{} — {message}", app_title())),
        }
        return;
    }
    let Some(key) = with_shell(|s| s.active).flatten() else {
        return;
    };
    permissions::cancel_tab(ui.window as usize, key);
    if url == HOME && active().is_none() {
        return;
    }
    let browser = match materialize(key) {
        Ok(browser) => browser,
        Err(message) => {
            set_text(ui.window, &format!("{} — {message}", app_title()));
            return;
        }
    };
    if let Some(frame) = browser.main_frame() {
        close_find(false);
        show_address(ui.address, &url);
        frame.load_url(Some(&CefString::from(url.as_str())));
        if let Some(host) = browser.host() {
            host.set_focus(1);
        }
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
    let Some((window, panel, next, was_active, closing)) = change(|s| {
        let index = s.tabs.iter().position(|tab| tab.key == key)?;
        let tab = s.tabs.remove(index);
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
                && let Some(source) = tab.return_tab
                && let Some(source_index) = s.tabs.iter().position(|t| {
                    t.key == source && !t.unloading && !t.retiring && !t.close_requested
                })
            {
                return source_index;
            }
            index.min(s.tabs.len() - 1)
        });
        Some((s.controls.window, tab.panel, next, was_active, s.closing))
    })
    .flatten() else {
        return;
    };
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
        continue_window_close(window);
    } else if let Some(next) = next {
        if was_active {
            select(next);
        } else {
            refresh_rows(false);
        }
        queue_session_save();
    } else {
        close_window(window);
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
    if message == WM_NCDESTROY {
        unsafe {
            PostMessageW(parent as HWND, PAGE_CLOSED, key, 0);
        }
    }
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
    let message = wide(&format!(
        "Downloads in progress: {count}\nClosing this tab now may interrupt them.\n\nClose this tab after the downloads finish?"
    ));
    let confirmed = unsafe {
        MessageBoxW(
            window,
            message.as_ptr(),
            windows_sys::w!("Download in progress"),
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
    if exiting()
        || with_shell(|s| s.closing) != Some(false)
        || !confirm_window_download_close(window)
    {
        return;
    }
    if !exiting() {
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
    let count = WINDOWS.with(|windows| {
        windows
            .borrow()
            .keys()
            .map(|&window| download_guard::count(window, None))
            .sum::<usize>()
    });
    if count != 0 {
        let message = wide(&format!(
            "Downloads in progress: {count}\nExiting Flit may interrupt them. Exit anyway?"
        ));
        if unsafe {
            MessageBoxW(
                window,
                message.as_ptr(),
                windows_sys::w!("Download in progress"),
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
    // Save once before asking CEF to tear down any window. Mid-exit writes
    // would lose the windows that already closed.
    if let Some(state) = profile_snapshot() {
        store_session(state);
    }
    let timer = RECOVERY_WINDOW.replace(0);
    if timer != 0 {
        unsafe {
            KillTimer(timer as HWND, RECOVERY_TIMER);
        }
    }
    EXIT_SESSION.with(|slot| {
        slot.replace(Some(ExitSession {
            current,
            remaining: order,
        }))
    });
    let _scope = WindowScope::enter(current);
    begin_window_close(current as HWND);
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
    let message = wide(&format!(
        "Downloads in progress: {count}\nClosing this window may interrupt them. Close anyway?"
    ));
    unsafe {
        MessageBoxW(
            window,
            message.as_ptr(),
            windows_sys::w!("Download in progress"),
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
    for tab in &s.tabs {
        let Some(url) = session::stored_url(&tab.url) else {
            continue;
        };
        if settings_target(&url).is_some() {
            continue;
        }
        if Some(tab.key) == s.active {
            selected = entries.len();
        }
        let title = &tab.title[..tab.title.floor_char_boundary(1024)];
        entries.push(Entry {
            url,
            title: title.to_owned(),
            pinned: tab.pinned,
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
        search_engine: SEARCH_ENGINE.get(),
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
            "Flit Browser — No previous session backup is available.",
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
            "Flit Browser — Some windows in the previous session could not be opened.",
        );
    } else if opened == 0 {
        set_text(
            window,
            "Flit Browser — Windows from the previous session are already open.",
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
            let Ok((key, appended)) = append_tab(&entry.url, &entry.title, entry.pinned, None)
            else {
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
        WM_PAINT
            if with_shell(|s| s.tabs.iter().any(|t| t.key == key && t.browser.is_none()))
                == Some(true) =>
        {
            let (font, waiting) =
                with_shell(|s| (s.font, s.deferred_restore && s.active == Some(key)))
                    .unwrap_or((null_mut(), false));
            chrome::paint_start(hwnd, font, waiting);
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
    focus_page();
}

fn reveal_toolbar(window: HWND) {
    if with_shell(|s| s.fullscreen.is_some()) == Some(true) {
        toggle_fullscreen(window);
    }
}

fn set_sidebar(open: bool, animate: bool) {
    let mut client_animation = 1;
    let allowed = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut client_animation as *mut i32).cast(),
            0,
        )
    } != 0
        && client_animation != 0;
    let Some((ui, moving)) = change(|s| {
        s.sidebar_open = open;
        s.sidebar_motion.to(open, animate && allowed);
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
    position_pages(&l, true);
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
    (url.scheme() == "chrome"
        && url.host_str() == Some("settings")
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
                windows_sys::w!("Could not open settings."),
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
            set_text(ui.window, &format!("{} — {message}", app_title()));
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
    Some((origin, connection))
}

fn show_site_info(window: HWND) {
    let Some((origin, connection)) = site_info() else {
        return;
    };
    let is_private = private();
    let question = if is_private {
        ""
    } else {
        "\n\nOpen this site's settings?"
    };
    let content = wide(&format!("{origin}\n\n{connection}{question}"));
    let selected = unsafe {
        MessageBoxW(
            window,
            content.as_ptr(),
            windows_sys::w!("Site information"),
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
        .unwrap_or_else(|| append_tab(MANAGER_URL, "", false, None).map(|(key, _)| key))
    {
        Ok(key) => key,
        Err(message) => {
            if let Some(ui) = controls() {
                set_text(ui.window, &format!("{} — {message}", app_title()));
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
    let title = windows_sys::w!("Bookmark all tabs");
    let count = tabs.len();
    let Some(request) = BookmarkRequest::save_tabs(&tabs) else {
        let body = if count == 0 {
            windows_sys::w!("Open a web page to save its tab.")
        } else {
            windows_sys::w!("This window has too much bookmark data to save at once.")
        };
        unsafe { MessageBoxW(window, body, title, MB_OK | MB_ICONINFORMATION) };
        return;
    };
    let body = wide(&format!(
        "Save {count} web {} to a new folder under Other bookmarks?\n\nTabs will stay open, and tabs waiting to load will stay on demand.",
        if count == 1 { "tab" } else { "tabs" }
    ));
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

fn show_menu(ui: Controls) {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    let add = |id, label: &str, enabled| unsafe {
        AppendMenuW(
            menu,
            MF_STRING | if enabled { 0 } else { MF_GRAYED },
            id,
            wide(label).as_ptr(),
        );
    };
    let separator = || unsafe {
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
    };
    let recent = if private() {
        with_shell(|s| closed_choices(&s.recently_closed)).unwrap_or_default()
    } else {
        RECENT_NORMAL.with(|items| closed_choices(&items.borrow()))
    };
    add(NEW_TAB, "New tab\tCtrl+T", true);
    add(NEW_WINDOW, "New window\tCtrl+N", true);
    add(PRIVATE_WINDOW, "New private window\tCtrl+Shift+N", true);
    add(
        RESTORE_TAB,
        "Reopen last closed\tCtrl+Shift+T",
        !recent.is_empty(),
    );
    add_recent_menu(menu, &recent);
    add(FOCUS_FILTER, "Search open tabs\tCtrl+Shift+A", true);
    add(CLOSE_WINDOW, "Close window\tCtrl+Shift+W", true);
    add(
        FREE_MEMORY,
        "Free memory from background tabs…",
        !background_tabs_for_release().is_empty(),
    );
    unsafe {
        AppendMenuW(
            menu,
            MF_STRING
                | if DEFER_BACKGROUND.get() {
                    MF_CHECKED
                } else {
                    0
                }
                | if private() { MF_GRAYED } else { 0 },
            TOGGLE_DEFER_BACKGROUND,
            windows_sys::w!("Load background links only when selected"),
        );
    }
    separator();
    add(FOCUS_SEARCH, "Search the web\tCtrl+K", true);
    add_search_engine_menu(menu, !private());
    add_site_setup_menu(menu, !private());
    add(SAVE_BOOKMARK, "Bookmark this page\tCtrl+D", !private());
    add(
        BOOKMARK_ALL_TABS,
        "Bookmark all tabs…\tCtrl+Shift+D",
        !private(),
    );
    add(BOOKMARKS, "Manage bookmarks\tCtrl+Shift+O", !private());
    add(IMPORT_BOOKMARKS, "Import bookmarks from HTML…", !private());
    add(EXPORT_BOOKMARKS, "Export bookmarks to HTML…", !private());
    add(HISTORY, "History\tCtrl+H", !private());
    add(DOWNLOADS, "Downloads\tCtrl+J", true);
    separator();
    add(FIND, "Find in page\tCtrl+F", true);
    add(PRINT, "Print…\tCtrl+P", true);
    if let Some(host) = host() {
        let percent = (100.0 * 1.2_f64.powf(host.zoom_level())).round() as i32;
        add(
            ZOOM_IN,
            "Zoom in\tCtrl++",
            host.can_zoom(ZoomCommand::IN) != 0,
        );
        add(
            ZOOM_OUT,
            "Zoom out\tCtrl+-",
            host.can_zoom(ZoomCommand::OUT) != 0,
        );
        add(
            ZOOM_RESET,
            &format!("Reset zoom ({percent}%)\tCtrl+0"),
            host.can_zoom(ZoomCommand::RESET) != 0,
        );
    }
    separator();
    add(SETTINGS, "Settings", !private());
    add(SITE_PERMISSIONS, "Site permissions", !private());
    add(
        CLEAR_BROWSING,
        "Clear browsing data…\tCtrl+Shift+Del",
        !private(),
    );
    add(PASSWORDS, "Password manager", !private());
    add(IMPORT_PASSWORDS, "Import passwords from CSV…", !private());
    let vault_site_at_open = vault_site();
    if let Some(site) = &vault_site_at_open {
        add(VAULT_SAVE, "Add or update Flit Vault login…", true);
        add(
            VAULT_REMOVE,
            "Remove Flit Vault login…",
            matches!(crate::vault_store::has(&site.origin), Ok(true)),
        );
    }
    add(EXTENSIONS, "Extensions", !private());
    add(
        DEVTOOLS,
        "Developer tools\tF12",
        host().is_some_and(|host| {
            host.has_dev_tools() != 0
                || host.can_execute_chrome_command(chrome_command(c"IDC_DEV_TOOLS_TOGGLE")) != 0
        }),
    );
    unsafe {
        AppendMenuW(
            menu,
            MF_STRING
                | if RESTORE_TABS.get() {
                    MF_CHECKED
                } else {
                    MF_UNCHECKED
                }
                | if private() { MF_GRAYED } else { 0 },
            TOGGLE_RESTORE,
            windows_sys::w!("Restore windows and tabs on startup"),
        );
    }
    if !private() && RESTORE_TABS.get() {
        add(
            RECOVERY_COPY,
            "Open previous session backup",
            PROFILE.with(|slot| slot.borrow().as_deref().is_some_and(session::has_backup)),
        );
    }
    separator();
    add(
        MATERIAL,
        if chrome::acrylic() {
            "Use Mica window frame"
        } else {
            "Use acrylic window frame"
        },
        true,
    );
    add(UPDATE, "About Flit and updates", true);
    add(EXIT_FLIT, "Exit Flit (all windows)", true);
    let mut rect = RECT::default();
    let selection = unsafe {
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
    match selection {
        // Handle Vault actions only as a native menu selection, outside WM_COMMAND.
        VAULT_SAVE | VAULT_REMOVE => {
            if let Some(site) = vault_site_at_open {
                vault_ui::manage(ui.window, &site, selection == VAULT_REMOVE, vault_site);
            }
        }
        item if item >= RECENT_MENU_FIRST => {
            if let Some((id, _)) = recent.get(item - RECENT_MENU_FIRST) {
                // The list may have changed while Windows was tracking the menu.
                unsafe { PostMessageW(ui.window, WM_COMMAND, REOPEN_RECENT, *id as isize) };
            }
        }
        0 => (),
        _ => unsafe {
            PostMessageW(ui.window, WM_COMMAND, selection, 0);
        },
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
                        .map_or_else(|| "Untitled window".into(), closed_title);
                    let count = state.entries.len();
                    let noun = if count == 1 { "tab" } else { "tabs" };
                    format!("Window: {title} ({count} {noun})")
                }
            };
            (record.id, label.replace('&', "&&"))
        })
        .collect()
}

fn add_recent_menu(menu: HMENU, items: &[(usize, String)]) {
    let label = windows_sys::w!("Recently closed");
    if items.is_empty() {
        unsafe { AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, label) };
        return;
    }
    let submenu = unsafe { CreatePopupMenu() };
    if submenu.is_null() {
        return;
    }
    for (index, (_, title)) in items.iter().enumerate() {
        unsafe {
            AppendMenuW(
                submenu,
                MF_STRING,
                RECENT_MENU_FIRST + index,
                wide(title).as_ptr(),
            );
        }
    }
    if unsafe { AppendMenuW(menu, MF_POPUP, submenu as usize, label) } == 0 {
        unsafe { DestroyMenu(submenu) };
    }
}

fn add_search_engine_menu(menu: HMENU, enabled: bool) {
    let choices = unsafe { CreatePopupMenu() };
    if choices.is_null() {
        return;
    }
    for (offset, (_, label)) in SearchEngine::ALL.into_iter().enumerate() {
        unsafe {
            AppendMenuW(
                choices,
                MF_STRING,
                SEARCH_ENGINE_FIRST + offset,
                wide(label).as_ptr(),
            );
        }
    }
    let current = SearchEngine::ALL
        .iter()
        .position(|(engine, _)| *engine == SEARCH_ENGINE.get());
    if let Some(index) = current {
        unsafe {
            CheckMenuRadioItem(
                choices,
                SEARCH_ENGINE_FIRST as u32,
                SEARCH_ENGINE_LAST as u32,
                (SEARCH_ENGINE_FIRST + index) as u32,
                MF_BYCOMMAND,
            );
        }
    }
    if unsafe {
        AppendMenuW(
            menu,
            MF_POPUP | if enabled { 0 } else { MF_GRAYED },
            choices as usize,
            windows_sys::w!("Search engine"),
        )
    } == 0
    {
        unsafe { DestroyMenu(choices) };
    }
}

fn add_site_setup_menu(menu: HMENU, enabled: bool) {
    let sources = unsafe { CreatePopupMenu() };
    if sources.is_null() {
        return;
    }
    unsafe {
        AppendMenuW(
            sources,
            MF_STRING,
            PREPARE_CHROME,
            windows_sys::w!("From Chrome bookmarks…"),
        );
        AppendMenuW(
            sources,
            MF_STRING,
            PREPARE_EDGE,
            windows_sys::w!("From Edge bookmarks…"),
        );
        AppendMenuW(
            sources,
            MF_STRING,
            PREPARE_CLIPBOARD,
            windows_sys::w!("From copied site links…"),
        );
        AppendMenuW(sources, MF_SEPARATOR, 0, null());
        AppendMenuW(
            sources,
            MF_STRING,
            IMPORT_CHROME_BOOKMARKS,
            windows_sys::w!("Import Chrome bookmarks and folders…"),
        );
        AppendMenuW(
            sources,
            MF_STRING,
            IMPORT_EDGE_BOOKMARKS,
            windows_sys::w!("Import Edge bookmarks and folders…"),
        );
        if AppendMenuW(
            menu,
            MF_POPUP | if enabled { 0 } else { MF_GRAYED },
            sources as usize,
            windows_sys::w!("Move sites and bookmarks"),
        ) == 0
        {
            DestroyMenu(sources);
        }
    }
}

fn site_setup_info(window: HWND, message: &str) {
    let message = wide(message);
    unsafe {
        MessageBoxW(
            window,
            message.as_ptr(),
            windows_sys::w!("Prepare sites in Flit"),
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
            site_setup_info(
                ui.window,
                &format!(
                    "No standard {} bookmarks profile was found.",
                    browser.label()
                ),
            );
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
            "There are no website bookmarks in this profile to import.",
        );
        return;
    }
    let title = format!("From {} - {}", browser.label(), profile.label);
    let Some(request) = BookmarkRequest::import_profile(&title, &nodes) else {
        site_setup_info(
            ui.window,
            "This profile contains too much bookmark data to import at once.",
        );
        return;
    };
    let detail = wide(&format!(
        "Import {count} bookmarks and their folders into Other bookmarks → {title}?\n\nExisting bookmarks will stay in place."
    ));
    if unsafe {
        MessageBoxW(
            ui.window,
            detail.as_ptr(),
            windows_sys::w!("Import bookmarks to Flit"),
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

fn prepare_site_tabs(window: HWND, label: &str, sites: Vec<String>) {
    if sites.is_empty() {
        site_setup_info(window, "There are no new site links to prepare.");
        return;
    }
    let body = wide(&format!(
        "{label}\n\nPrepare {} site tabs in this window?\n\n{}\n\nTabs load when selected. Sign in on each site as needed. Passwords and login sessions are not transferred.",
        sites.len(),
        sites.join("\n")
    ));
    if unsafe {
        MessageBoxW(
            window,
            body.as_ptr(),
            windows_sys::w!("Prepare sites in Flit"),
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
        if let Err(message) = append_tab(&url, url.trim_end_matches('/'), false, None) {
            failure = Some(message);
            break;
        }
        added = true;
    }
    if added {
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
    if let Some(message) = failure {
        site_setup_info(window, message);
    }
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
    let target = 28 * unsafe { GetDpiForWindow(hwnd) }.max(96) as i32 / 96;
    if close_only && ((lp as u16 as i16) as i32) < row.right - target {
        return None;
    }
    row_key(index)
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
    let Some((pinned, muted, copyable, unloadable, up, down, site, close_later)) =
        with_shell(|s| {
            let tab = s.tabs.iter().find(|t| t.key == key)?;
            Some((
                tab.pinned,
                tab.muted,
                session::stored_url(&tab.url).is_some(),
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
    for (id, label, enabled) in [
        (TAB_PIN, if pinned { "Unpin tab" } else { "Pin tab" }, true),
        (TAB_DUPLICATE, "Duplicate tab", copyable),
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
        let label = wide(&format!("Keep {site} active when freeing memory"));
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
        && Some(tab.key) != s.active
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
                windows_sys::w!("Could not save this site's memory setting."),
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
        return;
    }
    let message = wide(&format!(
        "Unload up to {} background tabs? They stay in the sidebar and reload when selected.\n\nKept sites, pinned tabs, downloads, camera or microphone use, and inspected tabs are skipped. Other audio and unsaved work can stop or be lost.",
        keys.len()
    ));
    if unsafe {
        MessageBoxW(
            window,
            message.as_ptr(),
            windows_sys::w!("Free tab memory?"),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        )
    } != IDYES
    {
        return;
    }
    change(|s| s.memory_queue.extend(keys));
    release_next_background_tab();
}

fn release_next_background_tab() {
    let next = change(|s| {
        if s.closing || s.tabs.iter().any(|tab| tab.unloading) {
            return None;
        }
        while let Some(key) = s.memory_queue.pop_front() {
            let Some(index) = s.tabs.iter().position(|tab| tab.key == key) else {
                continue;
            };
            if can_release_background(s, &s.tabs[index]) {
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
            windows_sys::w!(
                "This tab will reload when you select it. Audio and calls will stop, and unsaved changes may be lost. Unload the tab?"
            ),
            windows_sys::w!("Unload tab?"),
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
            if let Some(url) =
                with_shell(|s| session::stored_url(&s.tabs.iter().find(|tab| tab.key == key)?.url))
                    .flatten()
            {
                let _ = add_tab(&url);
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
    if matches!(id, ADDRESS | FILTER | FIND_TEXT) {
        let composing = (matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN)
            || (matches!(message, WM_CHAR | WM_SYSCHAR) && wp == VK_RETURN as usize))
            && keyboard::edit_composing(hwnd);
        if let Some(result) = keyboard::handle_edit_message(hwnd, message, wp, lp, composing) {
            return result;
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
            (_, VK_TAB) if !ctrl && !alt => {
                if let Some(ui) = controls() {
                    let chain: Vec<_> = ui
                        .chrome()
                        .into_iter()
                        .filter(|item| unsafe {
                            *item != ui.label
                                && IsWindowVisible(*item) != 0
                                && IsWindowEnabled(*item) != 0
                        })
                        .collect();
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
                PostMessageW(parent as HWND, WM_COMMAND, GO, 0);
            },
            FILTER => filter_input(VK_RETURN),
            FIND_TEXT => find_in_page(true, true),
            TABS => (),
            _ => return unsafe { DefSubclassProc(hwnd, message, wp, lp) },
        }
        return 0;
    }
    if id == TABS {
        match message {
            WM_CONTEXTMENU => {
                show_tab_menu(hwnd, parent as HWND, lp);
                return 0;
            }
            WM_LBUTTONDOWN | WM_MBUTTONDOWN => {
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
            WM_LBUTTONUP | WM_MBUTTONUP => {
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
                if change(|s| {
                    s.row_close = None;
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
    match message {
        WM_ACTIVATE => {
            if wp & 0xffff != WA_INACTIVE as usize
                && with_shell(|s| !s.private && !s.closing) == Some(true)
            {
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
            queue_session_save();
            0
        }
        WM_SIZE => {
            layout();
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
        WM_TIMER if wp == SIDEBAR_TIMER => {
            tick_sidebar(window);
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
        WM_SYSCOLORCHANGE | WM_SETTINGCHANGE | WM_DWMCOMPOSITIONCHANGED if controls().is_some() => {
            let dpi = unsafe { GetDpiForWindow(window) }.max(96) as i32;
            chrome::apply_backdrop(window, 8 * dpi / 96);
            unsafe {
                RedrawWindow(window, null(), null_mut(), RDW_INVALIDATE | RDW_ALLCHILDREN);
            }
            unsafe { DefWindowProcW(window, message, wp, lp) }
        }
        WM_DRAWITEM if lp != 0 => {
            let item = unsafe { &*(lp as *const DRAWITEMSTRUCT) };
            let dpi = unsafe { GetDpiForWindow(window) }.max(96) as i32;
            if wp == TABS {
                let Some((row, active)) =
                    with_shell(|s| (s.rows.get(item.itemID as usize).cloned(), s.active))
                else {
                    return 0;
                };
                let (label, pinned, muted, unloaded) =
                    row.as_ref().map_or(("", false, false, false), |row| {
                        (row.label.as_str(), row.pinned, row.muted, row.unloaded)
                    });
                chrome::draw_tab(
                    item,
                    label,
                    pinned,
                    muted,
                    unloaded,
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
            let kind = match wp {
                SIDEBAR => ButtonKind::Sidebar,
                BACK => ButtonKind::Back,
                FORWARD => ButtonKind::Forward,
                RELOAD if with_shell(|s| s.reload_is_stop) == Some(true) => ButtonKind::Stop,
                RELOAD => ButtonKind::Reload,
                GO => ButtonKind::Go,
                SAVE_BOOKMARK => ButtonKind::Bookmark,
                SITE_INFO => ButtonKind::SiteInfo,
                MENU => ButtonKind::Menu,
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
                        set_text(window, &format!("{} — {message}", app_title()));
                    } else if let Some(ui) = controls() {
                        focus_control(ui.address, true);
                    }
                }
                NEW_WINDOW | PRIVATE_WINDOW => {
                    if let Err(message) = open_window(id == PRIVATE_WINDOW, None, None) {
                        set_text(window, &format!("{} — {message}", app_title()));
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
                RESTORE_TAB => reopen_closed(None),
                REOPEN_RECENT if lp != 0 => reopen_closed(Some(lp as usize)),
                FREE_MEMORY => free_background_memory(window),
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
                    let index = if id == SELECT_TAB_LAST {
                        with_shell(|s| s.tabs.len().checked_sub(1)).flatten()
                    } else {
                        Some(id - SELECT_TAB_FIRST)
                    };
                    if let Some(index) = index {
                        select(index);
                        focus_page();
                    }
                }
                FULLSCREEN => toggle_fullscreen(window),
                ZOOM_IN | ZOOM_OUT | ZOOM_RESET => {
                    if let Some(host) = host() {
                        zoom_page(&host, id);
                    }
                }
                MENU => {
                    if let Some(ui) = controls() {
                        show_menu(ui);
                    }
                }
                HISTORY if !private() => open_builtin("chrome://history/"),
                BOOKMARKS if !private() => open_builtin(MANAGER_URL),
                SAVE_BOOKMARK => bookmark_page(),
                BOOKMARK_ALL_TABS if !private() => bookmark_all_tabs(window),
                IMPORT_BOOKMARKS => run_bookmark_request(BookmarkRequest::Import),
                EXPORT_BOOKMARKS => run_bookmark_request(BookmarkRequest::Export),
                SITE_INFO => show_site_info(window),
                DOWNLOADS => open_builtin("chrome://downloads/"),
                SETTINGS if !private() => open_builtin(SETTINGS_URL),
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
                    change(|s| {
                        if !s.updating_address {
                            s.address_edited = true;
                        }
                    });
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
            if let Some(native) = native.filter(|native| native.is_closed() == 0) {
                native.show();
            }
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
        PAGE_CHANGED => {
            refresh(false);
            queue_session_save();
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
            unsafe {
                KillTimer(window, SIDEBAR_TIMER);
                KillTimer(window, RECOVERY_TIMER);
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
                if empty {
                    quit_message_loop();
                }
            }
            unsafe { DefWindowProcW(window, message, wp, lp) }
        }
        _ => unsafe { DefWindowProcW(window, message, wp, lp) },
    }
}
