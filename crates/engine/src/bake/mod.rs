//! Baking tasks, commands, settings helpers, and the mesh-map baking API

mod auto_rebake;
mod bake_cage_inputs;
mod bake_checks;
mod bake_high_state;
mod bake_log;
mod bake_log_high;
mod bake_log_map;
mod bake_log_matching;
mod bake_map_state;
mod bake_mesh;
mod bake_meshes;
mod bake_sources;
mod bake_task;
mod canceled_bake;
mod high_poly_load;
mod high_poly_missed;
mod layout;
mod merged_high;
mod mesh_map_command;
mod model_patch;
mod paint_command;
mod painted_region;
mod preserve_tiles;
mod streamed_bake;
mod streamed_map;
mod sync_command;
mod tile_command;

pub use bake_cage_inputs::BakeCageInputs;
pub use bake_high_state::BakeHighState;
pub use bake_log::BakeLog;
pub use bake_log_high::BakeLogHigh;
pub use bake_log_map::BakeLogMap;
pub use bake_log_matching::BakeLogMatching;
pub use bake_map_state::BakeMapState;
pub use bake_mesh::BakeMesh;
pub use bake_meshes::BakeMeshes;
pub use bake_task::BakeTask;
pub use high_poly_load::HighPolyLoad;
pub use high_poly_missed::{HIDDEN_HIGH_POLY_VERTEX, high_poly_missed};
pub use merged_high::MergedHigh;
pub use mesh_map_command::{install_gpu_tracer, meshes_json, set_gpu_raytracing, settings_params};
pub use model_patch::settings_patch;
pub use painted_region::PaintedRegion;
pub use streamed_bake::StreamedBake;
pub use streamed_map::StreamedMap;
pub use texturecraft_bake::*;

use crate::CommandSpec;

type MaterialColors = std::sync::Arc<Vec<Option<std::sync::Arc<texturecraft_geom::BaseColor>>>>;

pub(crate) fn specs() -> Vec<CommandSpec> {
    let mut commands = mesh_map_command::specs();
    commands.extend(paint_command::specs());
    commands.extend(auto_rebake::specs());
    commands.extend(tile_command::specs());
    commands.extend(sync_command::specs());
    commands
}
