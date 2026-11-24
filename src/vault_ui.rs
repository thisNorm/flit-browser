use crate::{vault_store, window::wide};
use cef::*;
use std::{
    mem::size_of,
    ptr::{null, null_mut, write_volatile},
};
use windows_sys::{
    Win32::{
        Foundation::{ERROR_CANCELLED, HWND, NO_ERROR},
        Security::Credentials::{
            CREDUI_FLAGS_ALWAYS_SHOW_UI, CREDUI_FLAGS_DO_NOT_PERSIST,
            CREDUI_FLAGS_EXCLUDE_CERTIFICATES, CREDUI_FLAGS_GENERIC_CREDENTIALS, CREDUI_INFOW,
            CREDUI_MAX_USERNAME_LENGTH, CredUIPromptForCredentialsW,
        },
        UI::WindowsAndMessaging::{
            IDYES, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONQUESTION, MB_OK, MB_YESNO, MessageBoxW,
        },
    },
    w,
};

#[derive(PartialEq, Eq)]
pub(crate) struct Site {
    browser: i32,
    pub(crate) origin: String,
}

pub(crate) fn site(browser: &Browser) -> Option<Site> {
    if browser.is_valid() == 0 || browser.is_popup() != 0 || browser.is_loading() != 0 {
        return None;
    }
    let frame = browser.main_frame()?;
    if frame.is_valid() == 0 || frame.is_main() == 0 {
        return None;
    }
    let url = CefString::from(&frame.url()).to_string();
    let origin = vault_store::origin(&url)?;
    let entry = browser.host()?.visible_navigation_entry()?;
    // A pending entry or Chromium error page must not lend its URL or TLS state to the frame.
    if entry.is_valid() == 0 || CefString::from(&entry.url()).to_string() != url {
        return None;
    }
    let ssl = entry.sslstatus()?;
    (ssl.is_secure_connection() != 0
        && is_cert_status_error(ssl.cert_status()) == 0
        && ssl.content_status() == SslContentStatus::NORMAL_CONTENT)
        .then(|| Site {
            browser: browser.identifier(),
            origin,
        })
}

pub(crate) fn manage(owner: HWND, site: &Site, remove: bool, current: impl Fn() -> Option<Site>) {
    let still_current = || current().as_ref() == Some(site);
    if !still_current() {
        return;
    }
    let origin = &site.origin;
    let result = if remove {
        confirm_and_remove(owner, origin, still_current)
    } else {
        prompt_and_save(owner, origin, still_current)
    };
    if let Err(message) = result {
        unsafe {
            MessageBoxW(
                owner,
                wide(crate::i18n::text(message)).as_ptr(),
                w!("Flit Vault"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

fn confirm_and_remove(
    owner: HWND,
    origin: &str,
    still_current: impl FnOnce() -> bool,
) -> Result<(), &'static str> {
    if !vault_store::has(origin).map_err(|_| "Could not check the Flit Vault login.")? {
        return Ok(());
    }
    let message = wide(&if crate::i18n::korean() {
        format!(
            "{origin}의 Flit Vault 로그인을 삭제할까요?\n\nAI가 이 로그인을 더 이상 사용할 수 없게 됩니다."
        )
    } else {
        format!(
            "Remove the Flit Vault login for {origin}?\n\nThis revokes AI access to this saved login."
        )
    });
    let answer = unsafe {
        MessageBoxW(
            owner,
            message.as_ptr(),
            wide(crate::i18n::text("Remove Flit Vault login")).as_ptr(),
            MB_YESNO | MB_DEFBUTTON2 | MB_ICONQUESTION,
        )
    };
    if answer != IDYES {
        return Ok(());
    }
    if !still_current() {
        return Err("The active tab or its secure connection changed. The login was not removed.");
    }
    vault_store::remove(origin).map_err(|_| "Could not remove the Flit Vault login.")
}

const USERNAME_BUFFER: usize = CREDUI_MAX_USERNAME_LENGTH as usize + 1;
const PASSWORD_BUFFER: usize = 256 + 1; // CREDUI_MAX_PASSWORD_LENGTH plus NUL.

struct Wiped<T: Copy + Default, const N: usize>([T; N]);

impl<T: Copy + Default, const N: usize> Drop for Wiped<T, N> {
    fn drop(&mut self) {
        for unit in &mut self.0 {
            unsafe { write_volatile(unit, T::default()) };
        }
    }
}

fn decode<'a>(input: &[u16], output: &'a mut [u8]) -> Option<&'a str> {
    let end = input.iter().position(|&unit| unit == 0)?;
    if end == 0 {
        return None;
    }
    let mut len = 0;
    for ch in char::decode_utf16(input[..end].iter().copied()) {
        len += ch.ok()?.encode_utf8(&mut output[len..]).len();
    }
    std::str::from_utf8(&output[..len]).ok()
}

fn prompt_and_save(
    owner: HWND,
    origin: &str,
    still_current: impl FnOnce() -> bool,
) -> Result<(), &'static str> {
    let message = wide(&if crate::i18n::korean() {
        format!(
            "{origin}\n\n저장하면 Flit AI 에이전트가 이 사이트에서만 이 로그인을 사용할 수 있습니다. \
            이 사이트에 저장된 기존 Flit Vault 로그인은 교체됩니다."
        )
    } else {
        format!(
            "{origin}\n\nSaving lets the Flit AI agent use this login on this exact site. \
            It replaces any existing Flit Vault login for this site."
        )
    });
    let caption = wide(crate::i18n::text("Add or update Flit Vault login"));
    let target = wide(&format!("Flit Vault enrollment: {origin}"));
    let info = CREDUI_INFOW {
        cbSize: size_of::<CREDUI_INFOW>() as u32,
        hwndParent: owner,
        pszMessageText: message.as_ptr(),
        pszCaptionText: caption.as_ptr(),
        hbmBanner: null_mut(),
    };
    let mut user = Wiped([0u16; USERNAME_BUFFER]);
    let mut password = Wiped([0u16; PASSWORD_BUFFER]);
    let status = unsafe {
        CredUIPromptForCredentialsW(
            &info,
            target.as_ptr(),
            null(),
            0,
            user.0.as_mut_ptr(),
            USERNAME_BUFFER as u32,
            password.0.as_mut_ptr(),
            PASSWORD_BUFFER as u32,
            null_mut(),
            CREDUI_FLAGS_DO_NOT_PERSIST
                | CREDUI_FLAGS_GENERIC_CREDENTIALS
                | CREDUI_FLAGS_ALWAYS_SHOW_UI
                | CREDUI_FLAGS_EXCLUDE_CERTIFICATES,
        )
    };
    match status {
        ERROR_CANCELLED => return Ok(()),
        NO_ERROR => (),
        _ => return Err("Could not open the Windows credential prompt."),
    }
    if !still_current() {
        return Err("The active tab or its secure connection changed. The login was not saved.");
    }

    // Decode into buffers we also wipe, including if Windows returns invalid UTF-16.
    let mut user_utf8 = Wiped([0u8; 3 * USERNAME_BUFFER]);
    let mut password_utf8 = Wiped([0u8; 3 * PASSWORD_BUFFER]);
    let (Some(user), Some(password)) = (
        decode(&user.0, &mut user_utf8.0),
        decode(&password.0, &mut password_utf8.0),
    ) else {
        return Err("Enter a valid username and password.");
    };
    vault_store::save(origin, user, password).map_err(|_| "Could not save the Flit Vault login.")
}
