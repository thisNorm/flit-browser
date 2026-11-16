use crate::{
    agent_protocol::{self, Command},
    agent_view, launch_url, vault_login, vault_store,
};
use cef::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs, io,
    path::PathBuf,
    ptr::null_mut,
    sync::mpsc::{self, Sender},
    time::Duration,
};
use url::Url;
use windows_sys::{
    Win32::{
        Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType},
        System::{
            Com::CoCreateGuid,
            Console::{GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
        },
    },
    core::GUID,
};

const MAX_TABS: usize = 8;
const MAX_URL: usize = 4096;
const MAX_TEXT: usize = 24_000;
const MAX_LINKS: usize = 60;
const CALL_TIMEOUT: Duration = Duration::from_secs(12);
const CONTEXT_LOST: &str =
    "The agent browser stopped. Reconnect the MCP server to start a new session.";
const PAGE_CHANGED: &str = "Page changed. Take a new snapshot.";
type Reply = Sender<Result<Value, String>>;

// This expression runs in a fresh isolated JS world, never as caller-supplied code.
// Return capped rendered text and visible links, without reading control values or browser storage.
const READ_DOM: &str = r#"(() => {
  const root = document.body;
  const normalize = text => text.replace(/[^\S\r\n]+/g, ' ').replace(/\n{3,}/g, '\n\n').trim();
  const raw = root ? root.innerText || '' : '';
  const text = normalize(raw.slice(0, 24000));
  const visible = node => node.getClientRects().length && getComputedStyle(node).visibility === 'visible';
  const links = [];
  let linksTruncated = false;
  if (root) for (const a of root.querySelectorAll('a[href]')) {
    if (a.hasAttribute('download') || !visible(a)) continue;
    const href = a.href;
    if (!/^https?:\/\//i.test(href) || href.length > 4096) continue;
    if (links.length >= 60) { linksTruncated = true; break; }
    links.push({ url: href, text: normalize((a.innerText || a.getAttribute('aria-label') || a.title || '').slice(0, 256)) });
  }
  return { url: location.href, title: document.title.slice(0, 512),
           text, links, truncated: raw.length > 24000 || linksTruncated,
           login_form_detected: [...document.querySelectorAll('input[type=password]:enabled')].some(visible) };
})()"#;

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

#[derive(Default)]
struct State {
    tabs: BTreeMap<u32, Tab>,
    next_tab: u32,
    next_snapshot: u32,
    next_method: i32,
    next_login: u32,
    context: Option<RequestContext>,
    stopping: Option<Reply>,
}

struct Tab {
    url: String,
    title: String,
    status: &'static str,
    http_status: Option<i32>,
    error: Option<&'static str>,
    closing: bool,
    browser: Option<Browser>,
    observer: Option<Registration>,
    pending: Option<Pending>,
    last_snapshot: Option<(u32, String, Vec<Link>)>,
    login: Option<Login>,
}

impl Tab {
    fn new(url: String) -> Self {
        Self {
            url,
            title: String::new(),
            status: "loading",
            http_status: None,
            error: None,
            closing: false,
            browser: None,
            observer: None,
            pending: None,
            last_snapshot: None,
            login: None,
        }
    }
    fn info(&self, id: u32) -> Value {
        if self.login.is_some() {
            json!({"tab": id, "status": "signing_in"})
        } else {
            json!({"tab": id, "url": self.url, "title": self.title, "status": self.status,
                   "http_status": self.http_status, "error": self.error})
        }
    }
    fn invalidate(&mut self) {
        self.last_snapshot = None;
        if let Some(pending) = self.pending.take().filter(|p| {
            matches!(
                p.stage,
                Stage::Frame(Query::Snapshot) | Stage::World(Query::Snapshot) | Stage::Read
            )
        }) {
            let _ = pending.reply.send(Err(PAGE_CHANGED.into()));
        }
    }
}

#[derive(Clone, Deserialize)]
struct Link {
    text: String,
    url: String,
}

#[derive(Deserialize)]
struct Page {
    url: String,
    title: String,
    text: String,
    links: Vec<Link>,
    truncated: bool,
    login_form_detected: bool,
}

struct Pending {
    method: i32,
    stage: Stage,
    reply: Reply,
}

#[derive(Clone, Copy)]
enum Stage {
    Frame(Query),
    World(Query),
    Read,
    LoginForm,
    LoginVerify,
}

#[derive(Clone, Copy)]
enum Query {
    Snapshot,
    LoginForm,
    LoginVerify,
}

