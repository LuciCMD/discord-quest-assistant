//! Windows looks for a program's DLLs in its own folder before System32. A copy runs inside a game
//! folder in Steam mode, so a DLL there (planted, or a partial download) would load into it. This
//! flag makes Windows resolve the exe's own imports from System32 only (Windows 10 1607 and later).

fn main() {
    // LOAD_LIBRARY_SEARCH_SYSTEM32
    println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    println!("cargo:rerun-if-changed=build.rs");
}
