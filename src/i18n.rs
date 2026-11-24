use serde::Deserialize;
use std::{cell::Cell, fs::File, io::Read, path::Path};

const MAX_LOCAL_STATE_BYTES: usize = 16 * 1024 * 1024;

thread_local! {
    static KOREAN: Cell<bool> = const { Cell::new(false) };
}

#[derive(Deserialize)]
struct LocalState {
    intl: Option<Intl>,
}

#[derive(Deserialize)]
struct Intl {
    app_locale: Option<String>,
}

fn display_locale(bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<LocalState>(bytes)
        .ok()?
        .intl?
        .app_locale?;
    if value.is_empty()
        || value.len() > 32
        || !value
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphanumeric()))
    {
        return None;
    }
    Some(if value.eq_ignore_ascii_case("ko-KR") {
        "ko".into()
    } else {
        value
    })
}

pub(crate) fn startup(profile: &Path) -> String {
    let mut data = Vec::new();
    let preference = match File::open(profile.join("Local State")).and_then(|file| {
        file.take(MAX_LOCAL_STATE_BYTES as u64 + 1)
            .read_to_end(&mut data)
    }) {
        Ok(size) if size <= MAX_LOCAL_STATE_BYTES => display_locale(&data),
        _ => None,
    };
    let locale = preference
        .filter(|locale| {
            std::env::current_exe().ok().is_some_and(|exe| {
                exe.with_file_name("locales")
                    .join(format!("{locale}.pak"))
                    .is_file()
            })
        })
        .unwrap_or_else(|| "en-US".into());
    KOREAN.set(locale.eq_ignore_ascii_case("ko"));
    locale
}

pub(crate) fn korean() -> bool {
    KOREAN.get()
}

pub(crate) fn choose<'a>(english: &'a str, korean: &'a str) -> &'a str {
    if KOREAN.get() { korean } else { english }
}

