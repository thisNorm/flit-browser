use cef::*;
use windows_sys::Win32::System::{
    ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX2},
    Threading::GetCurrentProcess,
};

const REQUEST: &str = "Flit.TabMemory.Sample";
const REPLY: &str = "Flit.TabMemory.Result";

fn send(frame: &Frame, destination: ProcessId, name: &str, values: &[i32]) -> bool {
    if frame.is_valid() == 0 || frame.is_main() == 0 {
        return false;
    }
    let Some(mut message) = process_message_create(Some(&CefString::from(name))) else {
        return false;
    };
    let Some(args) = message.argument_list() else {
        return false;
    };
    if args.set_size(values.len()) == 0
        || values
            .iter()
            .enumerate()
            .any(|(i, &n)| args.set_int(i, n) == 0)
    {
        return false;
    }
    frame.send_process_message(destination, Some(&mut message));
    true
}

fn args(message: &ProcessMessage, name: &str, count: usize) -> Option<ListValue> {
    if message.is_valid() == 0 || CefString::from(&message.name()).to_string() != name {
        return None;
    }
    let args = message.argument_list()?;
    (args.size() == count && (0..count).all(|i| args.get_type(i) == ValueType::INT)).then_some(args)
}

pub(crate) fn request(frame: &Frame, id: i32) -> bool {
    send(frame, ProcessId::RENDERER, REQUEST, &[id])
}

pub(crate) fn read_reply(source: ProcessId, message: &ProcessMessage) -> Option<(i32, i32)> {
    if source != ProcessId::RENDERER {
        return None;
    }
    let args = args(message, REPLY, 2)?;
    (args.int(1) >= 0).then(|| (args.int(0), args.int(1)))
}

fn resident_mib() -> i32 {
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: size,
        ..Default::default()
    };
    // Query only this sandboxed renderer. Do not open a PID sent by another process.
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast(),
            size,
        )
    };
    if ok == 0 {
        return 0;
    }
    i32::try_from(counters.PrivateWorkingSetSize.div_ceil(1024 * 1024)).unwrap_or(0)
}

wrap_app! {
    pub(crate) struct ChildApp;
    impl App {
        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(MemoryRenderer::new())
        }
    }
}

wrap_render_process_handler! {
    struct MemoryRenderer;
    impl RenderProcessHandler {
        fn on_process_message_received(
            &self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>,
            source: ProcessId, message: Option<&mut ProcessMessage>,
        ) -> i32 {
            if source != ProcessId::BROWSER {
                return 0;
            }
            let Some(request) = message.and_then(|msg| args(msg, REQUEST, 1)) else { return 0; };
            let Some(frame) = frame.filter(|frame| frame.is_valid() != 0 && frame.is_main() != 0) else {
                return 0;
            };
            i32::from(send(frame, ProcessId::BROWSER, REPLY, &[request.int(0), resident_mib()]))
        }
    }
}
