//! Window > Layouts: panel arrangements saved under a name and switched from the menu.
//!
//! A named layout holds the painting and the baking dock (which panels are shown, where, and
//! how big). Switching to one touches nothing else: not the mode, the zoom, the wireframes, the
//! windows or the shelf. The layouts persist with `UiState`, so they work in the browser too.

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
pub use crate::layout_dialog::LayoutDialog;
pub use crate::name_error::NameError;
pub use crate::named_layout::NamedLayout;
use crate::state::UiState;
use crate::theme::caption;
use crate::windows::dialog;

impl UiState {
    fn named_layout_index(&self, name: &str) -> Option<usize> {
        self.named_layouts.iter().position(|layout| layout.name == name)
    }

    /// Whether a layout of this name (surrounding spaces ignored) is saved
    pub fn has_named_layout(&self, name: &str) -> bool {
        self.named_layout_index(name.trim()).is_some()
    }

    /// Save the current panel arrangement under `name`, replacing a layout of that name; false for an empty name
    pub fn save_named_layout(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }

        let (painting, baking) = if self.dock_is_baking { (&self.inactive_mode_dock, &self.dock) } else { (&self.dock, &self.inactive_mode_dock) };
        let layout = NamedLayout { name: name.to_string(), painting: painting.clone(), baking: baking.clone() };
        match self.named_layout_index(name) {
            Some(index) => self.named_layouts[index] = layout,
            None => self.named_layouts.push(layout),
        }
        true
    }

    /// Switch to the saved layout's panels; the mode and every other setting stay. False when there is none of that name.
    pub fn apply_named_layout(&mut self, name: &str) -> bool {
        let Some(layout) = self.named_layout_index(name).map(|index| self.named_layouts[index].clone()) else { return false };

        let (current, other) = if self.dock_is_baking { (layout.baking, layout.painting) } else { (layout.painting, layout.baking) };
        self.dock = current;
        self.inactive_mode_dock = other;

        // A popup open against the rail shows a panel of the old arrangement
        self.rail_panel = None;
        true
    }

    /// Give a saved layout another name
    pub fn rename_named_layout(&mut self, from: &str, to: &str) -> Result<(), NameError> {
        let to = to.trim();
        if to.is_empty() {
            return Err(NameError::Empty);
        }

        if to != from && self.named_layout_index(to).is_some() {
            return Err(NameError::Taken);
        }

        if let Some(index) = self.named_layout_index(from) {
            self.named_layouts[index].name = to.to_string();
        }
        Ok(())
    }

    pub fn delete_named_layout(&mut self, name: &str) {
        self.named_layouts.retain(|layout| layout.name != name);
    }
}

/// One entry per saved layout, under the catalog's items of the Layouts submenu
pub(crate) fn entries(app: &mut TextureCraftApp, ui: &mut egui::Ui) {
    if app.ui.named_layouts.is_empty() {
        return;
    }

    ui.separator();
    for name in app.ui.named_layouts.iter().map(|layout| layout.name.clone()).collect::<Vec<_>>() {
        if ui.button(&name).clicked() {
            app.ui.apply_named_layout(&name);
            ui.close();
        }
    }
}

/// The save, overwrite and manage dialogs
pub(crate) fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    forget_finished_focus(app, ctx);
    let Some(shown) = app.ui.layout_dialog.clone() else { return };

    let mut open = true;
    let next = match shown {
        LayoutDialog::Save(name) => save_dialog(app, ctx, &mut open, name),
        LayoutDialog::Overwrite(name) => overwrite_dialog(app, ctx, &mut open, name),
        LayoutDialog::Manage { renaming, deleting } => manage_dialog(app, ctx, &mut open, renaming, deleting),
    };
    app.ui.layout_dialog = if open { next } else { None };
    forget_finished_focus(app, ctx);
}

/// The next Save or rename focuses its field again. Also runs while no dialog is shown, since
/// loading or resetting the layout closes a dialog from outside.
fn forget_finished_focus(app: &TextureCraftApp, ctx: &egui::Context) {
    ctx.data_mut(|data| {
        if !matches!(app.ui.layout_dialog, Some(LayoutDialog::Save(_))) {
            data.remove_temp::<String>(egui::Id::unique(SAVE_FOCUS));
        }
        if !matches!(app.ui.layout_dialog, Some(LayoutDialog::Manage { renaming: Some(_), .. })) {
            data.remove_temp::<String>(egui::Id::unique(RENAME_FOCUS));
        }
    });
}

/// How much of the window height the list of saved layouts may take
const LIST_SCREEN_FRACTION: f32 = 0.5;

/// Memory keys of the text fields that were already focused for the current edit
const SAVE_FOCUS: &str = "named_layout_save_focused";
const RENAME_FOCUS: &str = "named_layout_rename_focused";

