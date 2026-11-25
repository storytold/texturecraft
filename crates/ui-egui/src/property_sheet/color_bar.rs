//! A uniform colour's bar with its eyedropper, and the colour picker window it opens beside the panel

use egui::{
    Align2, CornerRadius, CursorIcon, FontId, Id, Order, Pos2, Rect, Response, Role, Sense, Shadow, Stroke, StrokeKind, UiBuilder, Vec2, WidgetInfo, pos2, vec2,
};
use egui_phosphor::regular as ic;

use super::color_picker_rows::{FIELD_HEIGHT, FIELD_MIN_HEIGHT, hex_took_escape};
use super::{PropertySheet, SIDE_PADDING_POINTS};
use crate::color_picker::{ColorModel, ColorPick, ColorSpace, PickerState};
use crate::fill_material_panel::shown_color;
use crate::i18n::tr;
use crate::theme::HARD_SHADOW;

/// Height of a colour bar
const COLOR_BAR_HEIGHT: f32 = 16.0;

/// Room the bar leaves at the right edge for its eyedropper, and from the right edge to the eyedropper's centre
const EYEDROPPER_ROOM: f32 = 26.0;
const EYEDROPPER_INSET: f32 = 9.0;

/// Size of the eyedropper's click target
const EYEDROPPER_TARGET: f32 = 18.0;

/// Air between the panel and the picker standing beside it
const WINDOW_GAP: f32 = 6.0;

/// Height of the picker's title bar and size of its title
const TITLE_HEIGHT: f32 = 26.0;
const TITLE_TEXT_SIZE: f32 = 11.0;

/// Half the close glyph's ink width: its centre sits this far in from the right padding, so the ink ends where the content does
const CLOSE_INK_HALF_WIDTH: f32 = 4.0;

/// Size of the close glyph and of its click target
const CLOSE_GLYPH_SIZE: f32 = 12.0;
const CLOSE_TARGET: f32 = 18.0;

/// From the title bar to the picker's first row, so that row's control sits 10 below it like a card's under its top
const PICKER_TOP: f32 = 8.0;

/// Air under the picker's last line, as under a card's last control
const PICKER_BOTTOM: f32 = 10.0;

/// The picker's height before it was first measured, so it stays on screen beside a bar near the window's bottom
const PICKER_HEIGHT_GUESS: f32 = 340.0;

/// Corner radius of a colour bar
const BAR_RADIUS: u8 = 3;

/// Corner radius of the picker
const PICKER_RADIUS: u8 = 7;

/// The picker's shadow: offset down-right, no blur
const PICKER_SHADOW: Shadow = Shadow { offset: [2, 4], blur: 0, spread: 0, color: HARD_SHADOW };

/// Where the space and model last picked are kept, so the next picker opens with them
const PREFERENCES_KEY: &str = "color_picker_preferences";

/// Where the size a resize left (width, field height) is kept, so every picker opens with it, in this run and the next
const SIZE_KEY: &str = "color_picker_size";

/// Narrowest the picker resizes to: its dropdowns, hex field and slider tracks keep their room
const PICKER_MIN_WIDTH: f32 = 240.0;

/// Depth of the grip inside each edge (in the rows' side padding and the air under the last line), and side of a corner's grip (small enough to leave × its click)
const RESIZE_GRIP: f32 = 5.0;
const RESIZE_CORNER: f32 = 8.0;

type VisiblePickers = egui::cache::FramePublisher<Id, PickerState>;

/// The picker state stored at `state_id`, else a closed picker
fn picker_state(ctx: &egui::Context, state_id: Id) -> PickerState {
    ctx.memory_mut(|memory| memory.caches.cache::<VisiblePickers>().get(&state_id).copied()).unwrap_or_default()
}

