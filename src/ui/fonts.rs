//! Lexend for the interface and Cascadia Mono for times, minutes and paths, bundled under the SIL
//! Open Font License with their licences in `assets/fonts`
//! egui's own fonts stay behind them as fallbacks for missing glyphs

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

use super::theme;

const LEXEND_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Lexend-Regular.ttf");
const LEXEND_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Lexend-Medium.ttf");
const LEXEND_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Lexend-SemiBold.ttf");
const CASCADIA_MONO: &[u8] = include_bytes!("../../assets/fonts/CascadiaMono-Regular.ttf");

pub fn install(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let fallbacks = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let medium = FontFamily::Name(theme::MEDIUM.into());
    let semibold = FontFamily::Name(theme::SEMIBOLD.into());
    defs.families.insert(medium.clone(), fallbacks.clone());
    defs.families.insert(semibold.clone(), fallbacks);
    put_first(
        &mut defs,
        FontFamily::Proportional,
        "Lexend",
        LEXEND_REGULAR,
    );
    put_first(&mut defs, medium, "Lexend Medium", LEXEND_MEDIUM);
    put_first(&mut defs, semibold, "Lexend SemiBold", LEXEND_SEMIBOLD);
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
