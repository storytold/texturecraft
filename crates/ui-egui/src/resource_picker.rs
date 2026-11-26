//! A popup hanging under a slot that picks a resource (a filter, a channel's source) from tiles under bright group headers, behind edge tabs and a search field

use egui::text_edit::TextEditState;
use egui::{Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Rect, Response, Sense, Shadow, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as ic;

use crate::i18n::tr;
use crate::picker_tab::PickerTab;
use crate::property_sheet::{TAB_GAP, TAB_ICON_CENTER, TAB_ICON_SIZE, TAB_NAME_LEFT, TAB_PADDING, TAB_ROUNDING, TAB_TEXT_SIZE, TAB_TOP};
use crate::resource_picker_item::{ResourcePickerItem, search_query};
use crate::theme::{HARD_SHADOW, Tokens, semibold};
use crate::thumbs::Thumbs;
use crate::tile_picture::TilePicture;

type PickerSearches = egui::cache::FramePublisher<Id, (String, TextEditState)>;

/// Columns of tiles in the grid
const COLUMNS: usize = 4;
/// One row of tiles: the picture, two lines of name and air
const ROW_HEIGHT: f32 = 86.0;
/// Rows of tiles the grid shows at once
const GRID_ROWS: f32 = 4.0;
/// The grid's height, fixed so the popup keeps its place and size while searching or switching tabs; less only in a window too short for it
const GRID_HEIGHT: f32 = GRID_ROWS * ROW_HEIGHT;
/// Air between the slot and the popup hanging from it
const POPUP_GAP: f32 = 1.0;
/// Room kept between the popup's shadow and the window's top or bottom edge
const WINDOW_MARGIN: f32 = 8.0;
/// Where the popup goes when it doesn't fit under its slot: above it, then flush with the slot's right edge below or above
const HANG_FALLBACKS: [egui::RectAlign; 3] = [egui::RectAlign::TOP_START, egui::RectAlign::BOTTOM_END, egui::RectAlign::TOP_END];
const TILE_GAP: f32 = 4.0;
/// Air on each side of a tile inside its grid cell, so neighbouring tiles' hover fills never touch
const TILE_SIDE_GAP: f32 = 1.0;
const PICTURE_SIZE: f32 = 52.0;
const PICTURE_TOP: f32 = 6.0;
const PICTURE_ROUNDING: u8 = 3;
const TILE_ROUNDING: u8 = 5;
const NAME_SIZE: f32 = 10.0;
/// From the tile's top to the first name line's centre, and between the two lines: both lines sit inside the tile's box
const NAME_CENTER: f32 = PICTURE_SIZE + 14.0;
const NAME_LINE: f32 = 12.0;
const NAME_MARGIN: f32 = 4.0;
const IMPORT_GLYPH_SIZE: f32 = 16.0;
const LOADING_GLYPH_SIZE: f32 = 24.0;
/// The loading glyph is fainter than the other glyphs: it stands in for a picture still coming
const LOADING_GLYPH_OPACITY: f32 = 0.5;
const GLYPH_SIZE: f32 = 22.0;
/// The randomness badge: a see-through dark disc at the picture's bottom-right corner holding a die
const BADGE_RADIUS: f32 = 7.0;
const BADGE_ICON_SIZE: f32 = 10.0;
const BADGE_DISC: Color32 = Color32::from_black_alpha(140);
const BADGE_ICON: Color32 = Color32::from_gray(228);

/// The bright group header row and the dim sub-caption row
const HEADER_HEIGHT: f32 = 24.0;
const HEADER_SIZE: f32 = 12.0;
const CAPTION_HEIGHT: f32 = 18.0;
const CAPTION_SIZE: f32 = 11.0;
/// Header and caption text from the grid's left edge
const HEADER_INSET: f32 = 4.0;

/// The edge tab strip and its tabs
const STRIP_HEIGHT: f32 = 28.0;
const STRIP_ROUNDING: u8 = 6;
const COUNT_SIZE: f32 = 10.0;
const COUNT_GAP: f32 = 4.0;

/// The search row and its field
const SEARCH_ROW: f32 = 36.0;
const FIELD_HEIGHT: f32 = 24.0;
const FIELD_ROUNDING: u8 = 5;
const SEARCH_TEXT_SIZE: f32 = 12.0;
const SEARCH_ICON_X: f32 = 13.0;
const SEARCH_TEXT_X: f32 = 25.0;
const CLEAR_X: f32 = 13.0;
const CLEAR_SIZE: f32 = 12.0;
const SEARCH_ICON_SIZE: f32 = 13.0;

/// Air from the popup's border to the search field and the grid, the grid's right room for the scroll bar, air under the grid
const SIDE: f32 = 5.0;
const SCROLL_ROOM: f32 = 10.0;
const BOTTOM: f32 = 6.0;
const POPUP_ROUNDING: u8 = 7;
const BORDER: f32 = 1.0;
const SHADOW_OFFSET: [i8; 2] = [2, 4];

/// Opens the picker `popup_id` with an empty, focused search field, or closes it
pub(crate) fn toggle(ctx: &egui::Context, popup_id: Id) {
    if egui::Popup::is_id_open(ctx, popup_id) {
        egui::Popup::close_id(ctx, popup_id);
        return;
    }

    egui::Popup::open_id(ctx, popup_id);
    ctx.memory_mut(|memory| memory.caches.cache::<PickerSearches>().set(popup_id, (String::new(), TextEditState::default())));
    ctx.data_mut(|data| data.insert_temp(opened_id(popup_id), true));
}

/// Draws picker `popup_id` under `slot` (above it where the window ends first) at its width while open, on the tab holding `current` (scrolled to it when `reveal_current`, else at the top); returns the id clicked, and runs `tabs` only while open
pub(crate) fn show<K: Clone + PartialEq>(
    slot: &Response,
    popup_id: Id,
    thumbs: &mut Thumbs,
    current: Option<&K>,
    reveal_current: bool,
    tabs: impl FnOnce() -> Vec<PickerTab<K>>,
) -> Option<K> {
    let ctx = slot.ctx.clone();
    if !egui::Popup::is_id_open(&ctx, popup_id) {
        return None;
    }

    let tokens = Tokens::get(&ctx);
    let tabs = tabs();
    let width = slot.rect.width() - 2.0 * BORDER;
    let frame = Frame::NONE
        .fill(tokens.panel)
        .stroke(Stroke::new(BORDER, tokens.field_border))
        .corner_radius(CornerRadius::same(POPUP_ROUNDING))
        .shadow(Shadow { offset: SHADOW_OFFSET, blur: 0, spread: 0, color: HARD_SHADOW });

    // Shorten the grid only when it cannot fit above or below the slot
    let head = if tabs.len() > 1 { STRIP_HEIGHT } else { 0.0 } + SEARCH_ROW;
    let room = ctx.content_rect().height() - head - BOTTOM - 2.0 * BORDER - 2.0 * WINDOW_MARGIN;
    let grid_height = room.clamp(ROW_HEIGHT, GRID_HEIGHT);
    let mut picked = None;
    egui::Popup::from_response(slot)
        .id(popup_id)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&HANG_FALLBACKS)
        .gap(POPUP_GAP)
        .width(width)
        .frame(frame)
        .show(|ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);

            let just_opened = !ui.is_sizing_pass() && ui.data_mut(|data| data.remove_temp::<bool>(opened_id(popup_id))).unwrap_or(false);
            let current_tab = || current.and_then(|current| tabs.iter().position(|tab| tab.items().any(|item| &item.id == current)));
            let stored_tab = ui.data(|data| data.get_temp::<usize>(tab_id(popup_id)));
            let mut open_tab =
                if just_opened { current_tab().or(stored_tab) } else { stored_tab.or_else(current_tab) }.unwrap_or(0).min(tabs.len().saturating_sub(1));

            let (search, editor) = ui.memory_mut(|memory| memory.caches.cache::<PickerSearches>().get(&popup_id).cloned()).unwrap_or_default();
            if tabs.len() > 1
                && let Some(clicked) = tab_strip(ui, &tabs, open_tab, &search_query(&search))
            {
                open_tab = clicked;
            }

            ui.data_mut(|data| data.insert_temp(tab_id(popup_id), open_tab));
            let query = search_field(ui, popup_id, just_opened, search, editor);

            if let Some(tab) = tabs.get(open_tab) {
                let reveal = just_opened && reveal_current;
                let mut scroll = egui::ScrollArea::vertical().id_salt((popup_id, open_tab)).max_height(grid_height);

                // A new opening with nothing to reveal starts at the top, not where the last one was left
                if just_opened && !reveal {
                    scroll = scroll.vertical_scroll_offset(0.0);
                }

                scroll.show(ui, |ui| {
                    // A search that leaves few tiles must not shrink the popup, or egui moves it
                    ui.set_min_height(grid_height);
                    picked = grid(ui, thumbs, tab, &query, current, reveal.then_some(grid_height));
                });
            }
            ui.add_space(BOTTOM);
        });

    if picked.is_some() {
        egui::Popup::close_id(&ctx, popup_id);
    }

    picked
}

