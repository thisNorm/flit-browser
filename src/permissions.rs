//! One user decision for one foreground camera/microphone request.
use cef::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc as Shared,
};
use url::{Host, Url};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::{
        Controls::{
            TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TDF_ALLOW_DIALOG_CANCELLATION,
            TDF_POSITION_RELATIVE_TO_WINDOW, TDM_CLICK_BUTTON, TDN_CREATED, TDN_DESTROYED,
            TaskDialogIndirect,
        },
        WindowsAndMessaging::{GA_ROOT, GetAncestor, IDCANCEL, IDNO, IsWindowVisible, PostMessageW},
    },
};

const MIC: u32 =
    sys::cef_media_access_permission_types_t::CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE as u32;
const CAMERA: u32 =
    sys::cef_media_access_permission_types_t::CEF_MEDIA_PERMISSION_DEVICE_VIDEO_CAPTURE as u32;
const ALLOW_ONCE: i32 = 1001;

#[derive(Default)]
struct Ticket {
    tab: usize,
    cancelled: Cell<bool>,
    dialog: Cell<HWND>,
}

thread_local! {
    static PENDING: RefCell<HashMap<usize, Shared<Ticket>>> = RefCell::new(HashMap::new());
}

/// Call on the CEF UI thread. Owner must be the top-level HWND of this browser.
pub(crate) fn handler(owner_hwnd: usize, tab_key: usize) -> PermissionHandler {
    MediaPermission::new(owner_hwnd, tab_key)
}

/// Call before navigating/closing this tab or hiding it on a tab switch.
pub(crate) fn cancel_tab(owner: usize, tab: usize) {
    cancel(owner, Some(tab));
}

/// Call before closing the owner; also usable for an owner with a single popup.
pub(crate) fn cancel_window(owner: usize) {
    cancel(owner, None);
}

fn cancel(owner: usize, tab: Option<usize>) {
    let ticket = PENDING.with(|pending| pending.borrow().get(&owner).cloned());
    if let Some(ticket) = ticket.filter(|t| tab.is_none_or(|tab| tab == t.tab)) {
        ticket.cancelled.set(true);
        let hwnd = ticket.dialog.get();
        if !hwnd.is_null() {
            unsafe {
                PostMessageW(hwnd, TDM_CLICK_BUTTON as u32, IDCANCEL as WPARAM, 0);
            }
        }
    }
}

