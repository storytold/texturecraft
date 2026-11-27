//! Auto-rebake: a texture set's auto-rebake map is baked again in the background once what it is baked from has stopped changing

use texturecraft_engine::core::{BakeSource, BakedMap};

use crate::{BakeJob, TextureCraftApp, bake_report, state};

/// How long what an out-of-date auto-rebake map is baked from must stay the same before the rebake starts
const SETTLE_SECONDS: f64 = 0.5;

/// An out-of-date auto-rebake map: set, map, what a bake reads now, and the newest stroke when only painting changed it
pub type Due = (usize, BakedMap, BakeSource, Option<u64>);

/// What the app has seen of the out-of-date auto-rebake maps
#[derive(Default)]
pub struct AutoRebake {
    /// The project these records are of: another project starts with none
    project: u64,
    /// The due maps and the frame time they were first seen as they are
    pending: Option<(Vec<Due>, f64)>,
    /// Rebakes started that are still due: one that failed or was canceled is not tried again until something changes
    tried: Vec<Due>,
}

/// Start, or restart, the rebake of an out-of-date auto-rebake map; only in Bake mode, where its settings are changed
pub(crate) fn poll(app: &mut TextureCraftApp, ctx: &egui::Context) {
    if app.auto_rebake.project != app.session.project_id {
        app.auto_rebake = AutoRebake { project: app.session.project_id, ..AutoRebake::default() };
    }

    if app.ui.mode != state::Mode::Baking {
        app.auto_rebake.pending = None;
        return;
    }

    let stale = app.session.auto_rebake_due().into_iter().map(|(set, map, source)| (set, map, source, None));
    let painted = app.session.auto_rebake_painted().into_iter().map(|(set, map, source, stroke)| (set, map, source, Some(stroke)));
    let due: Vec<Due> = stale.chain(painted).collect();
    if let Some(job) = &app.bake {
        // Newest wins: a rebake whose map changed again while it ran starts over with the new settings
        if job.rebake.as_ref().is_some_and(|rebake| !due.contains(rebake)) {
            app.cancel_bake();
        }
        return;
    }

    app.auto_rebake.tried.retain(|tried| due.contains(tried));
    let waiting: Vec<Due> = due.into_iter().filter(|due| !app.auto_rebake.tried.contains(due)).collect();
    if waiting.is_empty() {
        app.auto_rebake.pending = None;
        return;
    }

    let now = app.last_time;
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(SETTLE_SECONDS));

    // A held press (a slider dragged and paused) is still editing: the wait starts once it is let go
    if pointer_held(ctx, app.docks.floating_windows()) {
        app.auto_rebake.pending = Some((waiting, now));
        return;
    }

    match &app.auto_rebake.pending {
        Some((seen, since)) if *seen == waiting && now - since >= SETTLE_SECONDS => {}
        Some((seen, _)) if *seen == waiting => return,
        _ => {
            app.auto_rebake.pending = Some((waiting, now));
            return;
        }
    }

    app.auto_rebake.pending = None;
    if let Some(rebake) = waiting.first().copied() {
        app.auto_rebake.tried.push(rebake);
        start(app, rebake);
    }
}

/// Whether a mouse button is down in the main window or in one of the floating panels' own `windows`
fn pointer_held(ctx: &egui::Context, windows: &[egui::ViewportId]) -> bool {
    ctx.input(|input| input.pointer.any_down()) || windows.iter().any(|window| ctx.input_for(*window, |input| input.pointer.any_down()))
}

/// Rebake only the map of `rebake`'s texture set in the background, shown in the Baking Log like any bake
fn start(app: &mut TextureCraftApp, rebake: Due) {
    let (set, map, ..) = rebake;

    // Restore the previous view before the rebake takes it over
    app.live_preview.release(&mut app.session);
    crate::id_color_pick::IdColorPick::end(app);

    // The progress footer names the maps this rebake makes, not every checked map of the set
    let maps = app.session.auto_rebake_maps(set, map);
    app.session.bake_preview_begin(vec![(set, maps.clone())]);
    app.session.begin_map_bake_log(&[set], &maps);
    app.bake_report = bake_report::BakeReport { auto_rebake: true, ..bake_report::BakeReport::started(app.last_time) };
    app.bake = Some(BakeJob { queue: vec![set], current: None, total: 1, mesh_id: app.session.mesh_id, only: None, rebake: Some(rebake) });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether a press is held, seen from the main window after a button went down in a floating panel's window (`window_listed`)
    fn held_after_a_press_in_a_floating_window(window_listed: bool) -> bool {
        use std::{cell::RefCell, rc::Rc};

        let window = egui::ViewportId::from_hash_of("floating panel");
        let context = egui::Context::default();
        context.set_embed_viewports(false);

        // Runs the window's pass inside its parent's, as eframe does
        let arriving = Rc::new(RefCell::new(Vec::new()));
        let queued = Rc::clone(&arriving);
        egui::Context::set_immediate_viewport_renderer(move |context, viewport| {
            let mut input = egui::RawInput { viewport_id: viewport.ids.this, events: queued.take(), ..Default::default() };
            input.viewports.insert(viewport.ids.this, egui::ViewportInfo { parent: Some(viewport.ids.parent), ..Default::default() });
            let mut output = context.run_ui(input, viewport.viewport_ui_cb);
            output.textures_delta.clear();
        });

        let at = egui::pos2(20.0, 20.0);
        let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE };
        *arriving.borrow_mut() = vec![egui::Event::PointerMoved(at), press];
        let windows: &[egui::ViewportId] = if window_listed { &[window] } else { &[] };
        let mut held = false;

        // The first pass hands the press to the window after the hold was read; the second reads it
        for _ in 0..2 {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                held = pointer_held(ui.ctx(), windows);
                ui.ctx().show_viewport_immediate(window, egui::ViewportBuilder::default(), |_, _| {});
            });
            output.textures_delta.clear();
        }
        held
    }

    #[test]
    fn a_press_held_in_a_floating_panels_window_counts_as_held() {
        assert!(held_after_a_press_in_a_floating_window(true), "the slider dragged in a floating panel is still editing");
        assert!(!held_after_a_press_in_a_floating_window(false), "the main window's own pointer is up");
    }
}
