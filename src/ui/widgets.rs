//! Slate's own widgets where egui's built-ins can't take the look

use std::ops::RangeInclusive;

use eframe::egui::{
    self, Align2, Button, Color32, CornerRadius, CursorIcon, FontId, Frame, Margin, Rect, Response,
    RichText, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, Vec2, WidgetInfo, WidgetType, pos2,
    vec2,
};

use super::theme::{self, Text};

/// 2 px accent ring just inside a widget with keyboard focus
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

/// Small and faint, so never for anything essential
pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .text_style(Text::Hint.style())
            .color(theme::FAINT),
    );
}

/// Flat rounded card with a heading and an optional hint
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

/// The card's one main action, at most one per card
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

/// Still a checkbox to screen readers, and Space flips it
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

/// Click to type a value or drag sideways to nudge it, with Shift for finer steps
/// `step` is the change per point dragged
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
    })
    .inner
}

/// Widest an open description gets, so it stays readable
const INFO_BOX_MAX_WIDTH: f32 = 600.0;
pub const INFO_ICON: f32 = 16.0;

/// Toggles its description open or shut on click, and returns whether it's open
/// `key` must be unique app-wide, since the state is kept under it between frames
pub fn info(ui: &mut Ui, key: &str) -> bool {
    let id = egui::Id::new(("info", key));
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(INFO_ICON), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if response.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, open, "More info"));
    let color = if open || response.hovered() || response.has_focus() {
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
    if response.has_focus() {
        painter.circle_filled(rect.center(), 11.0, theme::accent_wash());
    }
    response.on_hover_cursor(CursorIcon::PointingHand);
    open
}

/// A field label with its ⓘ after it, returning whether the description is open
pub fn label_info(ui: &mut Ui, label: &str) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(
            RichText::new(label)
                .text_style(Text::Control.style())
                .color(theme::INK),
        );
        info(ui, label)
    })
    .inner
}

/// The description under an entry's row, a quiet raised panel with an accent bar on its left
pub fn info_box(ui: &mut Ui, open: bool, text: &str) {
    if !open {
        return;
    }
    let width = ui.available_width().min(INFO_BOX_MAX_WIDTH);
    let margin = Margin {
        left: 14,
        right: 12,
        top: 8,
        bottom: 8,
    };
    let frame = Frame::new()
        .fill(theme::RAISED)
        .corner_radius(theme::RADIUS_PILL)
        .inner_margin(margin)
        .show(ui, |ui| {
            // Clamped, since egui asserts on a negative width in a column narrower than the margins
            ui.set_width((width - margin.sum().x).max(0.0));
            ui.add(egui::Label::new(RichText::new(text).color(theme::MUTED)).wrap());
        });
    let rect = frame.response.rect;
    // The box's own rounded left edge clipped to 3 px, as a 3 px wide rect can't hold 6 px corners
    let bar = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
    let edge = Rect::from_min_size(rect.min, vec2(rect.width().min(24.0), rect.height()));
    ui.painter()
        .with_clip_rect(bar.intersect(ui.clip_rect()))
        .rect_filled(edge, theme::RADIUS_PILL, theme::ACCENT);
}

/// No outline, and a 2 px accent underline while focused
pub fn filled_field(ui: &mut Ui, text: &mut String, placeholder: &str, width: f32) -> Response {
    let background = ui.painter().add(Shape::Noop);
    let response = TextEdit::singleline(text)
        // With a frame given, egui takes the text margin from the frame
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

/// Read-only monospace text in a sunken well, such as a folder path
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

/// 10 px across
pub fn status_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

/// A row of a short list to pick from, the picked one in accent wash and accent text
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

/// 6 px tall, filled in the accent up to `fraction`
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};

    /// With the app's fonts, since the ⓘ draws in the medium family
    fn test_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        super::super::fonts::install(&ctx);
        theme::apply(&ctx);
        ctx
    }

    fn run(ctx: &egui::Context, events: Vec<Event>, width: f32, ui_fn: impl FnMut(&mut Ui)) {
        let input = RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 400.0))),
            ..Default::default()
        };
        // No renderer here, so the font atlas updates are dropped on purpose
        ctx.run_ui(input, ui_fn).textures_delta.clear();
    }

    fn info_frame(ctx: &egui::Context, events: Vec<Event>) -> bool {
        let mut open = false;
        run(ctx, events, 400.0, |ui| open = info(ui, "test"));
        open
    }

    fn button(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn info_opens_on_click_not_hover_and_stays_until_clicked_again() {
        let ctx = test_ctx();
        assert!(!info_frame(&ctx, vec![]));
        let icon = ctx
            .read_response(egui::Id::new(("info", "test")))
            .unwrap()
            .rect
            .center();

        assert!(
            !info_frame(&ctx, vec![Event::PointerMoved(icon)]),
            "hover alone opened it"
        );
        info_frame(&ctx, vec![button(icon, true)]);
        assert!(
            info_frame(&ctx, vec![button(icon, false)]),
            "a click didn't open it"
        );
        assert!(info_frame(&ctx, vec![]), "it closed by itself");

        info_frame(&ctx, vec![button(icon, true)]);
        assert!(
            !info_frame(&ctx, vec![button(icon, false)]),
            "a second click didn't close it"
        );
    }

    #[test]
    fn info_box_takes_no_room_shut_and_at_most_600_px_open() {
        let ctx = test_ctx();
        let mut sizes = Vec::new();
        for open in [false, true] {
            run(&ctx, vec![], 1200.0, |ui| {
                let before = ui.cursor().min;
                info_box(
                    ui,
                    open,
                    "A description long enough to need wrapping at some width.",
                );
                sizes.push((ui.min_rect().width(), ui.cursor().min.y - before.y));
            });
        }
        assert_eq!(sizes[0].1, 0.0, "a shut box took room");
        assert!(sizes[1].1 > 0.0, "an open box took no room");
        assert!(
            sizes[1].0 <= INFO_BOX_MAX_WIDTH + 0.5,
            "box is {} wide",
            sizes[1].0
        );
    }
}

