use cef::{
    CefString, Frame, ImplDictionaryValue, ImplFrame, ImplValue, JsonWriterOptions, ThreadId,
};

pub(crate) const MANAGER_URL: &str = "chrome://bookmarks/";

pub(crate) enum Request {
    Save(CefString),
    Import,
    Folder(CefString),
    Export,
}

impl Request {
    pub(crate) fn save(url: &str, title: &str) -> Option<Self> {
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
        Some(Self::Save(json))
    }

    pub(crate) fn import_profile(
        title: &str,
        nodes: &[crate::site_setup::BookmarkNode],
    ) -> Option<Self> {
        Self::folder(Some(title), nodes)
    }

    pub(crate) fn save_tabs(nodes: &[crate::site_setup::BookmarkNode]) -> Option<Self> {
        if nodes.is_empty() {
            return None;
        }
        Self::folder(None, nodes)
    }

    fn folder(title: Option<&str>, nodes: &[crate::site_setup::BookmarkNode]) -> Option<Self> {
        // String-encode the JSON once more before using it as a JS expression.
        let json = serde_json::to_string(&(title, nodes)).ok()?;
        if json.len() > 2 * 1024 * 1024 {
            return None;
        }
        let quoted = serde_json::to_string(&json).ok()?;
        Some(Self::Folder(CefString::from(quoted.as_str())))
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
        let script = match self {
            Self::Save(json) => format!(
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
                json
            ),
            Self::Folder(quoted_json) => format!(
                r#"(async () => {{
    if (window !== window.top || location.href !== "chrome://bookmarks/") return;
    if (window.flitBookmarkFolderWriteRunning) return;
    window.flitBookmarkFolderWriteRunning = true;
    let saving = false;
    let added = 0;
    try {{
        const [title, nodes] = JSON.parse({});
        saving = title === null;
        document.title = saving ? "Bookmarks — Saving tabs" : "Bookmarks — Importing profile";
        const [root] = await chrome.bookmarks.getTree();
        const other = root.children.find(n => n.folderType === "other" && !n.syncing && !n.unmodifiable)
            ?? root.children.find(n => n.id === "2" && !n.syncing && !n.unmodifiable);
        if (!other) throw new Error("No local bookmark folder");
        const children = await chrome.bookmarks.getChildren(other.id);
        let destination;
        if (saving) {{
            const now = new Date();
            const pad = n => String(n).padStart(2, "0");
            const base = `Saved tabs - ${{now.getFullYear()}}-${{pad(now.getMonth() + 1)}}-${{pad(now.getDate())}} ${{pad(now.getHours())}}:${{pad(now.getMinutes())}}`;
            const titles = new Set(children.map(n => n.title));
            let name = base;
            for (let i = 2; titles.has(name); i++) name = `${{base}} (${{i}})`;
            destination = await chrome.bookmarks.create({{parentId: other.id, title: name}});
        }} else {{
            destination = children.find(n => !n.url && !n.unmodifiable && n.title === title)
                ?? await chrome.bookmarks.create({{parentId: other.id, title}});
        }}

        const append = async (parentId, items) => {{
            if (!items.length) return;
            const existing = await chrome.bookmarks.getChildren(parentId);
            const folders = new Map();
            const key = n => {{
                let address = n.url;
                try {{ address = new URL(address).href; }} catch {{}}
                return JSON.stringify([n.title, address]);
            }};
            const links = new Set();
            for (const child of existing) {{
                if (child.url) links.add(key(child));
                else if (!child.unmodifiable) folders.set(child.title, child);
            }}
            for (const node of items) {{
                if (node.url) {{
                    const id = key(node);
                    if (links.has(id)) continue;
                    await chrome.bookmarks.create({{parentId, title: node.title, url: node.url}});
                    links.add(id);
                    added += 1;
                    continue;
                }}
                const folder = folders.get(node.title)
                    ?? await chrome.bookmarks.create({{parentId, title: node.title}});
                folders.set(node.title, folder);
                await append(folder.id, node.children);
            }}
        }};
        await append(destination.id, nodes);
        document.title = saving
            ? `Bookmarks — Saved ${{added}} tabs`
            : `Bookmarks — Imported ${{added}} bookmarks`;
    }} catch {{
        document.title = `Bookmarks — ${{saving ? "Save" : "Import"}} incomplete (${{added}} added)`;
    }} finally {{
        window.flitBookmarkFolderWriteRunning = false;
    }}
}})();"#,
                quoted_json
            ),
            transfer => {
                let method = if matches!(transfer, Self::Import) {
                    "import"
                } else {
                    "export"
                };
                // Use the same API as the Bookmark Manager menu. The native
                // file picker and Chromium own the file and bookmark writes.
                format!(
                    r#"(async () => {{
    if (window !== window.top || location.href !== "chrome://bookmarks/") return;
    try {{
        await chrome.bookmarkManagerPrivate.{method}();
    }} catch {{
        document.title = "Bookmarks — Could not open file picker";
    }}
}})();"#
                )
            }
        };
        frame.execute_java_script(Some(&CefString::from(script.as_str())), None, 0);
        true
    }
}
