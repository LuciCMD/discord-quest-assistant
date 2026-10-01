//! Slate's own widgets, where egui's built-ins can't take the look: toggle, filled field, card,
//! primary button, the ⓘ with its description, choice rows and the progress bar.

use std::ops::RangeInclusive;

use eframe::egui::{
    self, Align2, Button, Color32, CornerRadius, CursorIcon, FontId, Frame, Margin, Rect, Response,
    RichText, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, Vec2, WidgetInfo, WidgetType, pos2,
    vec2,
};

use super::theme::{self, Text};

/// A 2 px accent ring just inside a widget that has keyboard focus.
fn focus_ring(ui: &Ui, response: &Response, radius: u8) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            radius,
            Stroke::new(2.0, theme::ACCENT),
            StrokeKind::Inside,
        );
    }
}

/// A hint: small, faint, never essential.
pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .text_style(Text::Hint.style())
            .color(theme::FAINT),
    );
}

/// A flat rounded card with a heading, an optional hint and its contents.
pub fn card<R>(
    ui: &mut Ui,
    heading: &str,
    card_hint: Option<&str>,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let inner = Frame::new()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0, theme::LINE_SOFT))
        .corner_radius(theme::RADIUS_CARD)
        .inner_margin(Margin::symmetric(18, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(heading)
                    .text_style(Text::CardHeading.style())
                    .color(theme::INK),
            );
            if let Some(text) = card_hint {
                hint(ui, text);
            }
            ui.add_space(8.0);
            add_contents(ui)
        })
        .inner;
    ui.add_space(12.0);
    inner
}

/// The one main action of a card: accent fill, dark text. At most one per card.
pub fn primary_button(ui: &mut Ui, label: &str) -> Response {
    let button = Button::new(RichText::new(label).color(theme::ON_ACCENT))
        .fill(theme::ACCENT)
        .stroke(Stroke::NONE)
        .corner_radius(theme::RADIUS_CONTROL);
    let response = ui.add(button);
    let wash = if response.is_pointer_button_down_on() {
        Some(theme::press_wash())
    } else if response.hovered() {
        Some(theme::hover_wash())
    } else {
        None
    };
    if let Some(color) = wash {
        ui.painter()
            .rect_filled(response.rect, theme::RADIUS_CONTROL, color);
    }
    focus_ring(ui, &response, theme::RADIUS_CONTROL);
    response
}

