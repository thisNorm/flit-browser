use crate::{
    bookmarks::{MANAGER_URL, SaveRequest},
    chrome::{self, ButtonKind},
    download_guard,
    keyboard::{self, Shortcut},
    navigation::{self, HOME},
    panel_motion::Motion,
    permissions,
    session::{self, Entry, Session},
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
        CreateFontW, DEFAULT_GUI_FONT, DeleteObject, GetMonitorInfoW, GetStockObject, HDC, HFONT,
        InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, RDW_ALLCHILDREN,
        RDW_INVALIDATE, RedrawWindow, ScreenToClient, UpdateWindow,
    },
    System::{LibraryLoader::GetModuleHandleW, SystemServices::SS_CENTERIMAGE},
    UI::{
        Controls::*,
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
            SetThreadDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            EnableWindow, GetFocus, GetKeyState, IsWindowEnabled, ReleaseCapture, SetCapture,
            SetFocus, VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_MENU, VK_RETURN, VK_SHIFT, VK_TAB,
        },
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

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
const PAGE_CHANGED: u32 = WM_APP + 1;
const PAGE_CLOSED: u32 = WM_APP + 2;
const CLOSE_BY_KEY: u32 = WM_APP + 3;
const OPEN_LINK: u32 = WM_APP + 4;
const TAB_MENU_ACTION: u32 = WM_APP + 5;
const TAB_CLOSE_CANCELLED: u32 = WM_APP + 6;
const POPUP_CLOSED: u32 = WM_APP + 7;
const OPEN_SETTINGS_WINDOW: u32 = WM_APP + 8;
const TAB_PIN: usize = 1;
const TAB_DUPLICATE: usize = 2;
const TAB_MUTE: usize = 3;
const TAB_CLOSE: usize = 4;
const TAB_UNLOAD: usize = 5;
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

struct Tab {
    key: usize,
    panel: HWND,
    browser: Option<Browser>,
    native_window: Option<Window>,
    native_close_sent: bool,
    close_requested: bool,
    retiring: bool,
    url: String,
    title: String,
    has_video_access: bool,
    has_audio_access: bool,
    back: bool,
    forward: bool,
    pinned: bool,
    muted: bool,
    unloading: bool,
    unloaded: bool,
    bookmark: Option<SaveRequest>,
}

struct Shell {
    controls: Controls,
    tabs: Vec<Tab>,
    active: Option<usize>,
    next_key: usize,
    sidebar_open: bool,
    sidebar_motion: Motion,
    filter: String,
    rows: Vec<Row>,
    font: HFONT,
    font_dpi: i32,
    find_open: bool,
    fullscreen: Option<WINDOWPLACEMENT>,
    recently_closed: Vec<String>,
    private: bool,
    address_edited: bool,
    updating_address: bool,
    row_close: Option<(usize, u32)>,
    links: VecDeque<(String, WindowOpenDisposition)>,
    popups: Vec<Browser>,
    pending_popups: BTreeSet<(usize, i32)>,
    settings_window: Option<Browser>,
    pending_settings: Option<(usize, String)>,
    closing: bool,
    closing_session: Option<Session>,
}

