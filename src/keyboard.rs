use std::{cell::Cell, ptr::null_mut};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETSEL},
        Input::KeyboardAndMouse::*,
        WindowsAndMessaging::{
            ES_READONLY, GWL_STYLE, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
            SendMessageW, WM_CHAR, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN,
            WM_KILLFOCUS, WM_NCDESTROY, WM_SYSCHAR, WM_SYSKEYDOWN,
        },
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Shortcut {
    FocusAddress,
    FocusSearch,
    Bookmark,
    NewTab,
    RestoreTab,
    CloseTab,
    Reload,
    HardReload,
    Back,
    Forward,
    NextTab,
    PreviousTab,
    /// One-based position in the full tab list.
    SelectTab(usize),
    LastTab,
    FocusFilter,
    ToggleSidebar,
    Find,
    FindNext,
    FindPrevious,
    History,
    Downloads,
    Print,
    ToggleFullscreen,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

pub(crate) fn shortcut(key: u16, ctrl: bool, shift: bool, alt: bool) -> Option<Shortcut> {
    use Shortcut::*;

    Some(match (key, ctrl, shift, alt) {
        (VK_L, true, false, false) | (VK_F6, false, false, false) | (VK_D, false, false, true) => {
            FocusAddress
        }
        (VK_K, true, false, false) => FocusSearch,
        (VK_D, true, false, false) => Bookmark,
        (VK_T, true, false, false) => NewTab,
        (VK_T, true, true, false) => RestoreTab,
        (VK_W, true, false, false) => CloseTab,
        (VK_R, true, false, false) | (VK_F5, false, false, false) => Reload,
        (VK_R, true, true, false) | (VK_F5, true, _, false) | (VK_F5, false, true, false) => {
            HardReload
        }
        (VK_LEFT, false, false, true) => Back,
        (VK_RIGHT, false, false, true) => Forward,
        (VK_TAB | VK_NEXT, true, false, false) => NextTab,
        (VK_TAB, true, true, false) | (VK_PRIOR, true, false, false) => PreviousTab,
        (VK_1..=VK_8, true, false, false) => SelectTab(usize::from(key - VK_1 + 1)),
        (VK_NUMPAD1..=VK_NUMPAD8, true, false, false) => {
            SelectTab(usize::from(key - VK_NUMPAD1 + 1))
        }
        (VK_9 | VK_NUMPAD9, true, false, false) => LastTab,
        (VK_A, true, true, false) => FocusFilter,
        (VK_B, true, true, false) => ToggleSidebar,
        (VK_F, true, false, false) => Find,
        (VK_F3, false, false, false) => FindNext,
        (VK_F3, false, true, false) => FindPrevious,
        (VK_H, true, false, false) => History,
        (VK_J, true, false, false) => Downloads,
        (VK_P, true, false, false) => Print,
        (VK_F11, false, false, false) => ToggleFullscreen,
        (VK_OEM_PLUS | VK_ADD, true, _, false) => ZoomIn,
        (VK_OEM_MINUS | VK_SUBTRACT, true, false, false) => ZoomOut,
        (VK_0 | VK_NUMPAD0, true, false, false) => ZoomReset,
        _ => return None,
    })
}

const SCAN_CODE: LPARAM = 0x01ff_0000;

#[derive(Clone, Copy)]
struct EditKey {
    hwnd: HWND,
    chars: TranslatedChars,
    scan: LPARAM,
}

#[derive(Clone, Copy)]
enum TranslatedChars {
    Control(WPARAM),
    Backspace,
    System(WPARAM, WPARAM),
}

impl EditKey {
    fn matches(self, hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> bool {
        self.hwnd == hwnd
            && self.scan == lp & SCAN_CODE
            && match self.chars {
                TranslatedChars::Control(ch) => message == WM_CHAR && wp == ch,
                TranslatedChars::Backspace => message == WM_CHAR && matches!(wp, 0x08 | 0x7f),
                TranslatedChars::System(lower, upper) => {
                    message == WM_SYSCHAR && (wp == lower || wp == upper)
                }
            }
    }
}

thread_local! {
    static EDIT_KEY: Cell<Option<EditKey>> = const { Cell::new(None) };
    static COMPOSING_EDIT: Cell<HWND> = const { Cell::new(null_mut()) };
}

/// Call before browser shortcut routing for every message to a native Unicode EDIT.
/// Return `Some` directly from the subclass; `None` leaves the message to normal routing.
pub(crate) fn handle_edit_message(
    hwnd: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> Option<LRESULT> {
    match message {
        WM_CHAR | WM_SYSCHAR => {
            // TranslateMessage may already have queued a control character before the keydown.
            return EDIT_KEY.with(|pending| {
                if pending.get()?.matches(hwnd, message, wp, lp) {
                    pending.set(None);
                    Some(0)
                } else {
                    None
                }
            });
        }
        WM_IME_STARTCOMPOSITION => COMPOSING_EDIT.set(hwnd),
        WM_IME_ENDCOMPOSITION | WM_KILLFOCUS | WM_NCDESTROY => {
            if COMPOSING_EDIT.get() == hwnd {
                COMPOSING_EDIT.set(null_mut());
            }
            if message == WM_NCDESTROY && EDIT_KEY.get().is_some_and(|key| key.hwnd == hwnd) {
                EDIT_KEY.set(None);
            }
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            EDIT_KEY.set(None);
            let key = wp as u16;
            let down = |key| unsafe { GetKeyState(i32::from(key)) < 0 };
            let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
            let expect_char = |chars| {
                EDIT_KEY.set(Some(EditKey {
                    hwnd,
                    chars,
                    scan: lp & SCAN_CODE,
                }));
            };

            // The caller routes browser actions after this hook; keep their text out of EDIT.
            if shortcut(key, ctrl, shift, alt).is_some() {
                match (message, key, ctrl, alt) {
                    (WM_KEYDOWN, VK_A..=VK_Z, true, false) => {
                        expect_char(TranslatedChars::Control(usize::from(key - VK_A + 1)));
                    }
                    (WM_KEYDOWN, VK_TAB, true, false) => {
                        expect_char(TranslatedChars::Control(0x09));
                    }
                    (WM_SYSKEYDOWN, VK_D, false, true) => {
                        expect_char(TranslatedChars::System(b'd' as usize, b'D' as usize));
                    }
                    _ => {}
                }
                return None;
            }

            if message != WM_KEYDOWN || !ctrl || shift || alt || COMPOSING_EDIT.get() == hwnd {
                return None;
            }
            match key {
                VK_A => {
                    expect_char(TranslatedChars::Control(0x01));
                    unsafe { SendMessageW(hwnd, EM_SETSEL, 0, -1) };
                }
                VK_BACK => {
                    expect_char(TranslatedChars::Backspace);
                    delete_word(hwnd, true);
                }
                VK_DELETE => {
                    expect_char(TranslatedChars::Control(0x7f));
                    delete_word(hwnd, false);
                }
                _ => return None,
            }
            return Some(0);
        }
        _ => {}
    }
    None
}

fn delete_word(hwnd: HWND, backward: bool) {
    if unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } & ES_READONLY as isize != 0 {
        return;
    }

    let len = unsafe { GetWindowTextLengthW(hwnd) };
    let Some(capacity) = len.checked_add(1).filter(|_| len > 0) else {
        return;
    };
    let mut text = vec![0u16; capacity as usize];
    let copied = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), capacity) };
    text.truncate(copied as usize);

    // The pointer results retain full UTF-16 positions; the packed return value does not.
    let (mut start, mut end) = (0u32, 0u32);
    unsafe {
        SendMessageW(
            hwnd,
            EM_GETSEL,
            (&mut start as *mut u32) as WPARAM,
            (&mut end as *mut u32) as LPARAM,
        );
    }
    let (start, end) = word_delete_span(&text, start as usize, end as usize, backward);
    if start != end {
        unsafe {
            SendMessageW(hwnd, EM_SETSEL, start, end as LPARAM);
            SendMessageW(hwnd, EM_REPLACESEL, 1, [0u16].as_ptr() as LPARAM);
        }
    }
}

