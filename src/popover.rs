//! Flit's owned toolbar popups, address suggestions, and Vault sheet.

use crate::exact_model::Surface;
use cef::*;
use std::{
    cell::RefCell,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{
        Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute},
        Gdi::{
            CreateRoundRectRgn, DeleteObject, GetMonitorInfoW, InvalidateRect,
            MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect, SetWindowRgn,
        },
    },
    System::Threading::GetCurrentThreadId,
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetFocus, IsWindowEnabled, SetFocus, VK_ESCAPE, VK_LBUTTON,
            VK_MBUTTON, VK_RBUTTON, VK_XBUTTON1, VK_XBUTTON2,
        },
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "FlitPopover";
const CLOSE_POPUP: u32 = WM_APP + 1;
const NATIVE_SUBCLASS: usize = 1;
const ACTIVATION_TIMER: usize = 1;
const REVEAL_TIMER: usize = 2;
const OPEN_MS: u32 = 148;
const CLOSE_MS: u32 = 108;
const ACTIVATION_GRACE_MS: u32 = 250;
pub(crate) const ADDRESS_KIND: &str = "address-suggestions";

#[derive(Clone, Copy)]
pub(crate) enum Anchor {
    /// Right-aligned under a toolbar button (screen coordinates).
    Below(HWND),
    /// Left-aligned under the address field (screen coordinates).
    Address(RECT),
    /// Centered near the top of the owner window, like a sheet.
    Top,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    hwnd: HWND,
    generation: usize,
}

#[derive(Clone, Copy)]
struct Placement {
    owner: HWND,
    focus: HWND,
    kind: &'static str,
    anchor: Anchor,
    width: i32,
    height: i32,
}

struct Popup {
    id: Identity,
    placement: Placement,
    content: Content,
    visible: bool,
    retiring: bool,
    dismiss_after: Option<Instant>,
    reveal: Option<Reveal>,
}

struct Reveal {
    y: i32,
    travel: i32,
    opacity: f32,
    transition: Option<Transition>,
}

#[derive(Clone, Copy)]
struct Transition {
    started: Instant,
    from: f32,
    closing: bool,
}

impl Transition {
    fn sample(self) -> (f32, bool) {
        let duration = if self.closing { CLOSE_MS } else { OPEN_MS };
        let t = (self.started.elapsed().as_secs_f32() * 1000.0 / duration as f32).min(1.0);
        let opacity = if self.closing {
            self.from * (1.0 - t * t)
        } else {
            1.0 - (1.0 - self.from) * (1.0 - t).powi(3)
        };
        (opacity, t == 1.0)
    }
}

enum Content {
    Pending,
    Native(HWND),
    Extension(Browser),
}

impl Popup {
    fn live(&self) -> bool {
        let closing = self.reveal.as_ref().and_then(|r| r.transition)
            .is_some_and(|t| t.closing);
        self.visible && !self.retiring && !closing
    }

    fn browser(&self) -> Option<&Browser> {
        match &self.content {
            Content::Extension(browser) => Some(browser),
            _ => None,
        }
    }

    fn has_browser(&self, id: i32) -> bool {
        self.browser()
            .is_some_and(|browser| browser.identifier() == id)
    }

    fn native(&self) -> Option<HWND> {
        match self.content {
            Content::Native(child) => Some(child),
            _ => None,
        }
    }
}

struct State {
    popup: Option<Popup>,
    generation: usize,
    closing: Vec<i32>,
    toggle: Option<(Instant, HWND, &'static str)>,
    extension_anchor: Option<Placement>,
    stopping: bool,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State {
        popup: None, generation: 0, closing: Vec::new(), toggle: None,
        extension_anchor: None, stopping: false,
    }) };
}

fn nonactivating(kind: &str) -> bool {
    matches!(kind, "vault" | ADDRESS_KIND)
}

fn popup_id(hwnd: HWND) -> Option<Identity> {
    STATE.with_borrow(|s| s.popup.as_ref().filter(|p| p.id.hwnd == hwnd).map(|p| p.id))
}

pub(crate) fn trigger_active(button: HWND) -> bool {
    STATE.with_borrow(|s| s.popup.as_ref().is_some_and(|p| {
        p.live() && matches!(p.placement.anchor, Anchor::Below(trigger) if trigger == button)
    }))
}

