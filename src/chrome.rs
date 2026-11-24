use crate::window::{ADDRESS, FILTER, FIND_TEXT, Layout, TAB_STATUS};
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND, POINT, RECT},
    Graphics::{Dwm::*, Gdi::*},
    UI::{
        Controls::{
            DRAWITEMSTRUCT, MARGINS, ODS_DISABLED, ODS_FOCUS, ODS_GRAYED, ODS_HOTLIGHT,
            ODS_NOFOCUSRECT, ODS_SELECTED,
        },
        WindowsAndMessaging::{
            SPI_GETHIGHCONTRAST, SendMessageW, SystemParametersInfoW, WM_GETFONT,
        },
    },
};

pub(crate) fn apply_backdrop(hwnd: HWND, _gap: i32) {
    let enabled = !high_contrast();
    let material = if enabled {
        if ACRYLIC.get() {
            DWMSBT_TRANSIENTWINDOW
        } else {
            DWMSBT_TABBEDWINDOW
        }
    } else {
        DWMSBT_NONE
    };
    // Keep the system material behind the shell. The toolbar and page stay legible.
    let margins = MARGINS {
        cxLeftWidth: -1,
        cxRightWidth: -1,
        cyTopHeight: -1,
        cyBottomHeight: -1,
    };
    let set = |attribute: DWMWINDOWATTRIBUTE, value: i32| unsafe {
        DwmSetWindowAttribute(hwnd, attribute as u32, (&value as *const i32).cast(), 4) >= 0
    };
    set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND);
    set(
        DWMWA_USE_IMMERSIVE_DARK_MODE,
        i32::from(dark()),
    );
    let success = set(DWMWA_SYSTEMBACKDROP_TYPE, material);
    // Controls and page HWNDs stay opaque; see `buffered` for the shell itself.
    let glass = enabled && success && unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) } >= 0;
    if !glass {
        unsafe {
            DwmExtendFrameIntoClientArea(hwnd, &MARGINS::default());
        }
    }
    if GLASS.replace(glass) != glass {
        // The caption is merged into the toolbar only while DWM draws the caption buttons.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                windows_sys::Win32::UI::WindowsAndMessaging::SWP_FRAMECHANGED
                    | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                    | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                    | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                    | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
            );
        }
    }
}

pub(crate) fn glass() -> bool {
    GLASS.get()
}

fn caption_bounds(hwnd: HWND) -> Option<RECT> {
    let mut bounds = RECT::default();
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_BUTTON_BOUNDS as u32,
            (&mut bounds as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    } >= 0;
    (ok && bounds.right > bounds.left).then_some(bounds)
}

/// Which caption button (HTMINBUTTON / HTMAXBUTTON / HTCLOSE) is at a screen point.
/// DWM stops hit-testing its buttons once the merged-caption window is maximized.
pub(crate) fn caption_button_at(hwnd: HWND, x: i32, y: i32) -> Option<u32> {
    let bounds = caption_bounds(hwnd)?;
    let mut window = RECT::default();
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut window) };
    let (x, y) = (x - window.left, y - window.top);
    if x < bounds.left || x >= bounds.right || y < bounds.top || y >= bounds.bottom {
        return None;
    }
    use windows_sys::Win32::UI::WindowsAndMessaging::{HTCLOSE, HTMAXBUTTON, HTMINBUTTON};
    Some(match (x - bounds.left) * 3 / (bounds.right - bounds.left) {
        0 => HTMINBUTTON,
        1 => HTMAXBUTTON,
        _ => HTCLOSE,
    })
}

/// Width the DWM caption buttons take at the top right of a merged-caption window.
pub(crate) fn caption_buttons(hwnd: HWND) -> i32 {
    if !GLASS.get() {
        return 0;
    }
    use windows_sys::Win32::UI::{
        HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
        WindowsAndMessaging::{GetClientRect, GetWindowRect, SM_CXSIZE},
    };
    if let Some(bounds) = caption_bounds(hwnd) {
        let mut window = RECT::default();
        let mut client = RECT::default();
        unsafe {
            if GetWindowRect(hwnd, &mut window) != 0 && GetClientRect(hwnd, &mut client) != 0 {
                let mut first = POINT { x: window.left + bounds.left, y: window.top };
                if ScreenToClient(hwnd, &mut first) != 0 && (0..client.right).contains(&first.x) {
                    return client.right - first.x;
                }
            }
        }
    }
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    (3 * unsafe { GetSystemMetricsForDpi(SM_CXSIZE, dpi) }).max(px(138, dpi as i32))
}

/// The control (and list row, or -1) under the mouse. Owner-drawn buttons never get
/// ODS_HOTLIGHT, and color-keyed sidebar pixels are click-through, so the window
/// tracks hover from the cursor position and the draw code reads it here.
pub(crate) fn set_hot(hot: (usize, isize)) -> (usize, isize) {
    HOT.replace(hot)
}

fn is_hot(item: &DRAWITEMSTRUCT, row: bool) -> bool {
    let (hwnd, index) = HOT.get();
    hwnd == item.hwndItem as usize && (!row || index == item.itemID as isize)
}

thread_local! {
    static HOT: std::cell::Cell<(usize, isize)> = const { std::cell::Cell::new((0, -1)) };
    static GLASS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static ACRYLIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static THEME: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    static SYSTEM_COLORS: std::cell::Cell<Option<(bool, bool)>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn acrylic() -> bool {
    ACRYLIC.get()
}

pub(crate) fn set_acrylic(on: bool) {
    ACRYLIC.set(on);
}

/// 0 follows Windows, 1 light, 2 dark.
pub(crate) fn theme() -> u8 {
    THEME.get()
}

pub(crate) fn set_theme(theme: u8) {
    THEME.set(theme);
}

pub(crate) fn refresh_system_colors() {
    SYSTEM_COLORS.set(None);
}

fn system_colors() -> (bool, bool) {
    if let Some(colors) = SYSTEM_COLORS.get() {
        return colors;
    }
    let colors = (windows_dark(), windows_high_contrast());
    SYSTEM_COLORS.set(Some(colors));
    colors
}

/// Dark shell colors: private windows always, otherwise the theme setting.
pub(crate) fn dark() -> bool {
    crate::window::private()
        || match THEME.get() {
            1 => false,
            2 => true,
            _ => system_colors().0,
        }
}

fn windows_dark() -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let mut light: u32 = 1;
    let mut size = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            windows_sys::w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            windows_sys::w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut light as *mut u32).cast(),
            &mut size,
        )
    } == 0;
    ok && light == 0
}

