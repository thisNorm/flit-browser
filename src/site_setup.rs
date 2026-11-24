use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::Read,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf, Prefix},
};

use serde::de::{MapAccess, Visitor};
use serde_json::Value;
use url::Url;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_READ,
};

pub(crate) const MAX_SITES: usize = 12;
const MAX_PASTED_TAB_BYTES: usize = 256 * 1024;
const MAX_PROFILE_ENTRIES: usize = 256;
const MAX_BOOKMARK_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_IMPORT_DEPTH: usize = 24;
const MAX_IMPORT_NODES: usize = 4096;
const MAX_IMPORT_BYTES: usize = 2 * 1024 * 1024;
const UNREADABLE_BOOKMARKS: &str = "Could not safely read the selected bookmarks file.";

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
    directory: PathBuf,
    files: Vec<&'static str>,
}

#[derive(serde::Serialize)]
pub(crate) struct BookmarkNode {
    pub title: String,
    pub url: Option<String>,
    pub children: Vec<BookmarkNode>,
}

type BookmarkRoot = (String, Value);

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
            let files: Vec<_> = ["Bookmarks", "AccountBookmarks"]
                .into_iter()
                .filter(|name| {
                    fs::symlink_metadata(directory.join(name))
                        .is_ok_and(|metadata| metadata.is_file() && !is_reparse(&metadata))
                })
                .collect();
            (!files.is_empty()).then_some(SourceProfile {
                label,
                directory,
                files,
            })
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

fn read_bookmark_roots(source: &SourceProfile) -> Result<Vec<BookmarkRoot>, &'static str> {
    let _directories = hold_directories(&source.directory).ok_or(UNREADABLE_BOOKMARKS)?;
    let mut roots = Vec::new();
    for name in &source.files {
        roots.extend(read_bookmark_file(&source.directory.join(name))?);
    }
    Ok(roots)
}

fn read_bookmark_file(path: &Path) -> Result<Vec<BookmarkRoot>, &'static str> {
    const TOO_LARGE: &str = "The bookmarks file is too large.";
    let file = fs::File::options()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| UNREADABLE_BOOKMARKS)?;
    let metadata = file.metadata().map_err(|_| UNREADABLE_BOOKMARKS)?;
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err(UNREADABLE_BOOKMARKS);
    }
    if metadata.len() > MAX_BOOKMARK_FILE_BYTES as u64 {
        return Err(TOO_LARGE);
    }
    let mut bytes = Vec::new();
    file.take(MAX_BOOKMARK_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UNREADABLE_BOOKMARKS)?;
    if bytes.len() > MAX_BOOKMARK_FILE_BYTES {
        return Err(TOO_LARGE);
    }
    parse_bookmark_roots(&bytes)
}

fn parse_bookmark_roots(bytes: &[u8]) -> Result<Vec<BookmarkRoot>, &'static str> {
    #[derive(serde::Deserialize)]
    struct BookmarkFile {
        #[serde(deserialize_with = "ordered_roots")]
        roots: Vec<BookmarkRoot>,
    }

    serde_json::from_slice::<BookmarkFile>(bytes)
        .map(|file| file.roots)
        .map_err(|_| "The bookmarks file is invalid or nested too deeply.")
}