/// Opens `state` on `linear` in the colour space and model the user last picked in any picker
fn open_picker(ctx: &egui::Context, state: &mut PickerState, linear: [f32; 3]) {
    let (space, model) = ctx.data(|data| data.get_temp::<(ColorSpace, ColorModel)>(Id::unique(PREFERENCES_KEY)).unwrap_or_default());
    state.space = space;
    state.model = model;
    state.open_on(linear);
}

impl PropertySheet<'_> {
    /// Opens the picker of the colour bar `bar` (its [`ColorPick::id`]) on `linear` from the next frame, as a click on the bar does
    pub(crate) fn open_color_picker(&self, bar: Id, linear: [f32; 3]) {
        let ctx = self.ui.ctx();
        let state_id = bar.with("picker");
        let mut state = picker_state(ctx, state_id);
        if !state.open {
            open_picker(ctx, &mut state, linear);
            ctx.memory_mut(|memory| memory.caches.cache::<VisiblePickers>().set(state_id, state));
        }
    }

    /// A uniform colour (linear): a bar across the card that opens the picker (with the project's `swatches`), and the eyedropper; `eyedropping` is the bar whose pick waits for its click
    pub(crate) fn color_bar(&mut self, label: &str, linear: [f32; 3], eyedropping: Option<Id>, swatches: &[[f32; 3]]) -> ColorPick {
        let center_y = self.take(super::ROW_HEIGHT).center().y;
        let bar = Rect::from_min_max(pos2(self.left, center_y - COLOR_BAR_HEIGHT / 2.0), pos2(self.right - EYEDROPPER_ROOM, center_y + COLOR_BAR_HEIGHT / 2.0));
        self.color_bar_at(bar, self.right - EYEDROPPER_INSET, label, linear, eyedropping, swatches)
    }

    /// [`Self::color_bar`] in the controls column, beside `label`
    pub(crate) fn color_bar_led(&mut self, label: &str, linear: [f32; 3], eyedropping: Option<Id>, swatches: &[[f32; 3]]) -> ColorPick {
        let (control, center_y) = self.row(label);
        let bar = Rect::from_min_max(
            pos2(control.left(), center_y - COLOR_BAR_HEIGHT / 2.0),
            pos2(control.right() - EYEDROPPER_ROOM, center_y + COLOR_BAR_HEIGHT / 2.0),
        );
        self.color_bar_at(bar, self.right - EYEDROPPER_INSET, label, linear, eyedropping, swatches)
    }

    /// A colour bar drawn in `bar`, its eyedropper centred at `eyedropper_x`
    pub(super) fn color_bar_at(
        &mut self,
        bar: Rect,
        eyedropper_x: f32,
        label: &str,
        linear: [f32; 3],
        eyedropping: Option<Id>,
        swatches: &[[f32; 3]],
    ) -> ColorPick {
        let id = self.named_id("color_bar", label);
        let response = self.ui.interact(bar, id, Sense::click());
        response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
        let tokens = self.tokens;
        let border = if response.hovered() { tokens.accent } else { tokens.field_border };
        self.ui.painter().rect(bar, CornerRadius::same(BAR_RADIUS), shown_color(linear), Stroke::new(1.0, border), StrokeKind::Inside);

        let waiting = eyedropping == Some(id);
        if waiting {
            crate::screen_pick::SwatchColor::of_bar(self.ui.ctx(), linear);
        }
        let glyph = Rect::from_center_size(pos2(eyedropper_x, bar.center().y), vec2(EYEDROPPER_TARGET, EYEDROPPER_TARGET));
        let mut pick = ColorPick::none(id);
        pick.eyedropper = self.eyedropper_at(glyph, waiting, tokens.text_dim);
        self.drawn(bar.bottom());

        let ctx = self.ui.ctx().clone();
        let state_id = id.with("picker");
        let mut state = picker_state(&ctx, state_id);
        if let Some(picked) = ColorPick::delivered(&ctx, id) {
            pick.picked = Some(picked);
        }
        let clicked = response.clicked();
        let was_open = state.open;
        if clicked {
            if state.open {
                state.open = false;
            } else {
                open_picker(&ctx, &mut state, linear);
            }
        }

        if state.open {
            state.follow(linear);
            if let Some(picked) = pick.picked {
                state.set_linear(picked);
            }

            let inside = self.picker_popup(&response, glyph, label, &mut state, waiting, swatches);
            pick.picked = pick.picked.or(inside.picked);
            pick.merge = inside.merge;
            pick.eyedropper |= inside.eyedropper;
            pick.swatch = inside.swatch;
        }

        // Keep a moved position while its bar is visible, including while the picker is closed
        if was_open || state.open || state.moved_to.is_some() {
            ctx.memory_mut(|memory| memory.caches.cache::<VisiblePickers>().set(state_id, state));
        }

        pick
    }

    /// The open picker: a window the panel's width until resized (its edges and corners drag its size, kept for every picker), its rows at its edges (the project's `swatches` last), titled `label`, standing beside the panel level with `bar` (its title bar drags it, × or Escape closes it, and so does a press off it, the bar and the bar's `eyedropper` unless an eyedropper pick waits); a space or model the user picks becomes the next picker's
    fn picker_popup(&mut self, bar: &Response, eyedropper: Rect, label: &str, state: &mut PickerState, waiting: bool, swatches: &[[f32; 3]]) -> ColorPick {
        let id = bar.id;
        let ctx = self.ui.ctx().clone();
        let size_id = Id::unique(SIZE_KEY);
        let kept = ctx.data_mut(|data| data.get_persisted::<[f32; 2]>(size_id));
        let [width, field_height] = kept.unwrap_or([self.right - self.left + 2.0 * SIDE_PADDING_POINTS, FIELD_HEIGHT]);
        let screen = ctx.content_rect();

        // A kept size may come from a bigger screen or a damaged settings file (max/min also turn NaN into a bound)
        let width = width.max(PICKER_MIN_WIDTH).min(screen.width().max(PICKER_MIN_WIDTH));
        let fixed_height = state.fixed_height.unwrap_or(PICKER_HEIGHT_GUESS - FIELD_HEIGHT);
        let field_height = field_height.max(FIELD_MIN_HEIGHT).min((screen.height() - fixed_height).max(FIELD_MIN_HEIGHT));
        let height = fixed_height + field_height;
        let panel = Rect::from_x_y_ranges(self.ui.clip_rect().left().min(self.left - SIDE_PADDING_POINTS)..=self.right + SIDE_PADDING_POINTS, screen.y_range());
        let corner = state.moved_to.unwrap_or_else(|| beside(panel, bar.rect.top(), vec2(width, height), screen));

        // The title bar stays on screen, so a drag can always bring the window back
        let corner = pos2(
            corner.x.clamp(screen.left(), (screen.right() - width).max(screen.left())),
            corner.y.clamp(screen.top(), (screen.bottom() - TITLE_HEIGHT).max(screen.top())),
        );

        // A press that picks from one of the picker's own lists must not close it
        let list_open = egui::Popup::is_any_open(&ctx);
        let tokens = self.tokens;
        let shown_in = (state.space, state.model);

        let shown = egui::Area::new(id.with("picker_area")).order(Order::Foreground).fixed_pos(corner).show(&ctx, |ui| {
            let background = ui.painter().add(egui::Shape::Noop);
            let title_bar = Rect::from_min_size(corner, vec2(width, TITLE_HEIGHT));
            let close = Rect::from_center_size(
                pos2(title_bar.right() - SIDE_PADDING_POINTS - CLOSE_INK_HALF_WIDTH, title_bar.center().y),
                vec2(CLOSE_TARGET, CLOSE_TARGET),
            );
            let drag = ui.interact(Rect::from_min_max(title_bar.min, pos2(close.left(), title_bar.bottom())), id.with("picker_title"), Sense::drag());
            drag.widget_info(|| WidgetInfo::labeled(Role::TitleBar, true, label));
            let drag = drag.on_hover_cursor(CursorIcon::Grab);
            let closed = ui.interact(close, id.with("picker_close"), Sense::click());
            closed.widget_info(|| WidgetInfo::labeled(Role::Button, true, tr("Close")));
            let painter = ui.painter();
            painter.text(
                pos2(title_bar.left() + SIDE_PADDING_POINTS, title_bar.center().y),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(TITLE_TEXT_SIZE),
                tokens.text_dim,
            );
            let close_color = if closed.hovered() { tokens.text } else { tokens.text_dim };
            painter.text(close.center(), Align2::CENTER_CENTER, ic::X, FontId::proportional(CLOSE_GLYPH_SIZE), close_color);
            painter.hline(title_bar.x_range(), title_bar.bottom(), Stroke::new(1.0, tokens.separator));

            let top = title_bar.bottom();
            let rows = Rect::from_min_max(pos2(corner.x, top + PICKER_TOP), pos2(corner.x + width, top + PICKER_TOP + screen.height()));
            let (pick, bottom) = ui
                .scope_builder(UiBuilder::new().max_rect(rows), |ui| {
                    PropertySheet::show(ui, id.with("picker_rows"), |sheet| {
                        let pick = sheet.color_picker_rows(state, waiting, swatches, field_height, id);
                        (pick, sheet.drawn_bottom)
                    })
                })
                .inner;
            let face = Rect::from_min_max(corner, pos2(corner.x + width, bottom + PICKER_BOTTOM));
            let shadow = PICKER_SHADOW.as_shape(face, CornerRadius::same(PICKER_RADIUS));
            let frame =
                egui::epaint::RectShape::new(face, CornerRadius::same(PICKER_RADIUS), tokens.chrome, Stroke::new(1.0, tokens.field_border), StrokeKind::Inside);
            ui.painter().set(background, egui::Shape::Vec(vec![shadow.into(), frame.into()]));

            // Registered after the rows and the title bar, so the grips win at the edges
            let mut resize = None;
            for (index, (grip, sides)) in grips(face).into_iter().enumerate() {
                let cursor = resize_cursor(sides);
                let response = ui.interact(grip, id.with(("picker_resize", index)), Sense::drag()).on_hover_cursor(cursor);
                if response.dragged() {
                    ui.ctx().set_cursor_icon(cursor);
                    resize = Some((sides, response.drag_delta()));
                }
            }
            (pick, face, drag.dragged().then(|| drag.drag_delta()), closed.clicked(), resize)
        });
        let (pick, face, dragged, closed, resize) = shown.inner;
        let measured_fixed_height = face.height() - field_height;
        if (measured_fixed_height - fixed_height).abs() > 1.0 {
            ctx.request_repaint();
        }
        state.fixed_height = Some(measured_fixed_height);
        if (state.space, state.model) != shown_in {
            ctx.data_mut(|data| data.insert_temp(Id::unique(PREFERENCES_KEY), (state.space, state.model)));
        }
        if let Some(delta) = dragged {
            state.moved_to = Some(corner + delta);
        }
        if let Some((sides, delta)) = resize {
            // Only the field grows taller: the other rows keep their pitch
            let rows = face.height() - field_height;
            let most = vec2(screen.width().max(PICKER_MIN_WIDTH), (screen.height() - rows).max(FIELD_MIN_HEIGHT));
            let (moved, size) = resized(corner, vec2(width, field_height), sides, delta, most);

            // A resized window stays where it was put instead of following the panel
            state.moved_to = Some(moved);
            ctx.data_mut(|data| data.insert_persisted(size_id, [size.x, size.y]));
        }

        // Alt+drag orbits the camera, which keeps the picker open
        let pressed_outside = ctx.input(|input| {
            input.pointer.primary_pressed()
                && !input.modifiers.alt
                && input.pointer.interact_pos().is_some_and(|at| !face.contains(at) && !bar.rect.contains(at) && !eyedropper.contains(at))
        });

        // Escape ends hex editing first even when egui has already dropped the field's focus
        let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape))
            && ctx.memory(|memory| memory.focused().is_none_or(|focused| focused == bar.id))
            && !hex_took_escape(&ctx);
        if closed || (!list_open && !waiting && (pressed_outside || escape)) {
            state.open = false;
            crate::color_picker::forget_window(&ctx);
            if pressed_outside {
                crate::color_picker::closed_by_press(&ctx);
            }
        } else {
            crate::color_picker::remember_window(&ctx, face);
        }
        pick
    }
}

