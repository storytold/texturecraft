//! Compares the composites of an import with images dumped by another tool: for every
//! `<dir>/<set>_<channel>_ours.png` (16-bit, the channel's stored encoding: sRGB for colour channels stored sRGB,
//! height as h / 2 + 0.5) the same channel of TextureCraft's composite is encoded alike and compared on the texels
//! `<set>_<channel>_islands.png` marks, in 8-bit steps. Also writes TextureCraft's image as `<set>_<channel>_tc.png`.
//!
//! `cargo run --release -p texturecraft-import --features dev-tools --example compare_dump -- <project file> <dump dir>`

use std::path::Path;

use texturecraft_compose::ImageStore;
use texturecraft_core::Channel;
use texturecraft_import::Cancel;
use texturecraft_import::import::{Hooks, import_path};
use texturecraft_import::safety_net::{SetScene, composite};
use texturecraft_texel::linear_to_srgb;

/// The other tool's file name of a channel.
fn dump_id(c: Channel) -> &'static str {
    match c {
        Channel::BaseColor => "base_color",
        Channel::Emissive => "emissive",
        other => other.id(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(file), Some(dir)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: compare_dump <project file> <dump dir>");
        std::process::exit(2);
    };
    let cancel = Cancel::new();
    let mut hooks = Hooks { progress: &|_, _| {}, cancel: &cancel, replay: None, kept_cooks: None, trace: None, accelerator: None, preview: None };
    let imported = match import_path(Path::new(file), &Default::default(), &mut hooks) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let images = ImageStore::default();
    println!("set\tchannel\tmean\tmax\ttexels\tmean (both on mesh)\tmax (both on mesh)");
    for (ts, set) in imported.project.texture_sets.iter().enumerate() {
        let channels: Vec<Channel> = set.channel_list();
        let Some(baked) = imported.baked.get(&ts) else { continue };
        let scene =
            SetScene { project: &imported.project, ts, mesh: &imported.mesh, pixels: &imported.pixels, baked, images: &images, graphs: &imported.graphs };
        let Some((ours, gbuf)) = composite(&scene, &channels) else { continue };
        for c in channels {
            let base = format!("{dir}/{}_{}", set.name, dump_id(c));
            let Ok(other) = image::open(format!("{base}_ours.png")) else { continue };
            let other = other.into_rgba16();
            let islands = image::open(format!("{base}_islands.png")).ok().map(|i| i.into_luma8());
            let srgb = c.kind() == texturecraft_core::ChannelKind::Color;
            let encode = |v: f32| match c {
                Channel::Height => v * 0.5 + 0.5,
                _ if srgb => linear_to_srgb(v.clamp(0.0, 1.0)),
                _ => v,
            };
            let (w, h) = other.dimensions();
            let mut tc = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::new(w, h);
            let (mut sum, mut max, mut n, mut texels) = (0.0f64, 0.0f32, 0usize, 0usize);
            let (mut sum_both, mut max_both, mut n_both) = (0.0f64, 0.0f32, 0usize);
            for y in 0..h.min(ours.height) {
                for x in 0..w.min(ours.resolution) {
                    let v = ours.value(c, x, y).map(encode);
                    tc.put_pixel(x, y, image::Rgba([v[0], v[1], v[2], 1.0].map(|f| (f.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16)));
                    if islands.as_ref().is_some_and(|m| m.get_pixel(x, y)[0] == 0) {
                        continue;
                    }
                    texels += 1;
                    let o = other.get_pixel(x, y);
                    for k in 0..c.components() {
                        let d = (v[k] - f32::from(o[k]) / 65535.0).abs() * 255.0;
                        sum += f64::from(d);
                        max = max.max(d);
                        n += 1;
                        if gbuf.tri.get((y * gbuf.width + x) as usize).is_some_and(|t| *t != texturecraft_geom::NO_TRI) {
                            sum_both += f64::from(d);
                            max_both = max_both.max(d);
                            n_both += 1;
                        }
                    }
                }
            }
            let _ = tc.save(format!("{base}_tc.png"));
            let mean = |s: f64, n: usize| if n == 0 { 0.0 } else { s / n as f64 };
            println!("{}\t{}\t{:.3}\t{:.2}\t{texels}\t{:.3}\t{:.2}", set.name, c.id(), mean(sum, n), max, mean(sum_both, n_both), max_both);
        }
    }
}
