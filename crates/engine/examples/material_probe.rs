use serde_json::json;
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    time::Instant,
};
use texturecraft_engine::{
    Session,
    compose::{DisplayKind, tile_bytes},
    core::LayerId,
    execute, file_cmds,
    geom::primitives,
};
fn drain(s: &mut Session, label: &str) {
    let dirty: Vec<_> = s.caches.iter().map(|c| c.dirty.len()).collect();
    let t = Instant::now();
    let mut peak = 0f64;
    let mut slices = 0;
    while s.pending_tiles() > 0 {
        let a = Instant::now();
        s.update_for(30., || t.elapsed().as_secs_f64() * 1000.);
        peak = peak.max(a.elapsed().as_secs_f64() * 1000.);
        slices += 1;
    }
    println!("{} dirty={:?} compose_ms={:.1} max_slice_ms={:.1} slices={}", label, dirty, t.elapsed().as_secs_f64() * 1000., peak, slices);
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut donor = Session::new_project("probe", primitives::plane(2., 1), "builtin:plane", None, 128)?;
    donor.import_resource_bytes("unique-material-probe.png", std::fs::read(&args[2])?)?;
    let layer = execute(&mut donor, "layer.add.image", &json!({"name":"unique-material-probe.png"}))?;
    let bytes = donor.smart_material_bytes(LayerId(layer["layer"].as_u64().unwrap()), "Image material")?;
    let t = Instant::now();
    let mut s = file_cmds::open_path(&args[1])?;
    println!("load_ms={:.1}", t.elapsed().as_secs_f64() * 1000.);
    drain(&mut s, "open");
    execute(&mut s, "layer.select", &json!({"layer":10}))?;
    execute(&mut s, "material.apply", &json!({"material":"gold_polished"}))?;
    drain(&mut s, "simple");
    let t = Instant::now();
    s.add_smart_material_bytes(&bytes)?;
    println!("apply_ms={:.1}", t.elapsed().as_secs_f64() * 1000.);
    drain(&mut s, "smart");
    let mut hash = DefaultHasher::new();
    for (i, c) in s.caches.iter().enumerate() {
        let empty = Default::default();
        let baked = s.doc.baked.get(&i).unwrap_or(&empty);
        for (x, y) in texturecraft_engine::compose::all_tiles(c.size.0, c.size.1) {
            for k in DisplayKind::ALL {
                tile_bytes(&c.composite, baked, k, x, y).hash(&mut hash);
            }
        }
    }
    println!("display_hash={:x}", hash.finish());
    Ok(())
}
