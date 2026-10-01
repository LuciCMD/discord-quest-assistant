//! Fonts: Roboto for the interface and Cascadia Mono for times and paths, both bundled (SIL Open
//! Font License, licence files beside them in `assets/fonts`). egui's own fonts stay behind them as
//! fallbacks for any glyph they lack.

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

use super::theme;

const ROBOTO_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Roboto-Regular.ttf");
const ROBOTO_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Roboto-Medium.ttf");
const CASCADIA_MONO: &[u8] = include_bytes!("../../assets/fonts/CascadiaMono-Regular.ttf");

pub fn install(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let fallbacks = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let medium = FontFamily::Name(theme::MEDIUM.into());
    defs.families.insert(medium.clone(), fallbacks);
    put_first(
        &mut defs,
        FontFamily::Proportional,
        "Roboto",
        ROBOTO_REGULAR,
    );
    put_first(&mut defs, medium, "Roboto Medium", ROBOTO_MEDIUM);
    put_first(
        &mut defs,
        FontFamily::Monospace,
        "Cascadia Mono",
        CASCADIA_MONO,
    );
    ctx.set_fonts(defs);
}

fn put_first(defs: &mut FontDefinitions, family: FontFamily, name: &str, bytes: &'static [u8]) {
    defs.font_data
        .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    defs.families
        .entry(family)
        .or_default()
        .insert(0, name.to_owned());
}
