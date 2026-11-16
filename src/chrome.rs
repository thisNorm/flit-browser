use crate::window::Layout;
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

pub(crate) fn apply_backdrop(hwnd: HWND, gap: i32) {
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
    let margins = MARGINS {
        cxLeftWidth: gap,
        cxRightWidth: gap,
        cyTopHeight: gap / 2,
        cyBottomHeight: gap,
    };
    let set = |attribute: DWMWINDOWATTRIBUTE, value: i32| unsafe {
        DwmSetWindowAttribute(hwnd, attribute as u32, (&value as *const i32).cast(), 4) >= 0
    };
    set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND);
    set(
        DWMWA_USE_IMMERSIVE_DARK_MODE,
        i32::from(crate::window::private()),
    );
    let success = set(DWMWA_SYSTEMBACKDROP_TYPE, material);
    // Keep text and page HWNDs opaque. Expose the DWM material in the
    // caption and in the slim frame around the web content.
    let glass = enabled && success && unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) } >= 0;
    if !glass {
        unsafe {
            DwmExtendFrameIntoClientArea(hwnd, &MARGINS::default());
        }
    }
    GLASS.set(glass);
}

thread_local! {
    static GLASS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static ACRYLIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn acrylic() -> bool {
    ACRYLIC.get()
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
    Go,
    NewTab,
    NewTabCompact,
    Bookmark,
    Menu,
}

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
    disabled: COLORREF,
    rim: COLORREF,
    shadow: COLORREF,
    high_contrast: bool,
}

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as u32 | (green as u32) << 8 | (blue as u32) << 16
}

