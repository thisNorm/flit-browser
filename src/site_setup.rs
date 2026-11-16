use std::{
    collections::BTreeSet,
    fmt, fs,
    io::Read,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf, Prefix},
};

use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde_json::Value;
use url::Url;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_READ,
};

pub(crate) const MAX_SITES: usize = 12;
const MAX_PROFILE_ENTRIES: usize = 256;
const MAX_BOOKMARK_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_IMPORT_DEPTH: usize = 24;
const MAX_IMPORT_NODES: usize = 4096;
const MAX_IMPORT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(crate) enum SourceBrowser {
    Chrome,
    Edge,
}

impl SourceBrowser {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
        }
    }
}

pub(crate) struct SourceProfile {
    pub label: String,
    path: PathBuf,
}

#[derive(serde::Serialize)]
pub(crate) struct BookmarkNode {
    pub title: String,
    pub url: Option<String>,
    pub children: Vec<BookmarkNode>,
}

pub(crate) fn discover(local_app_data: &Path, browser: SourceBrowser) -> Vec<SourceProfile> {
    let (vendor, product) = match browser {
        SourceBrowser::Chrome => ("Google", "Chrome"),
        SourceBrowser::Edge => ("Microsoft", "Edge"),
    };
    let root = local_app_data.join(vendor).join(product).join("User Data");
    let Some(_directories) = hold_directories(&root) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut profiles: Vec<_> = entries
        .take(MAX_PROFILE_ENTRIES)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let label = entry.file_name().into_string().ok()?;
            if !is_profile_name(&label) {
                return None;
            }
            let directory = entry.path();
            let _profile = hold_directory(&directory)?;
            let path = directory.join("Bookmarks");
            let metadata = fs::symlink_metadata(&path).ok()?;
            (metadata.is_file() && !is_reparse(&metadata)).then_some(SourceProfile { label, path })
        })
        .collect();
    profiles.sort_unstable_by(|left, right| left.label.cmp(&right.label));
    profiles
}

fn is_profile_name(name: &str) -> bool {
    name == "Default"
        || name.strip_prefix("Profile ").is_some_and(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn hold_directories(path: &Path) -> Option<Vec<fs::File>> {
    if !path.is_absolute()
        || path.components().any(|part| match part {
            Component::ParentDir => true,
            Component::Prefix(p) => matches!(p.kind(), Prefix::DeviceNS(_) | Prefix::Verbatim(_)),
            _ => false,
        })
    {
        return None;
    }
    // Open each directory before using it to reach the next component.
    let ancestors: Vec<_> = path.ancestors().collect();
    ancestors.into_iter().rev().map(hold_directory).collect()
}

fn hold_directory(path: &Path) -> Option<fs::File> {
    // Keep the directory open without write or delete sharing while using its name.
    let directory = fs::File::options()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let metadata = directory.metadata().ok()?;
    (metadata.is_dir() && !is_reparse(&metadata)).then_some(directory)
}

fn is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn read_bookmark_roots(source: &SourceProfile) -> Result<Vec<Value>, &'static str> {
    const UNREADABLE: &str = "Could not safely read the selected bookmarks file.";
    const TOO_LARGE: &str = "The bookmarks file is too large.";

    let directories =
        hold_directories(source.path.parent().ok_or(UNREADABLE)?).ok_or(UNREADABLE)?;
    let file = fs::File::options()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&source.path)
        .map_err(|_| UNREADABLE)?;
    let metadata = file.metadata().map_err(|_| UNREADABLE)?;
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err(UNREADABLE);
    }
    drop(directories);
    if metadata.len() > MAX_BOOKMARK_FILE_BYTES as u64 {
        return Err(TOO_LARGE);
    }
    let mut bytes = Vec::new();
    file.take(MAX_BOOKMARK_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UNREADABLE)?;
    if bytes.len() > MAX_BOOKMARK_FILE_BYTES {
        return Err(TOO_LARGE);
    }
    parse_bookmark_roots(&bytes)
}

fn parse_bookmark_roots(bytes: &[u8]) -> Result<Vec<Value>, &'static str> {
    #[derive(serde::Deserialize)]
    struct BookmarkFile {
        #[serde(deserialize_with = "ordered_roots")]
        roots: Vec<Value>,
    }

    serde_json::from_slice::<BookmarkFile>(bytes)
        .map(|file| file.roots)
        .map_err(|_| "The bookmarks file is invalid or nested too deeply.")
}

fn ordered_roots<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Value>, D::Error> {
    struct Roots;

    impl<'de> Visitor<'de> for Roots {
        type Value = Vec<Value>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map of bookmark roots")
        }

        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            // Reading the map directly preserves file order without extra serde features.
            let mut roots = Vec::new();
            while let Some((_, root)) = map.next_entry::<IgnoredAny, Value>()? {
                roots.push(root);
            }
            Ok(roots)
        }
    }

    deserializer.deserialize_map(Roots)
}

