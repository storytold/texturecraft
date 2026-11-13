//! Problems the next bake would have, listed above the bake bar before Bake is pressed

use std::sync::{Arc, Weak};

use egui_phosphor::regular as icon;
use texturecraft_engine::Session;
use texturecraft_engine::geom::Mesh;

use crate::TextureCraftApp;
use crate::i18n::trf;
use crate::theme::Tokens;

/// Share of a texture set's UV layout that may overlap before it is listed (as mesh import warns)
const UV_OVERLAP_WARNING: f32 = 0.01;

/// How many part names a warning spells out before "and N more"
const NAMES_SHOWN: usize = 3;

/// The warnings' gap above the bake bar
const WARNINGS_GAP: f32 = 8.0;

/// The warnings above bake mode's bake bar in the 3D view `viewport`, until a bake starts
pub(crate) fn bake_warnings(app: &TextureCraftApp, ctx: &egui::Context, viewport: egui::Rect) {
    if app.bake_progress().is_some() {
        return;
    }

    let warnings = cached_warnings(ctx, &app.session);
    if warnings.is_empty() {
        return;
    }

    let tokens = Tokens::get(ctx);
    let above_bar = super::BAKE_BAR_MARGIN + ctx.global_style().spacing.interact_size.y + WARNINGS_GAP;
    egui::Area::new(egui::Id::unique("bake_warnings")).constrain_to(viewport).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -above_bar)).show(
        ctx,
        |ui| {
            egui::Frame::popup(ui.style()).fill(tokens.panel).show(ui, |ui| {
                for warning in warnings.iter() {
                    ui.label(egui::RichText::new(format!("{}  {warning}", icon::WARNING)).color(tokens.warning));
                }
            });
        },
    );
}

/// The warnings for the texture sets the next bake includes, worked out again only when what they read changes
pub(crate) fn cached_warnings(ctx: &egui::Context, session: &Session) -> Arc<Vec<String>> {
    let id = egui::Id::unique("bake_check_warnings");
    let key = inputs_key(session);
    // A handle, not an address: a rebuilt high poly could reuse a dropped one's address
    let high = session.bake_meshes.merged_high().map_or_else(Weak::new, |high| Arc::downgrade(&high.mesh));
    if let Some((cached_key, cached_high, cached)) = ctx.data(|data| data.get_temp::<(String, Weak<Mesh>, Arc<Vec<String>>)>(id))
        && cached_key == key
        && cached_high.ptr_eq(&high)
    {
        return cached;
    }

    let fresh = Arc::new(warnings(session));
    ctx.data_mut(|data| data.insert_temp(id, (key, high, fresh.clone())));
    fresh
}

/// Everything the warnings read or show apart from the high poly (cached as a handle): the low poly, the UI language, and per included set its name, material, high-poly settings, UV tile resolutions and the tiles left out of the bake
fn inputs_key(session: &Session) -> String {
    let sets: Vec<_> = baked_sets(session)
        .filter_map(|index| {
            let set = session.doc.project.texture_sets.get(index)?;
            Some((index, &set.name, set.material, &set.bake.high_poly, &set.tile_resolutions, &set.bake_excluded_tiles))
        })
        .collect();

    format!("{}|{}|{sets:?}", session.mesh_id, crate::i18n::current().code)
}

/// The texture sets the next bake includes
fn baked_sets(session: &Session) -> impl Iterator<Item = usize> + '_ {
    session.doc.project.texture_sets.iter().enumerate().filter(|(_, set)| set.bake.common.is_enabled && set.has_enabled_bake_tiles()).map(|(index, _)| index)
}

/// Overlapping UVs per included set, its low-poly parts "By Mesh Name" pairs with nothing, then the high-poly parts no included set bakes from
pub(crate) fn warnings(session: &Session) -> Vec<String> {
    let mut lines = Vec::new();
    let included: Vec<usize> = baked_sets(session).collect();
    for &index in &included {
        let Some(set) = session.doc.project.texture_sets.get(index) else { continue };

        let overlap = session.bake_uv_overlap(index).unwrap_or(0.0);
        if overlap > UV_OVERLAP_WARNING {
            let percent = format!("{:.1}", overlap * 100.0);
            lines.push(trf(
                "{set}: {percent}% of the UV layout overlaps, so those texels bake one place for several",
                &[("set", &set.name), ("percent", &percent)],
            ));
        }

        let Ok(Some(pairs)) = session.bake_name_pairs(index) else { continue };

        let unpaired_low: Vec<&str> = pairs.iter().filter(|pair| pair.high.is_none()).filter_map(|pair| pair.low.as_deref()).collect();
        if !unpaired_low.is_empty() {
            lines.push(trf("{set}: no high poly part matches {names}", &[("set", &set.name), ("names", &listed(&unpaired_low))]));
        }
    }

    let unpaired_high = session.bake_unpaired_high_parts(&included).unwrap_or_default();
    if !unpaired_high.is_empty() {
        let names: Vec<&str> = unpaired_high.iter().map(String::as_str).collect();
        lines.push(trf("No low poly part matches {names}", &[("names", &listed(&names))]));
    }

    lines
}