fn redraw_trigger(anchor: Anchor) {
    if let Anchor::Below(button) = anchor {
        unsafe { InvalidateRect(button, null(), 0) };
    }
}

fn toggle_off(owner: HWND, kind: &str) -> bool {
    let (open, just_closed) = STATE.with_borrow_mut(|s| {
        let open = s
            .popup
            .as_ref()
            .is_some_and(|p| p.live() && p.placement.kind == kind && p.placement.owner == owner);
        let just_closed = s
            .toggle
            .take()
            .is_some_and(|(time, previous_owner, previous_kind)| {
                owner == previous_owner
                    && kind == previous_kind
                    && time.elapsed() < Duration::from_millis(300)
            });
        (open, just_closed)
    });
    if open {
        if let Some(id) = STATE.with_borrow(|s| s.popup.as_ref().map(|p| p.id)) {
            dismiss(id, true);
        }
    }
    open || just_closed
}

fn start(placement: Placement) -> Result<Identity, &'static str> {
    let (stopping, previous) = STATE.with_borrow(|s| (s.stopping, s.popup.as_ref().map(|p| p.id)));
    if stopping
        || unsafe { GetWindowThreadProcessId(placement.owner, null_mut()) != GetCurrentThreadId() }
    {
        return Err("The Flit popup owner is unavailable.");
    }
    if !(1..=4096).contains(&placement.width) || !(1..=4096).contains(&placement.height) {
        return Err("The Flit popup size is invalid.");
    }
    if let Some(previous) = previous
        && !close(previous, false)
    {
        return Err("The previous Flit popup is still closing.");
    }
    let hwnd = create_window(placement.owner, nonactivating(placement.kind))?;
    let id = STATE.with_borrow_mut(|s| {
        s.extension_anchor = None;
        s.generation = s.generation.wrapping_add(1);
        let id = Identity {
            hwnd,
            generation: s.generation,
        };
        s.popup = Some(Popup {
            id,
            placement,
            content: Content::Pending,
            visible: false,
            retiring: false,
            dismiss_after: None,
            reveal: None,
        });
        id
    });
    Ok(id)
}

fn create_window(owner: HWND, nonactivating: bool) -> Result<HWND, &'static str> {
    let name = crate::window::wide(CLASS);
    let instance = unsafe { windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(null()) };
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_DROPSHADOW,
        lpfnWndProc: Some(proc),
        hInstance: instance,
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        lpszClassName: name.as_ptr(),
        ..Default::default()
    };
    let ex = WS_EX_TOOLWINDOW | if nonactivating { WS_EX_NOACTIVATE } else { 0 };
    let hwnd = unsafe {
        RegisterClassExW(&class);
        CreateWindowExW(
            ex,
            name.as_ptr(),
            windows_sys::w!("Flit"),
            WS_POPUP | WS_CLIPCHILDREN,
            0,
            0,
            320,
            200,
            owner,
            null_mut(),
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        return Err("Could not open the Flit popup.");
    }
    let round = DWMWCP_ROUND;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&round as *const i32).cast(),
            4,
        );
    }
    Ok(hwnd)
}

/// Open a single Exact surface. Opening the active toolbar panel again toggles it off.
pub(crate) fn show_native(
    owner: HWND,
    kind: &'static str,
    anchor: Anchor,
    width: i32,
    height: i32,
    surface: &Surface,
) -> Result<(), &'static str> {
    if toggle_off(owner, kind) {
        return Ok(());
    }
    let focus = unsafe { GetFocus() };
    let id = start(Placement {
        owner,
        focus,
        kind,
        anchor,
        width,
        height,
    })?;
    let result = (|| {
        let child = crate::exact_ui::create(id.hwnd, owner, id.generation, surface)?;
        if child.is_null()
            || unsafe { GetParent(child) } != id.hwnd
            || unsafe {
                SetWindowSubclass(child, Some(native_proc), NATIVE_SUBCLASS, id.generation)
            } == 0
        {
            return Err("Could not attach the Flit popup surface.");
        }
        let attached = STATE.with_borrow_mut(|s| {
            let Some(popup) = s.popup.as_mut().filter(|p| p.id == id && !p.retiring) else {
                return false;
            };
            popup.content = Content::Native(child);
            true
        });
        if attached && place(id, height) {
            Ok(())
        } else {
            Err("Could not place the Flit popup.")
        }
    })();
    if result.is_err() {
        close(id, false);
    }
    result
}