pub(crate) fn toggle_acrylic(hwnd: HWND, gap: i32) {
    ACRYLIC.set(!ACRYLIC.get());
    apply_backdrop(hwnd, gap);
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ButtonKind {
    Sidebar,
    Back,
    Forward,
    Reload,
    Stop,
    Go,
    FindPrevious,
    FindNext,
    FindClose,
    NewTab,
    NewTabCompact,
    Bookmark,
    SiteInfo,
    Downloads(bool),
    FreeMemory,
    Menu,
    Extensions,
    Vault,
    /// A collapsible sidebar section header; the label is the control's text.
    Section(bool),
}

pub(crate) const TAB_CLOSE_WIDTH: i32 = 28;
pub(crate) const TAB_UNLOAD_WIDTH: i32 = 24;

struct Palette {
    surface: COLORREF,
    panel: COLORREF,
    navigation: COLORREF,
    text: COLORREF,
    field: COLORREF,
    filter: COLORREF,
    field_text: COLORREF,
    border: COLORREF,
    hot: COLORREF,
    active: COLORREF,
    active_text: COLORREF,
    muted: COLORREF,
    disabled: COLORREF,
    focus: COLORREF,
    high_contrast: bool,
}

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as u32 | (green as u32) << 8 | (blue as u32) << 16
}

pub(crate) fn high_contrast() -> bool {
    system_colors().1
}

fn windows_high_contrast() -> bool {
    // HIGHCONTRASTW is gated behind an unused windows-sys feature.
    #[repr(C)]
    struct HighContrast {
        size: u32,
        flags: u32,
        scheme: *mut u16,
    }
    const HCF_HIGHCONTRASTON: u32 = 1;
    let mut settings = HighContrast {
        size: std::mem::size_of::<HighContrast>() as u32,
        flags: 0,
        scheme: std::ptr::null_mut(),
    };
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            settings.size,
            std::ptr::from_mut(&mut settings).cast(),
            0,
        )
    };
    ok != 0 && settings.flags & HCF_HIGHCONTRASTON != 0
}

fn palette() -> Palette {
    let high_contrast = high_contrast();
    if high_contrast {
        unsafe {
            Palette {
                surface: GetSysColor(COLOR_BTNFACE),
                panel: GetSysColor(COLOR_WINDOW),
                navigation: GetSysColor(COLOR_BTNFACE),
                text: GetSysColor(COLOR_BTNTEXT),
                field: GetSysColor(COLOR_WINDOW),
                filter: GetSysColor(COLOR_WINDOW),
                field_text: GetSysColor(COLOR_WINDOWTEXT),
                border: GetSysColor(COLOR_BTNTEXT),
                hot: GetSysColor(COLOR_HIGHLIGHT),
                active: GetSysColor(COLOR_HIGHLIGHT),
                active_text: GetSysColor(COLOR_HIGHLIGHTTEXT),
                muted: GetSysColor(COLOR_WINDOWTEXT),
                disabled: GetSysColor(COLOR_GRAYTEXT),
                focus: GetSysColor(COLOR_HIGHLIGHT),
                high_contrast,
            }
        }
    } else if dark() {
        Palette {
            surface: rgb(0x1b, 0x1e, 0x26),
            panel: rgb(0x20, 0x24, 0x2e),
            navigation: rgb(0x21, 0x26, 0x30),
            text: rgb(0xed, 0xf0, 0xf6),
            field: rgb(0x2a, 0x31, 0x3d),
            filter: rgb(0x25, 0x2b, 0x36),
            field_text: rgb(0xf2, 0xf4, 0xf8),
            border: rgb(0x45, 0x50, 0x62),
            hot: rgb(0x34, 0x3f, 0x51),
            active: rgb(0x2b, 0x40, 0x67),
            active_text: rgb(0xe3, 0xec, 0xff),
            muted: rgb(0xa8, 0xb4, 0xc9),
            disabled: rgb(0x80, 0x8c, 0x9f),
            focus: rgb(0x9a, 0xb9, 0xff),
            high_contrast,
        }
    } else {
        Palette {
            surface: rgb(0xec, 0xf0, 0xf5),
            panel: rgb(0xf4, 0xf6, 0xfa),
            navigation: rgb(0xf3, 0xf5, 0xf9),
            text: rgb(0x23, 0x2d, 0x3f),
            field: rgb(0xff, 0xff, 0xff),
            filter: rgb(0xf4, 0xf6, 0xfa),
            field_text: rgb(0x18, 0x22, 0x35),
            border: rgb(0xce, 0xd6, 0xe2),
            hot: rgb(0xe2, 0xe9, 0xf4),
            active: rgb(0xe0, 0xea, 0xfc),
            active_text: rgb(0x20, 0x42, 0x74),
            muted: rgb(0x53, 0x62, 0x78),
            disabled: rgb(0x84, 0x90, 0xa3),
            focus: rgb(0x2a, 0x58, 0xd6),
            high_contrast,
        }
    }
}

fn px(dip: i32, dpi: i32) -> i32 {
    dip * dpi.max(96) / 96
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT {
        left,
        top,
        right,
        bottom,
    }
}

fn inset(r: RECT, x: i32, y: i32) -> RECT {
    rect(r.left + x, r.top + y, r.right - x, r.bottom - y)
}

