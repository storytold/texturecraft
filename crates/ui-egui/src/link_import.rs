//! The first procedural bake can take as long as a re-cook. Keep the native UI responsive.
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::TextureCraftApp;
use serde_json::{Value, json};
use texturecraft_engine::link_cmds::{CookedMaterial, cook_link_file};
use texturecraft_engine::materials::Place;

type ImportedMaterial = (Arc<Vec<u8>>, &'static str, CookedMaterial);

pub(super) struct LinkImport {
    project: u64,
    mesh: u64,
    canceled: Arc<AtomicBool>,
    set: usize,
    resolution: u32,
    place: Place,
    result: Receiver<texturecraft_engine::Result<ImportedMaterial>>,
}

impl Drop for LinkImport {
    fn drop(&mut self) {
        self.canceled.store(true, Ordering::Relaxed);
    }
}

impl TextureCraftApp {
    pub(super) fn start_link_import(&mut self, path: &str, params: &Value) -> Option<Value> {
        if !self.link_imports.is_empty() || !self.link_cooks.is_empty() {
            self.toast("Another MaterialCraft cook is still running", true);
            return None;
        }

        let set = self.session.ts();
        let texture_set = match self.session.doc.project.texture_set(set) {
            Ok(set) => set,
            Err(error) => {
                self.toast(error.to_string(), true);
                return None;
            }
        };

        // The artist can select a different layer while this import cooks
        let place = match Place::from_params(params) {
            Place::Selection => self.session.selected_layer().map_or_else(
                || texture_set.stack.last().map_or(Place::Index(0), |target| Place::Above(*target, false)),
                |target| Place::Above(target, self.session.doc.project.layer(target).is_ok_and(|layer| layer.is_group())),
            ),
            explicit => explicit,
        };
        let resolution = texture_set.resolution;
        let baked = self.session.baked(set).cloned();
        let path = PathBuf::from(path);
        let canceled = Arc::new(AtomicBool::new(false));
        let worker_canceled = Arc::clone(&canceled);
        let budget = self.session.link_memory_budget();
        let rx = super::spawn_worker("texturecraft-material-import", move || {
            let (source, cooked) = cook_link_file(&path, resolution, baked, budget, worker_canceled)?;

            // A pending installation only needs its source package and cooked outputs
            Ok((source.package, source.source_ext, cooked))
        });
        self.link_imports.push(LinkImport { project: self.session.project_id, mesh: self.session.mesh_id, canceled, set, resolution, place, result: rx });
        self.toast("Cooking MaterialCraft material", false);
        Some(json!({"pending":true}))
    }

    pub(super) fn poll_link_imports(&mut self, ctx: &egui::Context) {
        if self.link_imports.is_empty() {
            return;
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        if self.session.stroke_active() {
            return;
        }

        for mut job in std::mem::take(&mut self.link_imports) {
            if job.project != self.session.project_id
                || job.mesh != self.session.mesh_id
                || !self.session.doc.project.texture_set(job.set).is_ok_and(|set| set.resolution == job.resolution)
            {
                self.toast("Material import canceled because the target texture set changed", true);
                continue;
            }

            if let Place::Above(target, _) | Place::Below(target) = job.place
                && self.session.doc.project.locate(target).is_none_or(|(set, _, _)| set != job.set)
            {
                self.toast("Material import canceled because the target layer changed", true);
                continue;
            }

            let result = match job.result.try_recv() {
                Err(TryRecvError::Empty) => {
                    self.link_imports.push(job);
                    continue;
                }
                Err(TryRecvError::Disconnected) => Err(texturecraft_engine::EngineError::Other("the material import stopped".into())),
                Ok(result) => result,
            };

            match result.and_then(|(package, source_ext, cooked)| {
                self.session.add_cooked_material_to_set(&cooked, package, source_ext, job.set, std::mem::replace(&mut job.place, Place::Selection))
            }) {
                Ok(_) => self.toast("Material imported", false),
                Err(error) => self.toast(error.to_string(), true),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use texturecraft_engine::{Session, execute};

    #[test]
    fn pending_import_keeps_its_original_layer_position() {
        let Some(cli) = std::env::var_os("MATERIALCRAFT_CLI") else { return };
        let path = std::env::temp_dir().join(format!("texturecraft-pending-import-{}.mcraft", std::process::id()));
        let source = std::process::Command::new(cli).args(["new", "pbr"]).arg(&path).args(["--size", "32"]).output().unwrap();
        assert!(source.status.success());
        let session = Session::new_project("Paint", texturecraft_engine::geom::primitives::cube(1.0), "builtin:cube", None, 128).unwrap();
        let context = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut app = TextureCraftApp::new(&context, session);
        execute(&mut app.session, "layer.add.paint", &json!({"name":"Drop target"})).unwrap();
        let target = app.session.selected_layer().unwrap();

        let pixels = Arc::new(vec![0.25; 128 * 128]);
        let replaced = Arc::downgrade(&pixels);
        let baked = texturecraft_engine::compose::BakedMaps { resolution: 128, maps: [(texturecraft_engine::core::BakedMap::Curvature, pixels)].into() };
        app.session.doc.baked.insert(0, baked);

        app.start_link_import(path.to_str().unwrap(), &Value::Null).unwrap();
        app.session.doc.baked.remove(&0);
        execute(&mut app.session, "layer.add.paint", &json!({"name":"Later selection"})).unwrap();
        let later = app.session.selected_layer().unwrap();
        let job = app.link_imports.first_mut().unwrap();
        let result = job.result.recv_timeout(Duration::from_secs(60)).unwrap().unwrap();
        assert!(replaced.upgrade().is_none(), "a pending import retained replaced bake pixels");

        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Ok(result)).unwrap();
        job.result = receiver;
        app.poll_link_imports(&context.egui_ctx);
        std::fs::remove_file(path).unwrap();

        let imported = app.session.selected_layer().unwrap();
        let stack = &app.session.doc.project.texture_sets[0].stack;
        assert!(stack.ends_with(&[target, imported, later]), "{stack:?}");
    }
}