/// Give `edit` the keyboard once per edit (`what` names it), when it starts. Asking every frame
/// would take the focus back from the buttons after Tab.
fn focus_once(ui: &egui::Ui, edit: &egui::Response, key: &'static str, what: String) {
    let id = egui::Id::unique(key);
    if ui.data(|data| data.get_temp::<String>(id)).as_deref() != Some(what.as_str()) {
        edit.request_focus();
        ui.data_mut(|data| data.insert_temp(id, what));
    }
}

fn save_dialog(app: &mut TextureCraftApp, ctx: &egui::Context, open: &mut bool, mut name: String) -> Option<LayoutDialog> {
    let mut done = None;
    dialog("save_layout", tr("Save layout"), open).show(ctx, |ui| {
        ui.label(tr("Name of the layout"));
        let edit = ui.add(egui::TextEdit::singleline(&mut name).desired_width(240.0));
        focus_once(ui, &edit, SAVE_FOCUS, String::new());
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let valid = !name.trim().is_empty();
            if ui.add_enabled(valid, egui::Button::new(tr("Save"))).clicked() || (enter && valid) {
                done = Some(true);
            }
            if ui.button(tr("Cancel")).clicked() {
                done = Some(false);
            }
        });
    });

    match done {
        Some(true) if app.ui.has_named_layout(&name) => Some(LayoutDialog::Overwrite(name.trim().to_string())),
        Some(true) => {
            app.ui.save_named_layout(&name);
            app.toast(trf("Saved layout {name}", &[("name", &name.trim())]), false);
            None
        }
        Some(false) => None,
        None => Some(LayoutDialog::Save(name)),
    }
}

fn overwrite_dialog(app: &mut TextureCraftApp, ctx: &egui::Context, open: &mut bool, name: String) -> Option<LayoutDialog> {
    let mut answer = None;
    dialog("overwrite_layout", tr("Replace layout"), open).show(ctx, |ui| {
        ui.label(trf("A layout named {name} already exists. Replace it?", &[("name", &name)]));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(tr("Replace")).clicked() {
                answer = Some(true);
            }
            if ui.button(tr("Cancel")).clicked() {
                answer = Some(false);
            }
        });
    });

    match answer {
        Some(true) => {
            app.ui.save_named_layout(&name);
            app.toast(trf("Saved layout {name}", &[("name", &name)]), false);
            None
        }
        Some(false) => None,
        None => Some(LayoutDialog::Overwrite(name)),
    }
}

