//! An asset tile's right-click menu: what the asset is (name, usages, path, category), then what can be done with it; the Name of an asset the user owns is a field that renames it

use egui::{Align2, Color32, CornerRadius, FontId, Id, Response, Sense, Ui, pos2, vec2};
use serde_json::json;

use crate::TextureCraftApp;
use crate::i18n::{n_, tr};
use crate::shelf_entry::{LIBRARIES, ShelfEntry};
use crate::theme::Tokens;

/// A menu row's height and the inset of its hover face from the menu's sides, in points
const ROW_HEIGHT: f32 = 24.0;
const HOVER_INSET: f32 = 3.0;
const HOVER_ROUNDING: u8 = 3;
/// Where a row's text starts, from the menu's left edge, in points
const TEXT_X: f32 = 10.0;
/// The space after the text of the widest row, in points
const RIGHT_PADDING: f32 = 24.0;
/// The air between the info block's label column and its values, in points
const LABEL_GAP: f32 = 12.0;
/// The widest the info block's values get before the path is cut in the middle, in points
const MAX_VALUE_WIDTH: f32 = 220.0;
/// The menu's text size, in points
const TEXT_SIZE: f32 = 12.0;
/// The Name field's row and field heights, and the air before its text, in points
const FIELD_ROW_HEIGHT: f32 = 28.0;
const FIELD_HEIGHT: f32 = 24.0;
const FIELD_TEXT_INSET: f32 = 8.0;
/// The air above and below the separators, in points
const SEPARATOR_SPACE: f32 = 3.0;

/// The category's place in an asset's path: after the library and the type's folder
const CATEGORY_PART: usize = 2;

/// A menu just opened from the keyboard gives the focus to its first row that can be picked (ctx temp data)
const FOCUS_FIRST_ROW: &str = "shelf_menu_focus_first_row";

/// The right-click menu on the tile whose response is `response`
pub(crate) fn show(app: &mut TextureCraftApp, response: &Response, entry: &ShelfEntry) {
    // Clicks inside keep the menu open, so the Name field can be typed in; its rows close it themselves
    let mut menu = egui::Popup::context_menu(response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    response.ctx.accesskit_node_builder(response.id, |node| node.set_has_popup(egui::accesskit::HasPopup::Menu));

    // The keyboard opens it on the focused tile, and Escape gives the focus back to the tile
    let keyboard_id = response.id.with("menu_from_keyboard");
    let return_focus = menu.is_open() && response.ctx.input(|input| input.key_pressed(egui::Key::Escape));
    if response.has_focus()
        && response
            .ctx
            .input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::F10) || input.consume_key(egui::Modifiers::NONE, egui::Key::Menu))
    {
        menu = menu.open_memory(Some(egui::SetOpenCommand::Bool(true)));
        response.ctx.data_mut(|data| {
            data.insert_temp(keyboard_id, true);
            data.insert_temp(Id::unique(FOCUS_FIRST_ROW), true);
        });
    } else if response.secondary_clicked() {
        response.ctx.data_mut(|data| data.remove::<bool>(keyboard_id));
    }

    // A menu opened from the keyboard has no pointer position to open at: it opens at the tile
    if response.ctx.data(|data| data.get_temp::<bool>(keyboard_id)).unwrap_or_default() {
        menu = menu.anchor(response);
    }
    menu.show(|ui| {
        let tokens = Tokens::get(ui.ctx());
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let info = info_rows(entry);
        let width = menu_width(ui, &info, &row_labels(entry));
        ui.set_width(width);
        let column = label_column(ui, &info);

        // A saved brush or a project image is the user's own: its Name row is a field that renames it
        let mut shown_info = info.as_slice();
        let rename = match entry {
            ShelfEntry::SavedBrush { name, .. } => Some((name.as_str(), "brush.preset.rename")),
            ShelfEntry::Image(name) => Some((name.as_str(), "resource.rename")),
            _ => None,
        };
        if let Some((name, command)) = rename
            && let Some((_, rest)) = info.split_first()
        {
            shown_info = rest;
            let field_id = ui.make_persistent_id(("asset_name", entry.identity()));
            if let Some(renamed) = name_field(ui, width, column, field_id, name, &tokens)
                && let Some(result) = app.run(command, json!({"name": name, "to": renamed}))
                && let Some(new_name) = result["name"].as_str()
            {
                // The rows below would act on the old name: the menu draws again under the new one next frame
                keep_star_and_preview(app, entry, new_name);
                return;
            }
        }
        info_block(ui, width, column, shown_info, &tokens);
        separator(ui, &tokens);

        match entry {
            ShelfEntry::Image(name) => {
                let reloadable = app.session.doc.project.resource_sources.contains_key(name);
                let reload = row(ui, width, tr("Reload"), reloadable, &tokens);
                if reload.clicked() {
                    crate::shelf_updates::update_from_files(app, Some(vec![name.clone()]));
                    ui.close();
                }
                if !reloadable {
                    reload.on_hover_text(tr("Not imported from a file: no file to reload from"));
                }
                if row(ui, width, tr("Delete"), true, &tokens).clicked() {
                    app.run("resource.remove", json!({"name": name}));
                    ui.close();
                }
                separator(ui, &tokens);
                if row(ui, width, tr("Export resource..."), true, &tokens).clicked() {
                    export_image(app, name);
                    ui.close();
                }
            }
            ShelfEntry::SavedBrush { name, .. } => {
                if row(ui, width, tr("Replace with the current brush"), true, &tokens).clicked() {
                    app.run("brush.preset.store", json!({"name": name, "overwrite": true}));
                    ui.close();
                }
                if row(ui, width, tr("Delete"), true, &tokens).clicked() {
                    app.run("brush.preset.delete", json!({"name": name}));
                    ui.close();
                }
                separator(ui, &tokens);
                if row(ui, width, tr("Export resource..."), true, &tokens).clicked() {
                    export_brush(app, name);
                    ui.close();
                }
            }
            _ => {
                row(ui, width, tr("Export resource..."), false, &tokens).on_hover_text(tr("Built-in assets have no file to export"));
            }
        }
        custom_preview_rows(app, ui, width, entry, &tokens);
        show_in(app, ui, width, entry, &tokens);
        if row(ui, width, favorite_label(app, entry), true, &tokens).clicked() {
            crate::shelf_favorite::toggle(&mut app.ui.shelf_favorites, entry);
            ui.close();
        }
        if let ShelfEntry::Image(name) = entry {
            separator(ui, &tokens);
            if row(ui, width, tr("Add as decal (view center)"), true, &tokens).clicked() {
                app.run("layer.add.decal", json!({"image": name}));
                ui.close();
            }
        }

        separator(ui, &tokens);
        if row(ui, width, tr("Close"), true, &tokens).clicked() {
            ui.close();
        }
    });
    if return_focus {
        response.request_focus();
    }
}