pub(crate) fn bookmarked_sites(
    source: &SourceProfile,
    open: &BTreeSet<String>,
) -> Result<Vec<String>, &'static str> {
    Ok(sites_from_roots(&read_bookmark_roots(source)?, open))
}

fn sites_from_roots(roots: &[Value], open: &BTreeSet<String>) -> Vec<String> {
    let mut sites = Vec::new();
    for root in roots {
        if let Some(children) = folder_children(root, true) {
            collect_sites(children, open, &mut sites);
        }
        if sites.len() == MAX_SITES {
            break;
        }
    }
    sites
}

fn collect_sites(nodes: &[Value], open: &BTreeSet<String>, sites: &mut Vec<String>) {
    for node in nodes {
        if let Some(raw) = bookmark_url(node) {
            add_site(sites, open, raw);
        } else if let Some(children) = folder_children(node, false) {
            collect_sites(children, open, sites);
        }
        if sites.len() == MAX_SITES {
            break;
        }
    }
}

pub(crate) fn profile_bookmarks(
    source: &SourceProfile,
) -> Result<(Vec<BookmarkNode>, usize), &'static str> {
    import_roots(&read_bookmark_roots(source)?)
}

fn import_roots(roots: &[Value]) -> Result<(Vec<BookmarkNode>, usize), &'static str> {
    let mut remaining = MAX_IMPORT_NODES;
    let mut leaves = 0;
    let mut bookmarks = Vec::new();
    for root in roots {
        if let Some(node) = import_node(root, true, 1, &mut remaining, &mut leaves)? {
            bookmarks.push(node);
        }
    }
    let bytes = serde_json::to_vec(&bookmarks).map_err(|_| "Could not encode the bookmarks.")?;
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err("The imported bookmarks are too large.");
    }
    Ok((bookmarks, leaves))
}

fn import_node(
    node: &Value,
    root: bool,
    depth: usize,
    remaining: &mut usize,
    leaves: &mut usize,
) -> Result<Option<BookmarkNode>, &'static str> {
    let folder = folder_children(node, root);
    let url = if root {
        None
    } else {
        bookmark_url(node).filter(|raw| parsed_web_url(raw).is_some())
    };
    if folder.is_none() && url.is_none() {
        return Ok(None);
    }
    if depth > MAX_IMPORT_DEPTH {
        return Err("The bookmark folders are nested too deeply.");
    }
    *remaining = remaining
        .checked_sub(1)
        .ok_or("The bookmarks contain too many items.")?;

    let mut children = Vec::new();
    if let Some(folder) = folder {
        for child in folder {
            if let Some(child) = import_node(child, false, depth + 1, remaining, leaves)? {
                children.push(child);
            }
        }
    }
    *leaves += usize::from(url.is_some());
    Ok(Some(BookmarkNode {
        title: node
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        url: url.map(str::to_owned),
        children,
    }))
}

fn folder_children(node: &Value, root: bool) -> Option<&[Value]> {
    match node.get("type").and_then(Value::as_str) {
        Some("folder") => {}
        None if root => {}
        _ => return None,
    }
    node.get("children")?.as_array().map(Vec::as_slice)
}

fn bookmark_url(node: &Value) -> Option<&str> {
    if node.get("type").and_then(Value::as_str) == Some("url") {
        node.get("url")?.as_str()
    } else {
        None
    }
}

pub(crate) fn pasted_sites(input: &str, open: &BTreeSet<String>) -> Vec<String> {
    let mut sites = Vec::new();
    for raw in input.split_whitespace() {
        add_site(&mut sites, open, raw);
        if sites.len() == MAX_SITES {
            break;
        }
    }
    sites
}

fn add_site(sites: &mut Vec<String>, open: &BTreeSet<String>, raw: &str) {
    if let Some(origin) = site_origin(raw) {
        if !open.contains(&origin) && !sites.contains(&origin) {
            sites.push(origin);
        }
    }
}

pub(crate) fn site_origin(raw: &str) -> Option<String> {
    let url = parsed_web_url(raw.trim())?;
    Some(format!("{}/", url.origin().ascii_serialization()))
}