fn secure_origin(url: &Url) -> Option<String> {
    // Literal loopback is a potentially trustworthy origin under Secure Contexts.
    let trusted = match (url.scheme(), url.host()?) {
        ("https", _) => true,
        ("http", Host::Ipv4(ip)) => ip.is_loopback(),
        ("http", Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    (trusted && url.username().is_empty() && url.password().is_none())
        .then(|| url.origin().ascii_serialization())
}

fn page_url(frame: &Frame) -> String {
    CefString::from(&frame.url()).to_string()
}

fn eligible(
    owner: usize,
    browser: &Browser,
    frame: &Frame,
    origin: &str,
    url: &str,
    mask: u32,
) -> bool {
    if browser.is_valid() == 0
        || frame.is_valid() == 0
        || frame.is_main() == 0
        || page_url(frame) != url
    {
        return false;
    }
    let Some(parsed) = Url::parse(url).ok() else {
        return false;
    };
    if secure_origin(&parsed).as_deref() != Some(origin) {
        return false;
    }
    let Some(host) = browser.host() else {
        return false;
    };
    let child: HWND = host.window_handle().0.cast();
    let owner = owner as HWND;
    if owner.is_null()
        || child.is_null()
        || unsafe { IsWindowVisible(child) == 0 || GetAncestor(child, GA_ROOT) != owner }
    {
        return false;
    }
    if parsed.scheme() == "https"
        && !host
            .visible_navigation_entry()
            .and_then(|e| e.sslstatus())
            .is_some_and(|s| {
                s.is_secure_connection() != 0 && is_cert_status_error(s.cert_status()) == 0
            })
    {
        return false;
    }
    let Some(context) = host.request_context() else {
        return false;
    };
    let origin = CefString::from(origin);
    for (bit, setting, pref) in [
        (
            MIC,
            ContentSettingTypes::MEDIASTREAM_MIC,
            "hardware.audio_capture_enabled",
        ),
        (
            CAMERA,
            ContentSettingTypes::MEDIASTREAM_CAMERA,
            "hardware.video_capture_enabled",
        ),
    ] {
        if mask & bit != 0
            && (!context
                .preference(Some(&CefString::from(pref)))
                .is_some_and(|v| v.bool() != 0)
                || context.content_setting(Some(&origin), Some(&origin), setting)
                    == ContentSettingValues::BLOCK)
        {
            return false;
        }
    }
    true
}

wrap_permission_handler! {
    struct MediaPermission { owner: usize, tab: usize }
    impl PermissionHandler {
        fn on_request_media_access_permission(
            &self, browser: Option<&mut Browser>, frame: Option<&mut Frame>,
            requesting_origin: Option<&CefString>, mask: u32, callback: Option<&mut MediaAccessCallback>,
        ) -> i32 {
            // The CEF direct callback can choose a screen itself. Never take that path.
            if mask == 0 || mask & !(MIC | CAMERA) != 0 { return 0; }
            let Some(callback) = callback else { return 0 };
            let (Some(browser), Some(frame), Some(origin)) = (browser, frame, requesting_origin) else {
                callback.cancel();
                return 1;
            };
            // Chromium knows Permissions Policy and both origins for embedded frames.
            if frame.is_main() == 0 { return 0; }
            let origin = origin.to_string();
            let Some(origin) = Url::parse(&origin).ok().and_then(|u| secure_origin(&u)) else {
                callback.cancel();
                return 1;
            };
            let url = page_url(frame);
            if !eligible(self.owner, browser, frame, &origin, &url, mask) {
                callback.cancel();
                return 1;
            }
            let ticket = Shared::new(Ticket { tab: self.tab, ..Ticket::default() });
            let queued = PENDING.with(|pending| {
                use std::collections::hash_map::Entry;
                if let Entry::Vacant(entry) = pending.borrow_mut().entry(self.owner) {
                    entry.insert(ticket.clone());
                    true
                } else { false }
            });
            if !queued {
                callback.cancel();
                return 1;
            }
            let mut task = ShowPrompt::new(self.owner, ticket, browser.clone(), frame.clone(),
                origin, url, mask, callback.clone());
            if post_task(ThreadId::UI, Some(&mut task)) == 0 {
                PENDING.with(|pending| pending.borrow_mut().remove(&self.owner));
                callback.cancel();
            }
            1
        }
        // Notification ACCEPT is durable in CEF 156. Preserve Chromium's own prompt.
    }
}

wrap_task! {
    struct ShowPrompt {
        owner: usize, ticket: Shared<Ticket>, browser: Browser, frame: Frame,
        origin: String, url: String, mask: u32, callback: MediaAccessCallback,
    }
    impl Task {
        fn execute(&self) {
            let still_valid = || !self.ticket.cancelled.get() && eligible(self.owner,
                &self.browser, &self.frame, &self.origin, &self.url, self.mask);
            let allowed = still_valid() && show_dialog(self.owner, &self.ticket, &self.origin, self.mask)
                && still_valid();
            PENDING.with(|pending| pending.borrow_mut().remove(&self.owner));
            if allowed { self.callback.cont(self.mask); } else { self.callback.cancel(); }
        }
    }
}

unsafe extern "system" fn dialog_event(
    hwnd: HWND,
    event: u32,
    _: WPARAM,
    _: LPARAM,
    data: isize,
) -> i32 {
    // TaskDialogIndirect does not retain callback data after returning.
    let ticket = unsafe { &*(data as *const Ticket) };
    if event == TDN_CREATED as u32 {
        ticket.dialog.set(hwnd);
        if ticket.cancelled.get() {
            unsafe {
                PostMessageW(hwnd, TDM_CLICK_BUTTON as u32, IDCANCEL as WPARAM, 0);
            }
        }
    } else if event == TDN_DESTROYED as u32 {
        ticket.dialog.set(std::ptr::null_mut());
    }
    0
}

fn show_dialog(owner: usize, ticket: &Ticket, origin: &str, mask: u32) -> bool {
    use crate::i18n::{choose, text as tr};
    let device = if mask == MIC {
        choose("microphone", "마이크")
    } else if mask == CAMERA {
        choose("camera", "카메라")
    } else {
        choose("camera and microphone", "카메라와 마이크")
    };
    let instruction = crate::window::wide(&if crate::i18n::korean() {
        format!("{device} 사용을 허용할까요?")
    } else {
        format!("Use your {device}?")
    });
    let text = crate::window::wide(&format!(
        "{origin}\n\n{}",
        choose(
            "Allow access for this request only.",
            "이번 요청에만 사용을 허용합니다."
        )
    ));
    let (deny, allow, title) = (
        crate::window::wide(tr("Deny")),
        crate::window::wide(tr("Allow this time")),
        crate::window::wide(tr("Flit Browser — Site permission")),
    );
    let buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: IDNO,
            pszButtonText: deny.as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: ALLOW_ONCE,
            pszButtonText: allow.as_ptr(),
        },
    ];
    let config = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner as HWND,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW,
        pszWindowTitle: title.as_ptr(),
        pszMainInstruction: instruction.as_ptr(),
        pszContent: text.as_ptr(),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: IDNO,
        pfCallback: Some(dialog_event),
        lpCallbackData: ticket as *const Ticket as isize,
        ..Default::default()
    };
    let mut selected = IDCANCEL;
    let status = unsafe {
        TaskDialogIndirect(
            &config,
            &mut selected,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    status >= 0 && selected == ALLOW_ONCE
}
