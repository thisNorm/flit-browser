use std::{os::windows::ffi::OsStrExt, ptr::null_mut, sync::OnceLock};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, HWND},
    System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
        LoadLibraryExW,
    },
};

use crate::exact_model::{CreateSurface, Surface, UpdateSurface};

struct Exact {
    create: CreateSurface,
    update: UpdateSurface,
}

fn exact() -> Result<&'static Exact, &'static str> {
    static API: OnceLock<Exact> = OnceLock::new();
    if let Some(api) = API.get() {
        return Ok(api);
    }

    let exe = std::env::current_exe().map_err(|_| "Could not locate Flit Browser.")?;
    let library = exe.with_file_name("FlitExact.dll");
    let wide: Vec<u16> = library.as_os_str().encode_wide().chain([0]).collect();
    let module = unsafe {
        LoadLibraryExW(
            wide.as_ptr(),
            null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return Err("Could not load the Flit interface library. Repair the Flit installation.");
    }

    let create = unsafe { GetProcAddress(module, c"flit_exact_create_v1".as_ptr().cast()) };
    let update = unsafe { GetProcAddress(module, c"flit_exact_update_v1".as_ptr().cast()) };
    let (Some(create), Some(update)) = (create, update) else {
        unsafe { FreeLibrary(module) };
        return Err("The Flit interface library does not match this version.");
    };
    let api = Exact {
        // The versioned exports accept only HWNDs, integer IDs, and borrowed UTF-8 bytes.
        create: unsafe { std::mem::transmute::<_, CreateSurface>(create) },
        update: unsafe { std::mem::transmute::<_, UpdateSurface>(update) },
    };
    // Exact is used on the Win32 UI thread. Keep its memory mapped for the episode
    // because its Win32 class and renderer caches can outlive any one surface.
    Ok(API.get_or_init(|| api))
}

pub fn create(
    parent: HWND,
    owner: HWND,
    id: usize,
    surface: &Surface,
) -> Result<HWND, &'static str> {
    let model = serde_json::to_vec(surface).map_err(|_| "Could not read the interface values.")?;
    let panel = unsafe { (exact()?.create)(parent, owner, id, model.as_ptr(), model.len()) };
    if panel.is_null() {
        return Err("Could not open the Flit interface.");
    }
    Ok(panel)
}

pub fn update(panel: HWND, surface: &Surface) -> bool {
    let Ok(api) = exact() else { return false };
    serde_json::to_vec(surface).is_ok_and(|model| {
        (unsafe { (api.update)(panel, model.as_ptr(), model.len()) }) != 0
    })
}
