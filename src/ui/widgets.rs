//! The graphite theme's own widgets where egui's built-ins can't take the look

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

/// Height of a panel's title bar, above its seam
pub const PANEL_HEADER: f32 = 46.0;

/// A panel's title bar across the full width, with an optional readout on its right and the seam
/// under it
pub fn heading_bar(ui: &mut Ui, title: &str, readout: Option<(&str, Color32)>) {
    let (bar, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), PANEL_HEADER), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, title));
    let rect = bar;
    let painter = ui.painter_at(bar);
    let left = pos2(bar.left() + PANEL_PAD, bar.center().y);
    let mut room = bar.width() - 2.0 * PANEL_PAD;
    if let Some((text, color)) = readout {
        let galley = painter.layout_no_wrap(
            text.to_owned(),
            FontId::new(13.0, egui::FontFamily::Monospace),
            color,
        );
        room -= galley.size().x + 12.0;
        painter.galley(
            pos2(
                bar.right() - PANEL_PAD - galley.size().x,
                left.y - galley.size().y / 2.0,
            ),
            galley,
            color,
        );
    }
    let font = Text::PanelHeading.style().resolve(ui.style());
    let mut job = egui::text::LayoutJob::simple_singleline(title.to_owned(), font, theme::INK);
    job.wrap = egui::text::TextWrapping::truncate_at_width(room.max(1.0));
    let galley = painter.layout_job(job);
    painter.galley(
        pos2(left.x, left.y - galley.size().y / 2.0),
        galley,
        theme::INK,
    );
    ui.painter().hline(
        rect.x_range(),
        bar.bottom() - 0.5,
        Stroke::new(1.0, theme::LINE_SOFT),
    );
}

/// Panel content, inset from the seams
pub fn padded<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pad = PANEL_PAD as i8;
    Frame::new()
        .inner_margin(Margin {
            left: pad,
            right: pad,
            top: 14,
            bottom: 18,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui)
        })
        .inner
}

/// Inner padding of every panel
pub const PANEL_PAD: f32 = 18.0;

/// A heading inside a panel, with more room above it than below
pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(
        RichText::new(text)
            .text_style(Text::Section.style())
            .color(theme::MUTED),
    );
    ui.add_space(2.0);
}

/// Outline only, turning red on hover or focus, for actions that end or remove something
/// Callers keep it apart from other buttons so a slip can't reach it
pub fn danger_button(ui: &mut Ui, label: &str) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let measure = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), theme::MUTED);
    // Smaller than the actions it sits beside, so it never reads as the main one
    let size = vec2(
        measure.size().x + 20.0,
        28.0_f32.max(measure.size().y + 8.0),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    let hot = enabled && (response.hovered() || response.has_focus());
    let color = if hot { theme::BAD } else { theme::MUTED };
    let edge = if hot { theme::BAD } else { theme::LINE };
    let painter = ui.painter();
    if response.is_pointer_button_down_on() {
        painter.rect_filled(rect, theme::RADIUS_PILL, theme::BAD.gamma_multiply(0.14));
    }
    painter.rect_stroke(
        rect,
        theme::RADIUS_PILL,
        Stroke::new(1.0, edge),
        StrokeKind::Inside,
    );
    let galley = painter.layout_no_wrap(label.to_owned(), font, color);
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The panel's one main action, at most one per panel
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
    // Wraps rather than widening a narrow panel, leaving room for the ⓘ that follows
    let room = (ui.available_width() - 50.0 - INFO_ICON - ui.spacing().item_spacing.x).max(60.0);
    let galley = ui
        .painter()
        .layout(label.to_owned(), font, theme::INK, room);
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
        style.override_text_style = Some(egui::TextStyle::Monospace);
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
    // The box's own rounded left edge clipped to 2 px, as a 2 px wide rect can't hold its corners
    let bar = Rect::from_min_size(rect.min, vec2(2.0, rect.height()));
    let edge = Rect::from_min_size(rect.min, vec2(rect.width().min(24.0), rect.height()));
    ui.painter()
        .with_clip_rect(bar.intersect(ui.clip_rect()))
        .rect_filled(edge, theme::RADIUS_PILL, theme::ACCENT);
}