/// Moves the favorite star and custom preview of `entry`, just renamed to `new_name`, to its new identity
fn keep_star_and_preview(app: &mut TextureCraftApp, entry: &ShelfEntry, new_name: &str) {
    let (kind, old_id) = entry.identity();
    let new_id = match entry {
        ShelfEntry::SavedBrush { .. } => app
            .session
            .brush_presets
            .list
            .iter()
            .find(|preset| preset.name == new_name)
            .map(|preset| preset.file.as_ref().and_then(|file| file.to_str()).unwrap_or(&preset.name).to_owned()),
        _ => Some(new_name.to_owned()),
    };
    let Some(new_id) = new_id else { return };

    let ui = &mut app.ui;
    let favorites = ui.shelf_favorites.iter_mut().map(|favorite| (&favorite.kind, &mut favorite.id));
    let previews = ui.shelf_custom_previews.iter_mut().map(|preview| (&preview.kind, &mut preview.id));
    for (_, id) in favorites.chain(previews).filter(|(stored_kind, id)| *stored_kind == kind && *id == old_id) {
        id.clone_from(&new_id);
    }
}

/// The info block's label and value rows: rows only where the asset has the data
fn info_rows(entry: &ShelfEntry) -> Vec<(&'static str, String)> {
    let path = entry.path();
    let shown_path: Vec<&str> = path.iter().map(|part| LIBRARIES.iter().find(|(id, _)| id == part).map_or_else(|| tr(part), |(_, name)| tr(name))).collect();
    let mut rows =
        vec![(tr("Name"), entry.label().to_string()), (tr("Usages"), tr(entry.usage()).to_string()), (tr("Path"), format!("/{}/", shown_path.join("/")))];
    if let Some(category) = path.get(CATEGORY_PART) {
        rows.push((tr("Category"), tr(category).to_string()));
    }
    rows
}

