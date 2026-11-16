use crate::{
    chrome::{self, ButtonKind},
    navigation::{self, HOME},
};
use cef::*;
use std::{
    cell::{Cell, RefCell},
    os::windows::ffi::OsStrExt,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::{
        CreateFontW, DEFAULT_GUI_FONT, DeleteObject, GetStockObject, HDC, HFONT, InvalidateRect,
        RDW_ALLCHILDREN, RDW_INVALIDATE, RedrawWindow, UpdateWindow,
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
            SetFocus, VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_F5, VK_LEFT, VK_MENU, VK_RETURN,
            VK_RIGHT, VK_SHIFT, VK_TAB,
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
const PAGE_CHANGED: u32 = WM_APP + 1;
const PAGE_CLOSED: u32 = WM_APP + 2;
const CLOSE_BY_KEY: u32 = WM_APP + 3;
const OPEN_LINK: u32 = WM_APP + 4;
const WINDOW_CLASS: &str = "FlitBrowserWindow";
const APP_TITLE: &str = "Flit Browser";
const ICON_ID: usize = 32512;

#[derive(Clone, Copy)]
struct Controls {
    window: HWND,
    address: HWND,
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
    fn chrome(self) -> [HWND; 12] {
        [
            self.sidebar,
            self.back,
            self.forward,
            self.reload,
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
}

struct Tab {
    key: usize,
    panel: HWND,
    browser: Option<Browser>,
    url: String,
    title: String,
    back: bool,
    forward: bool,
}

struct Shell {
    controls: Controls,
    tabs: Vec<Tab>,
    active: Option<usize>,
    next_key: usize,
    sidebar_open: bool,
    filter: String,
    rows: Vec<Row>,
    font: HFONT,
    font_dpi: i32,
    find_open: bool,
    recently_closed: Vec<String>,
}

thread_local! {
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
    static EXIT_CODE: Cell<i32> = const { Cell::new(0) };
    static ADDRESS_EDITED: Cell<bool> = const { Cell::new(false) };
    static UPDATING_ADDRESS: Cell<bool> = const { Cell::new(false) };
    static ROW_CLOSE: Cell<Option<(usize, u32)>> = const { Cell::new(None) };
    static OPEN_LINKS: RefCell<std::collections::VecDeque<(String, bool)>> = const { RefCell::new(std::collections::VecDeque::new()) };
}

pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn with_shell<T>(f: impl FnOnce(&Shell) -> T) -> Option<T> {
    SHELL.with(|slot| slot.borrow().as_ref().map(f))
}

fn change<T>(f: impl FnOnce(&mut Shell) -> T) -> Option<T> {
    SHELL.with(|slot| slot.borrow_mut().as_mut().map(f))
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

wrap_client! {
    struct FlitClient { key: usize, window: usize }
    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(FlitLife::new(self.window, self.key))
        }
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(FlitDisplay::new(self.key))
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(FlitLoad::new(self.key))
        }
        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(FlitKeyboard::new(self.window))
        }
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(FlitRequest::new(self.window))
        }
        fn find_handler(&self) -> Option<FindHandler> {
            Some(FlitFind::new(self.key))
        }
    }
}

wrap_find_handler! {
    struct FlitFind { key: usize }
    impl FindHandler {
        fn on_find_result(
            &self, _browser: Option<&mut Browser>, _identifier: i32, count: i32,
            _selection: Option<&Rect>, active_match: i32, _final_update: i32,
        ) {
            find_update(self.key, count, active_match);
        }
    }
}

wrap_life_span_handler! {
    struct FlitLife { window: usize, key: usize }
    impl LifeSpanHandler {
        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            let browser = change(|state| state.tabs.iter_mut().find(|tab| tab.key == self.key)?.browser.take()).flatten();
            drop(browser);
            // Defer parent HWND destruction until the CEF callback returns.
            unsafe { PostMessageW(self.window as HWND, PAGE_CLOSED, self.key, 0); }
        }
        fn on_before_popup(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            _popup_id: i32, target_url: Option<&CefString>, _target_name: Option<&CefString>,
            disposition: WindowOpenDisposition, gesture: i32, _features: Option<&PopupFeatures>,
            _window: Option<&mut WindowInfo>, _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>, _extra: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            // Open ordinary user-initiated links in managed tabs. Popup scripts that
            // depend on window.opener need a real separate popup, not a duplicated URL.
            if gesture != 0 && matches!(disposition, WindowOpenDisposition::NEW_FOREGROUND_TAB
                | WindowOpenDisposition::NEW_BACKGROUND_TAB | WindowOpenDisposition::NEW_WINDOW) {
                queue_link(self.window, target_url, disposition != WindowOpenDisposition::NEW_BACKGROUND_TAB);
            }
            1
        }
    }
}