struct Login {
    attempt: u32,
    origin: String,
    page: String,
    reply: Reply,
    request: Option<Urlrequest>,
    destination: Option<(String, bool)>,
    submitted_credential: Option<vault_store::Credential>,
}

const VERIFY_LOGIN: &str = r#"(() => ({
    url: location.href,
    has_password: [...document.querySelectorAll('input[type=password]:enabled')]
        .some(x => x.getClientRects().length && getComputedStyle(x).visibility === 'visible'),
    needs_mfa: [...document.querySelectorAll('input[autocomplete~=one-time-code]:enabled')]
        .some(x => x.getClientRects().length && getComputedStyle(x).visibility === 'visible')
}))()"#;

#[derive(Deserialize)]
struct LoginPage {
    url: String,
    has_password: bool,
    needs_mfa: bool,
}

pub(crate) fn web_url(input: &str) -> Option<String> {
    if input.len() > MAX_URL {
        return None;
    }
    let parsed = Url::parse(launch_url::literal_http_url(input)?).ok()?;
    let normalized: String = parsed.into();
    (normalized.len() <= MAX_URL).then_some(normalized)
}

pub(crate) fn same_document_link(current: &str, target: &str) -> bool {
    let current = current.split_once('#').map_or(current, |(base, _)| base);
    target
        .split_once('#')
        .is_some_and(|(base, _)| base == current)
}

fn update(id: u32, change: impl FnOnce(&mut Tab)) {
    STATE.with(|state| {
        if let Some(tab) = state.borrow_mut().tabs.get_mut(&id).filter(|t| !t.closing) {
            change(tab);
        }
    });
}

pub(crate) fn loading(id: u32) {
    update(id, |tab| {
        tab.invalidate();
        tab.status = "loading";
        tab.error = None;
        tab.http_status = None;
    });
}

pub(crate) fn vault_allows_navigation(id: u32, url: &str, redirect: bool) -> bool {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(tab) = state.tabs.get_mut(&id) else {
            return false;
        };
        let Some(login) = &mut tab.login else {
            return true;
        };
        if redirect {
            return false;
        }
        if let Some((target, used)) = &mut login.destination {
            if !*used && target == url {
                *used = true;
                return true;
            }
        }
        false
    })
}

pub(crate) fn address(id: u32, url: String) {
    if let Some(url) = web_url(&url) {
        let unexpected = STATE.with(|state| {
            state
                .borrow()
                .tabs
                .get(&id)
                .and_then(|tab| tab.login.as_ref())
                .is_some_and(|login| {
                    login.page != url
                        && !login
                            .destination
                            .as_ref()
                            .is_some_and(|(target, _)| *target == url)
                })
        });
        if unexpected {
            finish_login(id, "action_required", "page_changed");
        }
        update(id, |tab| {
            if tab.url != url {
                tab.invalidate();
                tab.url = url;
            }
        });
    }
}

pub(crate) fn loaded(id: u32, url: String, http_status: i32) {
    if let Some(url) = web_url(&url) {
        address(id, url);
        update(id, |tab| {
            if !matches!(tab.status, "error" | "stopped") {
                tab.status = "ready";
                tab.http_status = (http_status > 0).then_some(http_status);
            }
        });
        let verify = STATE.with(|state| {
            let mut s = state.borrow_mut();
            let tab = s.tabs.get_mut(&id)?;
            let login = tab.login.as_ref()?;
            (tab.status == "ready"
                && login
                    .destination
                    .as_ref()
                    .is_some_and(|(url, used)| *used && *url == tab.url))
            .then(|| login.reply.clone())
        });
        if let Some(reply) = verify {
            if (200..300).contains(&http_status) {
                method(
                    id,
                    Stage::Frame(Query::LoginVerify),
                    reply,
                    "Page.getFrameTree",
                    None,
                );
            } else {
                finish_login(id, "action_required", "verification_page_failed");
            }
        }
    }
}

pub(crate) fn load_incomplete(id: u32, reason: &'static str) {
    let loading = STATE.with(|state| {
        state
            .borrow()
            .tabs
            .get(&id)
            .is_some_and(|tab| tab.status == "loading")
    });
    if loading {
        failed(id, reason);
    }
}

pub(crate) fn failed(id: u32, reason: &'static str) {
    update(id, |tab| {
        tab.invalidate();
        tab.status = "error";
        tab.error = Some(reason);
    });
    finish_login(id, "action_required", "navigation_failed");
}

