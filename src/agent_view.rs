use crate::agent;
use cef::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering::Relaxed},
};
use url::{Origin, Url};

const LOAD_FAILED: &str = "The page could not be loaded.";
const REDIRECT_BLOCKED: &str = "The page redirected to an unsupported destination.";

pub(crate) fn client(tab: u32) -> cef::Client {
    AgentClient::new(tab, Arc::new(AtomicBool::new(false)))
}

wrap_client! {
    struct AgentClient { tab: u32, pending: Arc<AtomicBool> }
    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(AgentLife::new(self.tab))
        }
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(AgentDisplay::new(self.tab))
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(AgentLoad::new(self.tab, self.pending.clone()))
        }
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(AgentRender::new())
        }
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(AgentRequest::new(self.tab, self.pending.clone()))
        }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> {
            Some(AgentJsDialog::new())
        }
        fn dialog_handler(&self) -> Option<DialogHandler> {
            Some(AgentFileDialog::new())
        }
        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(AgentPermission::new())
        }
        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(AgentDownload::new())
        }
    }
}

wrap_life_span_handler! {
    struct AgentLife { tab: u32 }
    impl LifeSpanHandler {
        fn on_before_popup(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            _popup_id: i32, _url: Option<&CefString>, _name: Option<&CefString>,
            _disposition: WindowOpenDisposition, _gesture: i32, _features: Option<&PopupFeatures>,
            _window: Option<&mut WindowInfo>, _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>, _extra: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            1
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            // Windows does not route window.print() through CefPrintHandler.
            if let Some(context) = browser.and_then(|b| b.host()).and_then(|h| h.request_context())
                && let Some(mut disabled) = cef::value_create()
            {
                disabled.set_bool(0);
                context.set_preference(
                    Some(&CefString::from("printing.enabled")),
                    Some(&mut disabled),
                    Some(&mut CefString::from("")),
                );
            }
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            agent::closed(self.tab);
        }
    }
}

wrap_display_handler! {
    struct AgentDisplay { tab: u32 }
    impl DisplayHandler {
        fn on_address_change(
            &self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            if frame.is_some_and(|frame| frame.is_main() != 0) && let Some(url) = url {
                agent::address(self.tab, url.to_string());
            }
        }

        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            agent::title(self.tab, title.map(CefString::to_string).unwrap_or_default());
        }
    }
}

wrap_load_handler! {
    struct AgentLoad { tab: u32, pending: Arc<AtomicBool> }
    impl LoadHandler {
        fn on_loading_state_change(
            &self, _browser: Option<&mut Browser>, loading: i32, _back: i32, _forward: i32,
        ) {
            // A provisional navigation can end without a document, e.g. a denied download.
            if loading == 0 && self.pending.swap(false, Relaxed) {
                agent::failed(self.tab, LOAD_FAILED);
            }
        }

        fn on_load_end(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, status: i32) {
            if let Some(frame) = frame.filter(|frame| frame.is_main() != 0) {
                let url = CefString::from(&frame.url()).to_string();
                if agent::web_url(&url).is_some() {
                    self.pending.store(false, Relaxed);
                    agent::loaded(self.tab, url, status);
                }
            }
        }

        fn on_load_error(
            &self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>,
            code: Errorcode, _text: Option<&CefString>, _url: Option<&CefString>,
        ) {
            if !frame.is_some_and(|frame| frame.is_main() != 0) {
                return;
            }
            let reason = match code {
                // CEF also reports this for rejected navigations that leave the page intact.
                Errorcode::ABORTED => return,
                Errorcode::UNSAFE_REDIRECT => REDIRECT_BLOCKED,
                _ => LOAD_FAILED,
            };
            self.pending.store(false, Relaxed);
            agent::failed(self.tab, reason);
        }
    }
}

wrap_render_handler! {
    struct AgentRender;
    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                *rect = Rect { x: 0, y: 0, width: 1024, height: 768 };
            }
        }
    }
}

wrap_request_handler! {
    struct AgentRequest { tab: u32, pending: Arc<AtomicBool> }
    impl RequestHandler {
        fn on_before_browse(
            &self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>,
            request: Option<&mut Request>, _gesture: i32, redirect: i32,
        ) -> i32 {
            let (Some(frame), Some(request)) = (frame, request) else { return 1 };
            if frame.is_main() == 0 {
                return 0;
            }
            let url = CefString::from(&request.url()).to_string();
            let method = CefString::from(&request.method()).to_string();
            if agent::web_url(&url).is_some() && matches!(method.as_str(), "GET" | "HEAD") {
                let current = CefString::from(&frame.url()).to_string();
                if redirect == 0 && !agent::same_document_link(&current, &url) {
                    self.pending.store(true, Relaxed);
                    agent::loading(self.tab);
                }
                return 0;
            }
            if redirect != 0 {
                self.pending.store(false, Relaxed);
                agent::failed(self.tab, REDIRECT_BLOCKED);
            }
            1
        }

        fn on_open_urlfrom_tab(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            _url: Option<&CefString>, disposition: WindowOpenDisposition, _gesture: i32,
        ) -> i32 {
            i32::from(disposition != WindowOpenDisposition::CURRENT_TAB)
        }

        fn resource_request_handler(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            _request: Option<&mut Request>, navigation: i32, download: i32,
            initiator: Option<&CefString>, _disable_default: Option<&mut i32>,
        ) -> Option<ResourceRequestHandler> {
            let http_origin = initiator
                .and_then(|value| Url::parse(&value.to_string()).ok())
                .filter(|url| url.scheme() == "http")
                .map(|url| url.origin());
            Some(AgentResource::new(http_origin, navigation != 0, download != 0))
        }

        fn auth_credentials(
            &self, _browser: Option<&mut Browser>, _origin: Option<&CefString>,
            _proxy: i32, _host: Option<&CefString>, _port: i32, _realm: Option<&CefString>,
            _scheme: Option<&CefString>, callback: Option<&mut AuthCallback>,
        ) -> i32 {
            if let Some(callback) = callback {
                callback.cancel();
                return 1;
            }
            0
        }

        fn on_select_client_certificate(
            &self, _browser: Option<&mut Browser>, _proxy: i32, _host: Option<&CefString>,
            _port: i32, _certificates: Option<&[Option<X509Certificate>]>,
            callback: Option<&mut SelectClientCertificateCallback>,
        ) -> i32 {
            if let Some(callback) = callback {
                callback.select(None);
            }
            1
        }

        fn on_render_process_terminated(
            &self, _browser: Option<&mut Browser>, _status: TerminationStatus,
            _code: i32, _text: Option<&CefString>,
        ) {
            self.pending.store(false, Relaxed);
            agent::failed(self.tab, "The page renderer stopped.");
        }
    }
}

