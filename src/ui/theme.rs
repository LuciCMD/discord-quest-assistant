//! Slate, the author's house theme, for egui. Every colour in the app comes from here.
//!
//! egui paints one fill per widget state instead of layering a wash over the fill, so the hover and
//! press fills are computed here from the wash tokens, once, and nowhere else.

use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Margin, Shadow, Stroke, Style,
    TextStyle, Visuals, style::HandleShape, style::WidgetVisuals,
};

// Surfaces, deepest first.
pub const ABYSS: Color32 = Color32::from_rgb(0x1A, 0x1B, 0x21);
pub const GROUND: Color32 = Color32::from_rgb(0x1F, 0x20, 0x27);
pub const SURFACE: Color32 = Color32::from_rgb(0x2A, 0x2B, 0x33);
pub const RAISED: Color32 = Color32::from_rgb(0x31, 0x32, 0x3B);
pub const FIELD: Color32 = Color32::from_rgb(0x3A, 0x3B, 0x46);
pub const LINE: Color32 = Color32::from_rgb(0x47, 0x48, 0x55);
pub const LINE_SOFT: Color32 = Color32::from_rgb(0x34, 0x35, 0x3F);

// Text.
pub const INK: Color32 = Color32::from_rgb(0xE4, 0xE4, 0xE8);
pub const MUTED: Color32 = Color32::from_rgb(0xA4, 0xA5, 0xAF);
pub const FAINT: Color32 = Color32::from_rgb(0x7E, 0x7F, 0x8A);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x1F, 0x1B, 0x2B);

// The one accent.
pub const ACCENT: Color32 = Color32::from_rgb(0xB3, 0x9D, 0xF0);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x7D, 0x6A, 0xB5);

// State.
pub const GOOD: Color32 = Color32::from_rgb(0x6C, 0xC7, 0x7A);
pub const WARN: Color32 = Color32::from_rgb(0xF5, 0xA7, 0x42);
pub const BAD: Color32 = Color32::from_rgb(0xEF, 0x6B, 0x67);

// Fixed one-offs from the reference app.
pub const POPUP: Color32 = Color32::from_rgb(0x34, 0x35, 0x3F);
pub const TOGGLE_TRACK_OFF: Color32 = Color32::from_rgb(0x5C, 0x5D, 0x69);
pub const TOGGLE_KNOB_OFF: Color32 = Color32::from_rgb(0xC9, 0xC9, 0xD0);
pub const SLIDER_TRACK: Color32 = Color32::from_rgb(0x4C, 0x4D, 0x59);

// Radii.
pub const RADIUS_CARD: u8 = 12;
pub const RADIUS_CONTROL: u8 = 8;
pub const RADIUS_PILL: u8 = 6;

/// Page content never runs wider than this, so lines stay readable.
pub const CONTENT_MAX_WIDTH: f32 = 720.0;

/// The medium-weight family: buttons, the selected rail item, group labels.
pub const MEDIUM: &str = "medium";

/// White at 6%, laid over a fill on hover.
#[must_use]
pub fn hover_wash() -> Color32 {
    Color32::from_white_alpha(15)
}

/// White at 12%, laid over a fill while pressed.
#[must_use]
pub fn press_wash() -> Color32 {
    Color32::from_white_alpha(31)
}

/// The accent at 16%: selected rows, focus and drag halos.
#[must_use]
pub fn accent_wash() -> Color32 {
    ACCENT.gamma_multiply(0.16)
}

/// Named text styles beyond egui's five.
#[derive(Clone, Copy)]
pub enum Text {
    CardHeading,
    Control,
    Hint,
}

impl Text {
    fn name(self) -> &'static str {
        match self {
            Self::CardHeading => "card-heading",
            Self::Control => "control",
            Self::Hint => "hint",
        }
    }

    #[must_use]
    pub fn style(self) -> TextStyle {
        TextStyle::Name(self.name().into())
    }
}

/// Applies Slate to every egui style (dark and light, though only dark is used).
pub fn apply(ctx: &egui::Context) {
    ctx.options_mut(|options| options.theme_preference = egui::ThemePreference::Dark);
    ctx.all_styles_mut(style_slate);
}

fn style_slate(style: &mut Style) {
    set_text_styles(style);
    set_spacing(style);
    style.visuals = visuals();
    style.animation_time = 0.12;
    style.interaction.selectable_labels = false;
}

fn set_text_styles(style: &mut Style) {
    let regular = |size| FontId::new(size, FontFamily::Proportional);
    let medium = |size| FontId::new(size, FontFamily::Name(MEDIUM.into()));
    style.text_styles = [
        (TextStyle::Heading, regular(24.0)),
        (TextStyle::Body, regular(13.0)),
        (TextStyle::Button, medium(13.0)),
        (
            TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        ),
        (TextStyle::Small, regular(12.0)),
        (Text::CardHeading.style(), regular(17.0)),
        (Text::Control.style(), regular(13.5)),
        (Text::Hint.style(), regular(12.0)),
    ]
    .into();
}

fn set_spacing(style: &mut Style) {
    let spacing = &mut style.spacing;
    spacing.item_spacing = egui::vec2(8.0, 8.0);
    spacing.button_padding = egui::vec2(16.0, 9.0);
    spacing.interact_size = egui::vec2(40.0, 34.0);
    spacing.menu_margin = Margin::symmetric(0, 6);
    spacing.window_margin = Margin::symmetric(18, 16);
    spacing.slider_width = 220.0;
    spacing.slider_rail_height = 4.0;
    spacing.icon_width = 20.0;
    spacing.icon_spacing = 12.0;
    spacing.combo_height = 280.0;
    // Descriptions read as short paragraphs, not one long line.
    spacing.tooltip_width = 300.0;
    spacing.scroll = egui::style::ScrollStyle::solid();
    spacing.scroll.bar_width = 10.0;
    spacing.scroll.bar_inner_margin = 2.0;
    spacing.scroll.bar_outer_margin = 0.0;
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    // `bg_fill` is what egui gives scrollbar thumbs (Slate: `line`); `weak_bg_fill` is buttons.
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
    v.window_stroke = Stroke::new(1.0, LINE_SOFT);
    v.window_shadow = popup_shadow();
    v.popup_shadow = popup_shadow();
    v.menu_corner_radius = CornerRadius::same(RADIUS_CONTROL);
    v.panel_fill = GROUND;
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

/// The only shadow in Slate: under floating popups.
fn popup_shadow() -> Shadow {
    Shadow {
        offset: [0, 3],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(115),
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
        for text in [Text::CardHeading, Text::Control, Text::Hint] {
            assert!(style.text_styles.contains_key(&text.style()));
        }
    }
}