/// The labels of the rows under the info block, for the menu's width
fn row_labels(entry: &ShelfEntry) -> Vec<&'static str> {
    let mut labels = vec![
        tr("Export resource..."),
        tr("Set custom preview..."),
        tr("Remove custom preview"),
        tr("Show in"),
        tr("Add to favorites"),
        tr("Remove from favorites"),
        tr("Close"),
    ];
    match entry {
        ShelfEntry::Image(_) => labels.extend([tr("Reload"), tr("Delete"), tr("Add as decal (view center)")]),
        ShelfEntry::SavedBrush { .. } => labels.extend([tr("Replace with the current brush"), tr("Delete")]),
        _ => {}
    }
    labels
}

/// Set custom preview... picks a picture to show on the tile; Remove custom preview, once one is set, shows the asset's own thumbnail again
fn custom_preview_rows(app: &mut TextureCraftApp, ui: &mut Ui, width: f32, entry: &ShelfEntry, tokens: &Tokens) {
    if row(ui, width, tr("Set custom preview..."), true, tokens).clicked() {
        pick_custom_preview(app, entry);
        ui.close();
    }
    if crate::shelf_custom_preview::file_for(&app.ui.shelf_custom_previews, entry).is_some()
        && row(ui, width, tr("Remove custom preview"), true, tokens).clicked()
    {
        crate::shelf_custom_preview::remove(&mut app.ui.shelf_custom_previews, entry);
        app.thumbs.retain_custom_previews(&app.ui.shelf_custom_previews);
        ui.close();
    }
}

/// Asks for a picture, then shows it on `entry`'s tile
#[cfg(not(target_arch = "wasm32"))]
fn pick_custom_preview(app: &mut TextureCraftApp, entry: &ShelfEntry) {
    use crate::file_dialogs::{Bound, FileDialog};

    let (kind, id) = entry.identity();
    let (kind, id) = (kind.to_owned(), id.to_owned());
    let dialog = FileDialog::open().filter(tr("Images"), crate::web_files::IMAGE_EXTS);
    app.ask_file(dialog, Bound::App, move |app, paths| {
        let Some(file) = paths.into_iter().next() else { return };
        if image::image_dimensions(&file).is_err() {
            app.toast(crate::i18n::trf("Could not read {file} as a picture", &[("file", &file.display())]), true);
            return;
        }

        app.thumbs.forget_custom_preview(&file);
        crate::shelf_custom_preview::set(&mut app.ui.shelf_custom_previews, kind, id, file);
        app.thumbs.retain_custom_previews(&app.ui.shelf_custom_previews);
    });
}

#[cfg(target_arch = "wasm32")]
fn pick_custom_preview(app: &mut TextureCraftApp, _: &ShelfEntry) {
    app.toast(tr("Custom previews need the desktop app"), true);
}

/// The row that adds the asset to the favorites, or takes a favorite out
fn favorite_label(app: &TextureCraftApp, entry: &ShelfEntry) -> &'static str {
    if crate::shelf_favorite::rank(&app.ui.shelf_favorites, entry).is_some() { tr("Remove from favorites") } else { tr("Add to favorites") }
}

fn text_width(ui: &Ui, text: &str) -> f32 {
    ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), FontId::proportional(TEXT_SIZE), Color32::WHITE).size().x)
}

/// The label column's width: the longest label and its gap
fn label_column(ui: &Ui, info: &[(&str, String)]) -> f32 {
    info.iter().map(|(label, _)| text_width(ui, label)).fold(0.0, f32::max) + LABEL_GAP
}

/// The widest row, the info block's values capped at [`MAX_VALUE_WIDTH`]
fn menu_width(ui: &Ui, info: &[(&str, String)], labels: &[&str]) -> f32 {
    let widest_value = info.iter().map(|(_, value)| text_width(ui, value)).fold(0.0, f32::max).min(MAX_VALUE_WIDTH);
    let widest_row = labels.iter().map(|label| text_width(ui, label)).fold(0.0, f32::max);
    TEXT_X + (label_column(ui, info) + widest_value).max(widest_row) + RIGHT_PADDING
}

/// The label (dim) and value rows, values `column` points after the labels; a value too wide is cut in the middle with "…"
fn info_block(ui: &mut Ui, width: f32, column: f32, info: &[(&str, String)], tokens: &Tokens) {
    let value_width = width - TEXT_X - column - RIGHT_PADDING;
    for (label, value) in info {
        let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
        let shown = cut_middle(ui, value, value_width);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{label} {value}")));
        let painter = ui.painter();
        painter.text(pos2(rect.left() + TEXT_X, rect.center().y), Align2::LEFT_CENTER, *label, FontId::proportional(TEXT_SIZE), tokens.text_dim);
        painter.text(pos2(rect.left() + TEXT_X + column, rect.center().y), Align2::LEFT_CENTER, shown, FontId::proportional(TEXT_SIZE), tokens.text);
    }
}