wrap_request_handler! {
    struct FlitRequest { window: usize }
    impl RequestHandler {
        fn on_open_urlfrom_tab(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            url: Option<&CefString>, disposition: WindowOpenDisposition, gesture: i32,
        ) -> i32 {
            if gesture != 0 && matches!(disposition, WindowOpenDisposition::NEW_FOREGROUND_TAB
                | WindowOpenDisposition::NEW_BACKGROUND_TAB | WindowOpenDisposition::NEW_WINDOW)
                && queue_link(self.window, url, disposition != WindowOpenDisposition::NEW_BACKGROUND_TAB) {
                1
            } else { 0 }
        }
    }
}

fn queue_link(window: usize, url: Option<&CefString>, foreground: bool) -> bool {
    let Some(url) = url.map(CefString::to_string) else {
        return false;
    };
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return false;
    }
    OPEN_LINKS.with(|links| {
        let mut links = links.borrow_mut();
        links.push_back((url, foreground));
        if unsafe { PostMessageW(window as HWND, OPEN_LINK, 0, 0) } == 0 {
            links.pop_back();
            false
        } else {
            true
        }
    })
}

wrap_display_handler! {
    struct FlitDisplay { key: usize }
    impl DisplayHandler {
        fn on_address_change(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            if frame.is_some_and(|frame| frame.is_main() != 0) {
                let url = url.map(CefString::to_string).unwrap_or_default();
                update_page(self.key, |tab| tab.url = url);
            }
        }
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            let title = title.map(CefString::to_string).unwrap_or_default();
            update_page(self.key, |tab| tab.title = title);
        }
    }
}

