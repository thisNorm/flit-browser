use crate::navigation;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

const FILE_NAME: &str = "session.json";
const BACKUP_FILE_NAME: &str = "session.previous.json";
const MAX_TABS: usize = 128;
pub(crate) const MAX_WINDOWS: usize = 16;
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
pub(crate) struct WindowState {
    pub entries: Vec<Entry>,
    pub selected: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Session {
    #[serde(flatten)]
    pub primary: WindowState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_windows: Vec<WindowState>,
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
    match read_session(&profile.join(FILE_NAME)) {
        Ok(Some(session)) => Some(session),
        Ok(None) => read_backup(profile),
        Err(_) => None,
    }
}

pub(crate) fn read_backup(profile: &Path) -> Option<Session> {
    read_session(&profile.join(BACKUP_FILE_NAME))
        .ok()
        .flatten()
        .filter(has_recoverable_tabs)
}

pub(crate) fn has_backup(profile: &Path) -> bool {
    fs::metadata(profile.join(BACKUP_FILE_NAME)).is_ok_and(|metadata| metadata.is_file())
}

/// Call once at normal startup, before saving any new session state.
pub(crate) fn backup_before_startup(profile: &Path) -> io::Result<()> {
    if let Some(mut session) = read_session(&profile.join(FILE_NAME))? {
        let bytes = session_bytes(&mut session)?;
        if has_recoverable_tabs(&session) {
            write_atomic(profile, BACKUP_FILE_NAME, &bytes)?;
        }
    }
    Ok(())
}

pub(crate) fn remove_backup(profile: &Path) -> io::Result<()> {
    match fs::remove_file(profile.join(BACKUP_FILE_NAME)) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn has_recoverable_tabs(session: &Session) -> bool {
    session.restore
        && std::iter::once(&session.primary)
            .chain(&session.additional_windows)
            .any(|window| {
                window
                    .entries
                    .iter()
                    .any(|entry| entry.url != navigation::HOME)
            })
}

fn read_session(path: &Path) -> io::Result<Option<Session>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Ok(None);
    }
    let session = serde_json::from_slice(&bytes).ok();
    Ok(session.as_ref().map(validated))
}

pub(crate) fn save(profile: &Path, session: &Session) -> io::Result<()> {
    let bytes = session_bytes(&mut validated(session))?;
    write_atomic(profile, FILE_NAME, &bytes)
}

fn session_bytes(session: &mut Session) -> io::Result<Vec<u8>> {
    let additional = std::mem::take(&mut session.additional_windows);
    let mut bytes = serde_json::to_vec(session)?;
    // Budget for the optional key, brackets, and commas before writing extra windows.
    let mut remaining =
        MAX_FILE_BYTES.saturating_sub(bytes.len() + br#","additional_windows":[]"#.len());
    for window in additional {
        let size = serde_json::to_vec(&window)?.len()
            + usize::from(!session.additional_windows.is_empty());
        let Some(next) = remaining.checked_sub(size) else {
            break;
        };
        remaining = next;
        session.additional_windows.push(window);
    }
    if !session.additional_windows.is_empty() {
        bytes = serde_json::to_vec(session)?;
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Session is too large.",
        ));
    }
    Ok(bytes)
}