/// The Name row of an asset the user owns: a field in the value column that returns the new name once typing ends (Enter or a click elsewhere) with a changed, non-blank name. Escape keeps the old one
fn name_field(ui: &mut Ui, width: f32, column: f32, field_id: egui::Id, name: &str, tokens: &Tokens) -> Option<String> {
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_ROW_HEIGHT), Sense::hover());
    let label = tr("Name");
    ui.painter().text(pos2(rect.left() + TEXT_X, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(TEXT_SIZE), tokens.text_dim);

    // Check raw Escape even if the menu consumed it
    let escaped = ui.input(|input| input.raw.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })));
    if escaped {
        ui.data_mut(|data| data.remove::<String>(field_id));
    }
    let mut text = ui.data(|data| data.get_temp::<String>(field_id)).unwrap_or_else(|| name.to_owned());

    // The field's text lines up with the values below it
    let field = egui::Rect::from_min_max(
        pos2(rect.left() + TEXT_X + column - FIELD_TEXT_INSET, rect.center().y - FIELD_HEIGHT / 2.0),
        pos2(rect.right() - TEXT_X, rect.center().y + FIELD_HEIGHT / 2.0),
    );
    ui.painter().rect(field, CornerRadius::same(HOVER_ROUNDING), tokens.field, egui::Stroke::new(1.0, tokens.field_border), egui::StrokeKind::Inside);
    let edit = egui::TextEdit::singleline(&mut text)
        .id(field_id)
        .font(FontId::proportional(TEXT_SIZE))
        .text_color(tokens.text)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(FIELD_TEXT_INSET as i8, 0)))
        .align(Align2::LEFT_CENTER);
    let response = ui.new_child(egui::UiBuilder::new().max_rect(field.shrink(1.0))).add_sized(field.shrink(1.0).size(), edit);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));

    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(field_id, text));
        return None;
    }

    ui.data_mut(|data| data.remove::<String>(field_id));
    let typed = text.trim();
    (response.lost_focus() && !escaped && !typed.is_empty() && typed != name).then(|| typed.to_owned())
}

/// `text`, or its start and end around "…" so it fits `width` points
fn cut_middle(ui: &Ui, text: &str, width: f32) -> String {
    if text_width(ui, text) <= width {
        return text.to_owned();
    }

    let chars: Vec<char> = text.chars().collect();
    let mut kept = chars.len();
    loop {
        let head = kept.div_ceil(2);
        let tail = kept / 2;
        let cut: String = chars[..head].iter().chain(['…'].iter()).chain(chars[chars.len() - tail..].iter()).collect();
        if kept == 0 || text_width(ui, &cut) <= width {
            return cut;
        }

        kept -= 1;
    }
}

/// A hairline across the menu between groups of rows
fn separator(ui: &mut Ui, tokens: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), SEPARATOR_SPACE * 2.0 + 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, tokens.separator));
}

/// The file manager's own name for showing a file in it
fn show_in_folder_label() -> &'static str {
    if cfg!(target_os = "windows") {
        tr("Show in Explorer...")
    } else if cfg!(target_os = "macos") {
        tr("Show in Finder...")
    } else {
        tr("Show in file manager...")
    }
}