fn high_contrast() -> bool {
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
                disabled: GetSysColor(COLOR_GRAYTEXT),
                rim: GetSysColor(COLOR_BTNFACE),
                shadow: GetSysColor(COLOR_BTNFACE),
                high_contrast,
            }
        }
    } else if crate::window::private() {
        Palette {
            surface: rgb(0x19, 0x1c, 0x2a),
            panel: rgb(0x23, 0x28, 0x38),
            navigation: rgb(0x2b, 0x31, 0x44),
            text: rgb(0xed, 0xf1, 0xfc),
            field: rgb(0x2b, 0x31, 0x44),
            filter: rgb(0x19, 0x1e, 0x2b),
            field_text: rgb(0xf1, 0xf4, 0xff),
            border: rgb(0x45, 0x50, 0x69),
            hot: rgb(0x43, 0x50, 0x69),
            active: rgb(0x3b, 0x4a, 0x67),
            active_text: rgb(0xe4, 0xed, 0xff),
            disabled: rgb(0xb2, 0xbf, 0xda),
            rim: rgb(0x48, 0x54, 0x6d),
            shadow: rgb(0x13, 0x17, 0x24),
            high_contrast,
        }
    } else {
        Palette {
            surface: rgb(0xe8, 0xed, 0xf5),
            panel: rgb(0xf3, 0xf7, 0xfc),
            navigation: rgb(0xf7, 0xfa, 0xfe),
            text: rgb(0x22, 0x2d, 0x40),
            field: rgb(0xfb, 0xfd, 0xff),
            filter: rgb(0xe9, 0xef, 0xf7),
            field_text: rgb(0x1d, 0x1d, 0x1f),
            border: rgb(0xd0, 0xd9, 0xe7),
            hot: rgb(0xdf, 0xe9, 0xf8),
            active: rgb(0xd9, 0xe8, 0xff),
            active_text: rgb(0x1c, 0x49, 0x84),
            disabled: rgb(0x6e, 0x6e, 0x73),
            rim: 0xffffff,
            shadow: rgb(0xd7, 0xe1, 0xef),
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

fn glass_plate(dc: HDC, r: RECT, diameter: i32, core: COLORREF, colors: &Palette, dpi: i32) {
    if !has_area(r) {
        return;
    }
    if colors.high_contrast {
        rounded(dc, r, diameter, core, Some(colors.border));
        return;
    }
    let shadow = px(2, dpi);
    rounded(
        dc,
        rect(r.left, r.top + shadow, r.right, r.bottom + shadow),
        diameter,
        colors.shadow,
        None,
    );
    rounded(dc, r, diameter, core, Some(colors.border));
    let rim = inset(r, 1, 1);
    if !has_area(rim) {
        return;
    }
    let saved = unsafe { SaveDC(dc) };
    if saved == 0 {
        return;
    }
    let region =
        unsafe { CreateRoundRectRgn(rim.left, rim.top, rim.right, rim.bottom, diameter, diameter) };
    if !region.is_null() {
        unsafe {
            ExtSelectClipRgn(dc, region, RGN_AND);
        }
        let color = |x, y, c: COLORREF| TRIVERTEX {
            x,
            y,
            Red: ((c & 255) as u16) << 8,
            Green: (((c >> 8) & 255) as u16) << 8,
            Blue: (((c >> 16) & 255) as u16) << 8,
            Alpha: 0,
        };
        let points = [
            color(rim.left, rim.top, colors.rim),
            color(rim.right, (rim.top + px(7, dpi)).min(rim.bottom), core),
        ];
        let mesh = GRADIENT_RECT {
            UpperLeft: 0,
            LowerRight: 1,
        };
        unsafe {
            GradientFill(
                dc,
                points.as_ptr(),
                2,
                (&mesh as *const GRADIENT_RECT).cast(),
                1,
                GRADIENT_FILL_RECT_V,
            );
            DeleteObject(region);
        }
    }
    unsafe {
        RestoreDC(dc, saved);
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

fn paint_strip(dc: HDC, bounds: RECT, layout: &Layout, colors: &Palette) {
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
    fill(dc, &bounds, colors.surface);
    let d = |n| px(n, layout.dpi);
    let address = layout.address_frame;
    if bounds.top < layout.top {
        for plate in [
            rect(d(10), d(8), d(56), d(52)),
            rect(d(57), d(8), d(164), d(52)),
            rect(layout.width - d(55), d(8), layout.width - d(9), d(52)),
        ] {
            glass_plate(dc, plate, d(37), colors.navigation, colors, layout.dpi);
        }
        glass_plate(dc, address, d(38), colors.field, colors, layout.dpi);
        search_icon(
            dc,
            (
                address.left + d(18),
                (address.top + address.bottom) / 2 - d(1),
            ),
            layout.dpi,
            colors.disabled,
        );
        if has_area(layout.find_frame) {
            glass_plate(
                dc,
                layout.find_frame,
                d(28),
                colors.field,
                colors,
                layout.dpi,
            );
        }
    }
    if layout.side > 0 && bounds.left < layout.side && bounds.bottom > layout.top {
        glass_plate(
            dc,
            layout.sidebar_bounds(),
            d(34),
            colors.panel,
            colors,
            layout.dpi,
        );
        let filter = layout.filter_frame;
        rounded(dc, filter, d(23), colors.filter, Some(colors.border));
        search_icon(
            dc,
            (filter.left + d(14), (filter.top + filter.bottom) / 2 - d(1)),
            layout.dpi,
            colors.disabled,
        );
    }
    let line = px(1, layout.dpi);
    let page = layout.page_bounds();
    if GLASS.get() && !colors.high_contrast {
        let black = unsafe { GetStockObject(BLACK_BRUSH) };
        let gap = px(8, layout.dpi);
        for margin in [
            rect(0, 0, gap - line, layout.height),
            rect(0, 0, layout.width, gap / 2),
            rect(page.right + line, 0, layout.width, layout.height),
            rect(0, page.bottom + line, layout.width, layout.height),
        ] {
            unsafe {
                FillRect(dc, &margin, black);
            }
        }
    }
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
pub(crate) fn paint_shell(window: HWND, layout: Layout) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(window, &mut paint) };
    if !dc.is_null() && has_area(paint.rcPaint) {
        let colors = palette();
        let p = layout.page_bounds();
        for region in [
            rect(0, 0, layout.width, p.top),
            rect(0, p.top, p.left, p.bottom),
            rect(p.right, p.top, layout.width, p.bottom),
            rect(0, p.bottom, layout.width, layout.height),
        ] {
            let mut dirty = RECT::default();
            if unsafe { IntersectRect(&mut dirty, &region, &paint.rcPaint) } != 0 {
                paint_strip(dc, dirty, &layout, &colors);
            }
        }
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

fn focus(item: &DRAWITEMSTRUCT, dpi: i32) {
    if item.itemState & (ODS_FOCUS | ODS_NOFOCUSRECT) == ODS_FOCUS {
        let bounds = inset(item.rcItem, px(2, dpi), px(2, dpi));
        if has_area(bounds) {
            unsafe { DrawFocusRect(item.hDC, &bounds) };
        }
    }
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
        ButtonKind::Go => stroke(dc, center, dpi, [(-3, -5), (2, 0), (-3, 5)]),
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
        ButtonKind::Menu => {
            for y in [-5, 0, 5] {
                stroke(dc, center, dpi, [(-1, y), (2, y)]);
            }
        }
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
    let hot = !disabled && item.itemState & ODS_HOTLIGHT != 0;
    let (base, ink) = match kind {
        ButtonKind::Go | ButtonKind::Bookmark => (colors.field, colors.field_text),
        ButtonKind::NewTab => (colors.panel, colors.text),
        _ => (colors.navigation, colors.text),
    };
    let background = if pressed {
        colors.active
    } else if hot {
        colors.hot
    } else {
        base
    };
    let foreground = if disabled {
        colors.disabled
    } else if pressed || (hot && colors.high_contrast) {
        colors.active_text
    } else {
        ink
    };
    fill(dc, &r, base);
    if pressed || hot || colors.high_contrast || labeled_new_tab(kind) {
        let border = (colors.high_contrast && !pressed && !hot).then_some(foreground);
        let fill_color = if labeled_new_tab(kind) && !pressed && !hot && !colors.high_contrast {
            colors.field
        } else {
            background
        };
        let shape = if labeled_new_tab(kind) {
            inset(r, px(1, dpi), px(1, dpi))
        } else {
            r
        };
        rounded(dc, shape, px(20, dpi), fill_color, border);
    }
    let labeled = matches!(kind, ButtonKind::NewTab);
    let x = if labeled {
        r.left + px(15, dpi)
    } else {
        r.left + (r.right - r.left) / 2
    };
    let y = r.top + (r.bottom - r.top) / 2;
    button_icon(dc, kind, (x, y), dpi, foreground);
    if labeled {
        text(
            item,
            "New tab",
            rect(r.left + px(27, dpi), r.top, r.right - px(8, dpi), r.bottom),
            foreground,
        );
    }
    focus(item, dpi);
    unsafe { RestoreDC(dc, saved) };
}

fn labeled_new_tab(kind: ButtonKind) -> bool {
    matches!(kind, ButtonKind::NewTab)
}

pub(crate) fn draw_tab(
    item: &DRAWITEMSTRUCT,
    label: &str,
    pinned: bool,
    muted: bool,
    unloaded: bool,
    dpi: i32,
) {
    let saved = begin_item(item);
    if saved == 0 {
        return;
    }
    let dc = item.hDC;
    let r = item.rcItem;
    let colors = palette();
    let selected = item.itemState & ODS_SELECTED != 0;
    let foreground = if selected {
        colors.active_text
    } else if unloaded || item.itemState & (ODS_DISABLED | ODS_GRAYED) != 0 {
        colors.disabled
    } else {
        colors.text
    };
    fill(dc, &r, colors.panel);
    if selected {
        rounded(
            dc,
            inset(r, px(1, dpi), px(2, dpi)),
            px(19, dpi),
            colors.active,
            None,
        );
    }
    let tab_ink = if selected && !colors.high_contrast && !crate::window::private() {
        rgb(0x00, 0x66, 0xcc)
    } else {
        foreground
    };
    let center = (r.left + px(16, dpi), r.top + (r.bottom - r.top) / 2);
    with_pen(dc, tab_ink, dpi, || {
        if pinned {
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
    if muted {
        with_pen(dc, foreground, dpi, || {
            let speaker = (r.right - px(34, dpi), center.1);
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
            stroke(dc, speaker, dpi, [(2, -2), (6, 2)]);
            stroke(dc, speaker, dpi, [(2, 2), (6, -2)]);
        });
    }
    text(
        item,
        label,
        rect(
            r.left + px(30, dpi),
            r.top,
            r.right - px(if muted { 45 } else { 27 }, dpi),
            r.bottom,
        ),
        foreground,
    );
    let x = r.right - px(14, dpi);
    let y = r.top + (r.bottom - r.top) / 2;
    if x - px(4, dpi) >= r.left {
        with_pen(dc, foreground, dpi, || {
            stroke(dc, (x, y), dpi, [(-4, -4), (4, 4)]);
            stroke(dc, (x, y), dpi, [(-4, 4), (4, -4)]);
        });
    }
    focus(item, dpi);
    unsafe { RestoreDC(dc, saved) };
}

pub(crate) fn field_colors(dc: HDC, id: usize) -> HBRUSH {
    let colors = palette();
    let (background, foreground) = match id {
        101 | 125 => (colors.field, colors.field_text),
        110 => (colors.filter, colors.field_text),
        126 => (colors.surface, colors.text),
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

pub(crate) fn paint_start(window: HWND, font: HFONT) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(window, &mut paint) };
    if !dc.is_null() {
        let colors = palette();
        let mut area = RECT::default();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(window, &mut area);
        }
        fill(dc, &paint.rcPaint, colors.field);
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(window) }.max(96) as i32;
        let large = unsafe {
            CreateFontW(
                -px(36, dpi),
                0,
                0,
                0,
                600,
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
        let saved = unsafe { SaveDC(dc) };
        if saved != 0 {
            let mut title = rect(
                px(20, dpi),
                (area.bottom / 3 - px(25, dpi)).max(px(20, dpi)),
                area.right - px(20, dpi),
                area.bottom / 3 + px(30, dpi),
            );
            unsafe {
                if !large.is_null() {
                    SelectObject(dc, large);
                }
                SetTextColor(dc, colors.text);
                SetBkMode(dc, TRANSPARENT as i32);
                DrawTextW(
                    dc,
                    if crate::window::private() {
                        windows_sys::w!("Private browsing")
                    } else {
                        windows_sys::w!("Flit")
                    },
                    -1,
                    &mut title,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
                if !font.is_null() {
                    SelectObject(dc, font);
                }
            }
            let mut subtitle = rect(
                px(20, dpi),
                title.bottom + px(10, dpi),
                area.right - px(20, dpi),
                title.bottom + px(38, dpi),
            );
            let mut hint = rect(
                px(20, dpi),
                subtitle.bottom + px(18, dpi),
                area.right - px(20, dpi),
                subtitle.bottom + px(44, dpi),
            );
            unsafe {
                DrawTextW(
                    dc,
                    if crate::window::private() {
                        windows_sys::w!(
                            "Flit clears private browsing data when the last private window closes."
                        )
                    } else {
                        windows_sys::w!("Search or enter a website in the address bar")
                    },
                    -1,
                    &mut subtitle,
                    DT_CENTER | DT_WORDBREAK,
                );
                SetTextColor(dc, colors.disabled);
                DrawTextW(
                    dc,
                    if crate::window::private() {
                        windows_sys::w!("Downloaded files stay on your device.")
                    } else {
                        windows_sys::w!(
                            "Ctrl+L  Address     Ctrl+T  New tab     Ctrl+Shift+A  Find a tab"
                        )
                    },
                    -1,
                    &mut hint,
                    DT_CENTER | DT_WORDBREAK,
                );
                RestoreDC(dc, saved);
            }
        }
        if !large.is_null() {
            unsafe {
                DeleteObject(large);
            }
        }
    }
    unsafe {
        EndPaint(window, &paint);
    }
}