/// Refresh only the visible Exact panel belonging to this owner.
pub(crate) fn update_native(owner: HWND, kind: &str, surface: &Surface) {
    let child = STATE.with_borrow(|s| {
        let popup = s.popup.as_ref()?;
        (popup.live() && popup.placement.owner == owner && popup.placement.kind == kind)
            .then(|| popup.native())
            .flatten()
    });
    if let Some(child) = child {
        let _ = crate::exact_ui::update(child, surface);
    }
}

/// Keep the native edit focused and leave other toolbar panels alone.
pub(crate) fn show_address(owner: HWND, anchor: RECT, height: i32, surface: &Surface) -> bool {
    let existing = STATE.with_borrow(|s| {
        s.popup.as_ref().map(|p| {
            let belongs_here =
                p.live() && p.placement.owner == owner && p.placement.kind == ADDRESS_KIND;
            p.native()
                .filter(|_| belongs_here)
                .map(|child| (p.id, child, p.placement.height))
        })
    });
    match existing {
        Some(Some((id, child, previous_height))) => {
            crate::exact_ui::update(child, surface)
                && (previous_height == height || place(id, height))
        }
        Some(None) => false,
        None => {
            show_native(
                owner,
                ADDRESS_KIND,
                Anchor::Address(anchor),
                440,
                height,
                surface,
            )
            .is_ok()
                && native_owner(ADDRESS_KIND) == Some(owner)
        }
    }
}

pub(crate) fn dismiss_address(owner: HWND) -> bool {
    let id = STATE.with_borrow(|s| {
        let p = s.popup.as_ref()?;
        (p.live() && p.placement.kind == ADDRESS_KIND && p.placement.owner == owner).then_some(p.id)
    });
    id.is_some_and(|id| close(id, false))
}

/// The owner of the one visible Exact panel of this kind, if it is open.
pub(crate) fn native_owner(kind: &str) -> Option<HWND> {
    STATE.with_borrow(|s| {
        let popup = s.popup.as_ref()?;
        (popup.live() && popup.placement.kind == kind && popup.native().is_some())
            .then_some(popup.placement.owner)
    })
}

/// Check that a posted Exact action came from the owner's current live panel.
pub(crate) fn native_kind(owner: HWND, child: HWND) -> Option<&'static str> {
    let (parent, kind) = STATE.with_borrow(|s| {
        let popup = s.popup.as_ref()?;
        (popup.live() && popup.placement.owner == owner && popup.native() == Some(child))
            .then_some((popup.id.hwnd, popup.placement.kind))
    })?;
    (unsafe { GetParent(child) } == parent).then_some(kind)
}

/// Replace the current extension inventory with the selected extension's CEF document.
pub(crate) fn show_page(url: &str, width: i32, height: i32) {
    let Some(extension) = extension_id(url) else {
        return;
    };
    let Some(placement) = STATE.with_borrow_mut(|s| {
        let retained = s.extension_anchor.take();
        match &s.popup {
            Some(popup)
                if popup.live()
                    && popup.placement.kind == "extensions"
                    && popup.native().is_some() =>
            {
                Some(popup.placement)
            }
            Some(_) => None,
            None => retained,
        }
        .map(|p| Placement { width, height, ..p })
    }) else {
        return;
    };
    let Ok(id) = start(placement) else { return };
    if !attach_browser(id, url, extension) || !place(id, height) {
        close(id, true);
    }
}

fn extension_id(url: &str) -> Option<&str> {
    let host = url
        .strip_prefix("chrome-extension://")?
        .split(['/', '?', '#'])
        .next()?;
    (host.len() == 32 && host.bytes().all(|letter| (b'a'..=b'p').contains(&letter))).then_some(host)
}