/// `names` joined, the first few spelled out and the rest counted
fn listed(names: &[&str]) -> String {
    let shown = names.iter().take(NAMES_SHOWN).copied().collect::<Vec<_>>().join(", ");
    match names.len().checked_sub(NAMES_SHOWN).filter(|more| *more > 0) {
        Some(more) => trf("{names} and {count} more", &[("names", &shown), ("count", &more)]),
        None => shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_engine::core::MeshMatchMode;

    /// Part `index` named `name` (an OBJ object): a quad at x offset `x` whose UVs span `u` to `u` + 0.2
    fn quad(name: &str, index: usize, x: f32, u: f32) -> String {
        let first = index * 4 + 1;
        format!(
            "o {name}\nv {x} 0 0\nv {} 0 0\nv {} 1 0\nv {x} 1 0\nvt {u} 0\nvt {} 0\nvt {} 1\nvt {u} 1\nf {first}/{first} {}/{} {}/{} {}/{}\n",
            x + 1.0,
            x + 1.0,
            u + 0.2,
            u + 0.2,
            first + 1,
            first + 1,
            first + 2,
            first + 2,
            first + 3,
            first + 3
        )
    }

    fn session(overlapping: bool) -> Session {
        let names = ["Box_low", "Lid_low", "Bolt_low", "Nut_low", "Pin_low"];
        let obj: String =
            names.iter().enumerate().map(|(index, name)| quad(name, index, index as f32 * 2.0, if overlapping { 0.0 } else { index as f32 * 0.2 })).collect();
        Session::from_mesh_bytes("parts.obj", obj.as_bytes(), 128, "parts").unwrap()
    }

    #[test]
    fn a_clean_setup_lists_nothing() {
        let mut session = session(false);
        assert_eq!(warnings(&session), Vec::<String>::new());
        session.add_high_poly_bytes("high.obj", b"o Box_high\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
        assert_eq!(warnings(&session), Vec::<String>::new(), "\"Always\" pairs every part");
    }

    #[test]
    fn renaming_a_set_refreshes_the_cached_warnings() {
        let mut session = session(true);
        let ctx = egui::Context::default();
        let before = cached_warnings(&ctx, &session);
        assert!(before.iter().any(|line| line.starts_with(&session.doc.project.texture_sets[0].name)), "{before:?}");

        session.doc.project.texture_sets[0].name = "Renamed".into();
        let after = cached_warnings(&ctx, &session);
        assert!(after.iter().any(|line| line.starts_with("Renamed:")), "{after:?}");
    }

    #[test]
    fn swapping_the_high_poly_refreshes_the_cached_warnings() {
        let mut session = session(false);
        session.doc.project.texture_sets[0].bake.high_poly.match_mode = MeshMatchMode::ByMeshName;
        session.add_high_poly_bytes("spare.obj", b"o Spare_high\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
        let ctx = egui::Context::default();
        assert!(cached_warnings(&ctx, &session).contains(&"No low poly part matches Spare_high".to_owned()));

        // The new high poly's mesh may take the dropped one's address
        drop(session.remove_high_poly(0).unwrap());
        session.add_high_poly_bytes("box.obj", b"o Box_high\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
        let after = cached_warnings(&ctx, &session);
        assert!(!after.iter().any(|line| line.contains("Spare_high")), "{after:?}");
    }

    #[test]
    fn leaving_the_overlapping_tile_out_refreshes_the_cached_warnings() {
        // One quad in tile 1001, two on the same UVs in tile 1002
        let obj = [quad("First_low", 0, 0.0, 0.0), quad("Second_low", 1, 2.0, 1.0), quad("Third_low", 2, 4.0, 1.0)].concat();
        let mut session = Session::from_mesh_bytes("tiles.obj", obj.as_bytes(), 128, "tiles").unwrap();
        let ctx = egui::Context::default();
        assert_eq!(cached_warnings(&ctx, &session).len(), 1, "tile 1002 overlaps");

        session.doc.project.texture_sets[0].bake_excluded_tiles.insert(1002);
        assert_eq!(*cached_warnings(&ctx, &session), Vec::<String>::new(), "tile 1001 alone has no overlap");
    }

    #[test]
    fn overlapping_uvs_and_unpaired_parts_are_listed() {
        let mut session = session(true);
        session
            .add_high_poly_bytes("high.obj", b"o Box_high\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\no Spare_high\nv 2 0 0\nv 3 0 0\nv 2 1 0\nf 4 5 6\n".to_vec())
            .unwrap();
        session.doc.project.texture_sets[0].bake.high_poly.match_mode = MeshMatchMode::ByMeshName;
        let set = session.doc.project.texture_sets[0].name.clone();

        assert_eq!(
            warnings(&session),
            [
                format!("{set}: 100.0% of the UV layout overlaps, so those texels bake one place for several"),
                format!("{set}: no high poly part matches Lid_low, Bolt_low, Nut_low and 1 more"),
                "No low poly part matches Spare_high".to_owned(),
            ]
        );

        session.doc.project.texture_sets[0].bake.common.is_enabled = false;
        assert_eq!(warnings(&session), Vec::<String>::new(), "a set the bake leaves out is not checked");
    }
}