/// The edge tabs on the popup's top: a chrome strip, the open tab a panel-coloured box joined to the body; while searching each counts its matches
fn tab_strip<K>(ui: &mut Ui, tabs: &[PickerTab<K>], open: usize, query: &str) -> Option<usize> {
    let tokens = Tokens::get(ui.ctx());
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), STRIP_HEIGHT), Sense::hover());
    let top_only = CornerRadius { nw: STRIP_ROUNDING, ne: STRIP_ROUNDING, sw: 0, se: 0 };
    ui.painter().rect_filled(strip, top_only, tokens.chrome);

    let mut clicked = None;
    let mut left = strip.left() + SIDE;
    for (index, tab) in tabs.iter().enumerate() {
        let is_open = index == open;
        let empty = tab.items().next().is_none();
        let color = if is_open {
            tokens.text
        } else if empty {
            tokens.button_border
        } else {
            tokens.text_dim
        };
        let name_font = if is_open { semibold(TAB_TEXT_SIZE) } else { FontId::proportional(TAB_TEXT_SIZE) };
        let name = ui.painter().layout_no_wrap(tab.name.to_owned(), name_font, color);
        let count = (!query.is_empty()).then(|| tab.items().filter(|item| item.matches(query)).count());
        let count_galley = count.map(|count| {
            let count_color = if count > 0 { tokens.accent } else { tokens.button_border };
            ui.painter().layout_no_wrap(count.to_string(), FontId::proportional(COUNT_SIZE), count_color)
        });
        let width = name.size().x + TAB_PADDING + count_galley.as_ref().map_or(0.0, |galley| COUNT_GAP + galley.size().x);

        let rect = Rect::from_min_max(pos2(left, strip.top() + TAB_TOP), pos2(left + width, strip.bottom()));
        let response = ui.interact(rect, ui.scope_id().with(("picker_tab", index)), Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, is_open, tab.name));
        if is_open {
            ui.painter().rect_filled(rect, CornerRadius { nw: TAB_ROUNDING, ne: TAB_ROUNDING, sw: 0, se: 0 }, tokens.panel);
        }
        let text_color = if response.hovered() && !is_open { tokens.text } else { color };
        let center_y = strip.center().y + TAB_TOP / 2.0;
        ui.painter().text(pos2(left + TAB_ICON_CENTER, center_y), Align2::CENTER_CENTER, tab.icon, FontId::proportional(TAB_ICON_SIZE), text_color);
        let name_left = left + TAB_NAME_LEFT;
        ui.painter().galley(pos2(name_left, center_y - name.size().y / 2.0), name.clone(), text_color);
        if let Some(galley) = count_galley {
            ui.painter().galley(pos2(name_left + name.size().x + COUNT_GAP, center_y - galley.size().y / 2.0), galley, Color32::PLACEHOLDER);
        }
        if response.clicked() {
            clicked = Some(index);
        }
        left += width + TAB_GAP;
    }
    clicked
}

