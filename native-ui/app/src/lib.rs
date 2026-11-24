#![cfg(target_os = "windows")]

mod access;
#[path = "../../../src/exact_model.rs"]
mod model;

use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    mem::size_of,
    ptr::{null, null_mut},
    rc::Rc,
    time::Instant,
};

use exact_painted::{Presenter, presenter::PainterChoice};
use exact_runner::{DataError, DataSource, Event, Value};
use model::{ACTION_MESSAGE, Surface};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::WM_MOUSELEAVE, HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*,
        WindowsAndMessaging::*,
    },
};

const PLAN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/surface.plan"));
const CLASS: windows_sys::core::PCWSTR = windows_sys::w!("FlitExactSurface");
const FRAME_TIMER: usize = 1;
const ACCESS_ACTION: u32 = WM_APP + 1;

#[derive(Clone)]
struct Source {
    surface: Rc<RefCell<Surface>>,
    owner: HWND,
    child: Rc<Cell<HWND>>,
}

impl Source {
    fn model(&self) -> Value {
        let surface = self.surface.borrow();
        let sections = surface.sections.iter().map(|section| {
            let items = section.items.iter().map(|item| {
                let actions = item.actions.iter().map(|action| {
                    record([
                        Value::Number(action.id as f64),
                        Value::str(&action.label),
                        Value::Bool(action.selected),
                        Value::str(match action.kind {
                            model::ActionKind::Link => "link",
                            model::ActionKind::Toggle => "toggle",
                            model::ActionKind::Choice => "choice",
                        }),
                    ])
                });
                record([
                    Value::str(&item.id),
                    Value::str(&item.title),
                    Value::str(&item.detail),
                    list(actions),
                ])
            });
            record([
                Value::str(&section.id),
                Value::str(&section.title),
                list(items),
            ])
        });
        record([
            Value::str(&surface.title),
            Value::str(&surface.subtitle),
            Value::Bool(surface.dark),
            Value::Bool(surface.compact),
            Value::Bool(surface.start),
            list(sections),
        ])
    }

    fn invoke(&self, value: f64) -> bool {
        let action = value as usize;
        let belongs = action as f64 == value
            && self
                .surface
                .borrow()
                .sections
                .iter()
                .flat_map(|section| &section.items)
                .flat_map(|item| &item.actions)
                .any(|entry| entry.id == action);
        let child = self.child.get();
        belongs
            && !child.is_null()
            && unsafe { PostMessageW(self.owner, ACTION_MESSAGE, action, child as LPARAM) } != 0
    }
}

fn record<const N: usize>(fields: [Value; N]) -> Value {
    Value::Record(Vec::from(fields).into())
}

fn list(values: impl Iterator<Item = Value>) -> Value {
    Value::List(values.collect::<Vec<_>>().into())
}

impl DataSource for Source {
    fn query(&mut self, name: &str, args: &[Value]) -> Result<Value, DataError> {
        match (name, args) {
            ("readSurface", [Value::Number(_)]) => Ok(self.model()),
            ("invokeAction", [Value::Number(id)]) if self.invoke(*id) => Ok(Value::Bool(true)),
            ("readSurface" | "invokeAction", _) => Err(DataError::BadArguments(name.into())),
            _ => Err(DataError::UnknownSource(name.into())),
        }
    }
}

struct Panel {
    source: Source,
    presenter: Presenter<Source>,
    start: Instant,
    pixels: Vec<u32>,
    size: (i32, i32, f32),
    pressed: bool,
    tracking_mouse: bool,
    access: Option<access::Access>,
}