pub(crate) fn text(value: &str) -> &str {
    if !KOREAN.get() {
        return value;
    }
    match value {
        "New tab" => "새 탭",
        "New window" => "새 창",
        "New private window" => "새 비공개 창",
        "Flit menu" => "Flit 메뉴",
        "Tabs" => "탭",
        "No matching tabs" => "일치하는 탭 없음",
        "No pages loaded" => "열린 페이지 없음",
        "Not loaded · no renderer" => "아직 로드되지 않음",
        "Measuring renderer…" => "RAM 확인 중…",
        "Renderer RAM unavailable" => "RAM 확인 불가",
        "Search or enter address" => "검색하거나 주소 입력",
        "Search tabs" => "탭 검색",
        "Find in page" => "페이지에서 찾기",
        "Back" => "뒤로",
        "Forward" => "앞으로",
        "Reload" => "새로고침",
        "Stop loading" => "로드 중지",
        "Go to address" => "주소 열기",
        "Site information" => "사이트 정보",
        "Show or hide vertical tabs" => "세로 탭 접기 또는 펼치기",
        "Settings" => "설정",
        "Display language…" => "표시 언어…",
        "Download settings…" => "다운로드 설정…",
        "Chromium settings…" => "Chromium 고급 설정…",
        "Close window" => "창 닫기",
        "Close tab" => "탭 닫기",
        "Reopen last closed" => "최근 닫은 항목 다시 열기",
        "Recently closed" => "최근 닫은 항목",
        "Search open tabs" => "열린 탭 검색",
        "Search engine" => "검색 엔진",
        "Search the web" => "웹에서 검색",
        "Free memory…" => "메모리 정리…",
        "Free memory from background tabs…" => "백그라운드 탭 메모리 정리…",
        "Load background links only when selected" => "백그라운드 링크는 탭을 선택할 때 로드",
        "Bookmark this page" => "현재 페이지 북마크에 추가",
        "Bookmark all tabs…" => "모든 탭을 북마크에 추가…",
        "Manage bookmarks" => "북마크 관리",
        "Import bookmarks from HTML…" => "HTML 파일에서 북마크 가져오기…",
        "Export bookmarks to HTML…" => "북마크를 HTML 파일로 내보내기…",
        "Import Chrome bookmarks and folders…" => "Chrome 북마크 및 폴더 가져오기…",
        "Import Edge bookmarks and folders…" => "Edge 북마크 및 폴더 가져오기…",
        "Move sites and bookmarks" => "사이트 및 북마크 옮기기",
        "From Chrome bookmarks…" => "Chrome 북마크에서 사이트 열기…",
        "From Edge bookmarks…" => "Edge 북마크에서 사이트 열기…",
        "From copied site links…" => "복사한 링크에서 사이트 열기…",
        "History" => "방문 기록",
        "Downloads" => "다운로드",
        "Print…" => "인쇄…",
        "Zoom in" => "확대",
        "Zoom out" => "축소",
        "Site permissions" => "사이트 권한",
        "Clear browsing data…" => "인터넷 사용 기록 삭제…",
        "Password manager" => "비밀번호 관리자",
        "Import passwords from CSV…" => "CSV 파일에서 비밀번호 가져오기…",
        "Add or update Flit Vault login…" => "Flit Vault 로그인 추가 또는 변경…",
        "Remove Flit Vault login…" => "Flit Vault 로그인 삭제…",
        "Extensions" => "확장 프로그램",
        "Developer tools" => "개발자 도구",
        "Restore windows and tabs on startup" => "시작할 때 창과 탭 복원",
        "Open previous session backup" => "이전 세션 백업 열기",
        "Use Mica window frame" => "마이카 창 효과 사용",
        "Use acrylic window frame" => "아크릴 창 효과 사용",
        "About Flit and updates" => "Flit 정보 및 업데이트",
        "Exit Flit (all windows)" => "Flit 종료 (모든 창)",
        "Pin tab" => "탭 고정",
        "Unpin tab" => "탭 고정 해제",
        "Duplicate tab" => "탭 복제",
        "Mute tab" => "탭 음소거",
        "Unmute tab" => "탭 음소거 해제",
        "Unload tab to free memory…" => "이 탭의 메모리 정리…",
        "Move tab up" => "탭 위로 이동",
        "Move tab down" => "탭 아래로 이동",
        "Keep tab open after downloads" => "다운로드 후에도 탭 유지",
        "Prepare sites in Flit" => "사이트 및 북마크 가져오기",
        "Import bookmarks to Flit" => "Flit으로 북마크 가져오기",
        "Could not open settings." => "설정을 열지 못했습니다.",
        "Could not save this site's memory setting." => "이 사이트의 메모리 설정을 저장하지 못했습니다.",
        "There are no eligible background tabs to unload right now." => "지금은 정리할 수 있는 백그라운드 탭이 없습니다.",
        "Free tab memory" => "탭 메모리 정리",
        "Free tab memory?" => "탭 메모리를 정리할까요?",
        "Unload tab?" => "이 탭의 메모리를 정리할까요?",
        "Reload this page? Unsaved changes may be lost." => "이 페이지를 새로고침할까요? 저장하지 않은 변경 사항이 사라질 수 있습니다.",
        "Leave this page? Unsaved changes may be lost." => "이 페이지에서 나갈까요? 저장하지 않은 변경 사항이 사라질 수 있습니다.",
        "Cam/Mic on" => "카메라·마이크 켜짐",
        "Camera + mic in use" => "카메라와 마이크 사용 중",
        "Camera on" => "카메라 켜짐",
        "Camera in use" => "카메라 사용 중",
        "Mic on" => "마이크 켜짐",
        "Microphone in use" => "마이크 사용 중",
        "Untitled page" => "제목 없는 페이지",
        "Untitled window" => "제목 없는 창",
        "After download" => "다운로드 후 닫힘",
        "Download in progress" => "다운로드 진행 중",
        "This site uses HTTP. The connection is not encrypted." => "이 사이트는 HTTP를 사용합니다. 연결이 암호화되지 않습니다.",
        "The connection uses verified HTTPS." => "확인된 HTTPS로 연결되어 있습니다.",
        "The browser cannot confirm a fully secure connection to this site." => "이 사이트와의 연결이 완전히 안전한지 확인할 수 없습니다.",
        "Bookmark all tabs" => "모든 탭을 북마크에 추가",
        "Open a web page to save its tab." => "저장할 웹 페이지를 먼저 여세요.",
        "This window has too much bookmark data to save at once." => "이 창에는 한 번에 저장할 수 있는 양보다 많은 탭이 있습니다.",
        "This tab will reload when you select it. Back and Forward history will be cleared, audio and calls will stop, and unsaved changes may be lost. Unload the tab?" => "이 탭은 다시 선택할 때 새로 로드됩니다. 뒤로·앞으로 기록이 지워지고, 소리와 통화가 멈추며, 저장하지 않은 변경 사항이 사라질 수 있습니다. 메모리를 정리할까요?",
        "Some saved windows could not be opened." => "저장된 창 일부를 열지 못했습니다.",
        "No previous session backup is available." => "이전 세션 백업이 없습니다.",
        "Some windows in the previous session could not be opened." => "이전 세션의 창 일부를 열지 못했습니다.",
        "Windows from the previous session are already open." => "이전 세션의 창이 이미 열려 있습니다.",
        "Flit Browser — Private" => "Flit Browser — 비공개",
        "CEF could not create the browser tab." => "브라우저 탭을 만들지 못했습니다.",
        "CEF did not provide a windowed Chrome-style browser." => "브라우저 화면을 만들지 못했습니다.",
        "Could not create the tab panel." => "탭 패널을 만들지 못했습니다.",
        "Could not initialize the new-tab page." => "새 탭 화면을 준비하지 못했습니다.",
        "Could not create the private browsing context." => "비공개 탐색을 시작하지 못했습니다.",
        "Could not create the Flit window." => "Flit 창을 만들지 못했습니다.",
        "Could not create the Flit navigation controls." => "Flit 탐색 도구를 만들지 못했습니다.",
        "Flit is exiting." => "Flit을 종료하는 중입니다.",
        "The tab is no longer available." => "탭이 더 이상 없습니다.",
        "The page is no longer available." => "페이지가 더 이상 없습니다.",
        "The Flit window is not available." => "Flit 창을 사용할 수 없습니다.",
        "The Flit window closed during tab creation." => "탭을 만드는 동안 Flit 창이 닫혔습니다.",
        "Copy a smaller list of site links, then try again." => "더 적은 수의 사이트 링크를 복사한 뒤 다시 시도하세요.",
        "Copy the site links as text, then try again." => "사이트 링크를 텍스트로 복사한 뒤 다시 시도하세요.",
        "Flit could not read the copied text." => "복사한 텍스트를 읽지 못했습니다.",
        "The clipboard is busy. Try again after copying the site links." => "클립보드를 사용할 수 없습니다. 사이트 링크를 다시 복사한 뒤 시도하세요.",
        "Flit could not find the local browser profiles." => "이 PC에서 브라우저 프로필을 찾지 못했습니다.",
        "There are no new site links to prepare." => "새로 준비할 사이트 링크가 없습니다.",
        "Could not safely read the selected bookmarks file." => "선택한 북마크 파일을 안전하게 읽지 못했습니다.",
        "The bookmarks file is too large." => "북마크 파일이 너무 큽니다.",
        "The bookmarks file is invalid or nested too deeply." => "북마크 파일이 올바르지 않거나 폴더가 너무 깊습니다.",
        "Could not encode the bookmarks." => "북마크를 변환하지 못했습니다.",
        "The imported bookmarks are too large." => "가져올 북마크가 너무 많습니다.",
        "The bookmark folders are nested too deeply." => "북마크 폴더가 너무 깊습니다.",
        "The bookmarks contain too many items." => "북마크 항목이 너무 많습니다.",
        "Remove Flit Vault login" => "Flit Vault 로그인 삭제",
        "Add or update Flit Vault login" => "Flit Vault 로그인 추가 또는 변경",
        "Could not check the Flit Vault login." => "Flit Vault 로그인을 확인하지 못했습니다.",
        "Could not remove the Flit Vault login." => "Flit Vault 로그인을 삭제하지 못했습니다.",
        "Could not save the Flit Vault login." => "Flit Vault 로그인을 저장하지 못했습니다.",
        "The active tab or its secure connection changed. The login was not removed." => "활성 탭이나 보안 연결이 바뀌어 로그인을 삭제하지 않았습니다.",
        "The active tab or its secure connection changed. The login was not saved." => "활성 탭이나 보안 연결이 바뀌어 로그인을 저장하지 않았습니다.",
        "Could not open the Windows credential prompt." => "Windows 자격 증명 입력창을 열지 못했습니다.",
        "Enter a valid username and password." => "올바른 사용자 이름과 비밀번호를 입력하세요.",
        "Deny" => "거부",
        "Allow this time" => "이번만 허용",
        "Flit Browser — Site permission" => "Flit Browser — 사이트 권한",
        "Enter search terms." => "검색어를 입력하세요.",
        "Enter an address or search terms." => "주소나 검색어를 입력하세요.",
        "The address is invalid." => "주소가 올바르지 않습니다.",
        "Enter an HTTP or HTTPS address without credentials." => "계정 정보가 없는 HTTP 또는 HTTPS 주소를 입력하세요.",
        _ => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn korean_mode_translates_dialogs_and_keeps_unknown_text() {
        KOREAN.set(true);
        assert_eq!(text("Download in progress"), "다운로드 진행 중");
        assert_eq!(text("Unload tab?"), "이 탭의 메모리를 정리할까요?");
        assert_eq!(text("example.com"), "example.com");
        KOREAN.set(false);
        assert_eq!(text("Download in progress"), "Download in progress");
    }

    #[test]
    fn display_language_uses_the_application_preference_only() {
        assert_eq!(
            display_locale(br#"{"intl":{"app_locale":"ko"}}"#).as_deref(),
            Some("ko")
        );
        assert_eq!(
            display_locale(br#"{"intl":{"app_locale":"ko-KR"}}"#).as_deref(),
            Some("ko")
        );
        assert_eq!(
            display_locale(br#"{"intl":{"app_locale":"fr"}}"#).as_deref(),
            Some("fr")
        );
        assert_eq!(
            display_locale(br#"{"intl":{"selected_languages":"ko"}}"#),
            None
        );
        assert_eq!(display_locale(br#"{"intl":{"app_locale":"../ko"}}"#), None);
        assert_eq!(display_locale(br#"{"intl":null}"#), None);
    }
}