/// The neutral search field (no accent outline, focused or not), focused the frame the picker opens, × clearing it; returns the lowercase query
fn search_field(ui: &mut Ui, popup_id: Id, just_opened: bool, mut search: String, editor: TextEditState) -> String {
    let tokens = Tokens::get(ui.ctx());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), SEARCH_ROW), Sense::hover());
    let field = Rect::from_center_size(row.center(), vec2(row.width() - 2.0 * SIDE, FIELD_HEIGHT));
    let painter = ui.painter();
    painter.rect(field, CornerRadius::same(FIELD_ROUNDING), tokens.field, Stroke::new(BORDER, tokens.field_border), StrokeKind::Inside);
    painter.text(
        pos2(field.left() + SEARCH_ICON_X, field.center().y),
        Align2::CENTER_CENTER,
        ic::MAGNIFYING_GLASS,
        FontId::proportional(SEARCH_ICON_SIZE),
        tokens.text_dim,
    );

    let text_rect = Rect::from_min_max(pos2(field.left() + SEARCH_TEXT_X, field.top()), pos2(field.right() - 2.0 * CLEAR_X, field.bottom()));
    let hint = egui::RichText::new(tr("Search")).color(tokens.text_dim).size(SEARCH_TEXT_SIZE);
    let field_id = popup_id.with("search_field");
    editor.store(ui.ctx(), field_id);

    let edit = egui::TextEdit::singleline(&mut search)
        .id(field_id)
        .frame(Frame::NONE)
        .hint_text(hint)
        .font(semibold(SEARCH_TEXT_SIZE))
        .text_color(tokens.text)
        .align(Align2::LEFT_CENTER)
        .desired_width(text_rect.width());
    let response = ui.put(text_rect, edit);
    if just_opened {
        response.request_focus();
    }

    if !search.is_empty() {
        let clear = Rect::from_center_size(pos2(field.right() - CLEAR_X, field.center().y), vec2(FIELD_HEIGHT, FIELD_HEIGHT));
        let clear = ui.interact(clear, popup_id.with("clear"), Sense::click());
        clear.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("Clear search")));
        let color = if clear.hovered() { tokens.text_strong } else { tokens.text };
        ui.painter().text(clear.rect.center(), Align2::CENTER_CENTER, ic::X, FontId::proportional(CLEAR_SIZE), color);
        if clear.clicked() {
            search.clear();
            response.request_focus();
        }
    }

    let query = search_query(&search);
    let state = TextEditState::load(ui.ctx(), field_id).unwrap_or_default();

    // Keep the text and undo history only while the picker is drawn
    ui.data_mut(|data| data.remove::<TextEditState>(field_id));
    ui.memory_mut(|memory| memory.caches.cache::<PickerSearches>().set(popup_id, (search, state)));

    query
}