thread_local! {
    static WINDOWS: RefCell<BTreeMap<usize, Shell>> = const { RefCell::new(BTreeMap::new()) };
    static CURRENT_WINDOW: Cell<usize> = const { Cell::new(0) };
    static LAST_NORMAL: Cell<usize> = const { Cell::new(0) };
    static PROFILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static PRIVATE_CONTEXT: RefCell<Option<RequestContext>> = const { RefCell::new(None) };
    static RESTORE_TABS: Cell<bool> = const { Cell::new(true) };
    static RECOVERY_WINDOW: Cell<usize> = const { Cell::new(0) };
    static EXIT_CODE: Cell<i32> = const { Cell::new(0) };
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
    if !RESTORE_TABS.get() || with_shell(|s| !s.private && !s.closing) != Some(true) {
        return;
    }
    if let Some(window) = normal_window() {
        if RECOVERY_WINDOW.get() == window as usize {
            return;
        }
        let previous = RECOVERY_WINDOW.replace(window as usize) as HWND;
        unsafe {
            if !previous.is_null() && previous != window {
                KillTimer(previous, RECOVERY_TIMER);
            }
            if SetTimer(window, RECOVERY_TIMER, 700, None) == 0 {
                RECOVERY_WINDOW.set(0);
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
                    if queue_link(self.window, Some(&url), WindowOpenDisposition::NEW_FOREGROUND_TAB) {
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
            i32::from(queue_link(self.window, url, disposition))
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
                queue_link(self.window, url, disposition);
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
            Some(FlitKeyboard::new(self.window))
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
            let proceed = unsafe { MessageBoxW(self.window as HWND, message,
                windows_sys::w!("Flit Browser"), MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) } == IDYES;
            let cancelled_close = !proceed && is_reload == 0
                && change(|s| {
                    s.closing_session = None;
                    let tab = s.tabs.iter().any(|tab| tab.key == self.key && tab.close_requested);
                    std::mem::replace(&mut s.closing, false) || tab
                }) == Some(true);
            callback.cont(i32::from(proceed), None);
            if cancelled_close {
                unsafe { PostMessageW(self.window as HWND, TAB_CLOSE_CANCELLED, self.key, 0); }
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
            if gesture == 0 {
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
                queue_link(self.window, target_url, disposition);
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
                        if private() {
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
                && queue_link(self.window, url, disposition) {
                1
            } else { 0 }
        }
    }
}

fn queue_link(window: usize, url: Option<&CefString>, disposition: WindowOpenDisposition) -> bool {
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
        s.links.push_back((url, disposition));
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
        fn on_loading_state_change(&self, _browser: Option<&mut Browser>, _loading: i32, back: i32, forward: i32) {
            let _scope = WindowScope::enter(self.window);
            update_page(self.key, |tab| { tab.back = back != 0; tab.forward = forward != 0; });
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
    struct FlitKeyboard { window: usize }
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

fn tooltips(ui: Controls) {
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
        return;
    }
    for (hwnd, text) in [
        (ui.sidebar, w!("Show or hide vertical tabs")),
        (ui.back, w!("Back (Alt+Left)")),
        (ui.forward, w!("Forward (Alt+Right)")),
        (ui.reload, w!("Reload (Ctrl+R)")),
        (ui.go, w!("Go to address (Enter)")),
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
    open_window(false, initial_url, saved.filter(|_| !skip_restore)).map(|_| ())
}

fn open_window(
    private: bool,
    initial_url: Option<&str>,
    saved: Option<Session>,
) -> Result<HWND, &'static str> {
    let name = wide(WINDOW_CLASS);
    let caption = wide(if private { PRIVATE_TITLE } else { APP_TITLE });
    let window = unsafe {
        CreateWindowExW(
            0,
            name.as_ptr(),
            caption.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1160,
            790,
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
            windows_sys::w!("Search tabs") as isize,
        );
        SendMessageW(
            ui.find_text,
            EM_SETCUEBANNER,
            1,
            windows_sys::w!("Find in page") as isize,
        );
    }
    WINDOWS.with(|windows| {
        windows.borrow_mut().insert(
            window as usize,
            Shell {
                controls: ui,
                tabs: Vec::new(),
                active: None,
                next_key: 1,
                sidebar_open: true,
                sidebar_motion: Motion::new(true),
                filter: String::new(),
                rows: Vec::new(),
                font: null_mut(),
                font_dpi: 0,
                find_open: false,
                fullscreen: None,
                recently_closed: Vec::new(),
                private,
                address_edited: false,
                updating_address: false,
                row_close: None,
                links: VecDeque::new(),
                popups: Vec::new(),
                pending_popups: BTreeSet::new(),
                settings_window: None,
                pending_settings: None,
                closing: false,
                closing_session: None,
            },
        );
    });
    if !private {
        LAST_NORMAL.set(window as usize);
    } else {
        unsafe {
            EnableWindow(ui.bookmark, 0);
        }
    }
    let dpi = unsafe { GetDpiForWindow(window) }.max(96) as i32;
    chrome::apply_backdrop(window, 8 * dpi / 96);
    tooltips(ui);
    layout();
    unsafe {
        ShowWindow(window, SW_SHOW);
        UpdateWindow(window);
    }
    let restored = if let Some(state) =
        saved.filter(|state| !private && state.restore && !state.entries.is_empty())
    {
        for entry in state.entries {
            append_tab(&entry.url, &entry.title, entry.pinned)?;
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
        select(selected);
    } else {
        add_tab(HOME)?;
    }
    if active().is_none() {
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
            .filter(|s| !s.closing)
            .or_else(|| windows.values().find(|s| !s.private && !s.closing))
            .map(|s| s.controls.window)
    })
}

pub(crate) fn show_existing(url: Option<&str>) {
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
    for hwnd in [ui.label, ui.filter, ui.tabs] {
        set_visible(hwnd, visible);
    }
    let show_empty =
        visible && with_shell(|s| !s.filter.is_empty() && s.rows.is_empty()) == Some(true);
    set_visible(ui.empty, show_empty);
    if visible {
        let x = l.side_offset;
        set_bounds(ui.label, x + d(22), l.top + d(11), d(46), d(30));
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
            l.height - l.top - d(109),
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
        for hwnd in ui.chrome().into_iter().chain(ui.find()).chain([ui.empty]) {
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
    let Some((ui, changed, index)) = change(|s| {
        let rows: Vec<_> = s
            .tabs
            .iter()
            .filter_map(|tab| {
                let label = tab_label(tab);
                (s.filter.is_empty()
                    || label.to_lowercase().contains(&s.filter)
                    || tab.url.to_lowercase().contains(&s.filter))
                .then(|| Row {
                    key: tab.key,
                    label: if let Some((capture, _)) = capture_labels(tab) {
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
        let index = rows.iter().position(|row| Some(row.key) == s.active);
        let changed = if rows == s.rows {
            None
        } else {
            s.rows = rows;
            Some(s.rows.clone())
        };
        (s.controls, changed, index)
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
    update_empty(ui);
}

fn refresh(force_address: bool) {
    refresh_rows(force_address);
    let Some((ui, url, title, capture, back, forward, web)) = with_shell(|s| {
        let tab = s.tabs.iter().find(|t| Some(t.key) == s.active)?;
        Some((
            s.controls,
            tab.url.clone(),
            clean(&tab.title, 100),
            capture_labels(tab).map(|(_, caption)| caption),
            tab.back,
            tab.forward,
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
        EnableWindow(ui.back, i32::from(back));
        EnableWindow(ui.forward, i32::from(forward));
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
        let tab = s.tabs.get(index)?;
        if tab.unloading || tab.retiring {
            return None;
        }
        let key = tab.key;
        s.active = Some(key);
        Some((
            key,
            s.tabs
                .iter()
                .enumerate()
                .map(|(i, t)| (t.panel, i == index))
                .collect::<Vec<_>>(),
            s.closing,
        ))
    })
    .flatten() else {
        return;
    };
    for (panel, visible) in panels {
        unsafe {
            ShowWindow(panel, if visible { SW_SHOWNA } else { SW_HIDE });
        }
    }
    layout();
    refresh(true);
    if !closing {
        if let Err(message) = load_deferred(key) {
            if let Some(ui) = controls() {
                set_text(ui.window, &format!("{} — {message}", app_title()));
            }
        }
        queue_session_save();
    }
}

fn add_tab(url: &str) -> Result<(), &'static str> {
    open_tab(url, true)
}

fn open_tab(url: &str, foreground: bool) -> Result<(), &'static str> {
    let (key, index) = append_tab(url, "", false)?;
    if foreground || with_shell(|s| s.active.is_none()) == Some(true) {
        select(index);
    } else {
        refresh_rows(false);
        load_deferred(key)?;
    }
    queue_session_save();
    Ok(())
}

fn append_tab(url: &str, title: &str, pinned: bool) -> Result<(usize, usize), &'static str> {
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
            panel,
            browser: None,
            native_window: None,
            native_close_sent: false,
            close_requested: false,
            retiring: false,
            url: url.to_owned(),
            title: title.to_owned(),
            has_video_access: false,
            has_audio_access: false,
            back: false,
            forward: false,
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

fn materialize(key: usize) -> Result<Browser, &'static str> {
    let (ui, panel, existing, muted) = with_shell(|s| {
        let tab = s.tabs.iter().find(|tab| tab.key == key && !tab.retiring)?;
        Some((s.controls, tab.panel, tab.browser.clone(), tab.muted))
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
    Ok(browser)
}

fn navigate() {
    let Some(ui) = controls() else {
        return;
    };
    let raw = text(ui.address, 16_384);
    let url = match navigation::address(&raw) {
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
        // Keep tab entries until the entire window closes. If a later page
        // vetoes closing, earlier tabs can still be selected and reloaded.
        tab.unloaded = tab.url != HOME;
        tab.has_video_access = false;
        tab.has_audio_access = false;
        tab.back = false;
        tab.forward = false;
        Some((s.controls.window, s.closing))
    })
    .flatten()
    {
        if closing {
            continue_window_close(window);
        } else {
            refresh_rows(true);
        }
        return;
    }
    let Some((window, panel, next, was_active, closing)) = change(|s| {
        let index = s.tabs.iter().position(|tab| tab.key == key)?;
        let tab = s.tabs.remove(index);
        if tab.url.starts_with("https://") || tab.url.starts_with("http://") {
            s.recently_closed.push(tab.url);
            if s.recently_closed.len() > 20 {
                s.recently_closed.remove(0);
            }
        }
        let next = if s.tabs.is_empty() {
            None
        } else {
            s.tabs
                .iter()
                .position(|t| Some(t.key) == s.active)
                .or_else(|| Some(index.min(s.tabs.len() - 1)))
        };
        Some((
            s.controls.window,
            tab.panel,
            next,
            Some(key) == s.active,
            s.closing,
        ))
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

fn close_tab(key: Option<usize>) {
    let Some((window, key, loaded)) = with_shell(|s| {
        if s.closing {
            return None;
        }
        let key = key.or(s.active)?;
        let tab = s
            .tabs
            .iter()
            .find(|t| t.key == key && !t.unloading && !t.retiring && !t.close_requested)?;
        Some((s.controls.window, key, tab.browser.is_some()))
    })
    .flatten() else {
        return;
    };
    if !confirm_download_close(window, Some(key)) {
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
    if with_shell(|s| s.closing) != Some(false) || !confirm_download_close(window, None) {
        return;
    }
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
        s.closing_session = (!s.private).then(|| snapshot(s));
        true
    }) == Some(true)
    {
        continue_window_close(window);
    }
}

fn confirm_download_close(window: HWND, tab: Option<usize>) -> bool {
    let count = download_guard::count(window as usize, tab);
    if count == 0 {
        return true;
    }
    let target = if tab.is_some() { "tab" } else { "window" };
    let message = wide(&format!(
        "Downloads in progress: {count}\nClosing this {target} may interrupt them. Close anyway?"
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
            if last_normal_window() {
                store_session(&state);
            }
        }
        unsafe {
            DestroyWindow(window);
        }
    }
}

fn snapshot(s: &Shell) -> Session {
    if !RESTORE_TABS.get() {
        return Session {
            entries: Vec::new(),
            selected: 0,
            restore: false,
        };
    }
    let mut entries = Vec::new();
    let mut selected = 0;
    for tab in &s.tabs {
        if settings_target(&tab.url).is_some() {
            continue;
        }
        if Some(tab.key) == s.active {
            selected = entries.len();
        }
        entries.push(Entry {
            url: tab.url.clone(),
            title: tab.title.clone(),
            pinned: tab.pinned,
        });
    }
    Session {
        entries,
        selected,
        restore: true,
    }
}

fn last_normal_window() -> bool {
    !private()
        && WINDOWS.with(|windows| windows.borrow().values().filter(|s| !s.private).count() == 1)
}

fn store_session(state: &Session) {
    PROFILE.with(|slot| {
        if let Some(profile) = slot.borrow().as_deref() {
            if let Err(error) = session::save(profile, state) {
                eprintln!("Flit Browser: Could not save tabs: {error}");
            }
        }
    });
}

fn save_session() {
    if let Some(state) = with_shell(|s| (!s.private).then(|| snapshot(s))).flatten() {
        store_session(&state);
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
            let font = with_shell(|s| s.font).unwrap_or(null_mut());
            chrome::paint_start(hwnd, font);
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
    if let Some(open) = with_shell(|s| s.sidebar_open) {
        set_sidebar(!open, true);
    }
}

fn toggle_fullscreen(window: HWND) {
    if let Some(placement) = change(|s| s.fullscreen.take()).flatten() {
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
        let mut saved = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        unsafe {
            let display = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
            if GetWindowPlacement(window, &mut saved) == 0
                || GetMonitorInfoW(display, &mut monitor) == 0
            {
                return;
            }
        }
        close_find(false);
        unsafe {
            KillTimer(window, SIDEBAR_TIMER);
        }
        change(|s| {
            s.fullscreen = Some(saved);
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
        SaveRequest::new(&session::stored_url(&tab.url)?, &clean(&tab.title, 240))
    })
    .flatten();
    let Some(request) = request else {
        open_builtin(MANAGER_URL);
        return;
    };
    let existing = with_shell(|s| {
        s.tabs
            .iter()
            .find(|tab| tab.url == MANAGER_URL)
            .map(|t| t.key)
    })
    .flatten();
    let key = match existing
        .map(Ok)
        .unwrap_or_else(|| append_tab(MANAGER_URL, "", false).map(|(key, _)| key))
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

fn toggle_find() {
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
    let restore = with_shell(|s| !s.recently_closed.is_empty()).unwrap_or(false);
    add(NEW_TAB, "New tab\tCtrl+T", true);
    add(NEW_WINDOW, "New window\tCtrl+N", true);
    add(PRIVATE_WINDOW, "New private window\tCtrl+Shift+N", true);
    add(RESTORE_TAB, "Reopen closed tab\tCtrl+Shift+T", restore);
    add(FOCUS_FILTER, "Search open tabs\tCtrl+Shift+A", true);
    add(CLOSE_WINDOW, "Close window\tCtrl+Shift+W", true);
    separator();
    add(SAVE_BOOKMARK, "Bookmark this page\tCtrl+D", !private());
    add(BOOKMARKS, "Manage bookmarks\tCtrl+Shift+O", !private());
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
            windows_sys::w!("Continue last tabs on startup"),
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
    let mut rect = RECT::default();
    unsafe {
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
        if selection != 0 {
            PostMessageW(ui.window, WM_COMMAND, selection as usize, 0);
        }
    }
}

fn shortcut_command(action: Shortcut) -> usize {
    match action {
        Shortcut::FocusAddress => FOCUS_ADDRESS,
        Shortcut::FocusSearch => FOCUS_SEARCH,
        Shortcut::Bookmark => SAVE_BOOKMARK,
        Shortcut::ManageBookmarks => BOOKMARKS,
        Shortcut::NewTab => NEW_TAB,
        Shortcut::NewWindow => NEW_WINDOW,
        Shortcut::PrivateWindow => PRIVATE_WINDOW,
        Shortcut::RestoreTab => RESTORE_TAB,
        Shortcut::CloseTab => CLOSE_TAB,
        Shortcut::CloseWindow => CLOSE_WINDOW,
        Shortcut::Reload => RELOAD,
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

fn tab_point(hwnd: HWND, lp: LPARAM, close_only: bool) -> Option<usize> {
    let hit = unsafe { SendMessageW(hwnd, LB_ITEMFROMPOINT, 0, lp) } as u32;
    if hit >> 16 != 0 {
        return None;
    }
    let index = (hit & 0xffff) as usize;
    let mut row = RECT::default();
    if unsafe {
        SendMessageW(
            hwnd,
            LB_GETITEMRECT,
            index,
            (&mut row as *mut RECT) as isize,
        )
    } == LB_ERR as isize
    {
        return None;
    }
    let x = (lp as u16 as i16) as i32;
    let y = ((lp >> 16) as u16 as i16) as i32;
    let target = 28 * unsafe { GetDpiForWindow(hwnd) }.max(96) as i32 / 96;
    if x < row.left
        || x >= row.right
        || y < row.top
        || y >= row.bottom
        || (close_only && x < row.right - target)
    {
        return None;
    }
    row_key(index)
}

fn show_tab_menu(list: HWND, parent: HWND, coordinates: LPARAM) {
    let mut point = POINT {
        x: (coordinates as u16 as i16) as i32,
        y: ((coordinates >> 16) as u16 as i16) as i32,
    };
    let key = if coordinates == -1 {
        let index = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) };
        let mut row = RECT::default();
        if index < 0
            || unsafe {
                SendMessageW(
                    list,
                    LB_GETITEMRECT,
                    index as usize,
                    (&mut row as *mut RECT) as LPARAM,
                )
            } < 0
        {
            return;
        }
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
    let Some((pinned, muted, copyable, unloadable)) = with_shell(|s| {
        let tab = s.tabs.iter().find(|t| t.key == key)?;
        Some((
            tab.pinned,
            tab.muted,
            session::stored_url(&tab.url).is_some(),
            can_unload(s, tab),
        ))
    })
    .flatten() else {
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
        (TAB_CLOSE, "Close tab", true),
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
        if action != 0 {
            PostMessageW(parent, TAB_MENU_ACTION, key, action as LPARAM);
        }
    }
}

fn can_unload(s: &Shell, tab: &Tab) -> bool {
    !s.closing
        && !tab.unloading
        && !tab.close_requested
        && Some(tab.key) != s.active
        && tab.browser.is_some()
        && (tab.url.starts_with("http://") || tab.url.starts_with("https://"))
        && session::stored_url(&tab.url).is_some()
        && download_guard::count(s.controls.window as usize, Some(tab.key)) == 0
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
        TAB_UNLOAD => unload_tab(key),
        _ => {}
    }
}

fn filter_input(command: usize) {
    let Some(ui) = controls() else {
        return;
    };
    let selection = (command == VK_RETURN as usize)
        .then(|| row_key(0))
        .flatten();
    if command == VK_RETURN as usize && selection.is_none() {
        return;
    }
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
    let _scope = WindowScope::enter(parent);
    let id = unsafe { GetDlgCtrlID(hwnd) } as usize;
    if matches!(id, ADDRESS | FILTER | FIND_TEXT) {
        let composing = (matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN)
            || (message == WM_CHAR && wp == VK_RETURN as usize))
            && keyboard::edit_composing(hwnd);
        if let Some(result) = keyboard::handle_edit_message(hwnd, message, wp, lp, composing) {
            return result;
        }
        if composing {
            return unsafe { DefSubclassProc(hwnd, message, wp, lp) };
        }
    }
    if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN) {
        let (ctrl, shift, alt) = modifiers();
        if let Some(action) = keyboard::shortcut(wp as u16, ctrl, shift, alt) {
            unsafe {
                PostMessageW(parent as HWND, WM_COMMAND, shortcut_command(action), 0);
            }
            return 0;
        }
        match (id, wp as u16) {
            (ADDRESS, VK_ESCAPE) => {
                refresh(true);
                return 0;
            }
            (FILTER, VK_ESCAPE) => {
                filter_input(wp);
                return 0;
            }
            (FIND_TEXT, VK_ESCAPE) => {
                close_find(true);
                return 0;
            }
            (TABS, VK_DELETE) => {
                let index = unsafe { SendMessageW(hwnd, LB_GETCURSEL, 0, 0) };
                if index >= 0 {
                    close_tab(row_key(index as usize));
                }
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
    if message == WM_CHAR && wp == VK_RETURN as usize {
        match id {
            ADDRESS => unsafe {
                PostMessageW(parent as HWND, WM_COMMAND, GO, 0);
            },
            FILTER => filter_input(wp),
            FIND_TEXT => find_in_page(true, true),
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
            }
            WM_CAPTURECHANGED | WM_CANCELMODE => {
                change(|s| s.row_close = None);
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
            if wp & 0xffff != WA_INACTIVE as usize && with_shell(|s| !s.private) == Some(true) {
                LAST_NORMAL.set(window as usize);
            }
            unsafe { DefWindowProcW(window, message, wp, lp) }
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
        WM_SIZE => {
            layout();
            0
        }
        WM_GETMINMAXINFO if lp != 0 => {
            let dpi = unsafe { GetDpiForWindow(window) }.max(96) as i32;
            unsafe {
                let info = &mut *(lp as *mut MINMAXINFO);
                info.ptMinTrackSize.x = 530 * dpi / 96;
                info.ptMinTrackSize.y = 340 * dpi / 96;
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
                if RESTORE_TABS.get() && with_shell(|s| !s.private && !s.closing) == Some(true) {
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
                let row = with_shell(|s| s.rows.get(item.itemID as usize).cloned()).flatten();
                let (label, pinned, muted, unloaded) =
                    row.as_ref().map_or(("", false, false, false), |row| {
                        (row.label.as_str(), row.pinned, row.muted, row.unloaded)
                    });
                chrome::draw_tab(item, label, pinned, muted, unloaded, dpi);
                return 1;
            }
            let kind = match wp {
                SIDEBAR => ButtonKind::Sidebar,
                BACK => ButtonKind::Back,
                FORWARD => ButtonKind::Forward,
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
            let id = wp & 0xffff;
            let notification = ((wp >> 16) & 0xffff) as u32;
            match id {
                GO => navigate(),
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
                RELOAD | HARD_RELOAD => {
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
                RESTORE_TAB => {
                    if let Some(url) = change(|s| s.recently_closed.pop()).flatten() {
                        let _ = add_tab(&url);
                    }
                }
                SIDEBAR => {
                    reveal_toolbar(window);
                    toggle_sidebar();
                }
                FOCUS_ADDRESS | FOCUS_SEARCH => {
                    reveal_toolbar(window);
                    if let Some(ui) = controls() {
                        focus_control(ui.address, true);
                    }
                }
                FOCUS_FILTER => {
                    reveal_toolbar(window);
                    if with_shell(|s| !s.sidebar_open || s.sidebar_motion.running()) == Some(true) {
                        set_sidebar(true, false);
                    }
                    if let Some(ui) = controls() {
                        focus_control(ui.filter, true);
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
                SITE_INFO => show_site_info(window),
                DOWNLOADS => open_builtin("chrome://downloads/"),
                SETTINGS if !private() => open_builtin(SETTINGS_URL),
                SITE_PERMISSIONS if !private() => open_builtin("chrome://settings/content"),
                CLEAR_BROWSING if !private() => open_builtin(CLEAR_DATA_URL),
                TOGGLE_RESTORE if !private() => {
                    RESTORE_TABS.set(!RESTORE_TABS.get());
                    save_session();
                }
                EXTENSIONS if !private() => open_builtin("chrome://extensions/"),
                PASSWORDS if !private() => open_builtin("chrome://password-manager/"),
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
                    toggle_find();
                }
                FOCUS_FIND_PREV | FOCUS_FIND_NEXT => {
                    if with_shell(|s| s.find_open) == Some(true) {
                        find_in_page(id == FOCUS_FIND_NEXT, true);
                    } else {
                        reveal_toolbar(window);
                        toggle_find();
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
                        change(|s| s.filter = value.trim().to_lowercase());
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
            close_tab(Some(wp));
            0
        }
        TAB_MENU_ACTION => {
            tab_action(wp, lp as usize);
            0
        }
        TAB_CLOSE_CANCELLED => {
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
            if let Some((key, url)) = change(|s| s.pending_settings.take()).flatten() {
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
            if let Some((url, disposition)) = link {
                if disposition == WindowOpenDisposition::NEW_WINDOW {
                    let _ = open_window(private(), Some(&url), None);
                } else {
                    let _ = open_tab(
                        &url,
                        disposition != WindowOpenDisposition::NEW_BACKGROUND_TAB,
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
        PAGE_CLOSED => {
            finish_closed_browser(wp);
            0
        }
        WM_CLOSE => {
            close_window(window);
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
                if empty {
                    quit_message_loop();
                }
            }
            unsafe { DefWindowProcW(window, message, wp, lp) }
        }
        _ => unsafe { DefWindowProcW(window, message, wp, lp) },
    }
}
