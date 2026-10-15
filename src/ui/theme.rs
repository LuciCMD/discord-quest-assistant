//! The graphite theme for egui, the only place colours come from
//!
//! Panels are split by 1 px seams rather than shadows, and colour is spent on state only: blue for
//! live and selected, green for done, amber for warnings, red for destructive actions
//! egui paints one fill per widget state instead of a wash over the fill, so hover and press fills
//! are computed here from the wash tokens and nowhere else

use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Margin, Shadow, Stroke, Style,
    TextStyle, Visuals, style::HandleShape, style::WidgetVisuals,
};

// Surfaces, deepest first
pub const ABYSS: Color32 = Color32::from_rgb(0x1A, 0x1C, 0x20);
pub const GROUND: Color32 = Color32::from_rgb(0x1F, 0x21, 0x26);
pub const SURFACE: Color32 = Color32::from_rgb(0x24, 0x27, 0x2D);
pub const RAISED: Color32 = Color32::from_rgb(0x2B, 0x2F, 0x36);
pub const FIELD: Color32 = Color32::from_rgb(0x30, 0x34, 0x3C);
pub const LINE: Color32 = Color32::from_rgb(0x45, 0x4A, 0x54);
/// The seams between panels
pub const LINE_SOFT: Color32 = Color32::from_rgb(0x33, 0x37, 0x3F);

// Text
pub const INK: Color32 = Color32::from_rgb(0xE3, 0xE6, 0xEB);
pub const MUTED: Color32 = Color32::from_rgb(0xA6, 0xAC, 0xB6);
/// 4.9:1 on the panels, the floor for small text
pub const FAINT: Color32 = Color32::from_rgb(0x8E, 0x94, 0x9F);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x0E, 0x17, 0x26);

// Live and selected
pub const ACCENT: Color32 = Color32::from_rgb(0x7C, 0xB3, 0xFF);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x3F, 0x6A, 0xA8);

// State
pub const GOOD: Color32 = Color32::from_rgb(0x6F, 0xCF, 0x8E);
pub const WARN: Color32 = Color32::from_rgb(0xE8, 0xAA, 0x5C);
pub const BAD: Color32 = Color32::from_rgb(0xF0, 0x7A, 0x70);

// Fixed one-offs
pub const POPUP: Color32 = Color32::from_rgb(0x2E, 0x32, 0x39);
pub const TOGGLE_TRACK_OFF: Color32 = Color32::from_rgb(0x4A, 0x4F, 0x59);
pub const TOGGLE_KNOB_OFF: Color32 = Color32::from_rgb(0xC9, 0xCD, 0xD4);
pub const SLIDER_TRACK: Color32 = Color32::from_rgb(0x3A, 0x3F, 0x48);

// Radii
pub const RADIUS_DIALOG: u8 = 10;
pub const RADIUS_CONTROL: u8 = 7;
pub const RADIUS_PILL: u8 = 5;

/// Lexend Medium, for buttons and labels
pub const MEDIUM: &str = "medium";
/// Lexend `SemiBold`, for headings
pub const SEMIBOLD: &str = "semibold";

/// White at 6%, laid over a fill on hover
#[must_use]
pub fn hover_wash() -> Color32 {
    Color32::from_white_alpha(15)
}

/// White at 12%, laid over a fill while pressed
#[must_use]
pub fn press_wash() -> Color32 {
    Color32::from_white_alpha(31)
}

/// The accent at 16%, for selected rows and focus halos
#[must_use]
pub fn accent_wash() -> Color32 {
    ACCENT.gamma_multiply(0.16)
}

/// Named text styles beyond egui's five
#[derive(Clone, Copy)]
pub enum Text {
    /// A panel's title
    PanelHeading,
    /// A heading inside a panel, such as Up Next
    Section,
    /// The picked game's name on its card
    Nameplate,
    Control,
    Hint,
}

impl Text {
    fn name(self) -> &'static str {
        match self {
            Self::PanelHeading => "panel-heading",
            Self::Section => "section",
            Self::Nameplate => "nameplate",
            Self::Control => "control",
            Self::Hint => "hint",
        }
    }

    #[must_use]
    pub fn style(self) -> TextStyle {
        TextStyle::Name(self.name().into())
    }
}

/// Every egui style, dark and light, though only dark is used
pub fn apply(ctx: &egui::Context) {
    ctx.options_mut(|options| options.theme_preference = egui::ThemePreference::Dark);
    ctx.all_styles_mut(style_graphite);
}

fn style_graphite(style: &mut Style) {
    set_text_styles(style);
    set_spacing(style);
    style.visuals = visuals();
    style.animation_time = 0.12;
    style.interaction.selectable_labels = false;
    // Icon-only buttons rely on the tooltip for their name, egui's half second felt slow
    style.interaction.tooltip_delay = 0.15;
}

fn set_text_styles(style: &mut Style) {
    let regular = |size| FontId::new(size, FontFamily::Proportional);
    let medium = |size| FontId::new(size, FontFamily::Name(MEDIUM.into()));
    let semibold = |size| FontId::new(size, FontFamily::Name(SEMIBOLD.into()));
    // Three steps for words, rank carried by weight between them
    style.text_styles = [
        (TextStyle::Heading, semibold(20.0)),
        (TextStyle::Body, regular(14.0)),
        (TextStyle::Button, medium(13.5)),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
        (TextStyle::Small, regular(12.5)),
        (Text::PanelHeading.style(), semibold(15.0)),
        (Text::Section.style(), medium(12.5)),
        (Text::Nameplate.style(), semibold(20.0)),
        (Text::Control.style(), regular(14.0)),
        (Text::Hint.style(), regular(12.5)),
    ]
    .into();
}