/// The open tab's items matching `query` as tiles under their headers and captions, `current` outlined and, given the grid's height in `reveal_in`, scrolled to; returns the one clicked
fn grid<K: Clone + PartialEq>(ui: &mut Ui, thumbs: &mut Thumbs, tab: &PickerTab<K>, query: &str, current: Option<&K>, reveal_in: Option<f32>) -> Option<K> {
    let tokens = Tokens::get(ui.ctx());
    let left = ui.max_rect().left() + SIDE;
    let cell = (ui.available_width() - SIDE - SCROLL_ROOM) / COLUMNS as f32;
    let mut picked = None;
    let mut place = 0;
    let mut row = Vec::new();

    for group in &tab.groups {
        let mut matching_items = group.items.iter().filter(|item| item.matches(query)).peekable();
        if matching_items.peek().is_none() {
            continue;
        }

        let group_top = ui.cursor().top();
        if let Some(header) = group.header {
            caption_row(ui, left, HEADER_HEIGHT, header, semibold(HEADER_SIZE), tokens.text);
        }

        let mut caption_top = group_top;
        let mut last_caption = None;
        for item in matching_items {
            if item.caption != last_caption {
                flush_row(ui, thumbs, &mut row, left, cell, current, &mut picked);
                if let Some(caption) = item.caption {
                    caption_top = ui.cursor().top();
                    caption_row(ui, left, CAPTION_HEIGHT, caption, FontId::proportional(CAPTION_SIZE), tokens.text_dim);
                }
                last_caption = item.caption;
            }

            row.push((place, item));
            place += 1;
            if let Some(grid_height) = reveal_in
                && current == Some(&item.id)
            {
                // The current's header or caption shows above it when both fit, so the artist sees where it sits
                let row_top = ui.cursor().top();
                let label_top = if item.caption.is_some() { caption_top } else { group_top };
                let from = if row_top + ROW_HEIGHT - label_top <= grid_height { label_top } else { row_top };
                ui.scroll_to_rect(Rect::from_min_max(pos2(left, from), pos2(left + cell, row_top + ROW_HEIGHT)), Some(Align::TOP));
            }
            if row.len() == COLUMNS {
                flush_row(ui, thumbs, &mut row, left, cell, current, &mut picked);
            }
        }
        flush_row(ui, thumbs, &mut row, left, cell, current, &mut picked);
    }

    picked
}