/// Top left corner of a window of `size` beside `panel` (left of it, or right of it when the screen ends first), level with `top` and on screen
fn beside(panel: Rect, top: f32, size: Vec2, screen: Rect) -> Pos2 {
    let left_of = panel.left() - WINDOW_GAP - size.x;
    let x = if left_of >= screen.left() { left_of } else { panel.right() + WINDOW_GAP };
    pos2(x, top.min(screen.bottom() - size.y).max(screen.top()))
}

/// The eight resize grips inside `face`'s edges and corners, each with the sides it moves (-1 left/top, 1 right/bottom, 0 neither); corners last, so they win where they overlap an edge
fn grips(face: Rect) -> [(Rect, Vec2); 8] {
    let edge = |sides: Vec2| {
        let x = match sides.x {
            x if x < 0.0 => face.left()..=face.left() + RESIZE_GRIP,
            x if x > 0.0 => face.right() - RESIZE_GRIP..=face.right(),
            _ => face.left()..=face.right(),
        };
        let y = match sides.y {
            y if y < 0.0 => face.top()..=face.top() + RESIZE_GRIP,
            y if y > 0.0 => face.bottom() - RESIZE_GRIP..=face.bottom(),
            _ => face.top()..=face.bottom(),
        };
        (Rect::from_x_y_ranges(x, y), sides)
    };
    let corner = |sides: Vec2| {
        let x = if sides.x < 0.0 { face.left() } else { face.right() - RESIZE_CORNER };
        let y = if sides.y < 0.0 { face.top() } else { face.bottom() - RESIZE_CORNER };
        (Rect::from_min_size(pos2(x, y), vec2(RESIZE_CORNER, RESIZE_CORNER)), sides)
    };
    [
        edge(vec2(-1.0, 0.0)),
        edge(vec2(1.0, 0.0)),
        edge(vec2(0.0, -1.0)),
        edge(vec2(0.0, 1.0)),
        corner(vec2(-1.0, -1.0)),
        corner(vec2(1.0, -1.0)),
        corner(vec2(-1.0, 1.0)),
        corner(vec2(1.0, 1.0)),
    ]
}

/// The pointer over a grip that moves `sides`
fn resize_cursor(sides: Vec2) -> CursorIcon {
    match (sides.x == 0.0, sides.y == 0.0) {
        (false, true) => CursorIcon::ResizeHorizontal,
        (true, false) => CursorIcon::ResizeVertical,
        _ if sides.x == sides.y => CursorIcon::ResizeNwSe,
        _ => CursorIcon::ResizeNeSw,
    }
}

/// The corner and size (width, field height) after a grip moving `sides` was dragged by `delta`, the size held between the least and `most`; a left or top edge moves the corner so the opposite edge stays put
fn resized(corner: Pos2, size: Vec2, sides: Vec2, delta: Vec2, most: Vec2) -> (Pos2, Vec2) {
    let least = vec2(PICKER_MIN_WIDTH, FIELD_MIN_HEIGHT);
    let mut corner = corner;
    let mut size = size;
    for axis in 0..2 {
        let grown = (size[axis] + sides[axis] * delta[axis]).clamp(least[axis], most[axis]);
        if sides[axis] < 0.0 {
            corner[axis] += size[axis] - grown;
        }
        size[axis] = grown;
    }
    (corner, size)
}
