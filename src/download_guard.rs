use cef::*;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::Rc as Shared,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const RECENT_LIMIT: usize = 12;

// CEF download callbacks and Flit's close handlers run on the same UI thread.
thread_local! {
    static DOWNLOADS: RefCell<Downloads> = const { RefCell::new(Downloads {
        active: BTreeMap::new(),
        recent: BTreeMap::new(),
    }) };
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct Origin {
    window: usize,
    key: usize,
    browser: i32,
    id: u32,
}

struct Downloads {
    active: BTreeMap<Origin, Option<Shared<DownloadItemCallback>>>,
    // Normal windows share the app profile. Each private window has its own list.
    recent: BTreeMap<Option<usize>, Vec<RecentRow>>,
}

struct RecentRow {
    origin: Origin,
    started_at: i64,
    entry: Recent,
}

#[derive(Clone, PartialEq)]
pub(crate) struct Recent {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub(crate) received_bytes: i64,
    pub percent: i32,
    pub complete: bool,
    pub(crate) completed_at: Option<SystemTime>,
    pub in_progress: bool,
    pub paused: bool,
    pub controllable: bool,
}

pub(crate) enum Control {
    Pause,
    Resume,
    Cancel,
}

pub(crate) fn recent(window: usize, private: bool) -> Vec<Recent> {
    DOWNLOADS.with_borrow(|downloads| {
        downloads
            .recent
            .get(&private.then_some(window))
            .into_iter()
            .flatten()
            .map(|row| row.entry.clone())
            .collect()
    })
}

fn completion_time(started_at: i64, ended_at: i64, observed_at: SystemTime) -> SystemTime {
    // CEF stores wall time in microseconds since 1601-01-01 UTC.
    const CEF_UNIX_EPOCH_MICROS: i64 = 11_644_473_600_000_000;
    if ended_at < CEF_UNIX_EPOCH_MICROS || ended_at < started_at {
        return observed_at;
    }
    UNIX_EPOCH
        .checked_add(Duration::from_micros((ended_at - CEF_UNIX_EPOCH_MICROS) as u64))
        .filter(|time| *time <= observed_at)
        .unwrap_or(observed_at)
}

fn remember(items: &mut Vec<RecentRow>, mut row: RecentRow) -> bool {
    let current = items.iter_mut().find(|current| current.entry.id == row.entry.id);
    if let Some(current) = current.as_deref().filter(|current| current.origin == row.origin) {
        // While the file is open, CEF can report an empty or temporary name.
        if !current.entry.name.is_empty()
            && (row.entry.name.is_empty()
                || (!row.entry.complete && row.entry.name.ends_with(".crdownload")))
        {
            row.entry.name = current.entry.name.clone();
        }
        if row.entry.received_bytes < 0 {
            row.entry.received_bytes = current.entry.received_bytes;
        }
        row.entry.completed_at = current.entry.completed_at.or(row.entry.completed_at);
    }
    row.entry.received_bytes = row.entry.received_bytes.max(0);
    if let Some(current) = current {
        if current.origin == row.origin && current.entry == row.entry {
            return false;
        }
        current.origin = row.origin;
        current.entry = row.entry;
        return true;
    }

    let index = items.partition_point(|current| {
        (current.started_at, current.entry.id) > (row.started_at, row.entry.id)
    });
    if index >= RECENT_LIMIT {
        return false;
    }
    items.insert(index, row);
    items.truncate(RECENT_LIMIT);
    true
}

pub(crate) fn control(window: usize, private: bool, id: u32, action: Control) -> bool {
    // Keep the callback alive if CEF reenters the tracker during the action.
    let callback = DOWNLOADS.with_borrow(|downloads| {
        let row = downloads
            .recent
            .get(&private.then_some(window))?
            .iter()
            .find(|row| row.entry.id == id)?;
        match action {
            Control::Pause if row.entry.paused => return None,
            Control::Resume if !row.entry.paused => return None,
            _ => {}
        }
        downloads.active.get(&row.origin)?.clone()
    });
    let Some(callback) = callback else {
        return false;
    };
    match action {
        Control::Pause => callback.pause(),
        Control::Resume => callback.resume(),
        Control::Cancel => callback.cancel(),
    }
    true
}

pub(crate) fn handler(window: usize, key: usize, private: bool) -> DownloadHandler {
    FlitDownload::new(window, key, private)
}

pub(crate) fn count(window: usize, tab: Option<usize>) -> usize {
    DOWNLOADS.with_borrow(|downloads| {
        downloads
            .active
            .keys()
            .filter(|origin| origin.window == window && tab.is_none_or(|tab| tab == origin.key))
            .count()
    })
}

// Once the source browser is destroyed, CEF stops delivering updates for its items.
pub(crate) fn clear_browser(window: usize, browser_id: i32) {
    let (changed, recent_changed) = clear(window, Some(browser_id));
    if changed {
        crate::window::downloads_changed(window);
    }
    if recent_changed {
        crate::window::download_progress_changed(window);
    }
}

pub(crate) fn clear_window(window: usize) {
    clear(window, None);
}

fn clear(window: usize, browser: Option<i32>) -> (bool, bool) {
    let (removed, recent_changed) = DOWNLOADS.with_borrow_mut(|downloads| {
        let expires = |origin: &Origin| {
            origin.window == window && browser.is_none_or(|id| id == origin.browser)
        };
        // Release the CEF callbacks after the tracker borrow ends.
        let removed: Vec<_> = downloads
            .active
            .extract_if(.., |origin, _| expires(origin))
            .collect();
        if browser.is_none() {
            downloads.recent.remove(&Some(window));
        }
        let mut recent_changed = false;
        for row in downloads.recent.values_mut().flatten() {
            let entry = &mut row.entry;
            if expires(&row.origin)
                && !entry.complete
                && (entry.in_progress || entry.paused || entry.controllable)
            {
                entry.in_progress = false;
                entry.paused = false;
                entry.controllable = false;
                recent_changed = true;
            }
        }
        (removed, recent_changed)
    });
    (!removed.is_empty(), recent_changed)
}

wrap_download_handler! {
    // CEF routes updates through the originating browser's client, one per Flit tab.
    struct FlitDownload { window: usize, key: usize, private: bool }
    impl DownloadHandler {
        fn can_download(
            &self, _browser: Option<&mut Browser>, _url: Option<&CefString>,
            _request_method: Option<&CefString>,
        ) -> i32 {
            // Match CEF's behavior without a handler; the Rust trait defaults to false.
            1
        }

        fn on_before_download(
            &self, browser: Option<&mut Browser>, item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>, _callback: Option<&mut BeforeDownloadCallback>,
        ) -> i32 {
            let suggested_name = suggested_name.map(CefString::to_string).filter(|name| !name.is_empty());
            if let (Some(browser), Some(item), Some(name)) = (browser, item, suggested_name)
                && item.is_valid() != 0
            {
                let origin = Origin {
                    window: self.window, key: self.key, browser: browser.identifier(), id: item.id(),
                };
                let start = item.start_time().val;
                let received_bytes = item.received_bytes();
                let percent = item.percent_complete();
                let in_progress = item.is_in_progress() != 0;
                let changed = DOWNLOADS.with_borrow_mut(|downloads| {
                    let rows = downloads.recent.entry(self.private.then_some(self.window)).or_default();
                    if let Some(row) = rows.iter_mut().find(|row| row.origin == origin) {
                        if row.entry.complete || row.entry.name == name {
                            return false;
                        }
                        row.entry.name = name;
                        return true;
                    }
                    remember(rows, RecentRow {
                        origin,
                        started_at: start,
                        entry: Recent {
                            id: origin.id, name, path: String::new(), received_bytes, percent,
                            complete: false, completed_at: None, in_progress,
                            paused: false, controllable: false,
                        },
                    })
                });
                if changed {
                    crate::window::download_progress_changed(self.window);
                }
            }
            // Leave target selection and security checks to the Chrome runtime.
            0
        }

        fn on_download_updated(
            &self, browser: Option<&mut Browser>, item: Option<&mut DownloadItem>,
            callback: Option<&mut DownloadItemCallback>,
        ) {
            let (Some(browser), Some(item)) = (browser, item) else { return };
            if browser.is_valid() == 0 || item.is_valid() == 0 {
                return;
            }
            let origin = Origin {
                window: self.window,
                key: self.key,
                browser: browser.identifier(),
                id: item.id(),
            };
            let in_progress = item.is_in_progress() != 0;
            let paused = item.is_paused() != 0;
            let complete = item.is_complete() != 0;
            let started_at = item.start_time().val;
            let completed_at = complete.then(|| {
                completion_time(started_at, item.end_time().val, SystemTime::now())
            });
            let live = (in_progress || paused)
                && !complete
                && item.is_canceled() == 0
                && item.is_interrupted() == 0;
            let callback = callback.filter(|_| live).map(|cb| Shared::new(cb.clone()));

            let path = CefString::from(&item.full_path()).to_string();
            let mut name = CefString::from(&item.suggested_file_name()).to_string();
            if (complete || name.is_empty())
                && let Some(file) = std::path::Path::new(&path).file_name()
            {
                name = file.to_string_lossy().into_owned();
            }
            let entry = Recent {
                id: origin.id,
                name,
                path,
                received_bytes: item.received_bytes(),
                percent: item.percent_complete(),
                complete,
                completed_at,
                in_progress,
                paused: live && paused,
                controllable: callback.is_some(),
            };
            let row = RecentRow {
                origin,
                started_at,
                entry,
            };
            let (changed, recent_changed, previous) = DOWNLOADS.with_borrow_mut(|downloads| {
                let items = downloads
                    .recent
                    .entry(self.private.then_some(self.window))
                    .or_default();
                let recent_changed = remember(items, row);
                let previous = if live {
                    downloads.active.insert(origin, callback)
                } else {
                    downloads.active.remove(&origin)
                };
                (live != previous.is_some(), recent_changed, previous)
            });
            drop(previous);
            if changed {
                crate::window::downloads_changed(self.window);
                if !live && self.key != 0 {
                    crate::window::download_ended(self.window, self.key, complete);
                }
            }
            if recent_changed {
                crate::window::download_progress_changed(self.window);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_only_plausible_cef_completion_times() {
        let cef_epoch = 11_644_473_600_000_000_i64;
        let observed_at = UNIX_EPOCH + Duration::from_secs(5);
        assert_eq!(
            completion_time(cef_epoch, cef_epoch + 2_000_000, observed_at),
            UNIX_EPOCH + Duration::from_secs(2)
        );
        for ended_at in [i64::MIN, 0, cef_epoch, cef_epoch + 6_000_000, i64::MAX] {
            assert_eq!(
                completion_time(cef_epoch + 1, ended_at, observed_at),
                observed_at
            );
        }
    }

    #[test]
    fn keeps_first_completion_and_known_bytes_for_the_same_origin() {
        let origin = Origin {
            window: 1,
            key: 2,
            browser: 3,
            id: 4,
        };
        let row = |origin: Origin, received_bytes, completed_at: Option<SystemTime>| RecentRow {
            origin,
            started_at: 1,
            entry: Recent {
                id: origin.id,
                name: "sample.txt".into(),
                path: String::new(),
                received_bytes,
                percent: -1,
                complete: completed_at.is_some(),
                completed_at,
                in_progress: completed_at.is_none(),
                paused: false,
                controllable: false,
            },
        };
        let mut rows = Vec::new();
        assert!(remember(&mut rows, row(origin, -1, None)));
        assert_eq!(rows[0].entry.received_bytes, 0);
        assert!(remember(&mut rows, row(origin, 512, None)));
        assert_eq!(rows[0].entry.received_bytes, 512);
        assert_eq!(rows[0].entry.completed_at, None);

        let completed_at = UNIX_EPOCH + Duration::from_secs(2);
        let later = completed_at + Duration::from_secs(1);
        assert!(remember(&mut rows, row(origin, 1024, Some(completed_at))));
        assert!(!remember(&mut rows, row(origin, -1, Some(later))));
        assert_eq!(rows[0].entry.received_bytes, 1024);
        assert_eq!(rows[0].entry.completed_at, Some(completed_at));

        let different_origin = Origin { browser: 8, ..origin };
        assert!(remember(&mut rows, row(different_origin, -1, None)));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entry.received_bytes, 0);
        assert_eq!(rows[0].entry.completed_at, None);
    }
}