fn attach_browser(id: Identity, url: &str, extension: &str) -> bool {
    let mut client = PopoverClient::new(extension.to_owned());
    let info = WindowInfo {
        runtime_style: RuntimeStyle::ALLOY,
        ..Default::default()
    }
    .set_as_child(
        sys::HWND(id.hwnd.cast()),
        &Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 200,
        },
    );
    let settings = BrowserSettings {
        background_color: 0xfffbfcfe,
        ..Default::default()
    };
    let Some(browser) = browser_host_create_browser_sync(
        Some(&info),
        Some(&mut client),
        Some(&CefString::from(url)),
        Some(&settings),
        None,
        None,
    ) else {
        return false;
    };
    if browser.is_valid() == 0 {
        return false;
    }
    let attached = STATE.with_borrow_mut(|s| {
        let Some(popup) = s.popup.as_mut().filter(|p| p.id == id && !p.retiring) else {
            return false;
        };
        popup.content = Content::Extension(browser.clone());
        true
    });
    if !attached && let Some(host) = browser.host() {
        STATE.with_borrow_mut(|s| s.closing.push(browser.identifier()));
        host.close_browser(1);
    }
    attached
}

pub(crate) fn hide() {
    let current = STATE.with_borrow(|s| {
        s.popup.as_ref().map(|p| {
            let extension = (p.live() && p.placement.kind == "extensions" && p.native().is_some())
                .then_some(p.placement);
            (p.id, extension)
        })
    });
    if let Some((id, extension)) = current
        && close(id, true)
    {
        STATE.with_borrow_mut(|s| {
            if s.popup.is_none() && s.generation == id.generation {
                s.extension_anchor = extension;
            }
        });
    }
}

/// The owner can route Escape here while a nonactivating panel is open.
pub(crate) fn escape(owner: HWND) -> bool {
    let id = STATE.with_borrow(|s| {
        let popup = s.popup.as_ref()?;
        (popup.live() && popup.placement.owner == owner).then_some(popup.id)
    });
    id.is_some_and(|id| dismiss(id, true))
}