/// A toggle switch in place of a checkbox. Still a checkbox to screen readers, and Space flips it.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let font = ui
        .style()
        .text_styles
        .get(&Text::Control.style())
        .cloned()
        .unwrap_or_else(|| FontId::proportional(13.5));
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, theme::INK);
    let size = vec2(38.0 + 12.0 + galley.size().x, 26.0_f32.max(galley.size().y));
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    let state = *on;
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, state, label));
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let t = ui
        .ctx()
        .animate_bool_with_time(response.id, *on, ui.style().animation_time);
    let track = Rect::from_min_size(pos2(rect.left(), rect.center().y - 7.0), vec2(38.0, 14.0));
    let knob = pos2(
        egui::lerp(track.left() + 10.0..=track.right() - 10.0, t),
        track.center().y,
    );
    let painter = ui.painter();
    painter.rect_filled(
        track,
        7,
        theme::TOGGLE_TRACK_OFF.lerp_to_gamma(theme::ACCENT_DIM, t),
    );
    if response.has_focus() {
        painter.circle_filled(knob, 15.0, theme::accent_wash());
    } else if response.hovered() {
        painter.circle_filled(knob, 13.0, theme::hover_wash());
    }
    painter.circle_filled(
        knob,
        10.0,
        theme::TOGGLE_KNOB_OFF.lerp_to_gamma(theme::ACCENT, t),
    );
    painter.galley(
        pos2(
            track.right() + 12.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        theme::INK,
    );
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A number in a small filled field beside a slider, for exact values: click it to type one, or
/// drag it sideways to nudge it (Shift for finer steps). `step` is the change per point dragged.
pub fn number_field(
    ui: &mut Ui,
    value: &mut f64,
    range: RangeInclusive<f64>,
    step: f64,
    decimals: usize,
    unit: &str,
    width: f32,
) -> Response {
    ui.scope(|ui| {
        let style = ui.style_mut();
        style.spacing.interact_size = vec2(width, 26.0);
        style.spacing.button_padding = vec2(8.0, 2.0);
        style.drag_value_text_style = egui::TextStyle::Monospace;
        let widgets = &mut style.visuals.widgets;
        for (state, fill) in [
            (&mut widgets.inactive, theme::FIELD),
            (
                &mut widgets.hovered,
                theme::FIELD.blend(theme::hover_wash()),
            ),
            (&mut widgets.active, theme::FIELD.blend(theme::press_wash())),
        ] {
            state.weak_bg_fill = fill;
            state.bg_fill = fill;
            state.bg_stroke = Stroke::NONE;
            state.fg_stroke.color = theme::INK;
            state.corner_radius = CornerRadius::same(6);
            state.expansion = 0.0;
        }
        style.visuals.extreme_bg_color = theme::FIELD;
        style.visuals.selection.stroke = Stroke::new(1.0, theme::ACCENT);
        let suffix = if unit.is_empty() {
            String::new()
        } else {
            format!(" {unit}")
        };
        ui.add(
            egui::DragValue::new(value)
                .range(range)
                .speed(step)
                .min_decimals(decimals)
                .max_decimals(decimals)
                .suffix(suffix),
        )
        .on_hover_text("Click to type a value, or drag sideways. Hold Shift for finer steps.")
    })
    .inner
}

/// A small ⓘ that shows `text` as soon as the pointer is over it.
pub fn info(ui: &mut Ui, text: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
    let color = if response.hovered() {
        theme::ACCENT
    } else {
        theme::FAINT
    };
    let painter = ui.painter();
    painter.circle_stroke(rect.center(), 6.5, Stroke::new(1.3, color));
    painter.text(
        rect.center() + vec2(0.0, 0.5),
        Align2::CENTER_CENTER,
        "i",
        FontId::new(10.5, egui::FontFamily::Name(theme::MEDIUM.into())),
        color,
    );
    if let (true, Some(pointer)) = (response.hovered(), response.hover_pos()) {
        description(ui, response.id, pointer, text);
    }
    response
}

/// A field label with its ⓘ after it.
pub fn label_info(ui: &mut Ui, label: &str, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(
            RichText::new(label)
                .text_style(Text::Control.style())
                .color(theme::INK),
        );
        info(ui, text);
    });
}

/// Gap between the pointer and a description.
const DESCRIPTION_GAP: f32 = 18.0;

/// A description beside the pointer: fully visible at once, on the side with room for it, clear of
/// the cursor and inside the window. It never takes the pointer, so it can't cover what it
/// describes and make itself close.
fn description(ui: &Ui, id: egui::Id, pointer: egui::Pos2, text: &str) {
    let ctx = ui.ctx();
    let style = ui.style();
    let margin = Margin::symmetric(12, 9);
    let font = egui::TextStyle::Body.resolve(style);
    let wrap = style.spacing.tooltip_width - margin.sum().x;
    let galley = ui.painter().layout(text.to_owned(), font, theme::INK, wrap);
    let size = galley.size() + margin.sum() + Vec2::splat(2.0);
    let screen = ctx.content_rect().shrink(8.0);

    // Beside the pointer on whichever side has more room; above or below it if neither side fits.
    let room_right = screen.right() - pointer.x - DESCRIPTION_GAP;
    let room_left = pointer.x - screen.left() - DESCRIPTION_GAP;
    let mut min = if room_right >= size.x || room_left >= size.x {
        let x = if room_right >= room_left {
            pointer.x + DESCRIPTION_GAP
        } else {
            pointer.x - DESCRIPTION_GAP - size.x
        };
        pos2(x, pointer.y - size.y / 2.0)
    } else {
        let below = screen.bottom() - pointer.y - DESCRIPTION_GAP;
        let y = if below >= size.y {
            pointer.y + DESCRIPTION_GAP + 6.0
        } else {
            pointer.y - DESCRIPTION_GAP - size.y
        };
        pos2(pointer.x - size.x / 2.0, y)
    };
    min.x = min
        .x
        .clamp(screen.left(), (screen.right() - size.x).max(screen.left()));
    min.y = min
        .y
        .clamp(screen.top(), (screen.bottom() - size.y).max(screen.top()));

    egui::Area::new(id.with("description"))
        .order(egui::Order::Tooltip)
        .fixed_pos(min)
        .interactable(false)
        .fade_in(false)
        .constrain(false)
        .show(ctx, |ui| {
            Frame::popup(ui.style())
                .inner_margin(margin)
                .show(ui, |ui| {
                    ui.painter()
                        .galley(ui.cursor().min, galley.clone(), theme::INK);
                    ui.allocate_space(galley.size());
                });
        });
}