fn splits_pair(text: &[u16], pos: usize) -> bool {
    pos > 0
        && pos < text.len()
        && (0xd800..=0xdbff).contains(&text[pos - 1])
        && (0xdc00..=0xdfff).contains(&text[pos])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WordClass {
    Space,
    Word,
    UrlPunctuation,
    Symbol,
}

fn adjacent(text: &[u16], pos: usize, backward: bool) -> Option<(usize, WordClass)> {
    let (start, end) = if backward {
        let start = pos.checked_sub(1)?;
        (start - usize::from(splits_pair(text, start)), pos)
    } else if pos < text.len() {
        let end = pos + 1;
        (pos, end + usize::from(splits_pair(text, end)))
    } else {
        return None;
    };
    let ch = char::decode_utf16(text[start..end].iter().copied())
        .next()?
        .unwrap_or(char::REPLACEMENT_CHARACTER);
    let class = if ch.is_whitespace() {
        WordClass::Space
    } else if ch.is_alphanumeric() || ch == '_' {
        WordClass::Word
    } else if ch.is_ascii_punctuation() {
        WordClass::UrlPunctuation
    } else {
        WordClass::Symbol
    };
    Some((if backward { start } else { end }, class))
}

fn word_delete_span(text: &[u16], start: usize, end: usize, backward: bool) -> (usize, usize) {
    let left = start.min(end).min(text.len());
    let right = start.max(end).min(text.len());
    let start = left - usize::from(splits_pair(text, left));
    let end = right + usize::from(splits_pair(text, right));
    if start != end {
        return (start, end);
    }

    // Skip adjacent whitespace, then delete one run of letters, punctuation, or symbols.
    let mut pos = start;
    let mut run = None;
    while let Some((next, class)) = adjacent(text, pos, backward) {
        if let Some(run) = run {
            if class != run {
                break;
            }
        } else if class != WordClass::Space {
            run = Some(class);
        }
        pos = next;
    }
    if backward { (pos, start) } else { (end, pos) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str, start: usize, end: usize, backward: bool) -> (usize, usize) {
        word_delete_span(
            &text.encode_utf16().collect::<Vec<_>>(),
            start,
            end,
            backward,
        )
    }

    #[test]
    fn deletes_korean_words_and_adjacent_whitespace() {
        assert_eq!(span("앞  한글  뒤", 7, 7, true), (3, 7));
        assert_eq!(span("앞  한글  뒤", 1, 1, false), (1, 5));
        assert_eq!(span("앞  한글  뒤", 4, 4, true), (3, 4));
    }

    #[test]
    fn stops_at_url_punctuation() {
        let url = "https://예시.한국/검색?q=한글";
        assert_eq!(span(url, 10, 10, true), (8, 10));
        assert_eq!(span(url, 8, 8, true), (5, 8));
        assert_eq!(span(url, 13, 13, false), (13, 14));
        assert_eq!(span(url, 14, 14, false), (14, 16));
    }

    #[test]
    fn uses_utf16_offsets_and_keeps_surrogate_pairs_whole() {
        assert_eq!(span("가😀나", 3, 3, true), (1, 3));
        assert_eq!(span("가😀나", 1, 1, false), (1, 3));
        assert_eq!(span("가😀나", 2, 2, true), (1, 3));
        assert_eq!(span("가😀나", 2, 2, false), (1, 3));
        assert_eq!(span("가👩‍💻나", 6, 6, true), (1, 6));
    }

    #[test]
    fn deletes_only_the_selection_and_does_nothing_at_text_edges() {
        for backward in [true, false] {
            assert_eq!(span("가😀나", 1, 3, backward), (1, 3));
            assert_eq!(span("가😀나", 2, 4, backward), (1, 4));
        }
        assert_eq!(span("가😀나", 0, 0, true), (0, 0));
        assert_eq!(span("가😀나", 4, 4, false), (4, 4));
        assert_eq!(span("", 0, 0, false), (0, 0));
    }

    #[test]
    fn maps_browser_shortcuts_without_taking_altgr_or_native_edit_keys() {
        use Shortcut::*;

        for (key, ctrl, shift, alt, expected) in [
            (VK_1, true, false, false, Some(SelectTab(1))),
            (VK_8, true, false, false, Some(SelectTab(8))),
            (VK_9, true, false, false, Some(LastTab)),
            (VK_PRIOR, true, false, false, Some(PreviousTab)),
            (VK_NEXT, true, false, false, Some(NextTab)),
            (VK_TAB, true, true, false, Some(PreviousTab)),
            (VK_R, true, false, false, Some(Reload)),
            (VK_R, true, true, false, Some(HardReload)),
            (VK_F5, true, false, false, Some(HardReload)),
            (VK_F5, false, true, false, Some(HardReload)),
            (VK_F6, false, false, false, Some(FocusAddress)),
            (VK_D, false, false, true, Some(FocusAddress)),
            (VK_K, true, false, false, Some(FocusSearch)),
            (VK_F3, false, false, false, Some(FindNext)),
            (VK_F3, false, true, false, Some(FindPrevious)),
            (VK_F11, false, false, false, Some(ToggleFullscreen)),
            (VK_D, true, false, true, None),
            (VK_K, true, false, true, None),
            (VK_A, true, false, false, None),
            (VK_BACK, true, false, false, None),
            (VK_DELETE, true, false, false, None),
            (VK_PROCESSKEY, false, false, false, None),
        ] {
            assert_eq!(shortcut(key, ctrl, shift, alt), expected, "key {key}");
        }
    }

    #[test]
    fn native_edit_deletes_once_and_undo_restores_the_text() {
        use std::ptr::null;
        use windows_sys::{
            Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, ES_AUTOHSCROLL, WS_POPUP,
            },
            w,
        };

        // A hidden EDIT on the test thread; no focus or input on the desktop is needed.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                w!("EDIT"),
                w!("앞 😀 뒤"),
                WS_POPUP | ES_AUTOHSCROLL as u32,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                null_mut(),
                null(),
            )
        };
        assert!(!hwnd.is_null());
        struct Edit(HWND, [u8; 256]);
        impl Drop for Edit {
            fn drop(&mut self) {
                handle_edit_message(self.0, WM_NCDESTROY, 0, 0);
                unsafe {
                    SetKeyboardState(self.1.as_ptr());
                    DestroyWindow(self.0);
                }
            }
        }

        let mut edit = Edit(hwnd, [0; 256]);
        assert_ne!(unsafe { GetKeyboardState(edit.1.as_mut_ptr()) }, 0);
        let mut keys = [0u8; 256];
        keys[VK_CONTROL as usize] = 0x80;
        assert_ne!(unsafe { SetKeyboardState(keys.as_ptr()) }, 0);

        let message = |msg, wp, lp| handle_edit_message(hwnd, msg, wp, lp);
        let text = || {
            let mut buffer = [0u16; 32];
            let len = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
            String::from_utf16(&buffer[..len as usize]).unwrap()
        };
        let select = |start, end| unsafe { SendMessageW(hwnd, EM_SETSEL, start, end) };
        let backspace = 0x000e_0001;
        select(6, 6);
        assert_eq!(message(WM_KEYDOWN, VK_BACK as usize, backspace), Some(0));
        assert_eq!(message(WM_CHAR, 0x7f, backspace), Some(0));
        assert_eq!(message(WM_CHAR, 0x7f, backspace), None);
        assert_eq!(text(), "앞 😀 ");

        assert_eq!(message(WM_CHAR, 0x1a, 0x002c_0001), None);
        unsafe { SendMessageW(hwnd, WM_CHAR, 0x1a, 0x002c_0001) };
        assert_eq!(text(), "앞 😀 뒤");

        assert_eq!(message(WM_KEYDOWN, VK_A as usize, 0x001e_0001), Some(0));
        assert_eq!(message(WM_CHAR, 0x01, 0x001e_0001), Some(0));
        assert_eq!(unsafe { SendMessageW(hwnd, EM_GETSEL, 0, 0) }, 6 << 16);
        select(2, 4);
        assert_eq!(
            message(WM_KEYDOWN, VK_DELETE as usize, 0x0153_0001),
            Some(0)
        );
        assert_eq!(text(), "앞  뒤");

        assert_eq!(message(WM_KEYDOWN, VK_H as usize, 0x0023_0001), None);
        assert_eq!(message(WM_CHAR, 0x08, 0x0023_0001), Some(0));
        assert_eq!(message(WM_IME_STARTCOMPOSITION, 0, 0), None);
        assert_eq!(message(WM_KEYDOWN, VK_BACK as usize, backspace), None);
        assert_eq!(message(WM_CHAR, 0x7f, backspace), None);
        assert_eq!(message(WM_IME_ENDCOMPOSITION, 0, 0), None);
        keys[VK_MENU as usize] = 0x80;
        assert_ne!(unsafe { SetKeyboardState(keys.as_ptr()) }, 0);
        assert_eq!(message(WM_SYSKEYDOWN, VK_A as usize, 0x201e_0001), None);
        assert_eq!(message(WM_SYSCHAR, b'@' as usize, 0x201e_0001), None);
    }
}
