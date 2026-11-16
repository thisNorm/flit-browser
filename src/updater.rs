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
    use cef::sys::{
        CEF_VERSION_MAJOR, CEF_VERSION_MINOR, CEF_VERSION_PATCH, CHROME_VERSION_BUILD,
        CHROME_VERSION_MAJOR, CHROME_VERSION_MINOR, CHROME_VERSION_PATCH,
    };
    let headline = format!(
        "Flit Browser {CURRENT_VERSION}\nCEF {CEF_VERSION_MAJOR}.{CEF_VERSION_MINOR}.{CEF_VERSION_PATCH} · Chromium {CHROME_VERSION_MAJOR}.{CHROME_VERSION_MINOR}.{CHROME_VERSION_BUILD}.{CHROME_VERSION_PATCH}"
    );
    let Some(update) = Update::discover() else {
        return format!(
            "{headline}\n\nAutomatic updates are not configured for this installation."
        );
    };
    if update.cache.join(READY).is_file() {
        return format!(
            "{headline}\n\nA verified update is staged. It will install after Flit closes."
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
        "Checking for updates in the background."
    } else {
        "Could not start the update check."
    };
    match status {
        Some(last) => format!("{headline}\n\n{current}\nLast check: {last}"),
        None => format!("{headline}\n\n{current}"),
    }
}