fn manage_dialog(
    app: &mut TextureCraftApp,
    ctx: &egui::Context,
    open: &mut bool,
    mut renaming: Option<(String, String)>,
    mut deleting: Option<String>,
) -> Option<LayoutDialog> {
    let mut finished = false;
    dialog("manage_layouts", tr("Manage layouts"), open).show(ctx, |ui| {
        ui.set_min_width(320.0);
        if let Some(name) = deleting.clone() {
            ui.label(trf("Delete the layout {name}?", &[("name", &name)]));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Delete")).clicked() {
                    app.ui.delete_named_layout(&name);
                    deleting = None;
                }
                if ui.button(tr("Cancel")).clicked() {
                    deleting = None;
                }
            });
            return;
        }

        if app.ui.named_layouts.is_empty() {
            ui.label(caption(tr("No saved layouts")));
        }

        // Many layouts scroll; Close stays below the list
        let list_height = ui.ctx().content_rect().height() * LIST_SCREEN_FRACTION;
        egui::ScrollArea::vertical().max_height(list_height).auto_shrink([false, true]).show(ui, |ui| {
            for name in app.ui.named_layouts.iter().map(|layout| layout.name.clone()).collect::<Vec<_>>() {
                ui.horizontal(|ui| match renaming.clone().filter(|(from, _)| *from == name) {
                    Some((from, mut to)) => {
                        let edit = ui.add(egui::TextEdit::singleline(&mut to).desired_width(180.0));
                        focus_once(ui, &edit, RENAME_FOCUS, from.clone());
                        let taken = app.ui.named_layouts.iter().any(|layout| layout.name == to.trim() && layout.name != from);
                        let valid = !to.trim().is_empty() && !taken;
                        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let mut keep = true;
                        if ui.add_enabled(valid, egui::Button::new(tr("Rename"))).clicked() || (enter && valid) {
                            let _ = app.ui.rename_named_layout(&from, &to);
                            keep = false;
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            keep = false;
                        }
                        if taken {
                            ui.label(caption(tr("That name is taken")));
                        }
                        renaming = keep.then_some((from, to));
                    }
                    None => {
                        ui.label(&name);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(tr("Delete")).clicked() {
                                deleting = Some(name.clone());
                            }
                            if ui.button(tr("Rename")).clicked() {
                                renaming = Some((name.clone(), name.clone()));
                            }
                        });
                    }
                });
            }
        });
        ui.add_space(8.0);
        if ui.button(tr("Close")).clicked() {
            finished = true;
        }
    });

    (!finished).then_some(LayoutDialog::Manage { renaming, deleting })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docking::Panel;
    use crate::state::{Mode, ViewLayout};

    fn closed(panel: Panel) -> UiState {
        let mut ui = UiState::default();
        ui.dock.close(panel);
        ui
    }

    #[test]
    fn switching_restores_the_panels_and_keeps_zoom_mode_and_wireframes() {
        let mut ui = closed(Panel::Assets);
        assert!(ui.save_named_layout("Sparse"));

        ui.dock.focus(Panel::Assets);
        ui.dock.close(Panel::Layers);
        ui.uv_zoom = 3.0;
        ui.uv_pan = [0.25, 0.5];
        ui.uv_wireframe = false;
        ui.wireframe_3d = true;
        ui.layout = ViewLayout::Only3D;
        ui.split_ratio = 0.2;
        ui.shelf_filter = "rust".into();
        let before = ui.clone();

        assert!(ui.apply_named_layout("Sparse"));

        assert!(!ui.dock.is_open(Panel::Assets) && ui.dock.is_open(Panel::Layers));
        let expected = UiState { dock: ui.dock.clone(), inactive_mode_dock: ui.inactive_mode_dock.clone(), ..before };
        assert_eq!(ui, expected, "only the docks changed");
    }

    #[test]
    fn both_mode_docks_are_saved_and_the_current_mode_stays() {
        let mut ui = UiState::default();
        ui.dock.close(Panel::Assets);
        ui.save_named_layout("Both");

        // Saved while painting, applied while baking: each dock lands in its own slot
        let mut baking = UiState { mode: Mode::Baking, dock_is_baking: true, ..UiState::default() };
        baking.inactive_mode_dock = UiState::default().dock;
        baking.named_layouts = ui.named_layouts.clone();
        baking.apply_named_layout("Both");

        assert_eq!(baking.mode, Mode::Baking);
        assert!(baking.dock_is_baking);
        assert!(baking.dock.is_open(Panel::MeshMapBakers) && !baking.dock.is_open(Panel::Layers), "the baking dock is the saved baking dock");
        assert!(
            baking.inactive_mode_dock.is_open(Panel::Layers) && !baking.inactive_mode_dock.is_open(Panel::Assets),
            "the painting dock is the saved painting dock"
        );
    }

    #[test]
    fn saving_under_a_taken_name_replaces_it_and_an_empty_name_saves_nothing() {
        let mut ui = UiState::default();
        assert!(!ui.save_named_layout("   "));
        ui.save_named_layout(" Mine ");
        ui.dock.close(Panel::Assets);
        ui.save_named_layout("Mine");

        assert_eq!(ui.named_layouts.len(), 1);
        assert_eq!(ui.named_layouts[0].name, "Mine");
        assert!(!ui.named_layouts[0].painting.is_open(Panel::Assets));
        assert!(ui.has_named_layout("Mine ") && !ui.apply_named_layout("Other"));
    }

    #[test]
    fn rename_and_delete() {
        let mut ui = UiState::default();
        ui.save_named_layout("A");
        ui.save_named_layout("B");

        assert_eq!(ui.rename_named_layout("A", "B"), Err(NameError::Taken));
        assert_eq!(ui.rename_named_layout("A", "  "), Err(NameError::Empty));
        assert_eq!(ui.rename_named_layout("A", "A"), Ok(()));
        assert_eq!(ui.rename_named_layout("A", " C "), Ok(()));
        let names: Vec<_> = ui.named_layouts.iter().map(|layout| layout.name.as_str()).collect();
        assert_eq!(names, ["C", "B"], "a renamed layout keeps its place");

        ui.delete_named_layout("C");
        assert_eq!(ui.named_layouts.len(), 1);
        assert!(!ui.apply_named_layout("C"));
    }

    #[test]
    fn named_layouts_persist_and_older_saved_state_loads_without_them() {
        let mut ui = closed(Panel::Assets);
        ui.save_named_layout("Sparse");
        let back: UiState = serde_json::from_str(&serde_json::to_string(&ui).unwrap()).unwrap();
        assert_eq!(back.named_layouts, ui.named_layouts);

        let mut old = serde_json::to_value(UiState::default()).unwrap();
        assert!(old.as_object_mut().unwrap().remove("named_layouts").is_some());
        let loaded: UiState = serde_json::from_value(old).unwrap();
        assert!(loaded.named_layouts.is_empty());
    }

    #[test]
    fn reset_and_loading_a_layout_file_keep_the_named_layouts() {
        let mut ui = UiState::default();
        ui.save_named_layout("Mine");
        ui.adopt_layout(UiState::default());
        assert!(ui.has_named_layout("Mine"));
    }
}