impl Panel {
    fn new(owner: HWND, surface: Surface) -> Result<Self, String> {
        let source = Source {
            surface: Rc::new(RefCell::new(surface)),
            owner,
            child: Rc::new(Cell::new(null_mut())),
        };
        let assets = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(ToOwned::to_owned))
            .unwrap_or_default();
        let (presenter, error) = Presenter::boot_with(
            PLAN,
            source.clone(),
            (480.0, 360.0),
            1.0,
            assets,
            PainterChoice::Cpu,
        )
        .map_err(|error| error.to_string())?;
        if let Some(error) = error {
            return Err(error);
        }
        let mut panel = Self {
            source,
            presenter,
            start: Instant::now(),
            pixels: Vec::new(),
            size: (0, 0, 1.0),
            pressed: false,
            tracking_mouse: false,
            access: None,
        };
        panel.settle();
        Ok(panel)
    }

    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    fn settle(&mut self) {
        self.presenter.run_commands(|| self.source.clone());
        self.presenter.tick(self.now());
    }

    fn accessibility_snapshot(&mut self) -> access::Snapshot {
        let (width, height, scale) = self.size;
        access::Snapshot::capture(
            &mut self.presenter,
            &self.source.surface.borrow().title,
            (width, height),
            scale,
        )
    }

    fn model_update(&mut self, surface: Surface) -> Result<(), String> {
        if *self.source.surface.borrow() == surface {
            return Ok(());
        }
        *self.source.surface.borrow_mut() = surface;
        let root = {
            let kernel = self.presenter.host().kernel();
            kernel
                .find_by_test_id("flit-surface")
                .first()
                .and_then(|&key| kernel.node_by_key(key))
                .map(|node| node.id)
                .ok_or("Exact surface root is unavailable")?
        };
        let now = self.now();
        if let Some(error) = self
            .presenter
            .host_mut()
            .dispatch_at(root, Event::Refresh, now)
        {
            return Err(error);
        }
        self.settle();
        Ok(())
    }

    fn paint(&mut self, width: i32, height: i32, scale: f32) -> Result<&[u32], String> {
        if self.size != (width, height, scale) {
            if let Some(error) =
                self.presenter
                    .resize_scaled(width as f32 / scale, height as f32 / scale, scale)
            {
                return Err(error);
            }
            self.size = (width, height, scale);
        } else if !self.presenter.dirty() && !self.pixels.is_empty() {
            return Ok(&self.pixels);
        }

        let frame = self.presenter.frame();
        if !self.presenter.last_frame_succeeded()
            || (frame.width(), frame.height()) != (width as u32, height as u32)
        {
            self.presenter.recycle_frame(frame);
            return Err("Exact could not paint the panel bounds".into());
        }

        self.pixels.clear();
        self.pixels.extend(
            frame
                .data()
                .chunks_exact(4)
                .map(|pixel| u32::from_be_bytes([0, pixel[0], pixel[1], pixel[2]])),
        );
        self.presenter.recycle_frame(frame);
        Ok(&self.pixels)
    }
}

struct WindowPanel {
    handle: Rc<Cell<HWND>>,
    contents: RefCell<Panel>,
}

impl WindowPanel {
    fn new(contents: Panel) -> Self {
        Self {
            handle: contents.source.child.clone(),
            contents: RefCell::new(contents),
        }
    }
}

fn window_panel(hwnd: HWND) -> Option<Rc<WindowPanel>> {
    let storage = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowPanel;
    if storage.is_null() {
        return None;
    }
    // This HWND is owned by the UI thread. Its state has one raw Rc until WM_NCDESTROY.
    unsafe {
        Rc::increment_strong_count(storage);
        Some(Rc::from_raw(storage))
    }
}

fn edit<T>(hwnd: HWND, work: impl FnOnce(&mut Panel) -> T) -> Option<T> {
    let state = window_panel(hwnd)?;
    let mut panel = state.contents.try_borrow_mut().ok()?;
    Some(work(&mut panel))
}

fn redraw(hwnd: HWND) {
    let Some((dirty, animating)) = edit(hwnd, |panel| {
        (
            panel.presenter.dirty(),
            panel.presenter.needs_animation_frame(),
        )
    }) else {
        return;
    };
    unsafe {
        if dirty {
            InvalidateRect(hwnd, null(), 0);
        }
        if animating && IsWindowVisible(hwnd) != 0 {
            SetTimer(hwnd, FRAME_TIMER, 16, None);
        } else {
            KillTimer(hwnd, FRAME_TIMER);
        }
    }
}

fn publish_accessibility(hwnd: HWND) {
    let events = edit(hwnd, |panel| {
        let snapshot = panel.accessibility_snapshot();
        panel.access.as_mut()?.publish(snapshot)
    })
    .flatten();
    if let Some(events) = events {
        events.raise();
    }
}

fn pointer_position(lp: LPARAM) -> POINT {
    POINT {
        x: i32::from(lp as i16),
        y: i32::from((lp >> 16) as i16),
    }
}

fn pointer(hwnd: HWND, msg: u32, lp: LPARAM) {
    let at = pointer_position(lp);
    if msg == WM_LBUTTONDOWN {
        unsafe {
            if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_TABSTOP != 0 {
                SetFocus(hwnd);
            }
            SetCapture(hwnd);
        }
    }
    edit(hwnd, |panel| {
        let now = panel.now();
        let scale = panel.size.2;
        let (x, y) = (at.x as f32 / scale, at.y as f32 / scale);
        let _ = match msg {
            WM_LBUTTONDOWN => {
                panel.pressed = true;
                panel.presenter.pointer_down(x, y, now)
            }
            WM_LBUTTONUP => {
                panel.pressed = false;
                panel.presenter.pointer_up(x, y, now)
            }
            WM_MOUSEMOVE => {
                if !panel.tracking_mouse {
                    let mut event = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    panel.tracking_mouse = unsafe { TrackMouseEvent(&mut event) } != 0;
                }
                panel.presenter.pointer_move(x, y, now)
            }
            _ => unreachable!(),
        };
        panel.settle();
    });
    if msg == WM_LBUTTONUP && unsafe { GetCapture() } == hwnd {
        unsafe { ReleaseCapture() };
    }
    redraw(hwnd);
}

