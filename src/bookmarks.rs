use cef::{
    CefString, Frame, ImplDictionaryValue, ImplFrame, ImplValue, JsonWriterOptions, ThreadId,
};

pub(crate) const MANAGER_URL: &str = "chrome://bookmarks/";

pub(crate) struct SaveRequest(CefString);

impl SaveRequest {
    pub(crate) fn new(url: &str, title: &str) -> Option<Self> {
        let parsed = url::Url::parse(url).ok()?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return None;
        }
        let mut details = cef::dictionary_value_create()?;
        let mut value = cef::value_create()?;
        let title = if title.is_empty() { url } else { title };
        if details.set_string(Some(&"url".into()), Some(&url.into())) == 0
            || details.set_string(Some(&"title".into()), Some(&title.into())) == 0
            || value.set_dictionary(Some(&mut details)) == 0
        {
            return None;
        }
        let json = CefString::from(&cef::write_json(
            Some(&mut value),
            JsonWriterOptions::DEFAULT,
        ));
        if json.to_string().is_empty() {
            return None;
        }
        Some(Self(json))
    }

    /// Consume a pending request on the UI thread after the owned manager's main-frame load.
    /// True means the script was dispatched; the WebUI itself reports the API result.
    pub(crate) fn dispatch_on_load_end(self, frame: &Frame) -> bool {
        if cef::currently_on(ThreadId::UI) == 0
            || frame.is_valid() == 0
            || frame.is_main() == 0
            || CefString::from(&frame.url()).to_string() != MANAGER_URL
        {
            return false;
        }
        // The renderer may have started another navigation since the native frame check.
        let script = format!(
            r#"(async () => {{
    if (window !== window.top || location.href !== "chrome://bookmarks/") return;
    try {{
        const {{url, title}} = {};
        const existing = await chrome.bookmarks.search({{url}});
        if (existing.length) {{
            document.title = "Bookmarks — Already saved";
            return;
        }}
        const added = await chrome.bookmarks.create({{url, title}});
        if (added?.url !== url) throw new Error("Bookmark could not be verified");
        document.title = "Bookmarks — Page saved";
    }} catch {{
        document.title = "Bookmarks — Could not save page";
    }}
}})();"#,
            self.0,
        );
        frame.execute_java_script(Some(&CefString::from(script.as_str())), None, 0);
        true
    }
}