pub(crate) fn title(id: u32, title: String) {
    update(id, |tab| tab.title = title.chars().take(512).collect());
}

pub(crate) fn closed(id: u32) {
    let tab = STATE.with(|state| state.borrow_mut().tabs.remove(&id));
    if let Some(mut tab) = tab {
        if let Some(login) = tab.login.take() {
            if let Some(request) = login.request {
                request.cancel();
            }
            let _ = login
                .reply
                .send(Ok(login_state(id, "action_required", "tab_closed")));
        }
        tab.invalidate();
        drop(tab);
    }
    finish_stopping();
}

fn open(url: &str) -> Result<Value, String> {
    let url = web_url(url).ok_or("Enter a literal HTTP or HTTPS URL without credentials.")?;
    let (id, mut context) = STATE.with(|state| {
        let mut s = state.borrow_mut();
        if s.tabs.len() >= MAX_TABS {
            return Err("This agent session has eight open or closing tabs. Close a tab first.");
        }
        let context = s
            .context
            .clone()
            .ok_or("The isolated agent context is unavailable.")?;
        s.next_tab = s
            .next_tab
            .checked_add(1)
            .ok_or("Agent tab IDs exhausted.")?;
        let id = s.next_tab;
        s.tabs.insert(id, Tab::new(url.clone()));
        Ok((id, context))
    })?;
    let mut client = agent_view::client(id);
    let window = WindowInfo::default().set_as_windowless(sys::HWND(null_mut()));
    let settings = BrowserSettings {
        windowless_frame_rate: 5,
        javascript_access_clipboard: cef::State::DISABLED,
        javascript_dom_paste: cef::State::DISABLED,
        ..Default::default()
    };
    let Some(browser) = browser_host_create_browser_sync(
        Some(&window),
        Some(&mut client),
        Some(&CefString::from(url.as_str())),
        Some(&settings),
        None,
        Some(&mut context),
    ) else {
        closed(id);
        return Err("Flit could not open the agent tab.".into());
    };
    let observer = browser.host().and_then(|host| {
        host.set_audio_muted(1);
        let mut observer = AgentObserver::new(id);
        host.add_dev_tools_message_observer(Some(&mut observer))
    });
    let installed = observer.is_some();
    let result = STATE.with(|state| {
        let mut s = state.borrow_mut();
        let tab = s
            .tabs
            .get_mut(&id)
            .ok_or("Agent tab closed during startup.")?;
        tab.browser = Some(browser);
        tab.observer = observer;
        Ok::<_, &str>(tab.info(id))
    })?;
    if !installed {
        let _ = close(id);
        return Err("Flit could not attach its local page reader.".into());
    }
    Ok(result)
}

fn close(id: u32) -> Result<Value, String> {
    finish_login(id, "action_required", "tab_closed");
    let browser = STATE.with(|state| {
        let mut s = state.borrow_mut();
        let tab = s
            .tabs
            .get_mut(&id)
            .filter(|t| !t.closing && t.login.is_none())
            .ok_or("No such agent tab.")?;
        tab.invalidate();
        tab.closing = true;
        Ok::<_, &str>(tab.browser.clone())
    })?;
    if let Some(host) = browser.as_ref().and_then(|b| b.host()) {
        host.close_browser(1);
    } else {
        closed(id);
    }
    Ok(json!({"tab": id, "status": "closed"}))
}

fn stop_loading(id: u32) -> Result<Value, String> {
    let (browser, status) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let tab = state
            .tabs
            .get_mut(&id)
            .filter(|tab| !tab.closing)
            .ok_or("No such agent tab.")?;
        if tab.login.is_some() {
            return Ok((None, "signing_in"));
        }
        let browser = tab.browser.as_ref().ok_or("No such agent tab.")?;
        let main_loading = tab.status == "loading";
        if !main_loading && browser.is_loading() == 0 {
            return Ok((None, "not_loading"));
        }
        let browser = browser.clone();
        if main_loading {
            tab.invalidate();
            tab.status = "stopped";
        }
        Ok::<_, &str>((Some(browser), "stopped"))
    })?;
    // StopLoad can invoke callbacks; set the state and release the borrow first.
    if let Some(browser) = browser {
        browser.stop_load();
    }
    Ok(json!({"tab": id, "status": status}))
}