wrap_load_handler! {
    struct FlitLoad { key: usize }
    impl LoadHandler {
        fn on_loading_state_change(&self, _browser: Option<&mut Browser>, _loading: i32, back: i32, forward: i32) {
            update_page(self.key, |tab| { tab.back = back != 0; tab.forward = forward != 0; });
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
            if !matches!(event.type_, KeyEventType::RAWKEYDOWN | KeyEventType::KEYDOWN) {
                return 0;
            }
            let Some(command) = shortcut(
                event.windows_key_code as u16,
                event.modifiers & sys::cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0 as u32 != 0,
                event.modifiers & sys::cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0 as u32 != 0,
                event.modifiers & sys::cef_event_flags_t::EVENTFLAG_ALT_DOWN.0 as u32 != 0,
            ) else { return 0 };
            unsafe { PostMessageW(self.window as HWND, WM_COMMAND, command, 0); }
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
        (ui.bookmark, w!("Open bookmarks")),
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

pub(crate) fn open(initial_url: &str) -> Result<(), &'static str> {
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let classes = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_WIN95_CLASSES,
    };
    let name = wide(WINDOW_CLASS);
    let caption = wide(APP_TITLE);
    let window = unsafe {
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
            exe,
            null(),
        )
    };
    if window.is_null() {
        return Err("Could not create the Flit window.");
    }
    let dpi = unsafe { GetDpiForWindow(window) }.max(96) as i32;
    chrome::apply_backdrop(window, 8 * dpi / 96);
    let ui = Controls {
        window,
        address: edit(window, ADDRESS),
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
        bookmark: button(window, BOOKMARKS, "Bookmarks"),
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
    SHELL.with(|slot| {
        *slot.borrow_mut() = Some(Shell {
            controls: ui,
            tabs: Vec::new(),
            active: None,
            next_key: 1,
            sidebar_open: true,
            filter: String::new(),
            rows: Vec::new(),
            font: null_mut(),
            font_dpi: 0,
            find_open: false,
            recently_closed: Vec::new(),
        });
    });
    tooltips(ui);
    layout();
    unsafe {
        ShowWindow(window, SW_SHOW);
        UpdateWindow(window);
    }
    add_tab(initial_url)?;
    if initial_url == HOME {
        unsafe {
            SetFocus(ui.address);
            SendMessageW(ui.address, EM_SETSEL, 0, -1);
        }
    }
    Ok(())
}

pub(crate) fn startup_failed(message: &'static str) {
    eprintln!("Flit Browser: {message}");
    EXIT_CODE.set(1);
    if let Some(ui) = controls() {
        set_text(ui.window, &format!("{APP_TITLE} — {message}"));
        unsafe {
            PostMessageW(ui.window, WM_CLOSE, 0, 0);
        }
    }
}

pub(crate) fn initialization_failed() -> bool {
    EXIT_CODE.get() != 0 && controls().is_none()
}

pub(crate) fn finish() -> i32 {
    EXIT_CODE.get()
}

pub(crate) fn show_existing() {
    if let Some(ui) = controls() {
        unsafe {
            ShowWindow(ui.window, SW_RESTORE);
            SetForegroundWindow(ui.window);
        }
    }
}

pub(crate) struct Layout {
    pub width: i32,
    pub height: i32,
    pub dpi: i32,
    pub top: i32,
    pub side: i32,
    pub address_frame: RECT,
    pub filter_frame: RECT,
    pub find_frame: RECT,
}

impl Layout {
    fn read(window: HWND, sidebar_open: bool, find_open: bool) -> Self {
        let mut rect = RECT::default();
        unsafe {
            GetClientRect(window, &mut rect);
        }
        let width = rect.right.max(1);
        let height = rect.bottom.max(1);
        let dpi = unsafe { GetDpiForWindow(window) }.max(96) as i32;
        let d = |n| n * dpi / 96;
        let top = d(if find_open { 102 } else { 56 });
        let side = if sidebar_open {
            (width / 4).clamp(d(164), d(238))
        } else {
            0
        };
        Self {
            width,
            height,
            dpi,
            top,
            side,
            address_frame: RECT {
                left: d(173),
                top: d(10),
                right: (width - d(if sidebar_open { 54 } else { 94 })).max(d(245)),
                bottom: d(46),
            },
            filter_frame: RECT {
                left: d(13),
                top: top + d(43),
                right: side - d(13),
                bottom: top + d(74),
            },
            find_frame: if find_open {
                RECT {
                    left: (width - d(340)).max(d(12)),
                    top: d(63),
                    right: width - d(155),
                    bottom: d(94),
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
        let gap = self.scale(8);
        RECT {
            left: self.side + gap,
            top: self.top,
            right: (self.width - gap).max(self.side + gap + 1),
            bottom: (self.height - gap).max(self.top + 1),
        }
    }
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

fn layout() {
    let Some((ui, sidebar_open, find_open, panels)) = with_shell(|s| {
        (
            s.controls,
            s.sidebar_open,
            s.find_open,
            s.tabs
                .iter()
                .map(|t| (t.panel, t.browser.clone()))
                .collect::<Vec<_>>(),
        )
    }) else {
        return;
    };
    let l = Layout::read(ui.window, sidebar_open, find_open);
    let d = |n| l.scale(n);
    update_fonts(ui, l.dpi);
    set_bounds(ui.sidebar, d(11), d(10), d(36), d(36));
    set_bounds(ui.back, d(52), d(10), d(36), d(36));
    set_bounds(ui.forward, d(90), d(10), d(36), d(36));
    set_bounds(ui.reload, d(128), d(10), d(36), d(36));
    let a = l.address_frame;
    set_bounds(
        ui.address,
        a.left + d(14),
        a.top + d(8),
        a.right - a.left - d(74),
        d(21),
    );
    set_bounds(ui.bookmark, a.right - d(59), a.top + d(4), d(25), d(28));
    set_bounds(ui.go, a.right - d(34), a.top + d(4), d(24), d(28));
    set_bounds(ui.menu, l.width - d(46), d(10), d(36), d(36));
    for hwnd in [ui.label, ui.filter, ui.tabs] {
        unsafe {
            ShowWindow(hwnd, if sidebar_open { SW_SHOWNA } else { SW_HIDE });
        }
    }
    update_empty(ui);
    if sidebar_open {
        set_bounds(ui.label, d(17), l.top + d(9), d(52), d(28));
        set_bounds(ui.add, l.side - d(103), l.top + d(8), d(90), d(30));
        let f = l.filter_frame;
        set_bounds(
            ui.filter,
            f.left + d(8),
            f.top + d(6),
            f.right - f.left - d(16),
            d(21),
        );
        set_bounds(
            ui.tabs,
            d(12),
            l.top + d(83),
            l.side - d(24),
            l.height - l.top - d(96),
        );
        set_bounds(ui.empty, d(17), l.top + d(83), l.side - d(34), d(34));
    } else {
        set_bounds(ui.add, l.width - d(86), d(10), d(36), d(36));
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
    let p = l.page_bounds();
    let (width, height) = (p.right - p.left, p.bottom - p.top);
    for (panel, browser) in panels {
        set_bounds(panel, p.left, p.top, width, height);
        if let Some(host) = browser.and_then(|browser| browser.host()) {
            set_bounds(host.window_handle().0.cast(), 0, 0, width, height);
        }
    }
    unsafe {
        InvalidateRect(ui.window, null(), 0);
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
    UPDATING_ADDRESS.set(true);
    set_text(hwnd, if url == HOME { "" } else { url });
    UPDATING_ADDRESS.set(false);
    ADDRESS_EDITED.set(false);
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
    let visible = with_shell(|s| s.sidebar_open && !s.filter.is_empty() && s.rows.is_empty());
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
                .then_some(Row {
                    key: tab.key,
                    label,
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
            if with_shell(|s| s.sidebar_open) == Some(false) {
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
    let Some((ui, url, title, back, forward)) = with_shell(|s| {
        let tab = s.tabs.iter().find(|t| Some(t.key) == s.active)?;
        Some((
            s.controls,
            tab.url.clone(),
            clean(&tab.title, 100),
            tab.back,
            tab.forward,
        ))
    })
    .flatten() else {
        return;
    };
    if force_address
        || (!ADDRESS_EDITED.get()
            && text(ui.address, 16_384) != if url == HOME { "" } else { &url })
    {
        show_address(ui.address, &url);
    }
    let caption = if title.is_empty() {
        APP_TITLE.into()
    } else {
        format!("{APP_TITLE} — {title}")
    };
    if text(ui.window, 128) != caption {
        set_text(ui.window, &caption);
    }
    unsafe {
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
            .is_some_and(|tab| Some(tab.key) != s.active)
    }) == Some(true)
    {
        close_find(false);
    }
    let Some(panels) = change(|s| {
        s.active = Some(s.tabs.get(index)?.key);
        Some(
            s.tabs
                .iter()
                .enumerate()
                .map(|(i, t)| (t.panel, i == index))
                .collect::<Vec<_>>(),
        )
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
}

fn add_tab(url: &str) -> Result<(), &'static str> {
    open_tab(url, true)
}

fn open_tab(url: &str, foreground: bool) -> Result<(), &'static str> {
    let (ui, key) = change(|s| {
        let key = s.next_key;
        s.next_key += 1;
        (s.controls, key)
    })
    .ok_or("The Flit window is not available.")?;
    let (sidebar, find) = with_shell(|s| (s.sidebar_open, s.find_open)).unwrap_or((true, false));
    let l = Layout::read(ui.window, sidebar, find);
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
            url: url.to_owned(),
            title: String::new(),
            back: false,
            forward: false,
        });
        s.tabs.len() - 1
    })
    .ok_or("The Flit window closed during tab creation.")?;
    if url != HOME {
        materialize(key)?;
    }
    if foreground || with_shell(|s| s.active.is_none()) == Some(true) {
        select(index);
    } else {
        refresh_rows(false);
    }
    if url != HOME {
        if let Some(browser) =
            with_shell(|s| s.tabs.iter().find(|tab| tab.key == key)?.browser.clone()).flatten()
        {
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&CefString::from(url)));
            }
        }
    }
    Ok(())
}

fn materialize(key: usize) -> Result<Browser, &'static str> {
    let (ui, panel, existing) = with_shell(|s| {
        let tab = s.tabs.iter().find(|tab| tab.key == key)?;
        Some((s.controls, tab.panel, tab.browser.clone()))
    })
    .flatten()
    .ok_or("The tab is no longer available.")?;
    if let Some(browser) = existing {
        return Ok(browser);
    }
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(panel, &mut rect);
    }
    let (width, height) = (rect.right.max(1), rect.bottom.max(1));
    let mut win = WindowInfo {
        runtime_style: RuntimeStyle::CHROME,
        ..Default::default()
    }
    .set_as_child(
        sys::HWND(panel.cast()),
        &Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
    );
    win.ex_style |= WS_EX_NOACTIVATE;
    let mut client = FlitClient::new(key, ui.window as usize);
    let Some(browser) = browser_host_create_browser_sync(
        Some(&win),
        Some(&mut client),
        Some(&CefString::from(HOME)),
        Some(&BrowserSettings::default()),
        None,
        None,
    ) else {
        unsafe {
            InvalidateRect(panel, null(), 1);
        }
        return Err("CEF could not create the browser tab.");
    };
    let usable = browser.host().is_some_and(|host| {
        host.runtime_style() == RuntimeStyle::CHROME && !host.window_handle().0.is_null()
    });
    change(|s| {
        if let Some(tab) = s.tabs.iter_mut().find(|tab| tab.key == key) {
            tab.browser = Some(browser.clone());
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
    let Some(key) = with_shell(|s| s.active).flatten() else {
        return;
    };
    let browser = match materialize(key) {
        Ok(browser) => browser,
        Err(message) => {
            set_text(ui.window, &format!("{APP_TITLE} — {message}"));
            return;
        }
    };
    let raw = text(ui.address, 16_384);
    let url = match navigation::address(&raw) {
        Ok(url) => url,
        Err(message) => {
            let label = wide(message);
            unsafe {
                SendMessageW(
                    ui.address,
                    EM_SHOWBALLOONTIP,
                    0,
                    (&EDITBALLOONTIP {
                        cbStruct: std::mem::size_of::<EDITBALLOONTIP>() as u32,
                        pszTitle: wide(APP_TITLE).as_ptr(),
                        pszText: label.as_ptr(),
                        ttiIcon: TTI_WARNING as i32,
                    } as *const EDITBALLOONTIP) as isize,
                );
                SetFocus(ui.address);
            }
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

fn closed(key: usize) {
    let Some((window, panel, next, was_active)) = change(|s| {
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
        Some((s.controls.window, tab.panel, next, Some(key) == s.active))
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
    if let Some(next) = next {
        if was_active {
            select(next);
        } else {
            refresh_rows(false);
        }
    } else {
        unsafe {
            DestroyWindow(window);
        }
    }
}

fn close_tab(key: Option<usize>) {
    let Some((key, browser)) = with_shell(|s| {
        let key = key.or(s.active)?;
        let tab = s.tabs.iter().find(|t| t.key == key)?;
        Some((key, tab.browser.clone()))
    })
    .flatten() else {
        return;
    };
    if let Some(host) = browser.and_then(|b| b.host()) {
        host.close_browser(0);
    } else {
        closed(key);
    }
}

fn close_window(window: HWND) {
    close_find(false);
    let panels = change(|s| {
        let panels = s
            .tabs
            .iter()
            .filter(|t| t.browser.is_none())
            .map(|t| t.panel)
            .collect::<Vec<_>>();
        s.tabs.retain(|t| t.browser.is_some());
        panels
    })
    .unwrap_or_default();
    for panel in panels {
        unsafe {
            DestroyWindow(panel);
        }
    }
    let browsers = with_shell(|s| {
        s.tabs
            .iter()
            .filter_map(|tab| tab.browser.clone())
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    if browsers.is_empty() {
        unsafe {
            DestroyWindow(window);
        }
    } else {
        for browser in browsers {
            if let Some(host) = browser.host() {
                host.close_browser(0);
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
    let Some((ui, open)) = change(|s| {
        s.sidebar_open = !s.sidebar_open;
        (s.controls, s.sidebar_open)
    }) else {
        return;
    };
    if !open {
        let focus = unsafe { GetFocus() };
        if focus == ui.filter || focus == ui.tabs {
            focus_control(ui.sidebar, false);
        }
    }
    layout();
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
    }
}

fn host() -> Option<BrowserHost> {
    active().and_then(|b| b.host())
}

fn open_builtin(url: &'static str) {
    let existing =
        with_shell(|s| s.tabs.iter().find(|tab| tab.url == url).map(|t| t.key)).flatten();
    if let Some(key) = existing {
        select_key(key);
    } else if let Err(message) = add_tab(url) {
        if let Some(ui) = controls() {
            set_text(ui.window, &format!("{APP_TITLE} — {message}"));
        }
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
    add(RESTORE_TAB, "Reopen closed tab\tCtrl+Shift+T", restore);
    add(FOCUS_FILTER, "Search open tabs\tCtrl+Shift+A", true);
    separator();
    add(BOOKMARKS, "Bookmarks", true);
    add(HISTORY, "History\tCtrl+H", true);
    add(DOWNLOADS, "Downloads\tCtrl+J", true);
    separator();
    add(FIND, "Find in page\tCtrl+F", true);
    add(PRINT, "Print…\tCtrl+P", true);
    separator();
    add(SETTINGS, "Settings", true);
    add(PASSWORDS, "Password manager", true);
    add(EXTENSIONS, "Extensions", true);
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

fn shortcut(key: u16, ctrl: bool, shift: bool, alt: bool) -> Option<usize> {
    let command = match (key, ctrl, shift, alt) {
        (0x4C, true, false, false) => FOCUS_ADDRESS, // Ctrl+L
        (0x54, true, false, false) => NEW_TAB,       // Ctrl+T
        (0x54, true, true, false) => RESTORE_TAB,
        (0x57, true, false, false) => CLOSE_TAB, // Ctrl+W
        (0x52, true, false, false) | (VK_F5, false, false, false) => RELOAD,
        (VK_LEFT, false, false, true) => BACK,
        (VK_RIGHT, false, false, true) => FORWARD,
        (VK_TAB, true, false, false) => NEXT_TAB,
        (VK_TAB, true, true, false) => PREVIOUS_TAB,
        (0x41, true, true, false) => FOCUS_FILTER, // Ctrl+Shift+A
        (0x42, true, true, false) => SIDEBAR,      // Ctrl+Shift+B
        (0x46, true, false, false) => FIND,
        (0x48, true, false, false) => HISTORY,
        (0x4A, true, false, false) => DOWNLOADS,
        (0x50, true, false, false) => PRINT,
        _ => return None,
    };
    Some(command)
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
    }
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
    let id = unsafe { GetDlgCtrlID(hwnd) } as usize;
    if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN) {
        let key_down = |key: u16| unsafe { GetKeyState(key as i32) < 0 };
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        let alt = key_down(VK_MENU);
        if let Some(command) = shortcut(wp as u16, ctrl, shift, alt) {
            unsafe {
                PostMessageW(parent as HWND, WM_COMMAND, command, 0);
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
            WM_LBUTTONDOWN | WM_MBUTTONDOWN => {
                if let Some(key) = tab_point(hwnd, lp, message == WM_LBUTTONDOWN) {
                    ROW_CLOSE.set(Some((key, message)));
                    unsafe {
                        SetCapture(hwnd);
                    }
                    return 0;
                }
            }
            WM_LBUTTONUP | WM_MBUTTONUP => {
                if let Some((key, button)) = ROW_CLOSE.take() {
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
            WM_CAPTURECHANGED | WM_CANCELMODE => ROW_CLOSE.set(None),
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
    match message {
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
            let (sidebar, find) = with_shell(|s| (s.sidebar_open, s.find_open)).unwrap_or_default();
            chrome::paint_shell(window, Layout::read(window, sidebar, find));
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
                let label = with_shell(|s| {
                    s.rows
                        .get(item.itemID as usize)
                        .map(|row| row.label.clone())
                })
                .flatten();
                chrome::draw_tab(item, label.as_deref().unwrap_or_default(), dpi);
                return 1;
            }
            let kind = match wp {
                SIDEBAR => ButtonKind::Sidebar,
                BACK => ButtonKind::Back,
                FORWARD => ButtonKind::Forward,
                RELOAD => ButtonKind::Reload,
                GO => ButtonKind::Go,
                BOOKMARKS => ButtonKind::Bookmark,
                MENU => ButtonKind::Menu,
                NEW_TAB if with_shell(|s| s.sidebar_open) == Some(false) => {
                    ButtonKind::NewTabCompact
                }
                NEW_TAB => ButtonKind::NewTab,
                _ => return unsafe { DefWindowProcW(window, message, wp, lp) },
            };
            chrome::draw_button(item, kind, dpi);
            1
        }
        WM_CTLCOLOREDIT => chrome::field_colors(wp as HDC, false) as isize,
        WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC => chrome::field_colors(wp as HDC, true) as isize,
        WM_COMMAND => {
            let id = wp & 0xffff;
            let notification = ((wp >> 16) & 0xffff) as u32;
            match id {
                GO => navigate(),
                NEW_TAB => {
                    if let Some(ui) = controls() {
                        if !text(ui.filter, 256).is_empty() {
                            set_text(ui.filter, "");
                        }
                    }
                    if let Err(message) = add_tab(HOME) {
                        set_text(window, &format!("{APP_TITLE} — {message}"));
                    } else if let Some(ui) = controls() {
                        focus_control(ui.address, true);
                    }
                }
                BACK => {
                    close_find(false);
                    refresh(true);
                    if let Some(b) = active() {
                        b.go_back();
                    }
                }
                FORWARD => {
                    close_find(false);
                    refresh(true);
                    if let Some(b) = active() {
                        b.go_forward();
                    }
                }
                RELOAD => {
                    refresh(true);
                    if let Some(b) = active() {
                        b.reload();
                    }
                }
                CLOSE_TAB => close_tab(None),
                RESTORE_TAB => {
                    if let Some(url) = change(|s| s.recently_closed.pop()).flatten() {
                        let _ = add_tab(&url);
                    }
                }
                SIDEBAR => toggle_sidebar(),
                FOCUS_ADDRESS => {
                    if let Some(ui) = controls() {
                        focus_control(ui.address, true);
                    }
                }
                FOCUS_FILTER => {
                    if with_shell(|s| s.sidebar_open) == Some(false) {
                        toggle_sidebar();
                    }
                    if let Some(ui) = controls() {
                        focus_control(ui.filter, true);
                    }
                }
                NEXT_TAB | PREVIOUS_TAB => cycle_tab(id == PREVIOUS_TAB),
                MENU => {
                    if let Some(ui) = controls() {
                        show_menu(ui);
                    }
                }
                HISTORY => open_builtin("chrome://history/"),
                BOOKMARKS => open_builtin("chrome://bookmarks/"),
                DOWNLOADS => open_builtin("chrome://downloads/"),
                SETTINGS => open_builtin("chrome://settings/"),
                EXTENSIONS => open_builtin("chrome://extensions/"),
                PASSWORDS => open_builtin("chrome://password-manager/"),
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
                FIND => toggle_find(),
                FIND_PREVIOUS | FIND_NEXT => find_in_page(id == FIND_NEXT, true),
                FIND_CLOSE => close_find(true),
                PRINT => {
                    if let Some(host) = host() {
                        host.print();
                    }
                }
                ADDRESS if notification == EN_CHANGE && !UPDATING_ADDRESS.get() => {
                    ADDRESS_EDITED.set(true);
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
        OPEN_LINK => {
            let link = OPEN_LINKS.with(|links| links.borrow_mut().pop_front());
            if let Some((url, foreground)) = link {
                let _ = open_tab(&url, foreground);
            }
            0
        }
        PAGE_CHANGED => {
            refresh(false);
            0
        }
        PAGE_CLOSED => {
            closed(wp);
            0
        }
        WM_CLOSE => {
            close_window(window);
            0
        }
        WM_DESTROY => 0,
        WM_NCDESTROY => {
            let state = SHELL.with(|s| s.borrow_mut().take());
            if let Some(state) = state {
                if !state.font.is_null() {
                    unsafe {
                        DeleteObject(state.font);
                    }
                }
                quit_message_loop();
            }
            unsafe { DefWindowProcW(window, message, wp, lp) }
        }
        _ => unsafe { DefWindowProcW(window, message, wp, lp) },
    }
}
