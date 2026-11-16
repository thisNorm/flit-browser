use crate::navigation;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

const FILE_NAME: &str = "session.json";
const MAX_TABS: usize = 128;
const MAX_URL_BYTES: usize = 8 * 1024;
const MAX_TITLE_BYTES: usize = 1024;
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Entry {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Session {
    pub entries: Vec<Entry>,
    pub selected: usize,
    #[serde(default = "default_true")]
    pub restore: bool,
    #[serde(default = "default_true")]
    pub sidebar_open: bool,
    #[serde(default)]
    pub defer_background: bool,
}

fn default_true() -> bool {
    true
}

pub(crate) fn load(profile: &Path) -> Option<Session> {
    let mut bytes = Vec::new();
    File::open(profile.join(FILE_NAME))
        .ok()?
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_FILE_BYTES {
        return None;
    }
    let session = serde_json::from_slice(&bytes).ok()?;
    Some(validated(&session))
}

pub(crate) fn save(profile: &Path, session: &Session) -> io::Result<()> {
    let bytes = serde_json::to_vec(&validated(session))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Session is too large.",
        ));
    }

    let (temp, mut file) = loop {
        let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temp = profile.join(format!(".session-{}-{number}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => break (temp, file),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    };
    let written = file.write_all(&bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| fs::rename(&temp, profile.join(FILE_NAME)));
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub(crate) fn validated(session: &Session) -> Session {
    let mut entries = Vec::with_capacity(session.entries.len().min(MAX_TABS));
    let mut selected = 0;
    for (index, entry) in session.entries.iter().enumerate() {
        let active = index == session.selected;
        if entries.len() == MAX_TABS {
            if index > session.selected {
                break;
            }
            if !active {
                continue;
            }
        }
        let Some(url) = stored_url(&entry.url) else {
            continue;
        };
        if active {
            // Keep the active tab even if it is past the cap.
            if entries.len() == MAX_TABS {
                entries.pop();
            }
            selected = entries.len();
        }
        entries.push(Entry {
            url,
            title: entry.title[..entry.title.floor_char_boundary(MAX_TITLE_BYTES)].to_owned(),
            pinned: entry.pinned,
        });
    }
    Session {
        entries,
        selected,
        ..*session
    }
}

pub(crate) fn stored_url(input: &str) -> Option<String> {
    if input == navigation::HOME {
        return Some(input.to_owned());
    }
    if input.len() > MAX_URL_BYTES || input.trim() != input || input.chars().any(char::is_control) {
        return None;
    }
    // Stored URLs cannot be search terms, bare hostnames, or contain even empty userinfo.
    let (scheme, rest) = input.split_once("://")?;
    if !["http", "https", "chrome"]
        .iter()
        .any(|s| scheme.eq_ignore_ascii_case(s))
    {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains(['@', '\\']) {
        return None;
    }
    let url = navigation::address(input).ok()?;
    (url.len() <= MAX_URL_BYTES).then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Profile(PathBuf);

    impl Profile {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "flit-session-test-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Profile {
        fn drop(&mut self) {
            let _ = fs::remove_file(self.0.join(FILE_NAME));
            let _ = fs::remove_dir(&self.0);
        }
    }

    fn entry(url: &str, title: &str, pinned: bool) -> Entry {
        Entry {
            url: url.into(),
            title: title.into(),
            pinned,
        }
    }

    #[test]
    fn loads_valid_tabs_in_order_and_remaps_selection() {
        let profile = Profile::new();
        fs::write(
            profile.0.join(FILE_NAME),
            r#"{
            "entries": [
                {"url":"https://example.com/a?q=x@y#top","title":"예제"},
                {"url":"https://user:pass@example.com/","title":""},
                {"url":"file:///C:/test","title":""},
                {"url":"example.com","title":""},
                {"url":"chrome://crash/","title":""},
                {"url":"chrome://settings/","title":"Settings","pinned":true},
                {"url":"about:blank","title":""},
                {"url":"https://@example.com/","title":""},
                {"url":"http://localhost:8080/a","title":"Local"}
            ],
            "selected": 5
        }"#,
        )
        .unwrap();
        assert_eq!(
            load(&profile.0),
            Some(Session {
                entries: vec![
                    entry("https://example.com/a?q=x@y#top", "예제", false),
                    entry("chrome://settings/", "Settings", true),
                    entry("about:blank", "", false),
                    entry("http://localhost:8080/a", "Local", false),
                ],
                selected: 1,
                restore: true,
                sidebar_open: true,
                defer_background: false,
            })
        );
    }

    #[test]
    fn replaces_session_and_keeps_active_tab_when_capped() {
        let profile = Profile::new();
        let initial = Session {
            entries: vec![entry("about:blank", "", false)],
            selected: 0,
            restore: true,
            sidebar_open: true,
            defer_background: false,
        };
        save(&profile.0, &initial).unwrap();
        assert_eq!(load(&profile.0), Some(initial));
        let mut entries = vec![entry("https://secret@example.com/", "", false)];
        entries
            .extend((0..MAX_TABS).map(|i| entry(&format!("https://example.com/{i}"), "", false)));
        let title_prefix = "a".repeat(MAX_TITLE_BYTES - 1);
        entries.push(entry(
            "https://example.com/active",
            &format!("{title_prefix}가"),
            true,
        ));
        let selected = entries.len() - 1;
        save(
            &profile.0,
            &Session {
                entries,
                selected,
                restore: true,
                sidebar_open: true,
                defer_background: false,
            },
        )
        .unwrap();

        let restored = load(&profile.0).unwrap();
        assert_eq!(restored.entries.len(), MAX_TABS);
        assert_eq!(restored.entries[0].url, "https://example.com/0");
        assert_eq!(restored.selected, MAX_TABS - 1);
        assert_eq!(
            restored.entries[restored.selected],
            entry("https://example.com/active", &title_prefix, true)
        );
        let on_disk: Session =
            serde_json::from_slice(&fs::read(profile.0.join(FILE_NAME)).unwrap()).unwrap();
        assert_eq!(on_disk, restored);
    }

    #[test]
    fn keeps_browser_preferences_even_without_tabs() {
        let profile = Profile::new();
        let other = Profile::new();
        for restore in [false, true] {
            let empty = Session {
                entries: vec![],
                selected: 0,
                restore,
                sidebar_open: !restore,
                defer_background: restore,
            };
            save(&profile.0, &empty).unwrap();
            assert_eq!(load(&profile.0), Some(empty));
            assert!(load(&other.0).is_none());
        }
    }

    #[test]
    fn ignores_missing_corrupt_and_oversized_sessions() {
        let profile = Profile::new();
        let path = profile.0.join(FILE_NAME);
        assert!(load(&profile.0).is_none());
        fs::write(&path, b"{\"entries\":[").unwrap();
        assert!(load(&profile.0).is_none());
        let mut valid = br#"{"entries":[{"url":"about:blank","title":""}],"selected":0}"#.to_vec();
        valid.resize(MAX_FILE_BYTES + 1, b' ');
        fs::write(path, valid).unwrap();
        assert!(load(&profile.0).is_none());
    }
}
