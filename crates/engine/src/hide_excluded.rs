//! Hide/ignore excluded geometry: while on, the mesh parts the selected layer's geometry mask
//! leaves out are not drawn, and picking and strokes skip them, so paint reaches the parts
//! behind (`viewport.hideExcluded`).

use std::sync::Arc;

use serde_json::{Value, json};
use texturecraft_geom::glam::Vec2;
use texturecraft_geom::{Bvh, Mesh, surface_at};
use texturecraft_paint::SurfaceHit;

use crate::commands::{CommandSpec, always, boolp};
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "viewport.hideExcluded",
        "Hide/ignore excluded geometry",
        ["Viewport"],
        Some("Alt+H"),
        "{enabled?: bool} — toggles when omitted; hides the parts the selected layer's geometry mask excludes, and strokes and picking skip them",
        always,
        toggle
    )]
}

fn toggle(session: &mut Session, params: &Value) -> Result<Value> {
    session.cancel_stroke();
    session.hide_excluded = boolp(params, "enabled").unwrap_or(!session.hide_excluded);
    session.revision = session.revision.wrapping_add(1);
    Ok(json!({ "enabled": session.hide_excluded, "excludedParts": session.excluded_part_names() }))
}

impl Session {
    /// Per mesh part (submesh): whether the selected layer's geometry mask excludes it and the
    /// view hides it. Empty while the toggle is off or the layer shows on every part.
    pub fn excluded_parts(&self) -> Vec<bool> {
        if !self.hide_excluded {
            return Vec::new();
        }

        let Some(layer) = self.selected_layer() else { return Vec::new() };
        let Some(mask) = self.doc.project.layers.get(&layer).and_then(|l| l.geometry_mask.as_ref()) else { return Vec::new() };
        let ts = self.doc.project.locate(layer).map_or(self.ts(), |(ts, _, _)| ts);
        let Some(material) = self.doc.project.texture_sets.get(ts).map(|t| t.material) else { return Vec::new() };
        let excluded: Vec<bool> = self.mesh.submeshes.iter().map(|sm| sm.material == material && !mask.contains(&sm.name)).collect();
        if excluded.contains(&true) { excluded } else { Vec::new() }
    }

    fn excluded_part_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for (sm, _) in self.mesh.submeshes.iter().zip(self.excluded_parts()).filter(|(_, out)| *out) {
            if !names.contains(&sm.name) {
                names.push(sm.name.clone());
            }
        }
        names
    }

    /// Index ranges (first index, end) of the excluded parts.
    pub(crate) fn excluded_ranges(&self) -> Vec<(usize, usize)> {
        let excluded = self.excluded_parts();
        self.mesh.submeshes.iter().zip(excluded).filter(|(_, out)| *out).map(|(sm, _)| (sm.start as usize, sm.start as usize + sm.count as usize)).collect()
    }

    /// The mesh and its BVH without the excluded parts (their triangles collapse to a point, so
    /// triangle numbers stay the same), or None when nothing is excluded.
    pub(crate) fn paint_through_geometry(&self) -> Option<Arc<(Mesh, Bvh)>> {
        let ranges = self.excluded_ranges();
        if ranges.is_empty() {
            return None;
        }

        let mut mesh = (*self.mesh).clone();
        for (start, end) in ranges {
            for corners in mesh.indices.get_mut(start..end)?.as_chunks_mut::<3>().0 {
                *corners = [corners[0]; 3];
            }
        }
        let bvh = Bvh::new(&mesh);
        Some(Arc::new((mesh, bvh)))
    }

    /// The surface under `screen`, not counting excluded parts.
    pub(crate) fn pick_skipping_excluded(&self, screen: Vec2, viewport: Vec2, ranges: &[(usize, usize)]) -> Option<SurfaceHit> {
        let vp = viewport.max(Vec2::ONE);
        let ndc = Vec2::new(screen.x / vp.x * 2.0 - 1.0, 1.0 - screen.y / vp.y * 2.0);
        let ray = self.camera.ray(ndc, vp.x / vp.y);
        let keep = |triangle: u32| {
            let first = triangle as usize * 3;
            !ranges.iter().any(|&(start, end)| (start..end).contains(&first))
        };
        let hit = self.bvh.closest_filtered(ray.origin, ray.dir, f32::MAX, &keep)?;
        let s = surface_at(&self.mesh, hit.tri, hit.u, hit.v)?;
        Some(SurfaceHit { pos: s.pos, nrm: s.nrm, tri: hit.tri, uv: s.uv, material: self.mesh.material_of(hit.tri as usize).unwrap_or(0), screen })
    }
}