fn write_atomic(profile: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let (temp, mut file) = loop {
        let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temp = profile.join(format!(".session-{}-{number}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => break (temp, file),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    };
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| fs::rename(&temp, profile.join(name)));
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

fn validated(session: &Session) -> Session {
    let (primary, additional_windows) = if session.restore {
        (
            validated_window(&session.primary),
            session
                .additional_windows
                .iter()
                .take(MAX_WINDOWS - 1)
                .map(validated_window)
                .collect(),
        )
    } else {
        (
            WindowState {
                entries: Vec::new(),
                selected: 0,
            },
            Vec::new(),
        )
    };
    Session {
        primary,
        additional_windows,
        ..*session
    }
}

pub(crate) fn validated_window(window: &WindowState) -> WindowState {
    let mut entries = Vec::with_capacity(window.entries.len().min(MAX_TABS));
    let mut selected = 0;
    for (index, entry) in window.entries.iter().enumerate() {
        let active = index == window.selected;
        if entries.len() == MAX_TABS {
            if index > window.selected {
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
    WindowState { entries, selected }
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
            let _ = fs::remove_file(self.0.join(BACKUP_FILE_NAME));
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

    fn session(url: &str) -> Session {
        Session {
            primary: WindowState {
                entries: vec![entry(url, "", false)],
                selected: 0,
            },
            additional_windows: Vec::new(),
            restore: true,
            sidebar_open: true,
            defer_background: false,
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
                primary: WindowState {
                    entries: vec![
                        entry("https://example.com/a?q=x@y#top", "예제", false),
                        entry("chrome://settings/", "Settings", true),
                        entry("about:blank", "", false),
                        entry("http://localhost:8080/a", "Local", false),
                    ],
                    selected: 1,
                },
                additional_windows: Vec::new(),
                restore: true,
                sidebar_open: true,
                defer_background: false,
            })
        );
    }

    #[test]
    fn replaces_session_and_keeps_active_tab_when_capped() {
        let profile = Profile::new();
        let mut state = session(navigation::HOME);
        save(&profile.0, &state).unwrap();
        assert_eq!(load(&profile.0).as_ref(), Some(&state));
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
        state.primary = WindowState { entries, selected };
        state.additional_windows.push(state.primary.clone());
        save(&profile.0, &state).unwrap();

        let restored = load(&profile.0).unwrap();
        assert_eq!(restored.primary.entries.len(), 128);
        assert_eq!(restored.primary.entries[0].url, "https://example.com/0");
        assert_eq!(restored.primary.selected, 127);
        assert_eq!(
            restored.primary.entries[restored.primary.selected],
            entry("https://example.com/active", &title_prefix, true)
        );
        assert_eq!(restored.additional_windows, [restored.primary.clone()]);
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
                primary: WindowState {
                    entries: Vec::new(),
                    selected: 0,
                },
                additional_windows: Vec::new(),
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
    fn writes_the_legacy_shape_and_keeps_the_first_sixteen_windows() {
        let profile = Profile::new();
        let path = profile.0.join(FILE_NAME);
        let mut state = session("https://example.com/primary");
        save(&profile.0, &state).unwrap();
        let written: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(written.get("additional_windows").is_none());

        state.additional_windows = (0..17)
            .map(|i| session(&format!("https://example.com/window-{i}")).primary)
            .collect();
        save(&profile.0, &state).unwrap();
        let bytes = fs::read(&path).unwrap();
        let legacy: WindowState = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(legacy, state.primary);
        let written: Session = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(written.additional_windows, state.additional_windows[..15]);

        fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(load(&profile.0).unwrap(), written);
    }

    #[test]
    fn drops_extra_windows_that_exceed_the_file_budget() {
        let profile = Profile::new();
        let url_prefix = "https://example.com/";
        let long_url = format!(
            "{url_prefix}{}",
            "x".repeat(MAX_URL_BYTES - url_prefix.len())
        );
        // Each NUL takes six bytes after JSON escaping.
        let long_title = "\0".repeat(MAX_TITLE_BYTES);
        let heavy = WindowState {
            entries: vec![entry(&long_url, &long_title, false); 128],
            selected: 127,
        };
        let mut state = session("https://example.com/primary");
        state.primary = heavy.clone();
        state.additional_windows = vec![heavy.clone(); 3];
        save(&profile.0, &state).unwrap();
        let restored = load(&profile.0).unwrap();
        assert_eq!(restored.primary, heavy);
        assert_eq!(restored.additional_windows, [heavy]);
        assert!(fs::metadata(profile.0.join(FILE_NAME)).unwrap().len() <= 4 * 1024 * 1024);
    }

    #[test]
    fn falls_back_to_previous_only_when_current_is_absent_or_malformed() {
        let profile = Profile::new();
        fs::write(
            profile.0.join(BACKUP_FILE_NAME),
            br#"{"entries":[{"url":"file:///private","title":""},{"url":"https://example.com/previous","title":""}],"selected":1}"#,
        )
        .unwrap();
        for current in [None, Some("{"), Some(r#"{"entries":false,"selected":0}"#)] {
            let path = profile.0.join(FILE_NAME);
            if let Some(current) = current {
                fs::write(path, current).unwrap();
            }
            let restored = load(&profile.0).expect("recover the previous session");
            assert_eq!(restored, session("https://example.com/previous"));
        }

        let mut current = session("https://example.com/current");
        save(&profile.0, &current).unwrap();
        assert_eq!(load(&profile.0).as_ref(), Some(&current));
        current.primary.entries.clear();
        save(&profile.0, &current).unwrap();
        assert_eq!(load(&profile.0).as_ref(), Some(&current));
    }

    #[test]
    fn rotates_only_the_recoverable_current_session_on_startup() {
        let profile = Profile::new();
        let path = profile.0.join(FILE_NAME);
        let backup = profile.0.join(BACKUP_FILE_NAME);
        backup_before_startup(&profile.0).unwrap();
        assert!(!has_backup(&profile.0));

        let mut first = session("https://example.com/first");
        first
            .additional_windows
            .push(session("https://example.com/extra").primary);
        let second = session("https://example.com/second");
        save(&profile.0, &first).unwrap();
        backup_before_startup(&profile.0).unwrap();
        assert!(has_backup(&profile.0));
        assert_eq!(read_backup(&profile.0), Some(first.clone()));
        save(&profile.0, &second).unwrap();
        assert_eq!(read_backup(&profile.0), Some(first));
        backup_before_startup(&profile.0).unwrap();
        assert_eq!(read_backup(&profile.0), Some(second));

        // The whitespace detects an accidental rewrite of the fallback itself.
        let prior =
            br#" { "entries": [{"url":"https://example.com/backup","title":""}], "selected": 0 } "#;
        fs::write(&backup, prior).unwrap();
        fs::remove_file(&path).unwrap();
        backup_before_startup(&profile.0).unwrap();
        assert_eq!(fs::read(&backup).unwrap(), prior);
        for current in [
            "{",
            r#"{"entries":[],"selected":0}"#,
            r#"{"entries":[{"url":"about:blank","title":""}],"selected":0}"#,
            r#"{"entries":[{"url":"file:///private","title":""}],"selected":0}"#,
        ] {
            fs::write(&path, current).unwrap();
            backup_before_startup(&profile.0).unwrap();
            assert_eq!(fs::read(&backup).unwrap(), prior, "{current}");
        }
    }

    #[test]
    fn disabled_restore_wins_and_backup_removal_only_deletes_the_copy() {
        let profile = Profile::new();
        let current = profile.0.join(FILE_NAME);
        let backup = profile.0.join(BACKUP_FILE_NAME);
        let first = session("https://example.com/first");
        save(&profile.0, &first).unwrap();
        backup_before_startup(&profile.0).unwrap();

        let mut off = session("https://example.com/do-not-save");
        off.additional_windows.push(off.primary.clone());
        off.restore = false;
        off.sidebar_open = false;
        off.defer_background = true;
        save(&profile.0, &off).unwrap();
        let written = fs::read(&current).unwrap();
        let restored: Session = serde_json::from_slice(&written).unwrap();
        assert!(!restored.restore);
        assert!(!restored.sidebar_open);
        assert!(restored.defer_background);
        assert!(restored.primary.entries.is_empty());
        assert_eq!(restored.primary.selected, 0);
        assert!(restored.additional_windows.is_empty());
        assert_eq!(load(&profile.0), Some(restored.clone()));
        backup_before_startup(&profile.0).unwrap();
        assert_eq!(read_backup(&profile.0), Some(first));

        fs::write(&backup, b"{").unwrap();
        assert!(has_backup(&profile.0));
        assert!(read_backup(&profile.0).is_none());
        remove_backup(&profile.0).unwrap();
        remove_backup(&profile.0).unwrap();
        assert!(!has_backup(&profile.0));
        assert_eq!(fs::read(&current).unwrap(), written);
        assert_eq!(load(&profile.0), Some(restored));
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
