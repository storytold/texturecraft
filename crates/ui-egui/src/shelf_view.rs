//! How the shelf shows its assets: small, medium or large tiles, or a list (the menu at the type row's end)

use egui::{Rect, Ui, Vec2, vec2};
use egui_phosphor::regular as ic;
use serde::{Deserialize, Serialize};

use crate::TextureCraftApp;
use crate::grid_tile::GridTile;
use crate::i18n::tr;
use crate::widgets::icon_button;

/// A list row's height and its thumbnail's size, in points
const LIST_ROW_HEIGHT: f32 = 30.0;
const LIST_THUMBNAIL_SIZE: f32 = 26.0;
/// The gap between a list row's thumbnail and its name, in points
pub(crate) const LIST_NAME_GAP: f32 = 6.0;

/// The shelf's tile size, or the list
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShelfView {
    Small,
    /// A 68 x 78 grid step, as in the reference
    #[default]
    Medium,
    Large,
    List,
}

/// The views in the menu's order: icon and (English, translated where shown) name
const VIEWS: [(ShelfView, &str, &str); 4] = [
    (ShelfView::Small, ic::GRID_NINE, "Small"),
    (ShelfView::Medium, ic::GRID_FOUR, "Medium"),
    (ShelfView::Large, ic::SQUARE, "Large"),
    (ShelfView::List, ic::LIST, "List"),
];

impl ShelfView {
    /// A grid tile's measurements; None for the list
    fn grid(self) -> Option<GridTile> {
        match self {
            ShelfView::Small => Some(GridTile { width: 50.0, height: 58.0, thumbnail_size: 44.0, thumbnail_center_y: 23.0 }),
            ShelfView::Medium => Some(GridTile { width: 66.0, height: 76.0, thumbnail_size: 60.0, thumbnail_center_y: 31.0 }),
            ShelfView::Large => Some(GridTile { width: 88.0, height: 100.0, thumbnail_size: 82.0, thumbnail_center_y: 42.0 }),
            ShelfView::List => None,
        }
    }

    /// A tile's size in a grid `width` points wide (a list row spans it)
    pub(crate) fn tile_size(self, width: f32) -> Vec2 {
        match self.grid() {
            Some(tile) => vec2(tile.width, tile.height),
            None => vec2(width.max(LIST_ROW_HEIGHT), LIST_ROW_HEIGHT),
        }
    }

    /// Where the thumbnail goes in the tile `rect`
    pub(crate) fn thumbnail_rect(self, rect: Rect) -> Rect {
        match self.grid() {
            Some(tile) => Rect::from_center_size(rect.center_top() + vec2(0.0, tile.thumbnail_center_y), vec2(tile.thumbnail_size, tile.thumbnail_size)),
            None => Rect::from_center_size(rect.left_center() + vec2(LIST_ROW_HEIGHT * 0.5, 0.0), vec2(LIST_THUMBNAIL_SIZE, LIST_THUMBNAIL_SIZE)),
        }
    }

    /// Whether the name sits right of the thumbnail (a list) rather than under it
    pub(crate) fn name_beside(self) -> bool {
        self == ShelfView::List
    }
}

/// The view menu button: its icon is the current view's
pub(crate) fn menu(app: &mut TextureCraftApp, ui: &mut Ui, size: f32) {
    let current = app.ui.shelf_view;
    let icon = VIEWS.iter().find(|(view, _, _)| *view == current).map_or(ic::GRID_FOUR, |(_, icon, _)| icon);
    let response = icon_button(ui, icon, tr("Thumbnail size"), false, size);
    egui::Popup::menu(&response).show(|ui| {
        for (view, icon, label) in VIEWS {
            if ui.selectable_label(view == current, format!("{icon}  {}", tr(label))).clicked() {
                app.ui.shelf_view = view;
                ui.close();
            }
        }
    });
}