fn follow(id: u32, snapshot: u32, link: u32) -> Result<Value, String> {
    let (browser, url, same_document) = STATE.with(|state| {
        let mut s = state.borrow_mut();
        let tab = s
            .tabs
            .get_mut(&id)
            .filter(|t| !t.closing && t.login.is_none())
            .ok_or("No such agent tab.")?;
        let (_, _, links) = tab
            .last_snapshot
            .as_ref()
            .filter(|(revision, url, _)| *revision == snapshot && *url == tab.url)
            .ok_or(PAGE_CHANGED)?;
        let link = link
            .checked_sub(1)
            .and_then(|i| links.get(i as usize))
            .ok_or("No such link in this snapshot.")?;
        let url = link.url.clone();
        let same_document = same_document_link(&tab.url, &url);
        tab.invalidate();
        Ok::<_, &str>((
            tab.browser.clone().ok_or("No such agent tab.")?,
            url,
            same_document,
        ))
    })?;
    let frame = browser
        .main_frame()
        .ok_or("The agent tab has no main frame yet.")?;
    if !same_document {
        loading(id);
    }
    frame.load_url(Some(&CefString::from(url.as_str())));
    Ok(json!({"tab": id, "url": url, "status": if same_document { "ready" } else { "loading" }}))
}

fn login_state(id: u32, status: &str, reason: &str) -> Value {
    json!({"tab": id, "status": status, "reason": reason})
}

fn finish_login(id: u32, status: &str, reason: &str) {
    let login = STATE.with(|state| {
        let mut s = state.borrow_mut();
        let tab = s.tabs.get_mut(&id)?;
        let login = tab.login.take()?;
        tab.invalidate();
        Some(login)
    });
    if let Some(login) = login {
        if let Some(request) = login.request {
            request.cancel();
        }
        let _ = login.reply.send(Ok(login_state(id, status, reason)));
    }
}

fn sign_in(id: u32, reply: Reply) {
    let start = STATE.with(|state| {
        let mut s = state.borrow_mut();
        s.next_login = s.next_login.checked_add(1).ok_or("attempt_limit")?;
        let attempt = s.next_login;
        let tab = s
            .tabs
            .get_mut(&id)
            .filter(|t| !t.closing)
            .ok_or("no_such_tab")?;
        if tab.login.is_some() {
            return Err("already_signing_in");
        }
        if tab.status != "ready"
            || !tab
                .http_status
                .is_some_and(|code| (200..300).contains(&code))
        {
            return Err("page_not_ready");
        }
        let origin = vault_store::origin(&tab.url).ok_or("https_required")?;
        if !vault_store::has(&origin).map_err(|_| "vault_unavailable")? {
            return Err("not_saved");
        }
        tab.invalidate();
        tab.login = Some(Login {
            attempt,
            origin,
            page: tab.url.clone(),
            reply: reply.clone(),
            request: None,
            destination: None,
            submitted_credential: None,
        });
        Ok::<_, &str>(attempt)
    });
    let attempt = match start {
        Ok(attempt) => attempt,
        Err(reason) => {
            let _ = reply.send(Ok(login_state(id, "action_required", reason)));
            return;
        }
    };
    let mut timeout = LoginTimeout::new(id, attempt);
    if post_delayed_task(ThreadId::UI, Some(&mut timeout), 9000) == 0 {
        finish_login(id, "action_required", "could_not_start");
        return;
    }
    method(
        id,
        Stage::Frame(Query::LoginForm),
        reply,
        "Page.getFrameTree",
        None,
    );
}

wrap_task! {
    struct LoginTimeout { tab: u32, attempt: u32 }
    impl Task {
        fn execute(&self) {
            let still_waiting = STATE.with(|state| {
                state.borrow().tabs.get(&self.tab).and_then(|tab| tab.login.as_ref())
                    .is_some_and(|login| login.attempt == self.attempt)
            });
            if still_waiting {
                finish_login(self.tab, "action_required", "timed_out");
            }
        }
    }
}

