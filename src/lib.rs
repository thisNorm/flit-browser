#![cfg(all(target_os = "windows", target_arch = "x86_64"))]

mod chrome;
mod navigation;
mod updater;
mod window;

use cef::*;
use std::{fs, io::ErrorKind, path::PathBuf};
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

struct Launch {
    profile: PathBuf,
    url: String,
    default_profile: bool,
}

impl Launch {
    fn read() -> Result<Self, String> {
        let mut profile = None;
        let mut url = navigation::HOME.to_owned();
        for argument in std::env::args_os().skip(1) {
            let Some(argument) = argument.to_str() else {
                continue;
            };
            if let Some(value) = argument.strip_prefix("--flit-profile=") {
                profile = Some(PathBuf::from(value));
            } else if let Some(value) = argument.strip_prefix("--flit-url=") {
                url = navigation::address(value).map_err(str::to_owned)?;
            }
        }
        let default_profile = profile.is_none();
        let profile = match profile {
            Some(path) => path,
            None => {
                PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set.")?)
                    .join("Flit Browser")
                    .join("User Data")
            }
        };
        if !profile.is_absolute() {
            return Err("The Flit profile path must be absolute.".into());
        }
        fs::create_dir_all(&profile)
            .map_err(|e| format!("Could not create the Flit profile: {e}"))?;
        let profile = fs::canonicalize(profile)
            .map_err(|e| format!("Could not resolve the Flit profile: {e}"))?;
        // CEF/Chromium expects a native absolute path rather than the Win32 verbatim prefix.
        let path = profile
            .to_str()
            .ok_or("The Flit profile path contains unsupported characters.")?;
        let path = match path.strip_prefix(r"\\?\UNC\") {
            Some(unc) => PathBuf::from(format!(r"\\{unc}")),
            None => PathBuf::from(path.strip_prefix(r"\\?\").unwrap_or(path)),
        };
        let marker = path.join(".flit-browser-profile");
        match fs::read(&marker) {
            Ok(bytes) if bytes == b"Flit Browser profile v1\n" => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                if fs::read_dir(&path)
                    .map_err(|e| e.to_string())?
                    .next()
                    .is_some()
                {
                    return Err("Choose an empty directory or an existing Flit profile.".into());
                }
                use std::io::Write;
                fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&marker)
                    .and_then(|mut file| file.write_all(b"Flit Browser profile v1\n"))
                    .map_err(|e| format!("Could not mark the Flit profile: {e}"))?;
            }
            _ => return Err("The directory is not a Flit profile.".into()),
        }
        Ok(Self {
            profile: path,
            url,
            default_profile,
        })
    }
}

wrap_app! {
    struct FlitApp { initial_url: String }
    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(FlitProcess::new(self.initial_url.clone()))
        }
    }
}

wrap_browser_process_handler! {
    struct FlitProcess { initial_url: String }
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            if let Err(message) = window::open(&self.initial_url) {
                window::startup_failed(message);
            }
        }

        fn on_already_running_app_relaunch(
            &self, _command_line: Option<&mut CommandLine>, _current_directory: Option<&CefString>
        ) -> i32 {
            window::show_existing();
            1
        }
    }
}

fn error(message: &str) -> i32 {
    eprintln!("Flit Browser: {message}");
    let message = window::wide(message);
    let caption = window::wide("Flit Browser");
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
    1
}

// ABI from CEF's Windows bootstrap; keep this entry point for both browser and child processes.
#[unsafe(no_mangle)]
extern "C" fn RunWinMain(
    instance: sys::HINSTANCE,
    _command_line: *mut u16,
    _command_show: i32,
    sandbox_info: *mut u8,
    _version_info: *mut std::ffi::c_void,
) -> i32 {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let main_args = MainArgs { instance };
    if sandbox_info.is_null() {
        // Fail closed; the application is not supported without the official sandbox broker.
        return 1;
    }
    let subprocess_result = execute_process(Some(&main_args), None, sandbox_info);
    if subprocess_result >= 0 {
        return subprocess_result;
    }
    // No browser profile is accessed from the renderer/utility/GPU branch above.
    let launch = match Launch::read() {
        Ok(launch) => launch,
        Err(message) => return error(&message),
    };
    let path = CefString::from(launch.profile.to_str().expect("profile path validated"));
    updater::set_profile(launch.default_profile);
    let log = launch.profile.join("flit-cef.log");
    let mut app = FlitApp::new(launch.url);
    let settings = Settings {
        no_sandbox: 0,
        command_line_args_disabled: 1,
        root_cache_path: path.clone(),
        cache_path: path,
        log_file: CefString::from(log.to_string_lossy().as_ref()),
        chrome_app_icon_id: 32512,
        ..Default::default()
    };
    if initialize(
        Some(&main_args),
        Some(&settings),
        Some(&mut app),
        sandbox_info,
    ) == 0
    {
        return get_exit_code();
    }
    if !window::initialization_failed() {
        if launch.default_profile {
            updater::check_automatic();
        }
        run_message_loop();
    }
    drop(app);
    shutdown();
    let code = window::finish();
    if code == 0 && launch.default_profile {
        updater::apply_if_ready();
    }
    code
}
