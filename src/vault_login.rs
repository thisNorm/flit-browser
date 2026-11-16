use crate::vault_store;
use cef::*;
use serde::Deserialize;
use std::{collections::BTreeSet, ptr::write_volatile};
use url::{Url, form_urlencoded};

pub(crate) const UNSUPPORTED: &str = "unsupported_form";

// Only describe a traditional top-level POST form. The isolated renderer never
// receives the saved username/password, and this result goes only to Rust.
pub(crate) const FORM_SCRIPT: &str = r#"(() => {
  const visible = x => x.getClientRects().length && getComputedStyle(x).visibility === 'visible';
  const passwords = [...document.querySelectorAll('input[type=password]:enabled')].filter(visible);
  if (passwords.length !== 1) return {reason: passwords.length ? 'unsupported_form' : 'no_password_form'};
  const pass = passwords[0], f = pass.form;
  if (!f || f.method !== 'post' || f.enctype !== 'application/x-www-form-urlencoded' ||
      f.onsubmit || f.target && f.target !== '_self' || pass.readOnly ||
      ['new-password','one-time-code'].some(x => pass.autocomplete.split(/\s+/).includes(x))) {
    return {reason: 'unsupported_form'};
  }
  const action = new URL(f.action, location.href);
  if (action.origin !== location.origin || action.protocol !== 'https:' || action.username || action.password) {
    return {reason: 'different_origin'};
  }
  const fields = [...f.elements].filter(x => !x.matches(':disabled'));
  if (fields.length > 35 || fields.some(x => x.form !== f ||
      x.autocomplete?.split(/\s+/).some(v => v === 'one-time-code' || v === 'new-password') ||
      x.hasAttribute('formaction') || x.hasAttribute('formmethod') || x.hasAttribute('formenctype') ||
      x.hasAttribute('formtarget'))) return {reason:'unsupported_form'};
  const users = fields.filter(x => x instanceof HTMLInputElement &&
    ['text','email','tel'].includes(x.type) && visible(x) && !x.readOnly && x.name);
  const explicit = users.filter(x => x.autocomplete.split(/\s+/).includes('username'));
  const user = explicit.length === 1 ? explicit[0] : users.length === 1 ? users[0] : null;
  if (!user || !pass.name || user.name === pass.name || users.length !== 1) return {reason:'unsupported_form'};
  const hidden = [];
  for (const x of fields) {
    if (x === user || x === pass) continue;
    if (x instanceof HTMLInputElement && x.type === 'hidden' && x.name) {
      if (x.value.length > 4096 || x.name.length > 128) return {reason:'unsupported_form'};
      hidden.push([x.name, x.value]);
    } else if (x.name || x instanceof HTMLInputElement &&
               !['submit','button','reset'].includes(x.type)) {
      return {reason:'unsupported_form'};
    }
  }
  return {page: location.href, action: action.href, user: user.name, pass: pass.name, hidden};
})()"#;

#[derive(Deserialize)]
pub(crate) struct Probe {
    pub reason: Option<String>,
    #[serde(flatten)]
    pub form: Option<Form>,
}

#[derive(Deserialize)]
pub(crate) struct Form {
    pub page: String,
    action: String,
    user: String,
    pass: String,
    hidden: Vec<(String, String)>,
}

impl Form {
    pub(crate) fn action(&self, origin: &str) -> Option<Url> {
        if vault_store::origin(&self.action).as_deref() != Some(origin) || self.action.len() > 4096
        {
            return None;
        }
        let mut url = Url::parse(&self.action).ok()?;
        url.set_fragment(None);
        Some(url)
    }

    fn encode(&self, credential: &vault_store::Credential) -> Option<Secret> {
        let mut names = BTreeSet::new();
        for name in [&self.user, &self.pass]
            .into_iter()
            .chain(self.hidden.iter().map(|(n, _)| n))
        {
            if name.is_empty()
                || name.len() > 128
                || name.chars().any(char::is_control)
                || !names.insert(name)
            {
                return None;
            }
        }
        if self.hidden.len() > 30
            || self.hidden.iter().map(|(_, v)| v.len()).sum::<usize>() > 12 * 1024
        {
            return None;
        }
        let mut encoder = form_urlencoded::Serializer::new(String::new());
        encoder.extend_pairs(self.hidden.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        encoder.append_pair(&self.user, &credential.username);
        encoder.append_pair(&self.pass, &credential.password);
        Some(Secret(encoder.finish().into_bytes()))
    }
}

struct Secret(Vec<u8>);

impl Drop for Secret {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            // Volatile writes prevent optimization of this private temporary copy.
            unsafe { write_volatile(byte, 0) };
        }
    }
}

pub(crate) fn request(
    origin: &str,
    form: &Form,
    credential: &vault_store::Credential,
) -> Result<(Url, Request), &'static str> {
    let action = form.action(origin).ok_or("different_origin")?;
    let body = form.encode(credential).ok_or(UNSUPPORTED)?;
    let (Some(request), Some(mut data), Some(mut element)) = (
        request_create(),
        post_data_create(),
        post_data_element_create(),
    ) else {
        return Err("could_not_start");
    };
    element.set_to_bytes(body.0.len(), body.0.as_ptr());
    if data.add_element(Some(&mut element)) == 0 {
        return Err("could_not_start");
    }
    request.set_url(Some(&CefString::from(action.as_str())));
    request.set_method(Some(&CefString::from("POST")));
    request.set_header_by_name(
        Some(&CefString::from("Content-Type")),
        Some(&CefString::from("application/x-www-form-urlencoded")),
        1,
    );
    request.set_header_by_name(
        Some(&CefString::from("Origin")),
        Some(&CefString::from(origin)),
        1,
    );
    request.set_first_party_for_cookies(Some(&CefString::from(origin)));
    request.set_referrer(
        Some(&CefString::from(form.page.as_str())),
        ReferrerPolicy::ORIGIN,
    );
    request.set_post_data(Some(&mut data));
    use sys::cef_urlrequest_flags_t as F;
    request.set_flags(
        (F::UR_FLAG_ALLOW_STORED_CREDENTIALS
            | F::UR_FLAG_STOP_ON_REDIRECT
            | F::UR_FLAG_NO_DOWNLOAD_DATA
            | F::UR_FLAG_DISABLE_CACHE
            | F::UR_FLAG_NO_RETRY_ON_5XX)
            .0 as i32,
    );
    Ok((action, request))
}

pub(crate) fn continuation(
    request: &Urlrequest,
    action: &Url,
    origin: &str,
    credential: &vault_store::Credential,
) -> Option<String> {
    let response = request.response()?;
    // Never follow redirects that preserve POST, and never send the body again.
    if !matches!(response.status(), 301 | 302 | 303) {
        return None;
    }
    let location =
        CefString::from(&response.header_by_name(Some(&CefString::from("Location")))).to_string();
    if location.is_empty()
        || location.len() > 4096
        || location.chars().any(|c| c.is_control() || c == '\\')
    {
        return None;
    }
    let target = action.join(&location).ok()?;
    // Do not send POST response data into the renderer via a reflected URL.
    // Sites that require a continuation token in the redirect need a different flow.
    (vault_store::origin(target.as_str()).as_deref() == Some(origin)
        && target.query().is_none()
        && target.fragment().is_none()
        && !target.path().contains('%')
        && !target.path().contains(credential.password.as_str()))
    .then(|| target.into())
}