fn send_login(id: u32, data: vault_login::Probe) -> Result<(), &'static str> {
    let form = data.form.ok_or(match data.reason.as_deref() {
        Some("different_origin") => "different_origin",
        Some("no_password_form") => "no_password_form",
        _ => vault_login::UNSUPPORTED,
    })?;
    let (attempt, origin, mut context) = STATE.with(|state| {
        let s = state.borrow();
        let tab = s.tabs.get(&id).ok_or("no_such_tab")?;
        let login = tab.login.as_ref().ok_or("page_changed")?;
        if web_url(&form.page).as_deref() != Some(login.page.as_str()) || tab.url != login.page {
            return Err("page_changed");
        }
        Ok((
            login.attempt,
            login.origin.clone(),
            s.context.clone().ok_or("vault_unavailable")?,
        ))
    })?;
    let credential = vault_store::load(&origin)
        .map_err(|_| "vault_unavailable")?
        .ok_or("not_saved")?;
    let (action, mut request) = vault_login::request(&origin, &form, &credential)?;
    let mut client = VaultRequest::new(id, attempt, action);
    let request = urlrequest_create(Some(&mut request), Some(&mut client), Some(&mut context))
        .ok_or("could_not_start")?;
    update(id, |tab| {
        if let Some(login) = tab.login.as_mut().filter(|login| login.attempt == attempt) {
            login.request = Some(request);
            login.submitted_credential = Some(credential);
        }
    });
    Ok(())
}

wrap_urlrequest_client! {
    struct VaultRequest { tab: u32, attempt: u32, action: Url }
    impl UrlrequestClient {
        fn on_request_complete(&self, request: Option<&mut Urlrequest>) {
            let current = STATE.with(|state| {
                let mut state = state.borrow_mut();
                let tab = state.tabs.get_mut(&self.tab)?;
                let login = tab.login.as_mut().filter(|login| login.attempt == self.attempt)?;
                login.request.take();
                Some((login.origin.clone(), login.page.clone(), tab.browser.clone()?,
                      login.submitted_credential.take()?))
            });
            let Some((origin, page, browser, credential)) = current else { return };
            let target = request.and_then(|request|
                vault_login::continuation(request, &self.action, &origin, &credential))
                .filter(|url| url != &page);
            drop(credential);
            let Some((target, frame)) = target.zip(browser.main_frame()) else {
                finish_login(self.tab, "action_required", "login_not_verified");
                return;
            };
            update(self.tab, |tab| {
                if let Some(login) = tab.login.as_mut() {
                    login.destination = Some((target.clone(), false));
                }
            });
            frame.load_url(Some(&CefString::from(target.as_str())));
        }
    }
}

fn verify_login(id: u32, page: LoginPage) {
    let verified = STATE.with(|state| {
        let s = state.borrow();
        s.tabs.get(&id).and_then(|tab| {
            let login = tab.login.as_ref()?;
            Some(
                web_url(&page.url).as_deref() == Some(tab.url.as_str())
                    && login
                        .destination
                        .as_ref()
                        .is_some_and(|(url, used)| *used && url == &page.url)
                    && page.url != login.page
                    && !page.has_password
                    && vault_store::origin(&page.url).as_deref() == Some(login.origin.as_str()),
            )
        }) == Some(true)
    });
    if page.needs_mfa {
        finish_login(id, "action_required", "additional_verification");
    } else if verified {
        finish_login(id, "submitted", "destination_verified");
    } else {
        finish_login(id, "action_required", "login_not_verified");
    }
}

fn dictionary() -> Result<DictionaryValue, String> {
    dictionary_value_create().ok_or_else(|| "Flit could not allocate a page query.".into())
}

fn set_string(params: &DictionaryValue, key: &str, value: &str) {
    params.set_string(Some(&CefString::from(key)), Some(&CefString::from(value)));
}

fn method(id: u32, stage: Stage, reply: Reply, name: &str, mut params: Option<DictionaryValue>) {
    let browser = STATE.with(|state| {
        let mut s = state.borrow_mut();
        s.next_method = s.next_method.checked_add(1).unwrap_or(1);
        let method = s.next_method;
        let tab = s.tabs.get_mut(&id).filter(|t| !t.closing)?;
        let browser = tab.browser.clone()?;
        tab.pending = Some(Pending {
            method,
            stage,
            reply: reply.clone(),
        });
        Some((browser, method))
    });
    let Some((browser, method_id)) = browser else {
        if is_login_stage(stage) {
            finish_login(id, "action_required", "page_changed");
        } else {
            let _ = reply.send(Err("No such agent tab.".into()));
        }
        return;
    };
    if browser.host().is_none_or(|host| {
        host.execute_dev_tools_method(method_id, Some(&CefString::from(name)), params.as_mut()) == 0
    }) {
        if let Some(pending) = take_pending(id, method_id) {
            if is_login_stage(pending.stage) {
                finish_login(id, "action_required", "could_not_start");
            } else {
                let _ = pending
                    .reply
                    .send(Err("The page query could not start.".into()));
            }
        }
    }
}