fn keyboard(hwnd: HWND, down: bool, wp: WPARAM, lp: LPARAM) -> bool {
    let (code, key) = match wp as u16 {
        VK_SPACE => ("Space", " "),
        VK_RETURN => ("Enter", "Enter"),
        VK_TAB => ("Tab", "Tab"),
        VK_UP => ("ArrowUp", "ArrowUp"),
        VK_DOWN => ("ArrowDown", "ArrowDown"),
        VK_LEFT => ("ArrowLeft", "ArrowLeft"),
        VK_RIGHT => ("ArrowRight", "ArrowRight"),
        VK_HOME => ("Home", "Home"),
        VK_END => ("End", "End"),
        VK_PRIOR => ("PageUp", "PageUp"),
        VK_NEXT => ("PageDown", "PageDown"),
        _ => return false,
    };
    if [VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .into_iter()
        .any(|key| unsafe { GetKeyState(i32::from(key)) } < 0)
    {
        return false;
    }
    edit(hwnd, |panel| {
        let shift = unsafe { GetKeyState(i32::from(VK_SHIFT)) } < 0;
        panel.presenter.hold_modifier("ShiftLeft", shift);
        panel
            .presenter
            .hardware_key(code, key, down, lp & (1 << 30) != 0);
        panel.settle();
    });
    redraw(hwnd);
    true
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(hwnd, &mut ps) };
    let mut client = windows_sys::Win32::Foundation::RECT::default();
    unsafe { GetClientRect(hwnd, &mut client) };
    let (width, height) = (client.right, client.bottom);
    if !dc.is_null() && width > 0 && height > 0 {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        let ok = edit(hwnd, |panel| {
            let bits = panel.paint(width, height, scale)?;
            let format = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let lines = unsafe {
                StretchDIBits(
                    dc,
                    0,
                    0,
                    width,
                    height,
                    0,
                    0,
                    width,
                    height,
                    bits.as_ptr().cast(),
                    &format,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                )
            };
            Ok::<_, String>(lines > 0)
        });
        if !matches!(ok, Some(Ok(true))) {
            unsafe { FillRect(dc, &client, GetSysColorBrush(COLOR_WINDOW)) };
        }
    }
    unsafe { EndPaint(hwnd, &ps) };
}