/// No outline, and a 2 px accent underline while focused
/// `width` is the whole field, margins included, and the field never grows past it
pub fn filled_field(ui: &mut Ui, text: &mut String, placeholder: &str, width: f32) -> Response {
    let background = ui.painter().add(Shape::Noop);
    let height = ui.spacing().interact_size.y;
    // Left alone, egui's text layout runs a pixel or two past the asked width and widens the
    // whole column, so the field gets a box exactly that wide
    let response = ui
        .allocate_ui_with_layout(
            vec2(width.max(1.0), height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                TextEdit::singleline(text)
                    // With a frame given, egui takes the text margin from the frame
                    .frame(Frame::NONE.inner_margin(Margin::symmetric(12, 8)))
                    .font(Text::Control.style())
                    .text_color(theme::INK)
                    .hint_text(RichText::new(placeholder).color(theme::FAINT))
                    .desired_width(width.max(1.0))
                    .show(ui)
                    .response
                    .response
            },
        )
        .inner;
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

pub const ICON_BUTTON: f32 = 28.0;

/// A square button showing only an icon, faint and accent on hover, its name in a tooltip above
/// `why_not` disables it and is added to the tooltip
pub fn icon_button(
    ui: &mut Ui,
    icon: &egui::TextureHandle,
    name: &str,
    why_not: Option<&str>,
) -> Response {
    let enabled = why_not.is_none();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(ICON_BUTTON), sense);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name));
    let lit = enabled && (response.hovered() || response.has_focus());
    if lit {
        ui.painter()
            .rect_filled(rect, theme::RADIUS_PILL, theme::hover_wash());
    }
    focus_ring(ui, &response, theme::RADIUS_PILL);
    let tint = match (enabled, lit) {
        (false, _) => theme::FAINT.gamma_multiply(0.45),
        (true, true) => theme::ACCENT,
        (true, false) => theme::FAINT,
    };
    egui::Image::new(icon)
        .tint(tint)
        .paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(16.0)));

    let mut tip = egui::Tooltip::for_widget(&response);
    tip.popup = tip
        .popup
        .open(egui::Tooltip::should_show_tooltip(&response, false))
        .align(egui::RectAlign::TOP)
        // The buttons sit at a row's right end, where a centred tooltip may not fit
        .align_alternatives(&[
            egui::RectAlign::TOP_END,
            egui::RectAlign::BOTTOM,
            egui::RectAlign::BOTTOM_END,
        ]);
    tip.show(|ui| {
        ui.label(RichText::new(name).color(theme::INK));
        if let Some(why) = why_not {
            ui.label(RichText::new(why).color(theme::MUTED));
        }
    });
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A name that picks its entry when clicked, accent while picked or hovered
/// Cut short with an ellipsis past `max_width`, so the row's buttons always fit
pub fn select_label(ui: &mut Ui, text: &str, selected: bool, max_width: f32) -> Response {
    let line = |ui: &Ui, color: Color32| {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(1.0));
        ui.painter().layout_job(job)
    };
    let size = line(ui, theme::INK).size();
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, text));
    let color = if selected || response.hovered() || response.has_focus() {
        theme::ACCENT
    } else {
        theme::INK
    };
    ui.painter().galley(rect.min, line(ui, color), color);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// One line of body text cut short with an ellipsis past `max_width`, its full text kept for
/// screen readers
pub fn truncated(ui: &mut Ui, text: &str, color: Color32, max_width: f32) -> Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(1.0));
    let galley = ui.painter().layout_job(job);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    ui.painter().galley(rect.min, galley, color);
    response
}

/// 10 px across
pub fn status_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

pub const ROW_HEIGHT: f32 = 36.0;