fn ordered_roots<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<BookmarkRoot>, D::Error> {
    struct Roots;

    impl<'de> Visitor<'de> for Roots {
        type Value = Vec<BookmarkRoot>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map of bookmark roots")
        }

        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            // Reading the map directly preserves file order without extra serde features.
            let mut roots = Vec::new();
            while let Some(root) = map.next_entry()? {
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

fn sites_from_roots(roots: &[BookmarkRoot], open: &BTreeSet<String>) -> Vec<String> {
    let mut sites = Vec::new();
    for (_, root) in roots {
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

/// Links from every bookmark folder, with depth for choosing start-page shortcuts.
pub(crate) fn sidebar_bookmarks(file: &Path) -> Vec<(String, String, usize)> {
    fn walk(nodes: &[BookmarkNode], depth: usize, out: &mut Vec<(String, String, usize)>) {
        for node in nodes {
            if let Some(url) = &node.url {
                let title = if node.title.trim().is_empty() { url } else { &node.title };
                out.push((title.clone(), url.clone(), depth));
            }
            walk(&node.children, depth + 1, out);
        }
    }
    let Ok((roots, _)) = read_bookmark_file(file).and_then(|roots| import_roots(&roots)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for root in &roots {
        walk(&root.children, 0, &mut out);
    }
    out
}

pub(crate) fn profile_bookmarks(
    source: &SourceProfile,
) -> Result<(Vec<BookmarkNode>, usize), &'static str> {
    import_roots(&read_bookmark_roots(source)?)
}

fn import_roots(roots: &[BookmarkRoot]) -> Result<(Vec<BookmarkNode>, usize), &'static str> {
    let mut remaining = MAX_IMPORT_NODES;
    let mut root_indexes = BTreeMap::new();
    let mut bookmarks: Vec<BookmarkNode> = Vec::new();
    for (key, root) in roots {
        if let Some(mut node) = import_node(root, true, 1, &mut remaining)? {
            // Root names can differ between files, so merge by the root key.
            let index = *root_indexes.entry(key).or_insert(bookmarks.len());
            if index == bookmarks.len() {
                bookmarks.push(node);
            } else {
                bookmarks[index].children.append(&mut node.children);
            }
        }
    }
    let (bookmarks, leaves) = merge_siblings(bookmarks);
    let bytes = serde_json::to_vec(&bookmarks).map_err(|_| "Could not encode the bookmarks.")?;
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err("The imported bookmarks are too large.");
    }
    Ok((bookmarks, leaves))
}

fn merge_siblings(nodes: Vec<BookmarkNode>) -> (Vec<BookmarkNode>, usize) {
    let mut indexes = BTreeMap::new();
    let mut merged: Vec<BookmarkNode> = Vec::new();
    let mut leaves = 0;
    for mut node in nodes {
        // Match the destination's title and canonical URL comparison.
        let address = node
            .url
            .as_ref()
            .map(|raw| Url::parse(raw).map_or_else(|_| raw.clone(), |url| url.to_string()));
        let index = *indexes
            .entry((node.title.clone(), address))
            .or_insert(merged.len());
        if index == merged.len() {
            leaves += usize::from(node.url.is_some());
            merged.push(node);
        } else {
            merged[index].children.append(&mut node.children);
        }
    }
    for node in merged.iter_mut().filter(|node| !node.children.is_empty()) {
        let (children, count) = merge_siblings(std::mem::take(&mut node.children));
        node.children = children;
        leaves += count;
    }
    (merged, leaves)
}

fn import_node(
    node: &Value,
    root: bool,
    depth: usize,
    remaining: &mut usize,
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
            if let Some(child) = import_node(child, false, depth + 1, remaining)? {
                children.push(child);
            }
        }
    }
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

pub(crate) fn pasted_tab_urls(text: &str, limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let text = if text.len() > MAX_PASTED_TAB_BYTES {
        // Keep only complete lines so the byte cutoff cannot change a tab's destination.
        let Some(end) = text.as_bytes()[..MAX_PASTED_TAB_BYTES]
            .iter()
            .rposition(|&byte| byte == b'\n')
        else {
            return Vec::new();
        };
        &text[..=end]
    } else {
        text
    };

    let mut seen = BTreeSet::new();
    text.lines()
        .filter_map(|line| {
            let line = line.trim().trim_start_matches('\u{feff}').trim_start();
            parsed_web_url(line).map(|url| url.to_string())
        })
        .filter(|url| seen.insert(url.clone()))
        .take(limit)
        .collect()
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
    fn pasted_tab_urls_preserves_full_addresses_in_order() {
        let input = concat!(
            "\u{feff}  HTTPS://Example.TEST:443/a%2Fz?q=a@b#first \r\n",
            "\r\n",
            "https://example.test/a%2Fz?q=a@b#first\n",
            "https://user:pass@example.test/\n",
            "https://@example.test/\n",
            "https:///example.test/\n",
            "https://example.test:65536/\n",
            "file:///C:/private.html\n",
            "javascript:alert(1)\n",
            "https://example.test/a https://example.test/b\n",
            "https://example.test/a%2Fz?q=a@b#second\n",
            "https://example.test/b?q=a@b#first\r\n",
            "  http://[::1]:8080/c  \n",
            "https://later.test/",
        );
        assert_eq!(
            pasted_tab_urls(input, 4),
            [
                "https://example.test/a%2Fz?q=a@b#first",
                "https://example.test/a%2Fz?q=a@b#second",
                "https://example.test/b?q=a@b#first",
                "http://[::1]:8080/c",
            ]
        );
        assert!(pasted_tab_urls(input, 0).is_empty());
    }

    #[test]
    fn pasted_tab_urls_skips_lines_cut_by_the_byte_limit() {
        let input = format!(
            "https://kept.test/a\nhttps://cut.test/{}\nhttps://later.test/",
            "\u{1f98a}".repeat(MAX_PASTED_TAB_BYTES / 4)
        );
        assert_eq!(pasted_tab_urls(&input, usize::MAX), ["https://kept.test/a"]);
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
    fn import_merges_account_roots_and_counts_unique_links_per_folder() {
        let mut roots = parse_bookmark_roots(BOOKMARKS).unwrap();
        let account = br#"{
            "roots": {"synced": {"name":"Account mobile", "children":[
                {"type":"folder", "name":"Work", "children":[
                    {"type":"url", "name":"Inbox", "url":"https://z.test/inbox?q=a@b#unread"},
                    {"type":"url", "name":"Inbox", "url":"https://account.test/"}
                ]},
                {"type":"url", "name":"Inbox", "url":"https://z.test/inbox?q=a@b#unread"}
            ]}}
        }"#;
        roots.extend(parse_bookmark_roots(account).unwrap());

        let (bookmarks, leaves) = import_roots(&roots).unwrap();
        assert_eq!(bookmarks.len(), 2);
        let mobile = &bookmarks[0];
        assert_eq!(mobile.title, "Mobile bookmarks");
        let work = &mobile.children[0];
        assert_eq!(work.title, "Work");
        assert_eq!(
            work.children
                .iter()
                .map(|node| node.url.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["https://Z.test/inbox?q=a@b#unread", "https://account.test/"]
        );
        assert_eq!(leaves, 5);
    }

    #[test]
    fn import_returns_an_error_instead_of_a_partial_tree() {
        let mut deep = json!({"type":"folder", "children":[]});
        for _ in 0..MAX_IMPORT_DEPTH {
            deep = json!({"type":"folder", "children":[deep]});
        }
        assert!(import_roots(&[("other".to_owned(), deep)]).is_err());

        let empty = json!({"type":"folder", "children":[]});
        let wide = json!({"type":"folder", "children":vec![empty; MAX_IMPORT_NODES]});
        assert!(import_roots(&[("other".to_owned(), wide)]).is_err());

        let large = json!({"type":"folder", "name":"x".repeat(MAX_IMPORT_BYTES), "children":[]});
        assert!(import_roots(&[("other".to_owned(), large)]).is_err());
    }
}