fn has_area(r: RECT) -> bool {
    r.left < r.right && r.top < r.bottom
}

fn fill(dc: HDC, r: &RECT, color: COLORREF) {
    unsafe {
        SetDCBrushColor(dc, color);
        FillRect(dc, r, GetStockObject(DC_BRUSH));
    }
}

fn rounded(dc: HDC, r: RECT, diameter: i32, color: COLORREF, border: Option<COLORREF>) {
    if !has_area(r) {
        return;
    }
    let diameter = diameter.min(r.right - r.left).min(r.bottom - r.top);
    unsafe {
        let old_brush = SelectObject(dc, GetStockObject(DC_BRUSH));
        SetDCBrushColor(dc, color);
        let old_pen = SelectObject(
            dc,
            GetStockObject(if border.is_some() { DC_PEN } else { NULL_PEN }),
        );
        if let Some(border) = border {
            SetDCPenColor(dc, border);
        }
        RoundRect(dc, r.left, r.top, r.right, r.bottom, diameter, diameter);
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
    }
}

fn field_frame(dc: HDC, r: RECT, diameter: i32, fill: COLORREF, colors: &Palette, focused: bool, dpi: i32) {
    if !has_area(r) {
        return;
    }
    if focused {
        let edge = px(2, dpi);
        rounded(dc, r, diameter, colors.focus, None);
        rounded(dc, inset(r, edge, edge), diameter - edge * 2, fill, None);
    } else {
        rounded(dc, r, diameter, fill, Some(colors.border));
    }
}

fn search_icon(dc: HDC, center: (i32, i32), dpi: i32, color: COLORREF) {
    with_pen(dc, color, dpi, || {
        let (x, y) = center;
        let r = px(4, dpi);
        unsafe {
            let old = SelectObject(dc, GetStockObject(HOLLOW_BRUSH));
            Ellipse(dc, x - r, y - r, x + r + 1, y + r + 1);
            SelectObject(dc, old);
        }
        stroke(dc, center, dpi, [(3, 3), (6, 6)]);
    });
}

fn paint_strip(window: HWND, dc: HDC, bounds: RECT, layout: &Layout, colors: &Palette) {
    if !has_area(bounds) {
        return;
    }
    let saved = unsafe { SaveDC(dc) };
    if saved == 0 {
        return;
    }
    unsafe {
        IntersectClipRect(dc, bounds.left, bounds.top, bounds.right, bounds.bottom);
    }
    let glass = GLASS.get() && !colors.high_contrast;
    if glass {
        unsafe {
            FillRect(dc, &bounds, GetStockObject(BLACK_BRUSH)); // black is see-through
        }
    } else {
        fill(dc, &bounds, colors.surface);
    }
    let d = |n| px(n, layout.dpi);
    if bounds.top < layout.top {
        if !layout.fullscreen {
            if !glass {
                let toolbar = rect(0, layout.toolbar_y, layout.toolbar_right, layout.toolbar_bottom);
                fill(dc, &toolbar, colors.navigation);
            }
            field_frame(
                dc, layout.address_frame, d(18), colors.field, colors,
                layout.focused_field == ADDRESS, layout.dpi,
            );
            if layout.toolbar_y > 0 && bounds.top < layout.toolbar_y {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    DI_NORMAL, DrawIconEx, GCLP_HICONSM, GetClassLongPtrW,
                };
                unsafe {
                    let icon = GetClassLongPtrW(window, GCLP_HICONSM);
                    let size = d(18);
                    if icon != 0 {
                        DrawIconEx(
                            dc, d(14), (layout.toolbar_y - size) / 2, icon as _,
                            size, size, 0, std::ptr::null_mut(), DI_NORMAL,
                        );
                    }
                }
            }
        }
        if has_area(layout.find_frame) {
            field_frame(
                dc, layout.find_frame, d(16), colors.field, colors,
                layout.focused_field == FIND_TEXT, layout.dpi,
            );
        }
    }
    if layout.side > 0 && bounds.left < layout.side && bounds.bottom > layout.top {
        // On glass the sidebar is the window material itself, as in Arc or Aside.
        if !glass {
            rounded(dc, layout.sidebar_bounds(), d(16), colors.panel, Some(colors.border));
        }
        let filter = layout.filter_frame;
        field_frame(
            dc, filter, d(14), colors.filter, colors,
            layout.focused_field == FILTER, layout.dpi,
        );
        search_icon(
            dc,
            (filter.left + d(14), (filter.top + filter.bottom) / 2 - d(1)),
            layout.dpi,
            colors.muted,
        );
        let footer_y = layout.height - d(69);
        if footer_y > layout.top {
            fill(
                dc,
                &rect(layout.side_offset + d(17), footer_y, layout.side - d(14), footer_y + 1),
                colors.border,
            );
        }
    }
    let line = px(1, layout.dpi);
    let page = layout.page_bounds();
    for edge in [
        rect(page.left, page.top - line, page.right, page.top),
        rect(page.left - line, page.top, page.left, page.bottom),
        rect(page.right, page.top, page.right + line, page.bottom),
        rect(page.left, page.bottom, page.right, page.bottom + line),
    ] {
        fill(dc, &edge, colors.border);
    }
    unsafe {
        RestoreDC(dc, saved);
    }
}
/// GDI leaves alpha at zero, which DWM shows as see-through over the backdrop.
/// Draw into a buffer and mark what was painted opaque. With `solid` false,
/// black pixels stay see-through so the material shows between the plates.
fn buffered(dc: HDC, area: &RECT, solid: bool, draw: impl FnOnce(HDC)) {
    use windows_sys::Win32::UI::Controls::{
        BP_PAINTPARAMS, BPBF_TOPDOWNDIB, BPPF_ERASE, BeginBufferedPaint, BufferedPaintSetAlpha,
        EndBufferedPaint, GetBufferedPaintBits,
    };
    let params = BP_PAINTPARAMS {
        cbSize: std::mem::size_of::<BP_PAINTPARAMS>() as u32,
        dwFlags: BPPF_ERASE,
        prcExclude: std::ptr::null(),
        pBlendFunction: std::ptr::null(),
    };
    let mut target: HDC = std::ptr::null_mut();
    let buffer = if GLASS.get() {
        unsafe { BeginBufferedPaint(dc, area, BPBF_TOPDOWNDIB, &params, &mut target) }
    } else {
        0
    };
    if buffer == 0 {
        draw(dc);
        return;
    }
    draw(target);
    unsafe {
        let mut bits: *mut RGBQUAD = std::ptr::null_mut();
        let mut stride = 0;
        if solid {
            BufferedPaintSetAlpha(buffer, std::ptr::null(), 255);
        } else if GetBufferedPaintBits(buffer, &mut bits, &mut stride) >= 0 && !bits.is_null() {
            let width = (area.right - area.left).min(stride) as usize;
            for y in 0..(area.bottom - area.top) as usize {
                let row = bits.add(y * stride as usize).cast::<u32>();
                for pixel in std::slice::from_raw_parts_mut(row, width) {
                    if *pixel & 0x00ff_ffff != 0 {
                        *pixel |= 0xff00_0000;
                    }
                }
            }
        }
        EndBufferedPaint(buffer, 1);
    }
}