/// A row of a short list to pick from, the picked one in accent wash and accent text
/// Names wrap to two lines at most, `mono` for exe paths, and the detail shows only beside a
/// one-line name with room for a useful part of it
pub fn choice_row(ui: &mut Ui, selected: bool, text: &str, detail: &str, mono: bool) -> Response {
    let width = ui.available_width();
    let color = if selected { theme::ACCENT } else { theme::INK };
    let name = row_name(ui, text, width, mono, color);
    let height = (name.size().y + 16.0).max(ROW_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, text));
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, theme::RADIUS_PILL, theme::accent_wash());
    } else if response.hovered() {
        painter.rect_filled(rect, theme::RADIUS_PILL, theme::hover_wash());
    }
    focus_ring(ui, &response, theme::RADIUS_PILL);
    let left = pos2(rect.left() + 12.0, rect.center().y);
    let name_width = name.size().x;
    let one_line = name.rows.len() == 1;
    painter.galley(pos2(left.x, left.y - name.size().y / 2.0), name, color);
    let detail_left = left.x + name_width + 12.0;
    let detail_room = rect.right() - 10.0 - detail_left;
    // Under about 12 characters a path says nothing, so it's left out
    if !detail.is_empty() && one_line && detail_room >= DETAIL_MIN {
        let font = FontId::new(12.0, egui::FontFamily::Monospace);
        let mut job =
            egui::text::LayoutJob::simple_singleline(detail.to_owned(), font, theme::MUTED);
        job.wrap = egui::text::TextWrapping::truncate_at_width(detail_room);
        let galley = painter.layout_job(job);
        painter.galley(
            pos2(detail_left, left.y - galley.size().y / 2.0),
            galley,
            theme::MUTED,
        );
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The name as a row lays it out, two lines at most
fn row_name(
    ui: &Ui,
    text: &str,
    width: f32,
    mono: bool,
    color: Color32,
) -> std::sync::Arc<egui::Galley> {
    let font = if mono {
        FontId::new(13.0, egui::FontFamily::Monospace)
    } else {
        egui::TextStyle::Body.resolve(ui.style())
    };
    let room = (width - 24.0).max(1.0);
    // An exe path breaks between folders, so no file or folder name is split across lines
    let text = if mono {
        folder_lines(ui, text, &font, room)
    } else {
        text.to_owned()
    };
    let mut job = egui::text::LayoutJob::simple(text, font, color, room);
    job.wrap.max_rows = 2;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

/// How tall a `choice_row` with this text will be at this width, for a box it must fit
pub fn choice_row_height(ui: &Ui, text: &str, width: f32, mono: bool) -> f32 {
    (row_name(ui, text, width, mono, theme::INK).size().y + 16.0).max(ROW_HEIGHT)
}

/// Room a row's detail needs before it's worth showing
const DETAIL_MIN: f32 = 90.0;

/// A path in small mono, broken only between folders so no name is split
pub fn path_label(ui: &mut Ui, path: &str, color: Color32) {
    let font = FontId::new(12.0, egui::FontFamily::Monospace);
    let room = ui.available_width().max(40.0);
    let text = folder_lines(ui, path, &font, room);
    ui.add(
        egui::Label::new(RichText::new(text).font(font).color(color))
            // Lines are already broken between folders, and Truncate would keep only the first
            .wrap_mode(egui::TextWrapMode::Extend),
    );
}

/// `path` with a line break wherever the next folder wouldn't fit in `room`, never inside a name
fn folder_lines(ui: &Ui, path: &str, font: &FontId, room: f32) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for part in path.split_inclusive(['\\', '/']) {
        let candidate = format!("{line}{part}");
        let wide = ui
            .painter()
            .layout_no_wrap(candidate.clone(), font.clone(), theme::INK)
            .size()
            .x
            > room;
        if wide && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            part.clone_into(&mut line);
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines.join("\n")
}

/// 4 px tall, filled in the accent up to `fraction`
pub fn progress_bar(ui: &mut Ui, fraction: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    let fraction = fraction.clamp(0.0, 1.0);
    let painter = ui.painter();
    painter.rect_filled(rect, 2, theme::SLIDER_TRACK);
    let filled = Rect::from_min_max(
        rect.min,
        pos2(egui::lerp(rect.x_range(), fraction), rect.max.y),
    );
    painter.rect_filled(filled, 2, theme::ACCENT);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepState {
    Done,
    /// Ended before its time, stopped or closed, so the quest may not have counted
    Stopped,
    Live,
    Queued,
}

impl StepState {
    fn color(self) -> Color32 {
        match self {
            Self::Done => theme::GOOD,
            Self::Stopped => theme::WARN,
            Self::Live => theme::ACCENT,
            Self::Queued => theme::LINE,
        }
    }
}

/// One game on the pipeline strip
pub struct Step {
    pub state: StepState,
    pub name: String,
    /// Time played, time left or minutes queued
    pub detail: String,
}

/// Narrowest a step gets before the strip wraps onto another row
const STEP_MIN_WIDTH: f32 = 132.0;
const STEP_MAX_WIDTH: f32 = 190.0;
pub const STEP_HEIGHT: f32 = 68.0;
const STEP_RAIL: f32 = 14.0;
/// Room between rows for the rail to turn through
const STEP_ROW_GAP: f32 = 12.0;

/// Tonight as a strip of steps joined by a rail, wrapping onto more rows as it grows, with the
/// rail turning from each row's end to the next row's start so the order still reads
pub fn pipeline(ui: &mut Ui, steps: &[Step]) {
    let room = ui.available_width();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let per_row = (((room + STEP_RAIL) / (STEP_MIN_WIDTH + STEP_RAIL)).floor() as usize).max(1);
    #[allow(clippy::cast_precision_loss)]
    let width =
        ((room - STEP_RAIL * (per_row - 1) as f32) / per_row as f32).clamp(1.0, STEP_MAX_WIDTH);
    let mut placed: Vec<(Rect, StepState)> = Vec::with_capacity(steps.len());
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = STEP_ROW_GAP;
        for row in steps.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (i, step) in row.iter().enumerate() {
                    if i > 0 {
                        let (rail, _) =
                            ui.allocate_exact_size(vec2(STEP_RAIL, STEP_HEIGHT), Sense::hover());
                        ui.painter().hline(
                            rail.x_range(),
                            rail.center().y,
                            Stroke::new(1.0, step.state.color()),
                        );
                    }
                    let rect = step_box(ui, step, width).rect;
                    placed.push((rect, step.state));
                }
            });
        }
    });
    let painter = ui.painter();
    for pair in placed.windows(2) {
        let [(from, _), (to, state)] = pair else {
            continue;
        };
        // Steps on one row are joined above, this only turns the rail between rows
        if to.top() < from.bottom() {
            continue;
        }
        let out = from.right() + STEP_RAIL / 2.0;
        let back = to.left() - STEP_RAIL / 2.0;
        let middle = f32::midpoint(from.bottom(), to.top());
        painter.line(
            vec![
                from.right_center(),
                pos2(out, from.center().y),
                pos2(out, middle),
                pos2(back, middle),
                pos2(back, to.center().y),
                to.left_center(),
            ],
            Stroke::new(1.0, state.color()),
        );
    }
}