/// A header or caption line of `height`, its text `font` in `color`
fn caption_row(ui: &mut Ui, left: f32, height: f32, text: &str, font: FontId, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().text(pos2(left + HEADER_INSET, rect.center().y), Align2::LEFT_CENTER, text, font, color);
}

/// Lays out the tiles in `row` as one grid row and empties it, recording a click in `picked`
fn flush_row<K: Clone + PartialEq>(
    ui: &mut Ui,
    thumbs: &mut Thumbs,
    row: &mut Vec<(usize, &ResourcePickerItem<K>)>,
    left: f32,
    cell: f32,
    current: Option<&K>,
    picked: &mut Option<K>,
) {
    if row.is_empty() {
        return;
    }

    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    for (column, (place, item)) in row.drain(..).enumerate() {
        let x = left + column as f32 * cell;
        let tile = Rect::from_min_max(pos2(x + TILE_SIDE_GAP, rect.top()), pos2(x + cell - TILE_SIDE_GAP, rect.bottom() - TILE_GAP));
        if tile_ui(ui, thumbs, tile, (place, item), current == Some(&item.id)).clicked() {
            *picked = Some(item.id.clone());
        }
    }
}

/// One tile, `place` its order in the tab: its picture, its name in up to two lines; hovered = button fill, current = accent outline and a bold name
fn tile_ui<K>(ui: &mut Ui, thumbs: &mut Thumbs, rect: Rect, (place, item): (usize, &ResourcePickerItem<K>), chosen: bool) -> Response {
    let tokens = Tokens::get(ui.ctx());
    let response = ui.interact(rect, ui.scope_id().with(("picker_tile", place)), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListBoxOption, true, chosen, &item.label));
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let hovered = response.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, CornerRadius::same(TILE_ROUNDING), tokens.button);
    }
    if chosen {
        painter.rect_stroke(rect, CornerRadius::same(TILE_ROUNDING), Stroke::new(BORDER, tokens.accent), StrokeKind::Inside);
    }

    let picture = Rect::from_min_size(pos2(rect.center().x - PICTURE_SIZE / 2.0, rect.top() + PICTURE_TOP), vec2(PICTURE_SIZE, PICTURE_SIZE));
    let rounding = CornerRadius::same(PICTURE_ROUNDING);

    // Offscreen tiles must leave the thumbnail budget available to visible tiles
    let texture = match &item.picture {
        TilePicture::Material(id) => thumbs.material(ui.ctx(), id),
        TilePicture::Filter(id) => thumbs.filter(ui.ctx(), id),
        TilePicture::Alpha(alpha) => thumbs.alpha(ui.ctx(), alpha),
        TilePicture::Image(name, image) => Some(thumbs.image(ui.ctx(), name, image)),
        TilePicture::Procedural(kind) => thumbs.procedural(ui.ctx(), *kind),
        TilePicture::Generator(kind) => thumbs.generator(ui.ctx(), *kind),
        TilePicture::Swatch(_) | TilePicture::Glyph(_) | TilePicture::Import => None,
    };

    match (&item.picture, texture) {
        (_, Some(texture)) => {
            egui::Image::from_texture((texture.id(), picture.size())).corner_radius(rounding).paint_at(ui, picture);
        }
        (
            TilePicture::Material(_)
            | TilePicture::Filter(_)
            | TilePicture::Alpha(_)
            | TilePicture::Image(..)
            | TilePicture::Procedural(_)
            | TilePicture::Generator(_),
            None,
        ) => {
            painter.text(
                picture.center(),
                Align2::CENTER_CENTER,
                ic::ARROWS_CLOCKWISE,
                FontId::proportional(LOADING_GLYPH_SIZE),
                tokens.text_dim.gamma_multiply(LOADING_GLYPH_OPACITY),
            );
        }
        (TilePicture::Swatch(color), None) => {
            painter.rect_filled(picture, rounding, *color);
        }
        (TilePicture::Glyph(glyph), None) => {
            painter.rect(picture, rounding, tokens.field, Stroke::new(BORDER, tokens.field_border), StrokeKind::Inside);
            painter.text(picture.center(), Align2::CENTER_CENTER, *glyph, FontId::proportional(GLYPH_SIZE), tokens.text_dim);
        }
        (TilePicture::Import, None) => {
            painter.rect(picture, rounding, tokens.field, Stroke::new(BORDER, tokens.field_border), StrokeKind::Inside);
            let color = if hovered { tokens.text } else { tokens.text_dim };
            painter.text(picture.center(), Align2::CENTER_CENTER, ic::PLUS, FontId::proportional(IMPORT_GLYPH_SIZE), color);
        }
    }
    if let TilePicture::Generator(kind) = &item.picture
        && kind.includes_randomness()
    {
        randomness_badge(painter, picture);
    }
    name_lines(ui, rect, &item.label, hovered || chosen, chosen);

    response
}