#[cfg(test)]
mod info_edge_cases {
    use super::*;
    use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput};

    fn test_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        super::super::fonts::install(&ctx);
        theme::apply(&ctx);
        ctx
    }

    fn input(events: Vec<Event>, width: f32) -> RawInput {
        RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 600.0))),
            ..Default::default()
        }
    }

    fn run(ctx: &egui::Context, events: Vec<Event>, width: f32, ui_fn: impl FnMut(&mut Ui)) {
        ctx.run_ui(input(events, width), ui_fn)
            .textures_delta
            .clear();
    }

    fn button(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
        }
    }

    fn key(key: Key, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::default(),
        }
    }

    fn icon_id(key: &str) -> egui::Id {
        egui::Id::new(("info", key))
    }

    fn info_frame(ctx: &egui::Context, events: Vec<Event>, key: &str) -> bool {
        let mut open = false;
        run(ctx, events, 400.0, |ui| open = info(ui, key));
        open
    }

    fn icon_center(ctx: &egui::Context, key: &str) -> Pos2 {
        ctx.read_response(icon_id(key)).unwrap().rect.center()
    }

    #[test]
    fn space_and_enter_on_a_focused_icon_toggle_it() {
        let ctx = test_ctx();
        info_frame(&ctx, vec![], "kb");
        ctx.memory_mut(|m| m.request_focus(icon_id("kb")));
        info_frame(&ctx, vec![], "kb");
        assert!(
            ctx.memory(|m| m.has_focus(icon_id("kb"))),
            "icon can't take focus"
        );

        info_frame(&ctx, vec![key(Key::Space, true)], "kb");
        let open = info_frame(&ctx, vec![key(Key::Space, false)], "kb");
        assert!(open, "Space didn't open it");

        info_frame(&ctx, vec![key(Key::Enter, true)], "kb");
        let open = info_frame(&ctx, vec![key(Key::Enter, false)], "kb");
        assert!(!open, "Enter didn't close it");
    }

    #[test]
    fn two_icons_keep_separate_state() {
        let ctx = test_ctx();
        let pair = |ctx: &egui::Context, events: Vec<Event>| {
            let mut open = (false, false);
            run(ctx, events, 400.0, |ui| {
                open.0 = info(ui, "A");
                open.1 = info(ui, "B");
            });
            open
        };
        pair(&ctx, vec![]);
        let a = icon_center(&ctx, "A");
        let b = icon_center(&ctx, "B");
        assert_ne!(a, b);
        pair(&ctx, vec![button(a, true)]);
        assert_eq!(pair(&ctx, vec![button(a, false)]), (true, false));
        pair(&ctx, vec![button(b, true)]);
        assert_eq!(
            pair(&ctx, vec![button(b, false)]),
            (true, true),
            "several can be open"
        );
        pair(&ctx, vec![button(a, true)]);
        assert_eq!(pair(&ctx, vec![button(a, false)]), (false, true));
    }

    #[test]
    fn hovering_the_icon_shows_a_hand_cursor() {
        let ctx = test_ctx();
        info_frame(&ctx, vec![], "hand");
        let at = icon_center(&ctx, "hand");
        info_frame(&ctx, vec![Event::PointerMoved(at)], "hand");
        let mut out = ctx.run_ui(input(vec![], 400.0), |ui| {
            info(ui, "hand");
        });
        out.textures_delta.clear();
        assert_eq!(out.platform_output.cursor_icon, CursorIcon::PointingHand);
    }

    #[test]
    fn double_click_across_frames_ends_where_it_started() {
        let ctx = test_ctx();
        info_frame(&ctx, vec![], "dbl");
        let at = icon_center(&ctx, "dbl");
        info_frame(&ctx, vec![button(at, true)], "dbl");
        info_frame(&ctx, vec![button(at, false)], "dbl");
        info_frame(&ctx, vec![button(at, true)], "dbl");
        let open = info_frame(&ctx, vec![button(at, false)], "dbl");
        assert!(!open, "two clicks should toggle twice");
    }

    #[test]
    fn two_clicks_in_one_frame_toggle_at_most_once_without_panicking() {
        let ctx = test_ctx();
        info_frame(&ctx, vec![], "burst");
        let at = icon_center(&ctx, "burst");
        let open = info_frame(
            &ctx,
            vec![
                button(at, true),
                button(at, false),
                button(at, true),
                button(at, false),
            ],
            "burst",
        );
        // egui reports one click per frame, so this documents what a burst does
        let settled = info_frame(&ctx, vec![], "burst");
        assert_eq!(open, settled, "state changed with no input");
    }

    #[test]
    fn a_press_dragged_off_the_icon_does_not_toggle() {
        let ctx = test_ctx();
        info_frame(&ctx, vec![], "drag");
        let at = icon_center(&ctx, "drag");
        info_frame(&ctx, vec![button(at, true)], "drag");
        let away = at + vec2(120.0, 80.0);
        info_frame(&ctx, vec![Event::PointerMoved(away)], "drag");
        let open = info_frame(&ctx, vec![button(away, false)], "drag");
        assert!(!open, "a press released elsewhere counted as a click");
    }

    #[test]
    fn info_box_in_a_narrow_ui_does_not_panic() {
        let ctx = test_ctx();
        run(&ctx, vec![], 400.0, |ui| {
            ui.allocate_ui(vec2(12.0, 200.0), |ui| {
                info_box(ui, true, "Short text");
            });
        });
    }

    #[test]
    fn a_long_unbroken_description_stays_within_600_px() {
        let ctx = test_ctx();
        let text =
            "C:\\Program Files (x86)\\Steam\\steamapps\\common\\".repeat(30) + &"é".repeat(400);
        let mut width = 0.0;
        let mut height = 0.0;
        run(&ctx, vec![], 1400.0, |ui| {
            info_box(ui, true, &text);
            width = ui.min_rect().width();
            height = ui.min_rect().height();
        });
        assert!(width <= INFO_BOX_MAX_WIDTH + 0.5, "box is {width} wide");
        assert!(height > 40.0, "long text didn't wrap onto several lines");
    }

    #[test]
    fn box_fills_a_narrow_column_without_overflowing_it() {
        let ctx = test_ctx();
        let mut width = 0.0;
        run(&ctx, vec![], 1400.0, |ui| {
            ui.allocate_ui(vec2(300.0, 400.0), |ui| {
                info_box(
                    ui,
                    true,
                    "A description long enough to need wrapping at this width.",
                );
                width = ui.min_rect().width();
            });
        });
        assert!(
            (width - 300.0).abs() < 0.5,
            "box is {width} wide in a 300 px column"
        );
    }

    #[test]
    fn accent_bar_stays_inside_the_rounded_box() {
        let ctx = test_ctx();
        let mut boxed = Rect::NOTHING;
        let mut out = ctx.run_ui(input(vec![], 800.0), |ui| {
            info_box(ui, true, "Some description");
            boxed = ui.min_rect();
        });
        out.textures_delta.clear();
        let radius = f32::from(theme::RADIUS_PILL);
        let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
        let mut checked = 0;
        let mut worst = (0.0_f32, Pos2::ZERO);
        for primitive in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                continue;
            };
            for v in mesh.vertices.iter().filter(|v| v.color == theme::ACCENT) {
                checked += 1;
                for corner_y in [boxed.top() + radius, boxed.bottom() - radius] {
                    let center = pos2(boxed.left() + radius, corner_y);
                    let in_corner = v.pos.x < center.x
                        && ((corner_y < boxed.center().y && v.pos.y < center.y)
                            || (corner_y > boxed.center().y && v.pos.y > center.y));
                    let d = v.pos.distance(center);
                    if in_corner && d > worst.0 {
                        worst = (d, v.pos);
                    }
                }
            }
        }
        assert!(checked > 0, "no accent bar drawn");
        // Solid vertices sit 0.5 px inside the true edge, so the box's own are at radius - 0.5
        assert!(
            worst.0 <= radius,
            "bar pokes out of the box's {radius} px rounded corner at {:?} ({:?}), {:.2} px from the arc centre",
            worst.1,
            boxed,
            worst.0
        );
    }
}