fn is_login_stage(stage: Stage) -> bool {
    !matches!(
        stage,
        Stage::Frame(Query::Snapshot) | Stage::World(Query::Snapshot) | Stage::Read
    )
}

fn take_pending(id: u32, method: i32) -> Option<Pending> {
    STATE.with(|state| {
        let mut s = state.borrow_mut();
        let tab = s.tabs.get_mut(&id)?;
        if tab.pending.as_ref()?.method == method {
            tab.pending.take()
        } else {
            None
        }
    })
}

fn snapshot(id: u32, reply: Reply) {
    let state = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let tab = s
            .tabs
            .get_mut(&id)
            .filter(|t| !t.closing)
            .ok_or("No such agent tab.")?;
        if tab.login.is_some() {
            return Ok::<_, &str>(("signing_in", tab.info(id)));
        }
        tab.invalidate();
        Ok::<_, &str>((tab.status, tab.info(id)))
    });
    match state {
        Ok(("ready", _)) => method(
            id,
            Stage::Frame(Query::Snapshot),
            reply,
            "Page.getFrameTree",
            None,
        ),
        Ok((_, info)) => {
            let _ = reply.send(Ok(info));
        }
        Err(error) => {
            let _ = reply.send(Err(error.into()));
        }
    }
}

fn page_result(id: u32, mut page: Page) -> Result<Value, String> {
    let url = web_url(&page.url).ok_or("This page is not a supported HTTP or HTTPS document.")?;
    page.links.retain_mut(|link| {
        if let Some(url) = web_url(&link.url) {
            link.url = url;
            link.text.truncate(link.text.floor_char_boundary(512));
            true
        } else {
            false
        }
    });
    page.truncated |= page.text.len() > MAX_TEXT;
    page.text.truncate(page.text.floor_char_boundary(MAX_TEXT));
    page.title.truncate(page.title.floor_char_boundary(512));
    page.links.truncate(MAX_LINKS);
    STATE.with(|state| {
        let mut s = state.borrow_mut();
        s.next_snapshot = s
            .next_snapshot
            .checked_add(1)
            .ok_or("Snapshot IDs exhausted.")?;
        let revision = s.next_snapshot;
        let tab = s
            .tabs
            .get_mut(&id)
            .filter(|t| !t.closing && t.status == "ready" && t.url == url)
            .ok_or(PAGE_CHANGED)?;
        let links = page
            .links
            .iter()
            .enumerate()
            .map(|(i, link)| json!({"id": i + 1, "text": link.text, "url": link.url}))
            .collect::<Vec<_>>();
        tab.last_snapshot = Some((revision, url, page.links));
        let mut result = tab.info(id);
        result["title"] = json!(page.title);
        result["text"] = json!(page.text);
        result["links"] = json!(links);
        result["snapshot"] = json!(revision);
        result["truncated"] = json!(page.truncated);
        result["login_form_detected"] = json!(page.login_form_detected);
        Ok(result)
    })
}

fn receive(id: u32, pending: Pending, bytes: Option<&[u8]>) -> Result<(), String> {
    let result: Value = bytes
        .filter(|bytes| bytes.len() <= 512 * 1024)
        .and_then(|bytes| serde_json::from_slice(bytes).ok())
        .ok_or("The page was unavailable or the page query exceeded its size limit.")?;
    match pending.stage {
        Stage::Frame(query) => {
            let frame = result
                .pointer("/frameTree/frame/id")
                .and_then(Value::as_str)
                .ok_or("Could not locate the main page.")?;
            let params = dictionary()?;
            set_string(&params, "frameId", frame);
            set_string(&params, "worldName", "Flit Agent Reader");
            method(
                id,
                Stage::World(query),
                pending.reply,
                "Page.createIsolatedWorld",
                Some(params),
            );
        }
        Stage::World(query) => {
            let context = result
                .get("executionContextId")
                .and_then(Value::as_i64)
                .and_then(|id| i32::try_from(id).ok())
                .ok_or("Could not create the local page reader.")?;
            let params = dictionary()?;
            let (stage, expression) = match query {
                Query::Snapshot => (Stage::Read, READ_DOM),
                Query::LoginForm => (Stage::LoginForm, vault_login::FORM_SCRIPT),
                Query::LoginVerify => (Stage::LoginVerify, VERIFY_LOGIN),
            };
            set_string(&params, "expression", expression);
            params.set_int(Some(&CefString::from("contextId")), context);
            params.set_bool(Some(&CefString::from("returnByValue")), 1);
            params.set_bool(Some(&CefString::from("silent")), 1);
            method(id, stage, pending.reply, "Runtime.evaluate", Some(params));
        }
        Stage::Read => {
            let page = result
                .pointer("/result/value")
                .cloned()
                .and_then(|data| serde_json::from_value(data).ok())
                .ok_or("This document did not return a readable page snapshot.")?;
            let _ = pending.reply.send(page_result(id, page));
        }
        Stage::LoginForm => {
            let data = result
                .pointer("/result/value")
                .cloned()
                .and_then(|data| serde_json::from_value(data).ok())
                .ok_or("Could not check the login form.")?;
            if let Err(reason) = send_login(id, data) {
                finish_login(id, "action_required", reason);
            }
        }
        Stage::LoginVerify => {
            let page = result
                .pointer("/result/value")
                .cloned()
                .and_then(|data| serde_json::from_value(data).ok())
                .ok_or("Could not verify the destination page.")?;
            verify_login(id, page);
        }
    }
    Ok(())
}