fn dismiss(id: Identity, restore: bool) -> bool {
    let placement = STATE.with_borrow_mut(|s| {
        let popup = s.popup.as_mut().filter(|p| p.id == id && p.live())?;
        let reveal = popup.reveal.as_mut()?;
        reveal.transition = Some(Transition {
            started: Instant::now(),
            from: reveal.opacity,
            closing: true,
        });
        Some(popup.placement)
    });
    let Some(placement) = placement else {
        return close(id, restore);
    };
    if !crate::panel_motion::enabled() || unsafe { SetTimer(id.hwnd, REVEAL_TIMER, 15, None) } == 0 {
        return close(id, restore);
    }
    unsafe {
        let style = GetWindowLongPtrW(id.hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(id.hwnd, GWL_EXSTYLE, style | WS_EX_TRANSPARENT as isize);
        KillTimer(id.hwnd, ACTIVATION_TIMER);
        if restore && GetForegroundWindow() == id.hwnd {
            SetForegroundWindow(placement.owner);
            restore_owner_focus(placement);
        }
    }
    redraw_trigger(placement.anchor);
    true
}

unsafe fn restore_owner_focus(placement: Placement) {
    unsafe {
        if GetForegroundWindow() != placement.owner || IsWindowEnabled(placement.owner) == 0 {
            return;
        }
        let focus = placement.focus;
        if (focus == placement.owner || IsChild(placement.owner, focus) != 0)
            && IsWindowVisible(focus) != 0
            && IsWindowEnabled(focus) != 0
        {
            SetFocus(focus);
        } else if GetFocus().is_null() {
            SetFocus(placement.owner);
        }
    }
}

fn close(id: Identity, restore: bool) -> bool {
    let Some(placement) = STATE.with_borrow_mut(|s| {
        let popup = s.popup.as_mut().filter(|p| p.id == id && !p.retiring)?;
        popup.retiring = true;
        popup.visible = false;
        Some(popup.placement)
    }) else {
        return false;
    };
    let foreground = unsafe { GetForegroundWindow() };
    if unsafe { DestroyWindow(id.hwnd) } == 0 {
        STATE.with_borrow_mut(|s| {
            if let Some(popup) = s.popup.as_mut().filter(|p| p.id == id) {
                popup.retiring = false;
                popup.visible = unsafe { IsWindowVisible(id.hwnd) } != 0;
            }
        });
        return false;
    }
    if restore && foreground == id.hwnd {
        unsafe { restore_owner_focus(placement) };
    }
    redraw_trigger(placement.anchor);
    true
}

/// The last Flit window waits only for real CEF children still closing.
pub(crate) fn shutdown() -> bool {
    let (id, waiting) = STATE.with_borrow_mut(|s| {
        s.stopping = true;
        s.extension_anchor = None;
        let popup = s.popup.as_ref();
        (
            popup.map(|p| p.id),
            popup.and_then(Popup::browser).is_some() || !s.closing.is_empty(),
        )
    });
    if let Some(id) = id {
        close(id, false);
    }
    waiting
}

pub(crate) fn follow(owner: HWND) {
    let target = STATE.with_borrow(|s| {
        let popup = s.popup.as_ref()?;
        (popup.live() && popup.placement.owner == owner).then_some((popup.id, popup.placement))
    });
    if let Some((id, p)) = target {
        if p.kind == "vault" {
            place(id, p.height);
        } else {
            close(id, false);
        }
    }
}

fn children(id: Identity) -> (Option<HWND>, Option<Browser>) {
    STATE.with_borrow(|s| {
        s.popup
            .as_ref()
            .filter(|p| p.id == id && !p.retiring)
            .map(|p| (p.native(), p.browser().cloned()))
            .unwrap_or_default()
    })
}

fn fit_child(id: Identity) {
    let (native, browser) = children(id);
    let child = native.or_else(|| {
        browser
            .and_then(|browser| browser.host())
            .map(|host| host.window_handle().0.cast())
    });
    if let Some(child) = child
        && !child.is_null()
    {
        let mut client = RECT::default();
        unsafe {
            if GetClientRect(id.hwnd, &mut client) != 0 {
                SetWindowPos(
                    child,
                    null_mut(),
                    0,
                    0,
                    client.right,
                    client.bottom,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }
}

fn place(id: Identity, height: i32) -> bool {
    let Some((p, first, native)) = STATE.with_borrow(|s| {
        s.popup
            .as_ref()
            .filter(|p| p.id == id && !p.retiring)
            .map(|p| (p.placement, !p.visible, p.native().is_some()))
    }) else {
        return false;
    };
    let Some((x, y, width, height_px, travel)) = bounds(p, height) else {
        return false;
    };
    let toolbar = native && travel != 0 && !crate::chrome::high_contrast();
    if toolbar {
        round_toolbar(id.hwnd, width, height_px);
    }
    let reveal = if first && toolbar && crate::panel_motion::enabled() {
        begin_reveal(id.hwnd, y, travel)
    } else {
        None
    };
    let top = reveal.as_ref().map_or(y, Reveal::top);
    let positioned = unsafe {
        SetWindowPos(
            id.hwnd,
            HWND_TOP,
            x,
            top,
            width,
            height_px,
            SWP_SHOWWINDOW | SWP_NOACTIVATE,
        ) != 0
    };
    if !positioned {
        return false;
    }
    let live = STATE.with_borrow_mut(|s| {
        let Some(popup) = s.popup.as_mut().filter(|p| p.id == id && !p.retiring) else {
            return false;
        };
        popup.visible = true;
        popup.placement.height = height;
        if first {
            popup.reveal = reveal;
        }
        true
    });
    if live {
        fit_child(id);
        redraw_trigger(p.anchor);
        if first && !nonactivating(p.kind) {
            let (native, browser) = children(id);
            unsafe { SetForegroundWindow(id.hwnd) };
            if let Some(child) = native {
                unsafe { SetFocus(child) };
            } else if let Some(host) = browser.and_then(|b| b.host()) {
                host.set_focus(1);
            }
        }
    }
    live
}

// Native popup windows are not always rounded by DWM, particularly when layered.
fn round_toolbar(hwnd: HWND, width: i32, height: i32) {
    unsafe {
        let curve = (20 * GetDpiForWindow(hwnd).max(96) as i32 / 96).min(width).min(height);
        let clip = CreateRoundRectRgn(0, 0, width + 1, height + 1, curve, curve);
        if !clip.is_null() && SetWindowRgn(hwnd, clip, 0) == 0 {
            DeleteObject(clip);
        }
    }
}

impl Reveal {
    fn top(&self) -> i32 {
        self.y + (self.travel as f32 * (1.0 - self.opacity)).round() as i32
    }
}

fn begin_reveal(hwnd: HWND, y: i32, travel: i32) -> Option<Reveal> {
    let opacity = 0.42;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED as isize);
        if SetLayeredWindowAttributes(hwnd, 0, (opacity * 255.0) as u8, LWA_ALPHA) == 0
            || SetTimer(hwnd, REVEAL_TIMER, 15, None) == 0
        {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
            return None;
        }
    }
    Some(Reveal {
        y, travel, opacity,
        transition: Some(Transition { started: Instant::now(), from: opacity, closing: false }),
    })
}

fn tick_reveal(id: Identity) {
    let frame = STATE.with_borrow_mut(|s| {
        let popup = s.popup.as_mut().filter(|p| p.id == id && !p.retiring)?;
        let reveal = popup.reveal.as_mut()?;
        let transition = reveal.transition?;
        let (opacity, finished) = transition.sample();
        reveal.opacity = opacity;
        if finished {
            reveal.transition = None;
        }
        Some((reveal.top(), opacity, finished, transition.closing))
    });
    let Some((top, opacity, finished, closing)) = frame else {
        unsafe { KillTimer(id.hwnd, REVEAL_TIMER) };
        return;
    };
    if finished {
        unsafe { KillTimer(id.hwnd, REVEAL_TIMER) };
        if closing {
            close(id, false);
            return;
        }
    }
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(id.hwnd, &mut rect) != 0 && rect.top != top {
            SetWindowPos(id.hwnd, null_mut(), rect.left, top, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER);
        }
        SetLayeredWindowAttributes(id.hwnd, 0, (opacity * 255.0).round() as u8, LWA_ALPHA);
    }
}

fn trigger_rect(button: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    (unsafe { GetWindowRect(button, &mut rect) } != 0).then_some(rect)
}

fn bounds(p: Placement, height: i32) -> Option<(i32, i32, i32, i32, i32)> {
    let mut frame = RECT::default();
    if unsafe { GetWindowRect(p.owner, &mut frame) } == 0 {
        return None;
    }
    let dpi = unsafe { GetDpiForWindow(p.owner) }.max(96) as i32;
    let scale = |dip: i32| dip.clamp(1, 4096) * dpi / 96;
    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let anchor_rect = match p.anchor {
        Anchor::Below(button) => trigger_rect(button)?,
        Anchor::Address(field) => field,
        Anchor::Top => frame,
    };
    let handle = unsafe { MonitorFromRect(&anchor_rect, MONITOR_DEFAULTTONEAREST) };
    let area = if unsafe { GetMonitorInfoW(handle, &mut monitor) } != 0 {
        monitor.rcWork
    } else {
        frame
    };
    let (left, right) = if matches!(p.anchor, Anchor::Address(_)) {
        let left = area.left.max(frame.left + scale(8));
        let right = area.right.min(frame.right - scale(8));
        if left < right { (left, right) } else { (area.left, area.right) }
    } else {
        (area.left, area.right)
    };
    let w = scale(p.width).min((right - left).max(1));
    let limit = ((frame.bottom - frame.top) * 85 / 100).max(scale(120));
    let h = scale(height)
        .min(limit)
        .min((area.bottom - area.top).max(1));
    let (x, y, h, travel) = match p.anchor {
        Anchor::Below(_) => {
            let button = anchor_rect;
            let gap = scale(6);
            let below = button.bottom + gap;
            let above = button.top - gap;
            let space_below = (area.bottom - below).max(1);
            let space_above = (above - area.top).max(1);
            let aligned_right = button.right - w + scale(4);
            let x = if aligned_right >= area.left { aligned_right } else { button.left - scale(4) };
            if space_below >= h || space_below >= space_above {
                (x, below, h.min(space_below), -scale(8))
            } else {
                let h = h.min(space_above);
                (x, above - h, h, scale(8))
            }
        }
        Anchor::Address(field) => {
            let gap = scale(4);
            let below = field.bottom + gap;
            let above = field.top - h - gap;
            (field.left, if below + h > area.bottom && above >= area.top { above } else { below }, h, 0)
        }
        Anchor::Top => (
            frame.left + (frame.right - frame.left - w) / 2,
            frame.top + scale(64),
            h,
            0,
        ),
    };
    Some((
        x.clamp(left, (right - w).max(left)),
        y.clamp(area.top, (area.bottom - h).max(area.top)),
        w,
        h,
        travel,
    ))
}

fn post_close(id: Identity, restore: bool) -> i32 {
    unsafe { PostMessageW(id.hwnd, CLOSE_POPUP, id.generation, restore as isize) }
}

fn defer_deactivation(id: Identity) -> bool {
    let deadline = Instant::now() + Duration::from_millis(u64::from(ACTIVATION_GRACE_MS));
    if unsafe { SetTimer(id.hwnd, ACTIVATION_TIMER, ACTIVATION_GRACE_MS, None) } == 0 {
        return false;
    }
    STATE.with_borrow_mut(|s| {
        if let Some(popup) = s.popup.as_mut().filter(|p| p.id == id) {
            popup.dismiss_after = Some(deadline);
        }
    });
    true
}

unsafe extern "system" fn native_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    subclass: usize,
    generation: usize,
) -> LRESULT {
    if matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN) && wp == usize::from(VK_ESCAPE) {
        let parent = unsafe { GetParent(hwnd) };
        if !parent.is_null()
            && post_close(
                Identity {
                    hwnd: parent,
                    generation,
                },
                true,
            ) != 0
        {
            return 0;
        }
    }
    if msg == WM_NCDESTROY {
        unsafe { RemoveWindowSubclass(hwnd, Some(native_proc), subclass) };
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ACTIVATE if wp & 0xffff == WA_INACTIVE as usize => {
            let menu = STATE.with_borrow(|s| {
                let p = s.popup.as_ref()?;
                let native_extensions = p.placement.kind == "extensions" && p.native().is_some();
                (p.id.hwnd == hwnd && p.live() && !nonactivating(p.placement.kind))
                    .then_some((p.id, p.placement, native_extensions))
            });
            if let Some((id, placement, native_extensions)) = menu {
                let left_down = unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON)) } < 0;
                if let Anchor::Below(button) = placement.anchor
                    && let Some(button) = trigger_rect(button)
                {
                    let mut point = POINT::default();
                    if left_down
                        && unsafe { GetCursorPos(&mut point) } != 0
                        && point.x >= button.left
                        && point.x < button.right
                        && point.y >= button.top
                        && point.y < button.bottom
                    {
                        STATE.with_borrow_mut(|s| {
                            s.toggle = Some((Instant::now(), placement.owner, placement.kind));
                        });
                    }
                }
                // UIA may move focus before it dispatches Invoke to the Exact child.
                if native_extensions
                    && !left_down
                    && ![VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2]
                        .into_iter()
                        .any(|key| unsafe { GetAsyncKeyState(i32::from(key)) } < 0)
                    && defer_deactivation(id)
                {
                    return 0;
                }
                dismiss(id, false);
            }
            0
        }
        WM_TIMER if wp == ACTIVATION_TIMER => {
            let expired = STATE.with_borrow_mut(|s| {
                let popup = s.popup.as_mut().filter(|p| p.id.hwnd == hwnd && p.live())?;
                if Instant::now() < popup.dismiss_after? {
                    return None;
                }
                popup.dismiss_after = None;
                Some(popup.id)
            });
            if let Some(id) = expired {
                unsafe { KillTimer(hwnd, ACTIVATION_TIMER) };
                if unsafe { GetForegroundWindow() } != hwnd {
                    dismiss(id, false);
                }
            }
            0
        }
        WM_TIMER if wp == REVEAL_TIMER => {
            if let Some(id) = popup_id(hwnd) {
                tick_reveal(id);
            }
            0
        }
        WM_MOUSEACTIVATE
            if STATE.with_borrow(|s| {
                s.popup
                    .as_ref()
                    .is_some_and(|p| p.id.hwnd == hwnd && nonactivating(p.placement.kind))
            }) =>
        {
            MA_NOACTIVATE as LRESULT
        }
        WM_DPICHANGED => {
            if let Some(id) = popup_id(hwnd) {
                let bounds = unsafe { &*(lp as *const RECT) };
                let (top, round) = STATE.with_borrow_mut(|s| {
                    let Some(popup) = s.popup.as_mut().filter(|p| p.id == id) else {
                        return (bounds.top, false);
                    };
                    let round = popup.native().is_some()
                        && matches!(popup.placement.anchor, Anchor::Below(_));
                    let mut top = bounds.top;
                    if let Some(reveal) = popup.reveal.as_mut() {
                        reveal.y = bounds.top;
                        top = reveal.top();
                    }
                    (top, round)
                });
                let width = (bounds.right - bounds.left).max(1);
                let height = (bounds.bottom - bounds.top).max(1);
                if round && !crate::chrome::high_contrast() {
                    round_toolbar(hwnd, width, height);
                }
                unsafe {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        bounds.left,
                        top,
                        width,
                        height,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                }
            }
            0
        }
        WM_SIZE => {
            if let Some(id) = popup_id(hwnd) {
                fit_child(id);
            }
            0
        }
        CLOSE_POPUP | WM_CLOSE => {
            if let Some(id) = popup_id(hwnd).filter(|id| msg == WM_CLOSE || id.generation == wp) {
                dismiss(id, msg == WM_CLOSE || lp != 0);
            }
            0
        }
        WM_DESTROY => {
            // Move the browser out of thread-local state before releasing its CEF reference.
            let retired = STATE.with_borrow_mut(|s| {
                let Some(popup) = s.popup.take_if(|p| p.id.hwnd == hwnd) else {
                    return None;
                };
                if let Some(browser) = popup.browser() {
                    s.closing.push(browser.identifier());
                }
                Some(popup)
            });
            drop(retired);
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn browser_popup(browser: i32) -> Option<Identity> {
    STATE.with_borrow(|s| {
        let popup = s.popup.as_ref()?;
        (!popup.retiring && popup.has_browser(browser)).then_some(popup.id)
    })
}

wrap_client! {
    struct PopoverClient { extension: String }
    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(PopoverLife::new())
        }
        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(PopoverKeyboard::new())
        }
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(PopoverRequest::new(self.extension.clone()))
        }
    }
}