pub(crate) fn paint_shell(window: HWND, layout: Layout) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(window, &mut paint) };
    if !dc.is_null() && has_area(paint.rcPaint) {
        let colors = palette();
        let p = layout.page_bounds();
        buffered(dc, &paint.rcPaint, false, |dc| {
            for region in [
                rect(0, 0, layout.width, p.top),
                rect(0, p.top, p.left, p.bottom),
                rect(p.right, p.top, layout.width, p.bottom),
                rect(0, p.bottom, layout.width, layout.height),
            ] {
                let mut dirty = RECT::default();
                if unsafe { IntersectRect(&mut dirty, &region, &paint.rcPaint) } != 0 {
                    paint_strip(window, dc, dirty, &layout, &colors);
                }
            }
        });
    }
    unsafe { EndPaint(window, &paint) };
}

fn begin_item(item: &DRAWITEMSTRUCT) -> i32 {
    let dc = item.hDC;
    let r = item.rcItem;
    let saved = unsafe { SaveDC(dc) };
    if saved != 0 {
        unsafe { IntersectClipRect(dc, r.left, r.top, r.right, r.bottom) };
    }
    saved
}

fn focus(item: &DRAWITEMSTRUCT, dpi: i32, colors: &Palette) {
    if item.itemState & (ODS_FOCUS | ODS_NOFOCUSRECT) == ODS_FOCUS {
        let bounds = inset(item.rcItem, px(2, dpi), px(2, dpi));
        if !has_area(bounds) {
            return;
        }
        if colors.high_contrast {
            unsafe { DrawFocusRect(item.hDC, &bounds) };
        } else {
            unsafe {
                let dc = item.hDC;
                let old_brush = SelectObject(dc, GetStockObject(HOLLOW_BRUSH));
                let old_pen = SelectObject(dc, GetStockObject(DC_PEN));
                SetDCPenColor(dc, colors.focus);
                for edge in 0..px(2, dpi) {
                    let r = inset(bounds, edge, edge);
                    if has_area(r) {
                        let radius = (px(12, dpi) - edge * 2).max(1).min(r.right - r.left);
                        RoundRect(dc, r.left, r.top, r.right, r.bottom, radius, radius);
                    }
                }
                SelectObject(dc, old_pen);
                SelectObject(dc, old_brush);
            }
        }
    }
}

pub(crate) fn draw_splitter(item: &DRAWITEMSTRUCT, dpi: i32) {
    let saved = begin_item(item);
    if saved == 0 {
        return;
    }
    let colors = palette();
    let r = item.rcItem;
    let center_x = (r.left + r.right) / 2;
    let center_y = (r.top + r.bottom) / 2;
    let half_height = px(16, dpi).min((r.bottom - r.top) / 2);
    fill(item.hDC, &r, colors.navigation);
    fill(
        item.hDC,
        &RECT {
            left: center_x - px(1, dpi),
            right: center_x + px(1, dpi),
            top: center_y - half_height,
            bottom: center_y + half_height,
        },
        if is_hot(item, false) || item.itemState & ODS_FOCUS != 0 {
            colors.text
        } else {
            colors.muted
        },
    );
    focus(item, dpi, &colors);
    unsafe { RestoreDC(item.hDC, saved) };
}