wrap_dev_tools_message_observer! {
    struct AgentObserver { tab: u32 }
    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(
            &self, _browser: Option<&mut Browser>, method: i32, success: i32, result: Option<&[u8]>,
        ) {
            if let Some(pending) = take_pending(self.tab, method) {
                let reply = pending.reply.clone();
                let login = is_login_stage(pending.stage);
                if let Err(error) = receive(self.tab, pending, if success == 0 { None } else { result }) {
                    if login {
                        finish_login(self.tab, "action_required", "page_changed");
                    } else {
                        let _ = reply.send(Err(error));
                    }
                }
            }
        }
    }
}

fn execute(command: Command, reply: Reply) {
    let value = match command {
        Command::Open { url } => open(&url),
        Command::List => Ok(STATE.with(|state| {
            json!({"tabs": state.borrow().tabs.iter()
            .filter(|(_, tab)| !tab.closing).map(|(&id, tab)| tab.info(id)).collect::<Vec<_>>()})
        })),
        Command::Snapshot { tab } => {
            snapshot(tab, reply);
            return;
        }
        Command::Follow {
            tab,
            snapshot,
            link,
        } => follow(tab, snapshot, link),
        Command::Close { tab } => close(tab),
        Command::StopLoading { tab } => stop_loading(tab),
        Command::SignIn { tab } => {
            sign_in(tab, reply);
            return;
        }
    };
    let _ = reply.send(value);
}

fn stop(reply: Reply) {
    let ids = STATE.with(|state| {
        let mut s = state.borrow_mut();
        s.stopping = Some(reply);
        s.tabs
            .iter()
            .filter(|(_, tab)| !tab.closing)
            .map(|(&id, _)| id)
            .collect::<Vec<_>>()
    });
    for id in ids {
        let _ = close(id);
    }
    finish_stopping();
}

fn finish_stopping() {
    let reply = STATE.with(|state| {
        let mut s = state.borrow_mut();
        if s.tabs.is_empty() {
            let reply = s.stopping.take();
            if reply.is_some() {
                s.context = None;
            }
            reply
        } else {
            None
        }
    });
    if let Some(reply) = reply {
        // Let the final on_before_close callback return before the main thread shuts CEF down.
        let mut task = AgentTask::new(None, reply.clone(), true);
        if post_task(ThreadId::UI, Some(&mut task)) == 0 {
            let _ = reply.send(Err(CONTEXT_LOST.into()));
        }
    }
}

wrap_task! {
    struct AgentTask { command: Option<Command>, reply: Reply, stopped: bool }
    impl Task {
        fn execute(&self) {
            if self.stopped {
                let _ = self.reply.send(Ok(Value::Null));
            } else if let Some(command) = self.command.clone() {
                execute(command, self.reply.clone());
            } else {
                stop(self.reply.clone());
            }
        }
    }
}

wrap_app! {
    struct AgentApp { ready: Sender<bool> }
    impl App {
        fn on_before_command_line_processing(
            &self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>,
        ) {
            if process_type.is_none_or(|kind| kind.to_string().is_empty()) {
                if let Some(command) = command_line {
                    command.append_switch(Some(&CefString::from("disable-component-extensions-with-background-pages")));
                    command.append_switch(Some(&CefString::from("disable-background-networking")));
                    command.append_switch(Some(&CefString::from("disable-extensions")));
                    #[cfg(feature = "vault-fixture")]
                    command.append_switch_with_value(
                        Some(&CefString::from("ignore-certificate-errors-spki-list")),
                        Some(&CefString::from(env!("FLIT_FIXTURE_SPKI"))),
                    );
                }
            }
        }
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(AgentProcess::new(self.ready.clone()))
        }
    }
}

