//! The window icon and the queue buttons' icons, rendered from SVG by `build.rs`
//! The exe's own icon is embedded there too, as a resource

use std::sync::Arc;

use eframe::egui::{self, ColorImage, IconData, TextureHandle, TextureOptions};

const SIZE: u32 = 64;
const RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon-64.rgba"));

/// White on clear, tinted where drawn
const BUTTON_SIZE: usize = 32;
const QUEUE_TOP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/queue-top.rgba"));
const QUEUE_BOTTOM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/queue-bottom.rgba"));

pub fn window_icon() -> Arc<IconData> {
    debug_assert_eq!(RGBA.len(), (SIZE * SIZE * 4) as usize);
    Arc::new(IconData {
        rgba: RGBA.to_vec(),
        width: SIZE,
        height: SIZE,
    })
}

/// Lucide's list-start and list-end, uploaded once per window
#[derive(Clone)]
pub struct QueueIcons {
    pub top: TextureHandle,
    pub bottom: TextureHandle,
}

impl QueueIcons {
    pub fn load(ctx: &egui::Context) -> Self {
        let texture = |name: &str, rgba: &[u8]| {
            debug_assert_eq!(rgba.len(), BUTTON_SIZE * BUTTON_SIZE * 4);
            let image = ColorImage::from_rgba_unmultiplied([BUTTON_SIZE, BUTTON_SIZE], rgba);
            ctx.load_texture(name, image, TextureOptions::LINEAR)
        };
        Self {
            top: texture("queue-top", QUEUE_TOP),
            bottom: texture("queue-bottom", QUEUE_BOTTOM),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_icon_is_a_full_square_that_isnt_blank() {
        let icon = window_icon();
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
        let opaque = icon.rgba.chunks(4).filter(|p| p[3] > 0).count();
        assert!(
            opaque > (SIZE * SIZE / 4) as usize,
            "only {opaque} visible pixels"
        );
    }

    #[test]
    fn button_icons_are_white_so_a_tint_shows_true() {
        for rgba in [QUEUE_TOP, QUEUE_BOTTOM] {
            assert_eq!(rgba.len(), BUTTON_SIZE * BUTTON_SIZE * 4);
            let drawn: Vec<&[u8]> = rgba.chunks(4).filter(|p| p[3] > 200).collect();
            assert!(drawn.len() > 60, "only {} solid pixels", drawn.len());
            assert!(
                drawn.iter().all(|p| p[0] > 240 && p[1] > 240 && p[2] > 240),
                "currentColor wasn't swapped for white"
            );
        }
    }

    const ICO: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon.ico"));
    const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

    fn u16_at(b: &[u8], at: usize) -> usize {
        usize::from(u16::from_le_bytes([b[at], b[at + 1]]))
    }

    fn u32_at(b: &[u8], at: usize) -> usize {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
    }

    fn png_size(png: &[u8]) -> (usize, usize) {
        let be = |at: usize| u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
        (be(16) as usize, be(20) as usize)
    }

    #[test]
    fn the_ico_holds_every_size_smallest_first_with_matching_images() {
        assert_eq!((u16_at(ICO, 0), u16_at(ICO, 2)), (0, 1), "not an icon file");
        let count = u16_at(ICO, 4);
        let mut sizes = Vec::new();
        for i in 0..count {
            let e = 6 + 16 * i;
            // 0 in the directory means 256
            let w = if ICO[e] == 0 {
                256
            } else {
                usize::from(ICO[e])
            };
            let h = if ICO[e + 1] == 0 {
                256
            } else {
                usize::from(ICO[e + 1])
            };
            assert_eq!(w, h, "entry {i} isn't square");
            assert_eq!(u16_at(ICO, e + 6), 32, "entry {i} isn't 32 bpp");
            let (len, off) = (u32_at(ICO, e + 8), u32_at(ICO, e + 12));
            let image = &ICO[off..off + len];
            if image.starts_with(PNG_MAGIC) {
                assert_eq!(png_size(image), (w, h), "entry {i} PNG size");
            } else {
                assert!(w < 256, "the 256 entry must be PNG");
                // BMP height covers the XOR and AND masks
                assert_eq!(
                    (u32_at(image, 4), u32_at(image, 8)),
                    (w, h * 2),
                    "entry {i} BMP size"
                );
            }
            sizes.push(w);
        }
        assert_eq!(sizes, [16, 20, 24, 32, 40, 48, 64, 128, 256]);
    }

    #[test]
    fn the_window_icon_is_straight_alpha() {
        let icon = window_icon();
        // premultiplied colour can never exceed its alpha, so one such pixel proves straight alpha
        let straight = icon
            .rgba
            .chunks(4)
            .filter(|p| p[3] > 0 && p[3] < 128)
            .any(|p| p[..3].iter().any(|&c| c > p[3]));
        assert!(straight, "faint edge pixels look premultiplied");
    }

    #[test]
    fn the_window_icon_fills_the_square() {
        let icon = window_icon();
        let size = icon.width as usize;
        let seen: Vec<(usize, usize)> = icon
            .rgba
            .chunks(4)
            .enumerate()
            .filter(|(_, p)| p[3] > 0)
            .map(|(i, _)| (i % size, i / size))
            .collect();
        let (x0, x1) = (
            seen.iter().map(|p| p.0).min(),
            seen.iter().map(|p| p.0).max(),
        );
        let (y0, y1) = (
            seen.iter().map(|p| p.1).min(),
            seen.iter().map(|p| p.1).max(),
        );
        let span = (x1.unwrap() - x0.unwrap()).max(y1.unwrap() - y0.unwrap()) + 1;
        assert!(
            span >= size - 4,
            "content spans only {span} of {size} px, so it's rendered small"
        );
    }

    /// The resource link arg reaches every target, so the test exe carries the same icon as the app
    #[test]
    #[cfg(windows)]
    fn the_exe_carries_an_icon_group() {
        const RT_ICON: usize = 3;
        const RT_GROUP_ICON: usize = 14;
        const RT_VERSION: usize = 16;
        let exe = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        let pe = u32_at(&exe, 0x3c);
        assert_eq!(&exe[pe..pe + 4], b"PE\0\0");
        let sections = u16_at(&exe, pe + 6);
        let opt = pe + 24;
        let dirs = opt + if u16_at(&exe, opt) == 0x20b { 112 } else { 96 };
        let res_rva = u32_at(&exe, dirs + 2 * 8);
        assert_ne!(res_rva, 0, "no resource section");
        let first = opt + u16_at(&exe, pe + 20);
        let file_off = |rva: usize| {
            (0..sections)
                .map(|i| first + 40 * i)
                .find_map(|s| {
                    let (va, size) = (u32_at(&exe, s + 12), u32_at(&exe, s + 16));
                    (va..va + size)
                        .contains(&rva)
                        .then(|| rva - va + u32_at(&exe, s + 20))
                })
                .unwrap()
        };
        let base = file_off(res_rva);
        let entries = |dir: usize| {
            let n = u16_at(&exe, dir + 12) + u16_at(&exe, dir + 14);
            (0..n)
                .map(|i| {
                    (
                        u32_at(&exe, dir + 16 + 8 * i),
                        u32_at(&exe, dir + 20 + 8 * i) & 0x7fff_ffff,
                    )
                })
                .collect::<Vec<_>>()
        };
        // type, then name, then language, then the data entry
        let data = |kind: usize| -> Vec<(usize, Vec<u8>)> {
            let mut out = Vec::new();
            for (_, names) in entries(base).into_iter().filter(|e| e.0 == kind) {
                for (id, langs) in entries(base + names) {
                    for (_, leaf) in entries(base + langs) {
                        let at = file_off(u32_at(&exe, base + leaf));
                        let len = u32_at(&exe, base + leaf + 4);
                        out.push((id, exe[at..at + len].to_vec()));
                    }
                }
            }
            out
        };
        // A version block would make Task Manager name a stand-in after this app, not the game
        assert!(data(RT_VERSION).is_empty(), "the exe has a version block");
        let groups = data(RT_GROUP_ICON);
        assert_eq!(groups.len(), 1, "expected one icon group");
        let icons = data(RT_ICON);
        let group = &groups[0].1;
        let count = u16_at(group, 4);
        assert_eq!(count, 9);
        assert_eq!(icons.len(), count);
        for i in 0..count {
            let e = 6 + 14 * i;
            let w = if group[e] == 0 {
                256
            } else {
                usize::from(group[e])
            };
            let bytes = u32_at(group, e + 8);
            let id = u16_at(group, e + 12);
            let image = &icons.iter().find(|(i, _)| *i == id).unwrap().1;
            assert_eq!(image.len(), bytes, "entry {i} length");
            if image.starts_with(PNG_MAGIC) {
                assert_eq!(png_size(image), (w, w), "entry {i}");
            }
        }
    }
}
