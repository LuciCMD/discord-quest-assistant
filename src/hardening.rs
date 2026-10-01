//! Process hardening that has to happen before anything else loads.

/// Makes every later `LoadLibrary` by bare name search System32 only, never the exe's own folder.
/// The build already does this for the exe's imports (`build.rs`); this covers DLLs that libraries
/// load at run time, such as the OpenGL and theme DLLs the window code asks for.
pub fn system32_dlls_only() -> bool {
    use windows_sys::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories,
    };
    // SAFETY: a plain Win32 call with a constant flag; it takes no pointers and only changes the
    // DLL search order for this process.
    #[allow(unsafe_code)]
    let ok = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
    ok != 0
}