fn parsed_web_url(raw: &str) -> Option<Url> {
    let (scheme, tail) = raw.split_once("://")?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || raw
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '\\' | '"'))
    {
        return None;
    }
    // The URL parser can repair an empty authority or discard empty userinfo.
    let authority = tail.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    (url.has_host() && url.username().is_empty() && url.password().is_none()).then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BOOKMARKS: &[u8] = br#"{
        "meta": {"type":"url", "url":"https://ignored.test"},
        "roots": {
            "synced": {"type":"folder", "name":"Mobile bookmarks", "children":[
                {"type":"folder", "name":"Work", "children":[
                    {"type":"url", "name":"Inbox", "url":"https://Z.test/inbox?q=a@b#unread"},
                    {"type":"url", "name":"Secret", "url":"https://user:pass@secret.test/"},
                    {"type":"url", "name":"Local", "url":"file:///C:/local.html"}
                ]},
                {"type":"url", "name":"Docs", "url":"http://a.test:8080/docs"},
                {"type":"url", "url":12}, null
            ]},
            "bookmark_bar": {"type":"folder", "name":"Bar", "children":[
                {"type":"url", "name":"Another", "url":"https://z.test/another"}
            ]}
        }
    }"#;

    #[test]
    fn accepts_only_standard_profile_names() {
        for label in ["Default", "Profile 0", "Profile 42"] {
            assert!(is_profile_name(label));
        }
        for label in [
            "Guest Profile",
            "Profile ",
            "Profile -1",
            "Profile 1/2",
            "Profile １",
        ] {
            assert!(!is_profile_name(label));
        }
    }

    #[test]
    fn normalizes_origins_and_rejects_ambiguous_or_non_web_urls() {
        assert_eq!(
            site_origin(" HTTPS://Example.TEST:443/a?q=1#b ").as_deref(),
            Some("https://example.test/")
        );
        assert_eq!(
            site_origin("http://[::1]:8080/login").as_deref(),
            Some("http://[::1]:8080/")
        );
        for raw in [
            "https://user@example.test/",
            "https://:pass@example.test/",
            "https://@example.test/",
            "https:///example.test/",
            "https://example.test:65536/",
            "https://exam\tple.test/",
            r"https://other.test\@example.test/",
            "https:example.test/",
            "example.test",
            "chrome://settings/",
            "javascript:alert(1)",
            "file:///C:/local.html",
        ] {
            assert_eq!(site_origin(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn copied_links_keep_first_origin_order_and_stop_at_the_limit() {
        let mut input =
            String::from("https://two.test/a\r\nhttp://one.test\tHTTPS://TWO.TEST/b invalid\n");
        for i in 0..MAX_SITES {
            input.push_str(&format!("https://site-{i}.test/a\n"));
        }
        let sites = pasted_sites(&input, &BTreeSet::from(["https://site-0.test/".to_owned()]));
        assert_eq!(sites.len(), MAX_SITES);
        assert_eq!(sites[0], "https://two.test/");
        assert_eq!(sites[1], "http://one.test/");
        assert!(!sites.iter().any(|site| site == "https://site-0.test/"));
        assert_eq!(sites[MAX_SITES - 1], "https://site-10.test/");
    }

    #[test]
    fn sites_follow_bookmark_file_order_and_ignore_bad_nodes() {
        let roots = parse_bookmark_roots(BOOKMARKS).unwrap();
        assert_eq!(
            sites_from_roots(&roots, &BTreeSet::new()),
            ["https://z.test/", "http://a.test:8080/"]
        );
        for invalid in [b"{".as_slice(), b"{}", br#"{"roots":[]}"#] {
            assert!(parse_bookmark_roots(invalid).is_err());
        }
    }

    #[test]
    fn import_preserves_folders_titles_and_full_safe_urls() {
        let (bookmarks, leaves) = import_roots(&parse_bookmark_roots(BOOKMARKS).unwrap()).unwrap();
        assert_eq!(leaves, 3);
        assert_eq!(
            serde_json::to_value(bookmarks).unwrap(),
            json!([
                {"title":"Mobile bookmarks", "url":null, "children":[
                    {"title":"Work", "url":null, "children":[
                        {"title":"Inbox", "url":"https://Z.test/inbox?q=a@b#unread", "children":[]}
                    ]},
                    {"title":"Docs", "url":"http://a.test:8080/docs", "children":[]}
                ]},
                {"title":"Bar", "url":null, "children":[
                    {"title":"Another", "url":"https://z.test/another", "children":[]}
                ]}
            ])
        );
    }

    #[test]
    fn import_returns_an_error_instead_of_a_partial_tree() {
        let mut deep = json!({"type":"folder", "children":[]});
        for _ in 0..MAX_IMPORT_DEPTH {
            deep = json!({"type":"folder", "children":[deep]});
        }
        assert!(import_roots(&[deep]).is_err());

        let empty = json!({"type":"folder", "children":[]});
        let wide = json!({"type":"folder", "children":vec![empty; MAX_IMPORT_NODES]});
        assert!(import_roots(&[wide]).is_err());

        let large = json!({"type":"folder", "name":"x".repeat(MAX_IMPORT_BYTES), "children":[]});
        assert!(import_roots(&[large]).is_err());
    }
}
