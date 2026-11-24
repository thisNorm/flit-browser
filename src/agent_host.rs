//! Lets an MCP agent process (`Flit.exe --flit-agent`) drive real tabs in the
//! running Flit. The agent process keeps speaking MCP on stdio and forwards each
//! typed command over a local named pipe, one JSON line per request and reply.

use crate::agent_protocol::Command;
use cef::*;
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    os::windows::io::FromRawHandle,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering::Relaxed},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE,
        GetLastError, INVALID_HANDLE_VALUE,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    },
    System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_WAIT, WaitNamedPipeW,
    },
};

/// Close the tabs an AI session opened once it disconnects. A user setting.
pub(crate) static AUTO_CLOSE: AtomicBool = AtomicBool::new(true);

const MAX_LINE: u64 = 512 * 1024;
const LAUNCH_WAIT: Duration = Duration::from_secs(25);

fn pipe_name(profile: &Path) -> Vec<u16> {
    // FNV-1a of the profile path: one pipe per profile without the path in the name.
    let path = profile.to_string_lossy().to_lowercase();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
    }
    crate::window::wide(&format!(r"\\.\pipe\FlitBrowser-agent-{hash:016x}"))
}

/// The profile a plain `Flit.exe` launch uses; the agent process can only reach that one.
fn default_profile() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
        .join("Flit Browser")
        .join("User Data");
    Some(std::fs::canonicalize(&path).unwrap_or(path))
}

/// Serve agent sessions for this profile, one at a time, on a background thread.
pub(crate) fn serve(profile: &Path) {
    let name = pipe_name(profile);
    let spawned = std::thread::Builder::new()
        .name("flit-agent-host".into())
        .spawn(move || {
            loop {
                // First-instance: refuse to share the name with a pipe someone else made.
                let pipe = unsafe {
                    CreateNamedPipeW(
                        name.as_ptr(),
                        PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                        1,
                        64 * 1024,
                        64 * 1024,
                        0,
                        std::ptr::null(),
                    )
                };
                if pipe == INVALID_HANDLE_VALUE {
                    eprintln!("Flit Browser: AI connections are unavailable (pipe in use).");
                    return;
                }
                let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) } != 0
                    || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
                if !connected {
                    unsafe { CloseHandle(pipe) };
                    continue;
                }
                session(unsafe { File::from_raw_handle(pipe as _) });
            }
        });
    if let Err(error) = spawned {
        eprintln!("Flit Browser: could not start AI connections: {error}");
    }
}

fn session(pipe: File) {
    let Ok(writer) = pipe.try_clone() else { return };
    let mut writer = writer;
    let mut reader = BufReader::new(pipe);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.by_ref().take(MAX_LINE).read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) if !line.ends_with('\n') => break,
            Ok(_) => {}
        }
        let reply = match serde_json::from_str::<Command>(&line) {
            Ok(command) => crate::agent::request(Some(command)),
            Err(_) => Err("Invalid agent request.".to_owned()),
        };
        if serde_json::to_writer(&mut writer, &reply).is_err() || writer.write_all(b"\n").is_err() {
            break;
        }
    }
    let auto_close = AUTO_CLOSE.load(Relaxed);
    if auto_close {
        let _ = crate::agent::request(None);
    }
    let mut task = SessionDone::new(auto_close);
    post_task(ThreadId::UI, Some(&mut task));
}

wrap_task! {
    struct SessionDone { auto_close: bool }
    impl Task {
        fn execute(&self) {
            if !self.auto_close {
                crate::agent::detach_owner_tabs();
            }
            crate::window::agent_session_done(self.auto_close);
        }
    }
}

/// The agent process's end of the pipe.
pub(crate) struct Relay {
    reader: BufReader<File>,
    writer: File,
}

impl Relay {
    /// Reach the running Flit; with `launch`, start it first when it is not running.
    pub(crate) fn connect(launch: bool) -> Option<Self> {
        let name = pipe_name(&default_profile()?);
        let deadline = Instant::now() + LAUNCH_WAIT;
        let mut launched = false;
        loop {
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                let pipe = unsafe { File::from_raw_handle(handle as _) };
                let writer = pipe.try_clone().ok()?;
                return Some(Self {
                    reader: BufReader::new(pipe),
                    writer,
                });
            }
            if unsafe { GetLastError() } == ERROR_PIPE_BUSY {
                // Another AI session holds the browser; wait for our turn.
                unsafe { WaitNamedPipeW(name.as_ptr(), 2000) };
            } else if !launch {
                return None;
            } else if !launched {
                launched = true;
                let exe = std::env::current_exe().ok()?;
                std::process::Command::new(exe)
                    .arg("--flit-agent-launch")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .ok()?;
            } else {
                std::thread::sleep(Duration::from_millis(250));
            }
            if Instant::now() > deadline {
                return None;
            }
        }
    }

    pub(crate) fn call(&mut self, command: &Command) -> Result<Value, String> {
        const LOST: &str = "Flit closed. Ask again to reopen it.";
        serde_json::to_writer(&mut self.writer, command).map_err(|_| LOST)?;
        self.writer.write_all(b"\n").map_err(|_| LOST)?;
        let mut line = String::new();
        match self.reader.by_ref().take(MAX_LINE * 64).read_line(&mut line) {
            Ok(n) if n > 0 => serde_json::from_str(&line).map_err(|_| LOST.to_owned())?,
            _ => Err(LOST.to_owned()),
        }
    }
}