fn text(item: &DRAWITEMSTRUCT, label: &str, mut bounds: RECT, color: COLORREF) {
    if label.is_empty() || !has_area(bounds) {
        return;
    }
    let dc = item.hDC;
    let label: Vec<u16> = label.encode_utf16().collect();
    unsafe {
        let font = SendMessageW(item.hwndItem, WM_GETFONT, 0, 0);
        if font != 0 {
            SelectObject(dc, font as _);
        }
        SetTextColor(dc, color);
        SetBkMode(dc, TRANSPARENT as i32);
        DrawTextW(
            dc,
            label.as_ptr(),
            label.len() as i32,
            &mut bounds,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
    }
}

fn stroke<const N: usize>(dc: HDC, (x, y): (i32, i32), dpi: i32, path: [(i32, i32); N]) {
    let points = path.map(|(dx, dy)| POINT {
        x: x + dx.signum() * px(dx.abs(), dpi),
        y: y + dy.signum() * px(dy.abs(), dpi),
    });
    unsafe { Polyline(dc, points.as_ptr(), N as i32) };
}

thread_local! {
    /// Windows' icon font at one pixel size: (size, HFONT).
    static ICON_FONT: std::cell::Cell<(i32, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Segoe Fluent Icons (Windows 11), or Segoe MDL2 Assets (Windows 10) when it is missing.
fn icon_font(size: i32) -> HFONT {
    let (cached, font) = ICON_FONT.get();
    if cached == size && font != 0 {
        return font as HFONT;
    }
    unsafe {
        if font != 0 {
            DeleteObject(font as HGDIOBJ);
        }
        // ANTIALIASED_QUALITY (4): grayscale edges; ClearType fringes show on a lone glyph.
        let make = |face| CreateFontW(-size, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 4, 0, face);
        let mut chosen = make(windows_sys::w!("Segoe Fluent Icons"));
        // GDI quietly substitutes a missing face; check which one it picked.
        let screen = GetDC(std::ptr::null_mut());
        let old = SelectObject(screen, chosen);
        let mut name = [0u16; 32];
        GetTextFaceW(screen, name.len() as i32, name.as_mut_ptr());
        SelectObject(screen, old);
        ReleaseDC(std::ptr::null_mut(), screen);
        let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        if String::from_utf16_lossy(&name[..end]) != "Segoe Fluent Icons" {
            DeleteObject(chosen);
            chosen = make(windows_sys::w!("Segoe MDL2 Assets"));
        }
        ICON_FONT.set((size, chosen as usize));
        chosen
    }
}

/// One icon-font glyph centered on a point: anti-aliased and symmetric, unlike GDI lines.
fn glyph(dc: HDC, (x, y): (i32, i32), size: i32, color: COLORREF, code: u16) {
    unsafe {
        let old = SelectObject(dc, icon_font(size));
        SetTextColor(dc, color);
        SetBkMode(dc, TRANSPARENT as i32);
        let mut bounds = rect(x - size, y - size, x + size, y + size);
        DrawTextW(dc, &code, 1, &mut bounds, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP | DT_NOPREFIX);
        SelectObject(dc, old);
    }
}

fn with_pen(dc: HDC, color: COLORREF, dpi: i32, draw: impl FnOnce()) {
    unsafe {
        let pen = CreatePen(PS_SOLID, (dpi + 42) / 96, color);
        if !pen.is_null() {
            let previous = SelectObject(dc, pen);
            if !previous.is_null() {
                draw();
                SelectObject(dc, previous);
            }
            DeleteObject(pen);
        }
    }
}

fn memory_icon(dc: HDC, center: (i32, i32), dpi: i32) {
    stroke(
        dc,
        center,
        dpi,
        [(-5, -4), (5, -4), (5, 4), (-5, 4), (-5, -4)],
    );
    for x in [-3, 0, 3] {
        stroke(dc, center, dpi, [(x, -6), (x, -4)]);
        stroke(dc, center, dpi, [(x, 4), (x, 6)]);
    }
    stroke(dc, center, dpi, [(-2, 0), (2, 0)]);
}

fn button_icon(dc: HDC, kind: ButtonKind, center: (i32, i32), dpi: i32, color: COLORREF) {
    with_pen(dc, color, dpi, || match kind {
        ButtonKind::Sidebar => {
            stroke(
                dc,
                center,
                dpi,
                [(-7, -5), (7, -5), (7, 5), (-7, 5), (-7, -5)],
            );
            stroke(dc, center, dpi, [(-2, -5), (-2, 5)]);
        }
        ButtonKind::Back | ButtonKind::Forward => {
            let tip = if matches!(kind, ButtonKind::Back) {
                -6
            } else {
                6
            };
            stroke(dc, center, dpi, [(-tip, 0), (tip, 0)]);
            stroke(dc, center, dpi, [(tip / 6, -5), (tip, 0), (tip / 6, 5)]);
        }
        ButtonKind::Reload => {
            let (x, y) = center;
            let radius = px(6, dpi);
            unsafe {
                SetArcDirection(dc, AD_COUNTERCLOCKWISE);
                Arc(
                    dc,
                    x - radius,
                    y - radius,
                    x + radius + 1,
                    y + radius + 1,
                    x + px(5, dpi),
                    y - px(4, dpi),
                    x + radius,
                    y + px(3, dpi),
                );
            }
            stroke(dc, center, dpi, [(5, -7), (5, -3), (1, -3)]);
        }
        ButtonKind::Stop => {
            stroke(dc, center, dpi, [(-5, -5), (5, 5)]);
            stroke(dc, center, dpi, [(5, -5), (-5, 5)]);
        }
        ButtonKind::Go => stroke(dc, center, dpi, [(-3, -5), (2, 0), (-3, 5)]),
        ButtonKind::FindPrevious => stroke(dc, center, dpi, [(-4, 2), (0, -2), (4, 2)]),
        ButtonKind::FindNext => stroke(dc, center, dpi, [(-4, -2), (0, 2), (4, -2)]),
        ButtonKind::FindClose => {
            stroke(dc, center, dpi, [(-4, -4), (4, 4)]);
            stroke(dc, center, dpi, [(4, -4), (-4, 4)]);
        }
        ButtonKind::SiteInfo => {
            for y in [-4, 0, 4] {
                stroke(dc, center, dpi, [(-6, y), (6, y)]);
            }
            for (x, y) in [(-2, -4), (3, 0), (0, 4)] {
                stroke(dc, center, dpi, [(x, y - 2), (x, y + 2)]);
            }
        }
        ButtonKind::NewTab | ButtonKind::NewTabCompact => {
            stroke(dc, center, dpi, [(-5, 0), (5, 0)]);
            stroke(dc, center, dpi, [(0, -5), (0, 5)]);
        }
        ButtonKind::Bookmark => {
            stroke(
                dc,
                center,
                dpi,
                [
                    (0, -7),
                    (2, -2),
                    (7, -2),
                    (3, 1),
                    (5, 6),
                    (0, 3),
                    (-5, 6),
                    (-3, 1),
                    (-7, -2),
                    (-2, -2),
                    (0, -7),
                ],
            );
        }
        ButtonKind::Downloads(_) => {
            stroke(dc, center, dpi, [(0, -7), (0, 2)]);
            stroke(dc, center, dpi, [(-4, -2), (0, 2), (4, -2)]);
            stroke(dc, center, dpi, [(-6, 3), (-6, 6), (6, 6), (6, 3)]);
        }
        ButtonKind::FreeMemory => memory_icon(dc, center, dpi),
        ButtonKind::Menu => {
            for y in [-5, 0, 5] {
                stroke(dc, center, dpi, [(-1, y), (2, y)]);
            }
        }
        ButtonKind::Extensions => {
            stroke(dc, center, dpi, [(-6, -5), (-2, -5), (-2, -7), (2, -7), (2, -5), (6, -5), (6, -1), (4, -1), (4, 2), (6, 2), (6, 6), (-6, 6), (-6, 2), (-4, 2), (-4, -1), (-6, -1), (-6, -5)]);
        }
        ButtonKind::Vault => {
            stroke(dc, center, dpi, [(-2, 0), (-2, -4), (1, -7), (5, -7), (7, -5), (7, -1), (4, 2), (1, 2), (-4, 7), (-7, 7), (-7, 4), (-2, 0)]);
            stroke(dc, center, dpi, [(3, -3), (4, -3)]);
        }
        ButtonKind::Section(true) => stroke(dc, center, dpi, [(-4, -2), (0, 2), (4, -2)]),
        ButtonKind::Section(false) => stroke(dc, center, dpi, [(-2, -4), (2, 0), (-2, 4)]),
    });
}

pub(crate) fn draw_button(item: &DRAWITEMSTRUCT, kind: ButtonKind, dpi: i32) {
    let saved = begin_item(item);
    if saved == 0 {
        return;
    }
    let dc = item.hDC;
    let r = item.rcItem;
    let colors = palette();
    let disabled = item.itemState & (ODS_DISABLED | ODS_GRAYED) != 0;
    let pressed = !disabled && item.itemState & ODS_SELECTED != 0;
    let expanded = !disabled
        && matches!(kind, ButtonKind::Downloads(_) | ButtonKind::Menu | ButtonKind::Extensions | ButtonKind::Vault)
        && crate::popover::trigger_active(item.hwndItem);
    let hot = !disabled && (item.itemState & ODS_HOTLIGHT != 0 || is_hot(item, false));
    let (base, ink) = match kind {
        ButtonKind::Go | ButtonKind::Bookmark | ButtonKind::SiteInfo
        | ButtonKind::FindPrevious | ButtonKind::FindNext | ButtonKind::FindClose => {
            (colors.field, colors.field_text)
        }
        ButtonKind::NewTab | ButtonKind::FreeMemory => (sidebar_back(&colors), colors.text),
        ButtonKind::Section(_) => (sidebar_back(&colors), colors.muted),
        _ => (
            if GLASS.get() && !colors.high_contrast { sidebar_back(&colors) } else { colors.navigation },
            colors.text,
        ),
    };
    let background = if pressed || expanded {
        colors.active
    } else if hot {
        colors.hot
    } else {
        base
    };
    let foreground = if disabled {
        colors.disabled
    } else if pressed || expanded || (hot && colors.high_contrast) {
        colors.active_text
    } else {
        ink
    };
    fill(dc, &r, base);
    let section = matches!(kind, ButtonKind::Section(_)).then(|| window_text(item.hwndItem));
    let label = match kind {
        ButtonKind::NewTab => Some("New tab"),
        ButtonKind::FreeMemory => Some("Free memory…"),
        ButtonKind::Section(_) => section.as_deref(),
        _ => None,
    };
    if section.is_none() && (pressed || expanded || hot || colors.high_contrast || label.is_some()) {
        let border = (colors.high_contrast && !pressed && !expanded && !hot).then_some(foreground);
        let fill_color = if label.is_some() && !pressed && !hot && !colors.high_contrast {
            colors.field
        } else {
            background
        };
        let shape = if label.is_some() {
            inset(r, px(1, dpi), px(1, dpi))
        } else {
            r
        };
        rounded(dc, shape, px(14, dpi), fill_color, border);
    }
    let x = if section.is_some() {
        r.left + px(10, dpi)
    } else if label.is_some() {
        r.left + px(15, dpi)
    } else {
        r.left + (r.right - r.left) / 2
    };
    let y = r.top + (r.bottom - r.top) / 2;
    button_icon(dc, kind, (x, y), dpi, foreground);
    if matches!(kind, ButtonKind::Downloads(true)) {
        let badge = if colors.high_contrast {
            foreground
        } else {
            colors.active_text
        };
        rounded(
            dc,
            rect(
                x + px(7, dpi),
                y - px(12, dpi),
                x + px(13, dpi),
                y - px(6, dpi),
            ),
            px(6, dpi),
            badge,
            None,
        );
    }
    if let Some(label) = label {
        text(
            item,
            crate::i18n::text(label),
            rect(r.left + px(if section.is_some() { 22 } else { 27 }, dpi), r.top, r.right - px(8, dpi), r.bottom),
            foreground,
        );
    }
    focus(item, dpi, &colors);
    unsafe { RestoreDC(dc, saved) };
}

pub(crate) fn draw_tab(
    item: &DRAWITEMSTRUCT,
    label: &str,
    icon: usize,
    pinned: bool,
    muted: bool,
    unloaded: bool,
    unloadable: bool,
    active: bool,
    dpi: i32,
) {
    let saved = begin_item(item);
    if saved == 0 {
        return;
    }
    let dc = item.hDC;
    let r = item.rcItem;
    let colors = palette();
    let preview = !active && item.itemState & ODS_SELECTED != 0;
    let foreground = if active {
        colors.active_text
    } else if !preview && item.itemState & (ODS_DISABLED | ODS_GRAYED) != 0 {
        colors.disabled
    } else if unloaded && !preview {
        colors.muted
    } else {
        colors.text
    };
    let back = sidebar_back(&colors);
    fill(dc, &r, back);
    if active || preview {
        rounded(
            dc,
            inset(r, px(1, dpi), px(2, dpi)),
            px(14, dpi),
            if active { colors.active } else { back },
            preview.then_some(if colors.high_contrast {
                colors.text
            } else {
                colors.active_text
            }),
        );
    } else if is_hot(item, true) {
        rounded(dc, inset(r, px(1, dpi), px(2, dpi)), px(14, dpi), colors.hot, None);
    }
    let tab_ink = if active && !colors.high_contrast {
        let center = (r.top + r.bottom) / 2;
        let left = r.left + px(4, dpi);
        let width = px(2, dpi);
        rounded(
            dc, rect(left, center - px(8, dpi), left + width, center + px(8, dpi)),
            width, colors.focus, None,
        );
        colors.focus
    } else {
        foreground
    };
    let center = (r.left + px(16, dpi), r.top + (r.bottom - r.top) / 2);
    let icon = (!pinned && !unloaded && icon != 0).then_some(icon as HBITMAP);
    if let Some(icon) = icon {
        let size = px(16, dpi);
        draw_icon(dc, icon, center.0 - size / 2, center.1 - size / 2, size);
    }
    with_pen(dc, tab_ink, dpi, || {
        if icon.is_some() {
        } else if pinned {
            stroke(
                dc,
                center,
                dpi,
                [
                    (-3, -5),
                    (3, -5),
                    (2, -1),
                    (5, 2),
                    (-5, 2),
                    (-2, -1),
                    (-3, -5),
                ],
            );
            stroke(dc, center, dpi, [(0, 2), (0, 6)]);
        } else if unloaded {
            stroke(
                dc,
                center,
                dpi,
                [
                    (-3, -5),
                    (3, -5),
                    (5, -3),
                    (5, 3),
                    (3, 5),
                    (-3, 5),
                    (-5, 3),
                    (-5, -3),
                    (-3, -5),
                ],
            );
            stroke(dc, center, dpi, [(0, -3), (0, 0), (2, 2)]);
        } else {
            stroke(
                dc,
                center,
                dpi,
                [(-5, -5), (4, -5), (4, 5), (-5, 5), (-5, -5)],
            );
            stroke(dc, center, dpi, [(-5, -2), (4, -2)]);
        }
    });
    let offset = if unloadable { TAB_UNLOAD_WIDTH } else { 0 };
    if unloadable {
        let at = (
            r.right - px(TAB_CLOSE_WIDTH + TAB_UNLOAD_WIDTH / 2, dpi),
            center.1,
        );
        with_pen(dc, foreground, dpi, || memory_icon(dc, at, dpi));
    }
    if muted {
        with_pen(dc, foreground, dpi, || {
            let speaker = (r.right - px(34 + offset, dpi), center.1);
            stroke(
                dc,
                speaker,
                dpi,
                [
                    (-6, -2),
                    (-4, -2),
                    (0, -5),
                    (0, 5),
                    (-4, 2),
                    (-6, 2),
                    (-6, -2),
                ],
            );
            stroke(dc, speaker, dpi, [(2, -2), (6, 2), (2, -2)]);
            stroke(dc, speaker, dpi, [(2, 2), (6, -2), (2, 2)]);
        });
    }
    text(
        item,
        label,
        rect(
            r.left + px(30, dpi),
            r.top,
            r.right - px((if muted { 45 } else { 27 }) + offset, dpi),
            r.bottom,
        ),
        foreground,
    );
    let x = r.right - px(14, dpi);
    let y = r.top + (r.bottom - r.top) / 2;
    if x - px(4, dpi) >= r.left {
        // The icon font's close glyph (ChromeClose), like the window's own close button;
        // 1-2px GDI diagonals round differently at 125-150% and look tilted.
        glyph(dc, (x, y), px(10, dpi), foreground, 0xE8BB);
    }
    focus(item, dpi, &colors);
    unsafe { RestoreDC(dc, saved) };
}

pub(crate) fn field_colors(dc: HDC, id: usize) -> HBRUSH {
    let colors = palette();
    let (background, foreground) = match id {
        101 | 125 => (colors.field, colors.field_text),
        110 => (colors.filter, colors.field_text),
        126 => (colors.field, colors.muted),
        TAB_STATUS if !colors.high_contrast => (sidebar_back(&colors), colors.muted),
        108 | 111 | 112 | 194 | 196 => (sidebar_back(&colors), colors.text),
        // The sidebar's scroll thumb (SIDEBAR_THUMB).
        209 => (if dark() { rgb(0x5a, 0x64, 0x7c) } else { rgb(0xb0, 0xb9, 0xc8) }, colors.text),
        _ => (colors.panel, colors.text),
    };
    unsafe {
        SetTextColor(dc, foreground);
        SetBkColor(dc, background);
        SetBkMode(dc, OPAQUE as i32);
        SetDCBrushColor(dc, background);
        GetStockObject(DC_BRUSH)
    }
}

pub(crate) fn paint_dark_edit_cue(window: HWND, id: usize, cue: &str, print_dc: HDC) -> bool {
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::GetFocus,
        WindowsAndMessaging::{GetClientRect, GetWindowTextLengthW},
    };
    if !dark() || high_contrast() || unsafe { GetFocus() == window || GetWindowTextLengthW(window) != 0 } {
        return false;
    }
    let mut paint = PAINTSTRUCT::default();
    let dc = if print_dc.is_null() {
        unsafe { BeginPaint(window, &mut paint) }
    } else {
        print_dc
    };
    if !dc.is_null() {
        let colors = palette();
        let mut area = RECT::default();
        unsafe {
            GetClientRect(window, &mut area);
            let saved = SaveDC(dc);
            if saved != 0 {
                fill(dc, &area, if id == FILTER { colors.filter } else { colors.field });
                let font = SendMessageW(window, WM_GETFONT, 0, 0);
                if font != 0 {
                    SelectObject(dc, font as _);
                }
                SetTextColor(dc, colors.muted);
                SetBkMode(dc, TRANSPARENT as i32);
                let cue: Vec<u16> = cue.encode_utf16().collect();
                DrawTextW(
                    dc, cue.as_ptr(), cue.len() as i32, &mut area,
                    DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                );
                RestoreDC(dc, saved);
            }
        }
    }
    if print_dc.is_null() {
        unsafe { EndPaint(window, &paint) };
    }
    true
}

pub(crate) fn paint_page_placeholder(window: HWND, font: HFONT, waiting: bool) {
    let mut event = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(window, &mut event) };
    if !dc.is_null() {
        let colors = palette();
        fill(dc, &event.rcPaint, colors.field);
        if waiting {
            let mut area = RECT::default();
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(window, &mut area);
                let saved = SaveDC(dc);
                if saved != 0 {
                    if !font.is_null() {
                        SelectObject(dc, font);
                    }
                    SetTextColor(dc, colors.text);
                    SetBkMode(dc, TRANSPARENT as i32);
                    DrawTextW(dc, if crate::i18n::korean() {
                        windows_sys::w!("이 창으로 전환하면 선택한 페이지가 열립니다.")
                    } else {
                        windows_sys::w!("Switch to this window to load the selected page.")
                    }, -1, &mut area, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
                    RestoreDC(dc, saved);
                }
            }
        }
    }
    unsafe { EndPaint(window, &event) };
}

/// Close to the material's own tint, which keeps antialiased text edges clean.
pub(crate) fn material_key() -> Option<COLORREF> {
    let colors = palette();
    (GLASS.get() && !colors.high_contrast).then(|| sidebar_back(&colors))
}

fn sidebar_back(colors: &Palette) -> COLORREF {
    if GLASS.get() && !colors.high_contrast {
        if dark() {
            rgb(0x21, 0x23, 0x2b)
        } else {
            rgb(0xeb, 0xee, 0xf2)
        }
    } else {
        colors.panel
    }
}

fn window_text(hwnd: HWND) -> String {
    let mut buffer = [0u16; 64];
    let n = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, buffer.as_mut_ptr(), 64)
    };
    String::from_utf16_lossy(&buffer[..n.max(0) as usize])
}

