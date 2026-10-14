//! DLL search hardening, and the app icon rendered from `assets/icon.svg`
//!
//! Windows looks for DLLs beside the exe before System32, and in Steam mode the copy runs inside
//! a game folder, so a planted or half-downloaded DLL there would load into it
//! The flag resolves the exe's imports from System32 only (Windows 10 1607 and later)

use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use resvg::{tiny_skia, usvg};

const ICON_SVG: &str = "assets/icon.svg";
/// Sizes Windows picks from for the title bar, taskbar, Explorer and Alt+Tab at any scaling
const ICO_SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];
/// The window icon, which `src/ui/icon.rs` reads back with the same size
const WINDOW_ICON: u32 = 64;

fn main() -> Result<(), Box<dyn Error>> {
    // LOAD_LIBRARY_SEARCH_SYSTEM32
    println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ICON_SVG}");

    let out = PathBuf::from(env::var("OUT_DIR")?);
    let tree = usvg::Tree::from_data(&fs::read(ICON_SVG)?, &usvg::Options::default())?;
    fs::write(out.join("icon-64.rgba"), render(&tree, WINDOW_ICON)?)?;

    let ico = out.join("icon.ico");
    write_ico(&tree, &ico)?;
    if env::var("CARGO_CFG_TARGET_OS")? == "windows" {
        // Only the icon, since a version block would make Task Manager list a stand-in under
        // this app's name instead of the game's exe name
        let ico = ico.to_str().ok_or("OUT_DIR isn't valid UTF-8")?;
        let rc = out.join("icon.rc");
        let quoted = ico.replace('\\', "\\\\").replace('"', "\"\"");
        fs::write(&rc, format!("1 ICON \"{quoted}\"\n"))?;
        let rc = rc.to_str().ok_or("OUT_DIR isn't valid UTF-8")?;
        winresource::WindowsResource::new()
            .set_resource_file(rc)
            .compile()?;
    }
    Ok(())
}

fn write_ico(tree: &usvg::Tree, path: &Path) -> Result<(), Box<dyn Error>> {
    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in ICO_SIZES {
        let image = ico::IconImage::from_rgba_data(size, size, render(tree, size)?);
        dir.add_entry(ico::IconDirEntry::encode(&image)?);
    }
    dir.write(fs::File::create(path)?)?;
    Ok(())
}

/// Straight (not premultiplied) RGBA, scaled to fit a `size` square and centred in it
fn render(tree: &usvg::Tree, size: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("icon size can't be zero")?;
    let svg = tree.size();
    #[allow(clippy::cast_precision_loss)]
    let square = size as f32;
    let scale = square / svg.width().max(svg.height());
    let dx = (square - svg.width() * scale) / 2.0;
    let dy = (square - svg.height() * scale) / 2.0;
    resvg::render(
        tree,
        tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect())
}
