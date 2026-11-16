use cef::*;
use std::{cell::RefCell, collections::BTreeSet};

// CEF download callbacks and Flit's close handlers run on the same UI thread.
thread_local! {
    static ACTIVE: RefCell<BTreeSet<(usize, usize, i32, u32)>> = const { RefCell::new(BTreeSet::new()) };
}

pub(crate) fn handler(window: usize, key: usize) -> DownloadHandler {
    FlitDownload::new(window, key)
}

pub(crate) fn count(window: usize, tab: Option<usize>) -> usize {
    ACTIVE.with(|items| {
        items
            .borrow()
            .iter()
            .filter(|&&(owner, key, _, _)| owner == window && tab.is_none_or(|tab| tab == key))
            .count()
    })
}

// Once the source browser is destroyed, CEF stops delivering updates for its items.
pub(crate) fn clear_browser(window: usize, browser_id: i32) {
    let changed = ACTIVE.with(|items| {
        let mut items = items.borrow_mut();
        let before = items.len();
        items.retain(|&(owner, _, id, _)| (owner, id) != (window, browser_id));
        items.len() != before
    });
    if changed {
        crate::window::downloads_changed(window);
    }
}

pub(crate) fn clear_window(window: usize) {
    ACTIVE.with(|items| {
        items
            .borrow_mut()
            .retain(|&(owner, _, _, _)| owner != window)
    });
}

wrap_download_handler! {
    // CEF routes updates through the originating browser's client, one per Flit tab.
    struct FlitDownload { window: usize, key: usize }
    impl DownloadHandler {
        fn can_download(
            &self, _browser: Option<&mut Browser>, _url: Option<&CefString>,
            _request_method: Option<&CefString>,
        ) -> i32 {
            // Match CEF's behavior without a handler; the Rust trait defaults to false.
            1
        }

        fn on_before_download(
            &self, _browser: Option<&mut Browser>, _item: Option<&mut DownloadItem>,
            _suggested_name: Option<&CefString>, _callback: Option<&mut BeforeDownloadCallback>,
        ) -> i32 {
            // Leave target selection and security checks to the Chrome runtime.
            0
        }

        fn on_download_updated(
            &self, browser: Option<&mut Browser>, item: Option<&mut DownloadItem>,
            _callback: Option<&mut DownloadItemCallback>,
        ) {
            let (Some(browser), Some(item)) = (browser, item) else { return };
            if item.is_valid() == 0 {
                return;
            }
            let id = (self.window, self.key, browser.identifier(), item.id());
            let in_progress = item.is_in_progress() != 0;
            let changed = ACTIVE.with(|items| {
                let mut items = items.borrow_mut();
                if in_progress {
                    items.insert(id)
                } else {
                    items.remove(&id)
                }
            });
            if changed {
                crate::window::downloads_changed(self.window);
                if !in_progress && self.key != 0 {
                    crate::window::download_ended(self.window, self.key, item.is_complete() != 0);
                }
            }
        }
    }
}