/// A die on a see-through dark disc at `picture`'s bottom-right corner: the item's result depends on a random seed
fn randomness_badge(painter: &egui::Painter, picture: Rect) {
    let center = picture.right_bottom() - vec2(BADGE_RADIUS, BADGE_RADIUS);
    painter.circle_filled(center, BADGE_RADIUS, BADGE_DISC);
    painter.text(center, Align2::CENTER_CENTER, ic::DICE_FIVE, FontId::proportional(BADGE_ICON_SIZE), BADGE_ICON);
}

/// The tile's name under its picture: whole words on the first line, the rest on a second, each cut with "…" when too long
fn name_lines(ui: &Ui, rect: Rect, label: &str, bright: bool, bold: bool) {
    let tokens = Tokens::get(ui.ctx());
    let painter = ui.painter();
    let color = if bright { tokens.text } else { tokens.text_dim };
    let font = if bold { semibold(NAME_SIZE) } else { FontId::proportional(NAME_SIZE) };
    let room = rect.width() - NAME_MARGIN;
    let fits = |text: &str| painter.layout_no_wrap(text.to_owned(), font.clone(), color).size().x <= room;

    // Each word's end in the trimmed name, so a first line of whole words is measured as a slice of it
    let trimmed = label.trim();
    let prefix = |end: usize| trimmed.get(..end).unwrap_or_default();
    let word_ends = trimmed
        .char_indices()
        .filter(|(at, character)| character.is_whitespace() && !prefix(*at).ends_with(char::is_whitespace))
        .map(|(at, _)| at)
        .chain(std::iter::once(trimmed.len()));

    // Most names fit whole: one measure, and the words are only measured for a name that wraps
    let first_line_end = if fits(trimmed) { None } else { word_ends.take_while(|end| fits(prefix(*end))).last() };

    let (first, second) = match first_line_end {
        Some(end) if end < trimmed.len() => (prefix(end), trimmed.get(end..).map(str::trim_start)),
        _ => (label, None),
    };

    for (index, line) in std::iter::once(first).chain(second).enumerate() {
        let mut job = egui::text::LayoutJob::simple_singleline(line.to_owned(), font.clone(), color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let galley = painter.layout_job(job);
        let center = pos2(rect.center().x, rect.top() + NAME_CENTER + index as f32 * NAME_LINE);
        painter.galley(center - galley.size() / 2.0, galley, color);
    }
}

/// Set while the picker waits for its first frame open
fn opened_id(popup_id: Id) -> Id {
    popup_id.with("opened")
}

/// The tab the picker shows
fn tab_id(popup_id: Id) -> Id {
    popup_id.with("tab")
}