wrap_browser_process_handler! {
    struct AgentProcess { ready: Sender<bool> }
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            // Chrome can initialize a temporary default profile on disk. Agent tabs
            // use only this separate off-the-record context, as private desktop tabs do.
            let context = request_context_create_context(Some(&RequestContextSettings::default()), None)
                .filter(|ctx| ctx.is_global() == 0 && CefString::from(&ctx.cache_path()).to_string().is_empty());
            let ephemeral = context.is_some();
            STATE.with(|state| state.borrow_mut().context = context);
            let _ = self.ready.send(ephemeral);
        }
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> io::Result<Self> {
        let mut guid = GUID::default();
        if unsafe { CoCreateGuid(&mut guid) } < 0 {
            return Err(io::Error::other(
                "Could not create an isolated Flit agent session ID.",
            ));
        }
        let id = u64::from_be_bytes(guid.data4);
        let path = std::env::temp_dir().join(format!(
            "FlitAgent-{:08x}{:04x}{:04x}{id:016x}",
            guid.data1, guid.data2, guid.data3,
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Only the newly created, unique session directory is owned here.
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("Flit agent: could not remove its temporary session files: {error}");
        }
    }
}

fn request(command: Option<Command>) -> Result<Value, String> {
    let (reply, result) = mpsc::channel();
    let mut task = AgentTask::new(command, reply, false);
    if post_task(ThreadId::UI, Some(&mut task)) == 0 {
        return Err(CONTEXT_LOST.into());
    }
    result
        .recv_timeout(CALL_TIMEOUT)
        .unwrap_or_else(|_| Err("Flit agent request timed out.".into()))
}

pub(crate) fn run(args: &MainArgs, sandbox_info: *mut u8) -> i32 {
    let pipes = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE]
        .into_iter()
        .all(|which| (unsafe { GetFileType(GetStdHandle(which)) }) == FILE_TYPE_PIPE);
    if !pipes {
        eprintln!("Flit agent mode requires redirected MCP stdin and stdout pipes.");
        return 1;
    }
    let mut scratch = None;
    let mut initialized = false;
    let mut startup_attempted = false;
    let mut ready = false;
    let protocol = agent_protocol::serve(|command| {
        if !initialized && matches!(command, Command::List) {
            return Ok(json!({"tabs": []}));
        }
        if !initialized && !matches!(command, Command::Open { .. }) {
            return Err("No such agent tab.".into());
        }
        if let Command::Open { url } = &command {
            web_url(url).ok_or("Enter a literal HTTP or HTTPS URL without credentials.")?;
        }
        if !startup_attempted {
            let dir = Scratch::new().map_err(|e| e.to_string())?;
            let path = dir
                .0
                .to_str()
                .ok_or("Temporary directory path is not Unicode.")?;
            let (signal, result) = mpsc::channel();
            let mut app = AgentApp::new(signal);
            let settings = Settings {
                no_sandbox: 0,
                command_line_args_disabled: 1,
                multi_threaded_message_loop: 1,
                windowless_rendering_enabled: 1,
                root_cache_path: CefString::from(path),
                log_file: CefString::from(dir.0.join("cef.log").to_string_lossy().as_ref()),
                ..Default::default()
            };
            startup_attempted = true;
            if initialize(Some(args), Some(&settings), Some(&mut app), sandbox_info) == 0 {
                return Err(format!(
                    "Flit agent could not start CEF ({}).",
                    get_exit_code()
                ));
            }
            scratch = Some(dir);
            initialized = true;
            ready = result.recv_timeout(CALL_TIMEOUT) == Ok(true);
        }
        if !ready {
            return Err("Flit could not start an isolated, in-memory browser context.".into());
        }
        request(Some(command))
    });
    if initialized {
        if request(None).is_err() {
            eprintln!("Flit agent: browser shutdown did not complete.");
            // Avoid removing files from an engine that is still running.
            std::mem::forget(scratch.take());
            return 1;
        }
        shutdown();
    }
    drop(scratch);
    if let Err(error) = protocol {
        eprintln!("Flit agent: MCP stream ended with an error: {error}");
        return 1;
    }
    0
}