fn set_spacing(style: &mut Style) {
    let spacing = &mut style.spacing;
    spacing.item_spacing = egui::vec2(8.0, 8.0);
    spacing.button_padding = egui::vec2(14.0, 8.0);
    spacing.interact_size = egui::vec2(40.0, 34.0);
    spacing.menu_margin = Margin::symmetric(0, 6);
    spacing.window_margin = Margin::symmetric(18, 16);
    spacing.slider_width = 220.0;
    spacing.slider_rail_height = 4.0;
    spacing.icon_width = 20.0;
    spacing.icon_spacing = 12.0;
    spacing.combo_height = 280.0;
    // Descriptions read as short paragraphs, not one long line
    spacing.tooltip_width = 300.0;
    // Floating and trackless, so a scrollbar never stands in for the 1 px seam beside it, in a
    // gutter of its own so it never sits over a row's buttons
    spacing.scroll = egui::style::ScrollStyle::floating();
    spacing.scroll.floating_allocated_width = spacing.scroll.bar_width;
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    // egui fills scrollbar thumbs with `bg_fill` and buttons with `weak_bg_fill`
    v.widgets.noninteractive = widget(SURFACE, SURFACE, Stroke::new(1.0, LINE_SOFT), MUTED);
    v.widgets.inactive = widget(LINE, FIELD, Stroke::NONE, INK);
    v.widgets.hovered = widget(
        LINE.blend(hover_wash()),
        FIELD.blend(hover_wash()),
        Stroke::NONE,
        INK,
    );
    v.widgets.active = widget(
        LINE.blend(press_wash()),
        FIELD.blend(press_wash()),
        Stroke::NONE,
        INK,
    );
    v.widgets.open = widget(LINE, FIELD, Stroke::NONE, INK);
    v.weak_text_color = Some(FAINT);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, INK);
    v.hyperlink_color = ACCENT;
    v.faint_bg_color = RAISED;
    v.extreme_bg_color = ABYSS;
    v.text_edit_bg_color = Some(FIELD);
    v.code_bg_color = ABYSS;
    v.warn_fg_color = WARN;
    v.error_fg_color = BAD;
    v.window_corner_radius = CornerRadius::same(RADIUS_CONTROL);
    v.window_fill = POPUP;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_shadow = popup_shadow();
    v.popup_shadow = popup_shadow();
    v.menu_corner_radius = CornerRadius::same(RADIUS_CONTROL);
    v.panel_fill = SURFACE;
    v.text_cursor.stroke = Stroke::new(2.0, ACCENT);
    v.slider_trailing_fill = true;
    v.handle_shape = HandleShape::Circle;
    v.interact_cursor = Some(CursorIcon::PointingHand);
    v.disabled_alpha = 0.42;
    v
}

fn widget(fill: Color32, button_fill: Color32, edge: Stroke, text: Color32) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: button_fill,
        bg_stroke: edge,
        corner_radius: CornerRadius::same(RADIUS_CONTROL),
        fg_stroke: Stroke::new(1.0, text),
        expansion: 0.0,
    }
}

/// The only shadow, under floating popups and dialogs
fn popup_shadow() -> Shadow {
    Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(120),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn washes_lighten_the_field_in_order() {
        let hover = FIELD.blend(hover_wash());
        let press = FIELD.blend(press_wash());
        assert!(hover.r() > FIELD.r() && press.r() > hover.r());
        assert!(
            press.r() < 0x60,
            "a wash is a slight lift, not a new colour"
        );
    }

    #[test]
    fn every_named_text_style_has_a_size() {
        let mut style = Style::default();
        set_text_styles(&mut style);
        for text in [
            Text::PanelHeading,
            Text::Section,
            Text::Nameplate,
            Text::Control,
            Text::Hint,
        ] {
            assert!(style.text_styles.contains_key(&text.style()));
        }
    }

    /// WCAG relative luminance
    fn luminance(c: Color32) -> f32 {
        let channel = |v: u8| {
            let s = f32::from(v) / 255.0;
            if s <= 0.039_28 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (hi, lo) = {
            let (x, y) = (luminance(a), luminance(b));
            if x > y { (x, y) } else { (y, x) }
        };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn text_meets_aa_on_every_surface_it_sits_on() {
        let pairs = [
            ("ink", INK, SURFACE),
            ("ink on raised", INK, RAISED),
            ("ink on a field", INK, FIELD),
            ("muted", MUTED, SURFACE),
            ("faint", FAINT, SURFACE),
            ("faint on ground", FAINT, GROUND),
            ("accent", ACCENT, SURFACE),
            ("good", GOOD, SURFACE),
            ("warn", WARN, SURFACE),
            ("bad", BAD, SURFACE),
            ("on accent", ON_ACCENT, ACCENT),
            ("ink on a popup", INK, POPUP),
            ("muted on a popup", MUTED, POPUP),
        ];
        let failing: Vec<String> = pairs
            .iter()
            .filter(|(_, fg, bg)| contrast(*fg, *bg) < 4.5)
            .map(|(name, fg, bg)| format!("{name}: {:.2}", contrast(*fg, *bg)))
            .collect();
        assert!(failing.is_empty(), "{failing:?}");
    }

    #[test]
    fn the_ground_is_soft_dark_not_black() {
        for surface in [ABYSS, GROUND, SURFACE] {
            assert!(
                surface.r() >= 0x18 && surface.g() >= 0x18 && surface.b() >= 0x18,
                "{surface:?} is near-black"
            );
        }
    }
}