/// A filled text field: `field` fill, no outline, a 2 px accent underline while focused.
pub fn filled_field(ui: &mut Ui, text: &mut String, placeholder: &str, width: f32) -> Response {
    let background = ui.painter().add(Shape::Noop);
    let response = TextEdit::singleline(text)
        // With a frame given, egui takes the text margin from the frame.
        .frame(Frame::NONE.inner_margin(Margin::symmetric(12, 8)))
        .font(Text::Control.style())
        .text_color(theme::INK)
        .hint_text(RichText::new(placeholder).color(theme::FAINT))
        .desired_width(width)
        .show(ui)
        .response
        .response;
    let rect = response.rect;
    let fill = if response.hovered() && !response.has_focus() {
        theme::FIELD.blend(theme::hover_wash())
    } else {
        theme::FIELD
    };
    ui.painter().set(
        background,
        egui::epaint::RectShape::filled(rect, theme::RADIUS_CONTROL, fill),
    );
    if response.has_focus() {
        let y = rect.bottom() - 1.0;
        ui.painter().line_segment(
            [pos2(rect.left() + 6.0, y), pos2(rect.right() - 6.0, y)],
            Stroke::new(2.0, theme::ACCENT),
        );
    }
    response
}

/// A read-only value shown as monospace text in a sunken well, e.g. a folder path.
pub fn value_well(ui: &mut Ui, text: &str) {
    Frame::new()
        .fill(theme::ABYSS)
        .corner_radius(theme::RADIUS_PILL)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).monospace().color(theme::MUTED));
        });
}

/// A 10 px status dot.
pub fn status_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

/// One row of a short list to pick from. The picked row has the accent wash and accent text.
pub fn choice_row(ui: &mut Ui, selected: bool, text: &str, detail: &str) -> Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, text));
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, theme::RADIUS_PILL, theme::accent_wash());
    } else if response.hovered() {
        painter.rect_filled(rect, theme::RADIUS_PILL, theme::hover_wash());
    }
    focus_ring(ui, &response, theme::RADIUS_PILL);
    let color = if selected { theme::ACCENT } else { theme::INK };
    let font = FontId::new(13.5, egui::FontFamily::Proportional);
    let left = pos2(rect.left() + 12.0, rect.center().y);
    let name = painter.layout(text.to_owned(), font, color, width - 24.0);
    let name_width = name.size().x;
    painter.galley(pos2(left.x, left.y - name.size().y / 2.0), name, color);
    if !detail.is_empty() {
        let clip = Rect::from_min_max(pos2(left.x + name_width + 12.0, rect.top()), rect.max);
        ui.painter_at(clip).text(
            pos2(clip.left(), left.y),
            Align2::LEFT_CENTER,
            detail,
            FontId::new(12.0, egui::FontFamily::Monospace),
            theme::MUTED,
        );
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A 6 px bar, filled in the accent up to `fraction`.
pub fn progress_bar(ui: &mut Ui, fraction: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    let fraction = fraction.clamp(0.0, 1.0);
    let painter = ui.painter();
    painter.rect_filled(rect, 3, theme::SLIDER_TRACK);
    let filled = Rect::from_min_max(
        rect.min,
        pos2(egui::lerp(rect.x_range(), fraction), rect.max.y),
    );
    painter.rect_filled(filled, 3, theme::ACCENT);
}