/// A saved bookmark with no open page in this window.
pub(crate) fn draw_bookmark(item: &DRAWITEMSTRUCT, label: &str, dpi: i32) {
    let saved = begin_item(item);
    if saved == 0 {
        return;
    }
    let dc = item.hDC;
    let r = item.rcItem;
    let colors = palette();
    let back = sidebar_back(&colors);
    fill(dc, &r, back);
    if item.itemState & ODS_SELECTED != 0 || is_hot(item, true) {
        rounded(dc, inset(r, px(1, dpi), px(2, dpi)), px(14, dpi), colors.hot, None);
    }
    let center = (r.left + px(16, dpi), r.top + (r.bottom - r.top) / 2);
    with_pen(dc, colors.text, dpi, || {
        stroke(dc, center, dpi, [(-4, -6), (4, -6), (4, 6), (0, 3), (-4, 6), (-4, -6)]);
    });
    text(
        item,
        label,
        rect(r.left + px(30, dpi), r.top, r.right - px(8, dpi), r.bottom),
        colors.text,
    );
    focus(item, dpi, &colors);
    unsafe { RestoreDC(dc, saved) };
}

/// A site icon kept as a premultiplied 32-bit DIB; deleted with its tab.
pub(crate) struct Favicon {
    bitmap: HBITMAP,
    size: i32,
}