fn step_box(ui: &mut Ui, step: &Step, width: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, STEP_HEIGHT), Sense::hover());
    let label = format!("{}, {}", step.name, step.detail);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &label));
    let edge = step.state.color();
    let (name_color, detail_color) = match step.state {
        StepState::Done | StepState::Stopped => (theme::INK, theme::MUTED),
        StepState::Live => (theme::INK, theme::ACCENT),
        StepState::Queued => (theme::MUTED, theme::FAINT),
    };
    let painter = ui.painter();
    if step.state == StepState::Live {
        painter.rect_filled(rect, theme::RADIUS_CONTROL, theme::accent_wash());
    }
    painter.rect_stroke(
        rect,
        theme::RADIUS_CONTROL,
        Stroke::new(1.0, edge),
        StrokeKind::Inside,
    );
    let mark = pos2(rect.left() + 17.0, rect.top() + 17.0);
    painter.circle_stroke(mark, 6.0, Stroke::new(1.2, edge));
    match step.state {
        StepState::Done => {
            let tick = [
                mark + vec2(-3.0, 0.2),
                mark + vec2(-0.8, 2.5),
                mark + vec2(3.2, -2.4),
            ];
            painter.line(tick.to_vec(), Stroke::new(1.4, edge));
        }
        StepState::Stopped => {
            painter.line_segment(
                [mark + vec2(0.0, -3.2), mark + vec2(0.0, 0.8)],
                Stroke::new(1.4, edge),
            );
            painter.circle_filled(mark + vec2(0.0, 2.9), 0.9, edge);
        }
        StepState::Live => {
            painter.circle_filled(mark, 3.0, edge);
        }
        StepState::Queued => {}
    }
    let text_left = rect.left() + 30.0;
    let room = (rect.right() - 10.0 - text_left).max(1.0);
    // The name wraps to two lines before it's cut, so similar names stay apart
    let mut job = egui::text::LayoutJob::simple(
        step.name.clone(),
        FontId::new(13.0, egui::FontFamily::Name(theme::MEDIUM.into())),
        name_color,
        room,
    );
    job.wrap.max_rows = 2;
    job.wrap.overflow_character = Some('…');
    let name = painter.layout_job(job);
    let name_bottom = rect.top() + 9.0 + name.size().y;
    painter.galley(pos2(text_left, rect.top() + 9.0), name, name_color);
    let mut job = egui::text::LayoutJob::simple_singleline(
        step.detail.clone(),
        FontId::new(12.0, egui::FontFamily::Monospace),
        detail_color,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    let detail = painter.layout_job(job);
    // Just under the name with clear space, whether the name took one line or two
    let detail_top = name_bottom + 6.0;
    painter.galley(pos2(text_left, detail_top), detail, detail_color);
    response.on_hover_ui(|ui| {
        ui.label(RichText::new(&step.name).color(theme::INK));
        ui.label(RichText::new(&step.detail).monospace().color(theme::MUTED));
    })
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

#[cfg(test)]
mod icon_buttons {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};

    fn test_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        super::super::fonts::install(&ctx);
        theme::apply(&ctx);
        // Real windows wait half a second, a test would only be slower for it
        ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
        ctx
    }

    fn texture(ctx: &egui::Context) -> egui::TextureHandle {
        let image = egui::ColorImage::filled([4, 4], Color32::WHITE);
        ctx.load_texture("test-icon", image, egui::TextureOptions::LINEAR)
    }

    /// The button sits 120 px down, so a tooltip has room above it
    fn frame(
        ctx: &egui::Context,
        icon: &egui::TextureHandle,
        why_not: Option<&str>,
        events: Vec<Event>,
        time: f64,
    ) -> Response {
        let input = RawInput {
            events,
            time: Some(time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0))),
            ..Default::default()
        };
        let mut out = None;
        ctx.run_ui(input, |ui| {
            ui.add_space(120.0);
            ui.horizontal(|ui| {
                ui.add_space(120.0);
                out = Some(icon_button(ui, icon, "Add to Bottom of Queue", why_not));
            });
        })
        .textures_delta
        .clear();
        out.unwrap()
    }

    fn press(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
        }
    }

    fn hover_tooltip(ctx: &egui::Context, why_not: Option<&str>) -> (Rect, Option<Rect>) {
        let icon = texture(ctx);
        let first = frame(ctx, &icon, why_not, vec![], 0.0);
        let at = first.rect.center();
        let mut response = first;
        for step in 1..6 {
            let events = if step == 1 {
                vec![Event::PointerMoved(at)]
            } else {
                vec![]
            };
            response = frame(ctx, &icon, why_not, events, f64::from(step) * 0.2);
        }
        let tip = ctx.memory(|m| m.area_rect(egui::Tooltip::tooltip_id(response.id, 0)));
        (response.rect, tip)
    }

    #[test]
    fn hovering_shows_the_name_above_the_button() {
        let (button, tip) = hover_tooltip(&test_ctx(), None);
        let tip = tip.expect("no tooltip while hovering");
        assert!(
            tip.bottom() <= button.top() + 0.5,
            "tooltip {tip:?} isn't above the button {button:?}"
        );
    }

    #[test]
    fn a_disabled_button_still_explains_itself_above() {
        let (button, tip) = hover_tooltip(&test_ctx(), Some("Already in the queue."));
        let tip = tip.expect("a disabled button showed no tooltip");
        assert!(tip.bottom() <= button.top() + 0.5);
    }

    #[test]
    fn a_disabled_button_ignores_clicks() {
        let ctx = test_ctx();
        let icon = texture(&ctx);
        let why = Some("Already running.");
        let at = frame(&ctx, &icon, why, vec![], 0.0).rect.center();
        frame(
            &ctx,
            &icon,
            why,
            vec![Event::PointerMoved(at), press(at, true)],
            0.1,
        );
        let response = frame(&ctx, &icon, why, vec![press(at, false)], 0.2);
        assert!(!response.clicked());

        let at = frame(&ctx, &icon, None, vec![], 0.3).rect.center();
        frame(
            &ctx,
            &icon,
            None,
            vec![Event::PointerMoved(at), press(at, true)],
            0.4,
        );
        let response = frame(&ctx, &icon, None, vec![press(at, false)], 0.5);
        assert!(response.clicked(), "the enabled button didn't click");
    }
}

#[cfg(test)]
mod paths {
    use super::*;

    #[test]
    fn a_path_breaks_only_between_folders() {
        let ctx = egui::Context::default();
        super::super::fonts::install(&ctx);
        let font = FontId::new(13.0, egui::FontFamily::Monospace);
        let path = "redbin/win64/red-win64-shipping.exe";
        let mut text = String::new();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            text = folder_lines(ui, path, &font, 140.0);
        })
        .textures_delta
        .clear();
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines.len() > 1,
            "a 140 px room should need two lines: {lines:?}"
        );
        assert_eq!(lines.concat(), path, "the path changed");
        for line in &lines[..lines.len() - 1] {
            assert!(line.ends_with('/'), "{line:?} breaks inside a name");
        }
        assert_eq!(lines.last(), Some(&"red-win64-shipping.exe"));
    }
}