wrap_keyboard_handler! {
    struct PopoverKeyboard;
    impl KeyboardHandler {
        fn on_pre_key_event(
            &self, browser: Option<&mut Browser>, event: Option<&KeyEvent>,
            _os_event: Option<&mut sys::MSG>, _shortcut: Option<&mut i32>,
        ) -> i32 {
            if let (Some(browser), Some(event)) = (browser, event)
                && matches!(event.type_, KeyEventType::RAWKEYDOWN | KeyEventType::KEYDOWN)
                && event.windows_key_code == i32::from(VK_ESCAPE)
                && let Some(id) = browser_popup(browser.identifier())
            {
                return post_close(id, true);
            }
            0
        }
    }
}

wrap_life_span_handler! {
    struct PopoverLife;
    impl LifeSpanHandler {
        fn on_before_popup(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _popup_id: i32,
            _url: Option<&CefString>, _name: Option<&CefString>, _disposition: WindowOpenDisposition,
            _gesture: i32, _features: Option<&PopupFeatures>, _window: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>, _settings: Option<&mut BrowserSettings>,
            _extra: Option<&mut Option<DictionaryValue>>, _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            1
        }
        fn do_close(&self, browser: Option<&mut Browser>) -> i32 {
            browser.and_then(|browser| browser_popup(browser.identifier())).map_or(0, |id| post_close(id, true))
        }
        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let Some(id) = browser.map(|browser| browser.identifier()) else { return };
            let (current, retired, quit) = STATE.with_borrow_mut(|s| {
                let (current, retired) = if let Some(popup) = s.popup.as_mut().filter(|p| p.has_browser(id)) {
                    (Some(popup.id), Some(std::mem::replace(&mut popup.content, Content::Pending)))
                } else if let Some(index) = s.closing.iter().position(|&closing| closing == id) {
                    s.closing.swap_remove(index);
                    (None, None)
                } else {
                    return (None, None, false);
                };
                let quit = s.stopping && s.closing.is_empty()
                    && s.popup.as_ref().and_then(Popup::browser).is_none();
                (current, retired, quit)
            });
            drop(retired);
            if let Some(current) = current {
                post_close(current, true);
            }
            if quit {
                quit_message_loop();
            }
        }
    }
}

wrap_request_handler! {
    struct PopoverRequest { extension: String }
    impl RequestHandler {
        fn on_before_browse(
            &self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>,
            request: Option<&mut Request>, _gesture: i32, _redirect: i32,
        ) -> i32 {
            if frame.is_some_and(|frame| frame.is_main() == 0) {
                return 0;
            }
            let url = request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default();
            i32::from(extension_id(&url) != Some(self.extension.as_str()))
        }
    }
}