impl Favicon {
    pub(crate) fn from_image(image: &cef::Image) -> Option<Self> {
        use cef::ImplImage;
        let (mut width, mut height) = (0, 0);
        let pixels = image.as_bitmap(
            2.0,
            cef::ColorType::BGRA_8888,
            cef::AlphaType::PREMULTIPLIED,
            Some(&mut width),
            Some(&mut height),
        )
        .or_else(|| image.as_bitmap(1.0, cef::ColorType::BGRA_8888, cef::AlphaType::PREMULTIPLIED, Some(&mut width), Some(&mut height)))?;
        if width <= 0 || height <= 0 || width > 256 || height > 256 {
            return None;
        }
        let mut bytes = Vec::new();
        use cef::ImplBinaryValue;
        bytes.resize(pixels.size(), 0);
        pixels.data(Some(&mut bytes), 0);
        if bytes.len() < (width * height * 4) as usize {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = unsafe {
            CreateDIBSection(std::ptr::null_mut(), &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0)
        };
        if bitmap.is_null() || bits.is_null() {
            return None;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), bits.cast::<u8>(), (width * height * 4) as usize);
        }
        Some(Self { bitmap, size: width.max(height) })
    }

    pub(crate) fn handle(&self) -> usize {
        self.bitmap as usize | (self.size as usize) << 48
    }
}

impl Drop for Favicon {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.bitmap) };
    }
}

fn draw_icon(dc: HDC, handle: HBITMAP, x: i32, y: i32, size: i32) {
    // The source size rides in the handle's top bits (see `Favicon::handle`).
    let raw = handle as usize;
    let source = (raw >> 48) as i32;
    let bitmap = (raw & 0xffff_ffff_ffff) as HBITMAP;
    unsafe {
        let memory = CreateCompatibleDC(dc);
        if memory.is_null() {
            return;
        }
        let old = SelectObject(memory, bitmap);
        SetStretchBltMode(dc, HALFTONE);
        AlphaBlend(
            dc,
            x,
            y,
            size,
            size,
            memory,
            0,
            0,
            source,
            source,
            BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 },
        );
        SelectObject(memory, old);
        DeleteDC(memory);
    }
}