/// The Show in row: a submenu that opens the folder of the asset's file, greyed for an asset with no file on this computer
fn show_in(app: &mut TextureCraftApp, ui: &mut Ui, width: f32, entry: &ShelfEntry, tokens: &Tokens) {
    let file = match entry {
        ShelfEntry::SavedBrush { name, .. } => app.session.brush_presets.file_for(name),
        ShelfEntry::Image(name) => app.session.doc.project.resource_sources.get(name).map(|source| std::path::PathBuf::from(&source.path)),
        _ => None,
    };
    let folder = file.as_deref().and_then(std::path::Path::parent).filter(|folder| folder.is_dir()).map(std::path::Path::to_path_buf);

    // Reserved under the row, so the row stays highlighted while its submenu is open
    let open_highlight = ui.painter().add(egui::Shape::Noop);
    let response = row(ui, width, tr("Show in"), folder.is_some(), tokens);
    let caret_color = if folder.is_some() { tokens.text } else { tokens.text_dim };
    ui.painter().text(
        pos2(response.rect.right() - TEXT_X, response.rect.center().y),
        Align2::RIGHT_CENTER,
        egui_phosphor::regular::CARET_RIGHT,
        FontId::proportional(TEXT_SIZE),
        caret_color,
    );
    let Some(folder) = folder else {
        let reason = match (file.is_some(), entry) {
            (true, _) => n_("The folder of this asset's file is gone"),
            (false, ShelfEntry::Image(_)) => n_("Imported before this session: no file to show"),
            (false, ShelfEntry::SavedBrush { .. }) => n_("This brush isn't saved to a file"),
            (false, _) => n_("Built-in assets have no file to show"),
        };

        response.on_hover_text(tr(reason));
        return;
    };

    let submenu = egui::containers::menu::SubMenu::new().show(ui, &response, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let label = show_in_folder_label();
        let submenu_width = text_width(ui, label) + TEXT_X + RIGHT_PADDING;
        if row(ui, submenu_width, label, true, tokens).clicked() {
            open_folder(app, &folder);
            ui.close();
        }
    });
    if submenu.is_some() {
        let highlight = egui::Shape::rect_filled(response.rect.shrink2(vec2(HOVER_INSET, 0.0)), CornerRadius::same(HOVER_ROUNDING), tokens.button);
        ui.painter().set(open_highlight, highlight);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn open_folder(app: &mut TextureCraftApp, folder: &std::path::Path) {
    if let Err(error) = app.file_dialogs.show_folder(folder) {
        app.toast(crate::i18n::trf("Could not open the folder: {error}", &[("error", &error)]), true);
    }
}

#[cfg(target_arch = "wasm32")]
fn open_folder(app: &mut TextureCraftApp, _: &std::path::Path) {
    app.toast(tr("Showing files needs the desktop app"), true);
}

/// One menu row: a hover face inset from the sides, its text dim and unclickable when not `enabled`
fn row(ui: &mut Ui, width: f32, label: &str, enabled: bool, tokens: &Tokens) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if enabled && ui.data_mut(|data| data.remove_temp::<bool>(Id::unique(FOCUS_FIRST_ROW))).unwrap_or_default() {
        response.request_focus();
    }
    let painter = ui.painter();
    if enabled && (response.hovered() || response.has_focus()) {
        painter.rect_filled(rect.shrink2(vec2(HOVER_INSET, 0.0)), CornerRadius::same(HOVER_ROUNDING), tokens.button);
    }
    let color = if enabled { tokens.text } else { tokens.text_dim };
    painter.text(pos2(rect.left() + TEXT_X, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(TEXT_SIZE), color);
    if enabled { response } else { response.on_hover_cursor(egui::CursorIcon::NotAllowed) }
}

/// Asks where to save image resource `name`, then writes it there as imported
#[cfg(not(target_arch = "wasm32"))]
fn export_image(app: &mut TextureCraftApp, name: &str) {
    use crate::file_dialogs::{Bound, FileDialog};

    let extension = std::path::Path::new(name).extension().and_then(|extension| extension.to_str()).unwrap_or_default().to_owned();
    let name = name.to_owned();
    let dialog = FileDialog::save().filter(tr("Image"), &[extension.as_str()]).file_name(&name);
    app.ask_file(dialog, Bound::Project, move |app, paths| {
        if let Some(path) = paths.first() {
            app.run("resource.export", json!({"name": name, "path": path.to_string_lossy()}));
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn export_image(app: &mut TextureCraftApp, _: &str) {
    app.toast(tr("Exporting resources needs the desktop app"), true);
}

/// Asks where to save saved brush `name`, then writes it there as a brush preset file
#[cfg(not(target_arch = "wasm32"))]
fn export_brush(app: &mut TextureCraftApp, name: &str) {
    use crate::file_dialogs::{Bound, FileDialog};
    use texturecraft_engine::brush_preset_cmds::BRUSH_EXT;

    let name = name.to_owned();
    let dialog = FileDialog::save().filter(tr("Brush preset"), &[BRUSH_EXT]).file_name(format!("{name}.{BRUSH_EXT}"));
    app.ask_file(dialog, Bound::App, move |app, paths| {
        if let Some(path) = paths.first() {
            app.run("brush.preset.export", json!({"name": name, "path": path.to_string_lossy()}));
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn export_brush(app: &mut TextureCraftApp, _: &str) {
    app.toast(tr("Exporting resources needs the desktop app"), true);
}