unsafe extern "system" fn surface_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => unsafe {
            let create = &*(lp as *const CREATESTRUCTW);
            let pending = &*(create.lpCreateParams as *const Rc<WindowPanel>);
            pending.handle.set(hwnd);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::into_raw(pending.clone()) as isize);
            1
        },
        WM_NCDESTROY => unsafe {
            let storage = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *const WindowPanel;
            if !storage.is_null() {
                let state = Rc::from_raw(storage);
                state.handle.set(null_mut());
            }
            KillTimer(hwnd, FRAME_TIMER);
            DefWindowProcW(hwnd, msg, wp, lp)
        },
        WM_GETOBJECT => {
            let response = edit(hwnd, |panel| panel.access.as_mut()?.object(wp, lp)).flatten();
            if let Some(response) = response {
                let result: accesskit_windows::LRESULT = response.into();
                return result.0;
            }
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
        }
        WM_PAINT => {
            unsafe { paint(hwnd) };
            publish_accessibility(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_SIZE | WM_DPICHANGED_AFTERPARENT => {
            unsafe { InvalidateRect(hwnd, null(), 0) };
            0
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            if let Some(events) = edit(hwnd, |panel| {
                panel.access.as_mut()?.focus(msg == WM_SETFOCUS)
            })
            .flatten()
            {
                events.raise();
            }
            unsafe { InvalidateRect(hwnd, null(), 0) };
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE => {
            pointer(hwnd, msg, lp);
            0
        }
        WM_CAPTURECHANGED | WM_CANCELMODE | WM_MOUSELEAVE => {
            edit(hwnd, |panel| {
                let now = panel.now();
                if msg == WM_MOUSELEAVE {
                    panel.tracking_mouse = false;
                    if !panel.pressed {
                        let _ = panel.presenter.pointer_move(-1.0, -1.0, now);
                    }
                } else if std::mem::take(&mut panel.pressed) {
                    let _ = panel.presenter.pointer_cancel(now);
                }
                panel.settle();
            });
            redraw(hwnd);
            0
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let mut at = pointer_position(lp);
            unsafe { ScreenToClient(hwnd, &mut at) };
            let delta = f32::from((wp >> 16) as i16) / 120.0 * 48.0;
            edit(hwnd, |panel| {
                let (x, y) = (at.x as f32 / panel.size.2, at.y as f32 / panel.size.2);
                let (dx, dy) = if msg == WM_MOUSEHWHEEL {
                    (delta, 0.0)
                } else {
                    (0.0, -delta)
                };
                panel.presenter.wheel_at(x, y, dx, dy);
                panel.settle();
            });
            redraw(hwnd);
            0
        }
        WM_TIMER if wp == FRAME_TIMER => {
            edit(hwnd, Panel::settle);
            redraw(hwnd);
            0
        }
        ACCESS_ACTION => {
            let focus = edit(hwnd, |panel| {
                let Some(action) = panel
                    .access
                    .as_ref()
                    .and_then(|access| access.decode_action(msg, wp, lp))
                else {
                    return false;
                };
                if !action.apply(&mut panel.presenter) {
                    return false;
                }
                panel.settle();
                action.wants_window_focus()
            }) == Some(true);
            if focus && unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32 & WS_TABSTOP != 0 {
                unsafe { SetFocus(hwnd) };
            }
            redraw(hwnd);
            publish_accessibility(hwnd);
            0
        }
        WM_GETDLGCODE => (DLGC_WANTTAB | DLGC_WANTARROWS | DLGC_WANTCHARS) as LRESULT,
        WM_KEYDOWN | WM_KEYUP if keyboard(hwnd, msg == WM_KEYDOWN, wp, lp) => 0,
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

unsafe fn model_from_bytes(bytes: *const u8, len: usize) -> Option<Surface> {
    if bytes.is_null() || len == 0 || len > 256 * 1024 {
        return None;
    }
    serde_json::from_slice(unsafe { std::slice::from_raw_parts(bytes, len) }).ok()
}

/// Flit calls this on the owner HWND's UI thread with a serialized surface.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn flit_exact_create_v1(
    parent: HWND,
    owner: HWND,
    id: usize,
    bytes: *const u8,
    len: usize,
) -> HWND {
    let Some(surface) = (unsafe { model_from_bytes(bytes, len) }) else {
        return null_mut();
    };
    let tab_stop = if surface.preserve_input_focus { 0 } else { WS_TABSTOP };
    let panel = match Panel::new(owner, surface) {
        Ok(panel) => panel,
        Err(error) => {
            eprintln!("Flit Exact: {error}");
            return null_mut();
        }
    };
    let instance = unsafe { GetModuleHandleW(null()) };
    let class = WNDCLASSW {
        lpfnWndProc: Some(surface_proc),
        hInstance: instance,
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        lpszClassName: CLASS,
        ..Default::default()
    };
    unsafe { RegisterClassW(&class) };
    let mut client = windows_sys::Win32::Foundation::RECT::default();
    unsafe { GetClientRect(parent, &mut client) };
    let state = Rc::new(WindowPanel::new(panel));
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED,
            CLASS,
            null(),
            WS_CHILD | tab_stop | WS_CLIPSIBLINGS,
            0,
            0,
            client.right,
            client.bottom,
            parent,
            id as *mut c_void,
            instance,
            (&raw const state).cast(),
        )
    };
    if hwnd.is_null() || state.handle.get() != hwnd {
        return null_mut();
    }

    // An opaque child surface keeps Exact's GDI pixels visible inside the extended DWM frame.
    if unsafe { SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA) } == 0 {
        unsafe { DestroyWindow(hwnd) };
        return null_mut();
    }
    let snapshot = state.contents.borrow_mut().accessibility_snapshot();
    // UIA initialization may dispatch a pending navigation that closes this very window.
    let access = access::Access::attach(hwnd, ACCESS_ACTION, snapshot);
    if state.handle.get() != hwnd {
        return null_mut();
    }
    state.contents.borrow_mut().access = Some(access);
    unsafe { ShowWindow(hwnd, SW_SHOWNA) };
    state.handle.get()
}

/// The HWND must have been returned by flit_exact_create_v1 on this thread.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn flit_exact_update_v1(
    hwnd: HWND,
    bytes: *const u8,
    len: usize,
) -> i32 {
    let Some(surface) = (unsafe { model_from_bytes(bytes, len) }) else {
        return 0;
    };
    let updated = edit(hwnd, |panel| panel.model_update(surface));
    redraw(hwnd);
    match updated {
        Some(Ok(())) => 1,
        Some(Err(error)) => {
            eprintln!("Flit Exact: {error}");
            0
        }
        None => 0,
    }
}
