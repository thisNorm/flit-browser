use std::{
    env, fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const REPOSITORY: &str = match option_env!("FLIT_UPDATE_REPOSITORY") {
    Some(value) => value,
    None => "",
};
const PUBLIC_KEY: &str = match option_env!("FLIT_UPDATE_RSA_MODULUS") {
    Some(value) => value,
    None => "",
};
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const WORKER: &str = include_str!("../installer/Update-Flit.ps1");
const CREATE_NO_WINDOW: u32 = 0x08000000;
const STATUS: &str = "status.txt";
const LAST_CHECK: &str = "last-check.txt";
const READY: &str = "ready.txt";
static NORMAL_PROFILE: AtomicBool = AtomicBool::new(false);

// Read the loaded library; the Rust headers can be older than the bundle.
unsafe extern "C" {
    fn cef_version_info(entry: i32) -> i32;
}

pub(crate) fn set_profile(normal: bool) {
    NORMAL_PROFILE.store(normal, Ordering::Relaxed);
}

struct Update {
    executable: PathBuf,
    cache: PathBuf,
    powershell: PathBuf,
    modules: PathBuf,
}

impl Update {
    fn discover() -> Option<Self> {
        if !NORMAL_PROFILE.load(Ordering::Relaxed) || REPOSITORY.is_empty() || PUBLIC_KEY.is_empty()
        {
            return None;
        }
        let executable = env::current_exe().ok()?;
        if !executable.parent()?.join("flit-installed.ini").is_file() {
            return None;
        }
        let cache = PathBuf::from(env::var_os("LOCALAPPDATA")?).join("Flit Browser/Updates");
        let runtime =
            PathBuf::from(env::var_os("SystemRoot")?).join("System32/WindowsPowerShell/v1.0");
        let powershell = runtime.join("powershell.exe");
        let modules = runtime.join("Modules");
        Some(Self {
            executable,
            cache,
            powershell,
            modules,
        })
    }

    fn start(&self, mode: &str) -> bool {
        fs::create_dir_all(&self.cache).is_ok()
            && Command::new(&self.powershell)
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    WORKER,
                ])
                .env("FLIT_UPDATER_MODE", mode)
                .env("FLIT_UPDATER_REPO", REPOSITORY)
                .env("FLIT_UPDATER_PUBLIC_KEY", PUBLIC_KEY)
                .env("FLIT_UPDATER_CURRENT_VERSION", CURRENT_VERSION)
                .env("FLIT_UPDATER_CACHE", &self.cache)
                .env("FLIT_UPDATER_EXE", &self.executable)
                .env("FLIT_UPDATER_PARENT_PID", process::id().to_string())
                .env("PSModulePath", &self.modules)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .is_ok()
    }
}

fn recent(path: &Path, max_age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|mtime| mtime.elapsed().ok())
        .is_some_and(|age| age < max_age)
}

pub(crate) fn check_automatic() {
    if let Some(update) = Update::discover() {
        if !update.cache.join(READY).is_file()
            && !recent(&update.cache.join(LAST_CHECK), Duration::from_secs(86_400))
        {
            update.start("Check");
        }
    }
}

pub(crate) fn apply_if_ready() {
    if let Some(update) = Update::discover() {
        if update.cache.join(READY).is_file() {
            update.start("Apply");
        }
    }
}

pub(crate) fn check_manual() -> String {
    let [
        cef_major,
        cef_minor,
        cef_patch,
        _,
        chrome_major,
        chrome_minor,
        chrome_build,
        chrome_patch,
    ] = std::array::from_fn(|index| unsafe { cef_version_info(index as i32) });
    let headline = format!(
        "Flit Browser {CURRENT_VERSION}\nCEF {cef_major}.{cef_minor}.{cef_patch} · Chromium {chrome_major}.{chrome_minor}.{chrome_build}.{chrome_patch}"
    );
    let Some(update) = Update::discover() else {
        return format!(
            "{headline}\n\n{}",
            crate::i18n::choose(
                "Automatic updates are not configured for this installation.\nTo update an installed copy on Windows, close Flit and run the newer FlitSetup-x64.exe.",
                "이 설치본에는 자동 업데이트가 설정되어 있지 않습니다.\n업데이트하려면 Flit을 닫고 새 FlitSetup-x64.exe를 실행하세요."
            )
        );
    };
    if update.cache.join(READY).is_file() {
        return format!(
            "{headline}\n\n{}",
            crate::i18n::choose(
                "A verified update is staged. It will install after Flit closes.",
                "확인된 업데이트가 준비되었습니다. Flit을 닫으면 설치됩니다."
            )
        );
    }
    let status = fs::read_to_string(update.cache.join(STATUS))
        .ok()
        .and_then(|s| {
            s.lines()
                .next()
                .map(|line| line.chars().take(180).collect::<String>())
        })
        .filter(|s| !s.is_empty());
    let current = if update.start("Manual") {
        crate::i18n::choose(
            "Checking for updates in the background.",
            "백그라운드에서 업데이트를 확인하는 중입니다."
        )
    } else {
        crate::i18n::choose(
            "Could not start the update check.",
            "업데이트 확인을 시작하지 못했습니다."
        )
    };
    match status {
        Some(last) => format!(
            "{headline}\n\n{current}\n{}: {last}",
            crate::i18n::choose("Last check", "마지막 확인")
        ),
        None => format!("{headline}\n\n{current}"),
    }
}