wrap_resource_request_handler! {
    struct AgentResource { http_origin: Option<Origin>, navigation: bool, download: bool }
    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            request: Option<&mut Request>, _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            let Some(request) = request.filter(|_| !self.download) else {
                return ReturnValue::CANCEL;
            };
            if !matches!(CefString::from(&request.method()).to_string().as_str(), "GET" | "HEAD") {
                return ReturnValue::CANCEL;
            }
            let Ok(url) = Url::parse(&CefString::from(&request.url()).to_string()) else {
                return ReturnValue::CANCEL;
            };
            // Permit incidental HTTPS; cleartext subresources stay on their HTTP page's origin.
            let main_navigation = self.navigation && request.resource_type() == ResourceType::MAIN_FRAME;
            let allowed = match url.scheme() {
                "https" => true,
                "http" => main_navigation || self.http_origin.as_ref() == Some(&url.origin()),
                _ => false,
            };
            if allowed && url.username().is_empty() && url.password().is_none() {
                ReturnValue::CONTINUE
            } else {
                ReturnValue::CANCEL
            }
        }

        fn on_protocol_execution(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            _request: Option<&mut Request>, allow: Option<&mut i32>,
        ) {
            if let Some(allow) = allow {
                *allow = 0;
            }
        }
    }
}

wrap_jsdialog_handler! {
    struct AgentJsDialog;
    impl JsdialogHandler {
        fn on_jsdialog(
            &self, _browser: Option<&mut Browser>, _origin: Option<&CefString>,
            _kind: JsdialogType, _message: Option<&CefString>, _prompt: Option<&CefString>,
            callback: Option<&mut JsdialogCallback>, suppress: Option<&mut i32>,
        ) -> i32 {
            if let Some(suppress) = suppress {
                *suppress = 1;
                return 0;
            }
            if let Some(callback) = callback {
                callback.cont(0, None);
            }
            1
        }

        fn on_before_unload_dialog(
            &self, _browser: Option<&mut Browser>, _message: Option<&CefString>,
            _reload: i32, callback: Option<&mut JsdialogCallback>,
        ) -> i32 {
            if let Some(callback) = callback {
                callback.cont(1, None);
            }
            1
        }
    }
}

wrap_dialog_handler! {
    struct AgentFileDialog;
    impl DialogHandler {
        fn on_file_dialog(
            &self, _browser: Option<&mut Browser>, _mode: FileDialogMode,
            _title: Option<&CefString>, _path: Option<&CefString>,
            _filters: Option<&mut CefStringList>, _extensions: Option<&mut CefStringList>,
            _descriptions: Option<&mut CefStringList>, callback: Option<&mut FileDialogCallback>,
        ) -> i32 {
            if let Some(callback) = callback {
                callback.cancel();
            }
            1
        }
    }
}

wrap_permission_handler! {
    struct AgentPermission;
    impl PermissionHandler {
        fn on_request_media_access_permission(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            _origin: Option<&CefString>, _permissions: u32, callback: Option<&mut MediaAccessCallback>,
        ) -> i32 {
            if let Some(callback) = callback {
                callback.cancel();
            }
            1
        }

        fn on_show_permission_prompt(
            &self, _browser: Option<&mut Browser>, _id: u64, _origin: Option<&CefString>,
            _permissions: u32, callback: Option<&mut PermissionPromptCallback>,
        ) -> i32 {
            if let Some(callback) = callback {
                callback.cont(PermissionRequestResult::DENY);
            }
            1
        }
    }
}

wrap_download_handler! {
    struct AgentDownload;
    impl DownloadHandler {
        fn can_download(
            &self, _browser: Option<&mut Browser>, _url: Option<&CefString>,
            _method: Option<&CefString>,
        ) -> i32 {
            0
        }

        fn on_before_download(
            &self, _browser: Option<&mut Browser>, _item: Option<&mut DownloadItem>,
            _name: Option<&CefString>, _callback: Option<&mut BeforeDownloadCallback>,
        ) -> i32 {
            // Dropping the continuation cancels without offering a save dialog.
            1
        }

        fn on_download_updated(
            &self, _browser: Option<&mut Browser>, item: Option<&mut DownloadItem>,
            callback: Option<&mut DownloadItemCallback>,
        ) {
            if item.is_some_and(|item| item.is_valid() != 0 && item.is_in_progress() != 0)
                && let Some(callback) = callback
            {
                callback.cancel();
            }
        }
    }
}
