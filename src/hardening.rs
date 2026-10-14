//! Process hardening that has to happen before anything else loads

/// Later `LoadLibrary` calls by bare name search System32 only, never the exe's own folder
/// `build.rs` covers the exe's imports, this covers DLLs loaded at run time (OpenGL, themes)
pub fn system32_dlls_only() -> bool {
    use windows_sys::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories,
    };
    // SAFETY: a plain Win32 call with a constant flag, no pointers, and it only changes this
    // process's DLL search order
    #[allow(unsafe_code)]
    let ok = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
    ok != 0
}
