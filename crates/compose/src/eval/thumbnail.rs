//! Isolated folder evaluation with the original stack available for anchor inputs.

use super::{
    Channel, ChannelKind, Composite, EvaluationInputs, Inputs, LayerId, ProjectorDepthCache, Tape, Texel, all_tiles, evaluation_channels, evaluation_fields,
    plan, tile_texels, to_u16, to_u16_signed,
};

/// Composite one layer over channel defaults, keeping anchors from the stack below it.
pub(crate) fn composite_layer(inputs: &Inputs, layer: LayerId, channel: Channel) -> Option<Composite> {
    let set = inputs.project.texture_set(inputs.ts).ok()?;
    let channels = evaluation_channels(inputs, set.channel_list());
    let (width, height) = (inputs.gbuf.width, inputs.gbuf.height);
    let tiles = all_tiles(width, height);
    let projectors = ProjectorDepthCache::default();
    let inputs = &EvaluationInputs::new(inputs, &projectors);
    let fields = evaluation_fields(inputs, &channels, &tiles);
    let mut composite = Composite::with_size(&[channel], width, height);
    let output = composite.channels.get_mut(&channel)?;

    let mut recorded = Vec::new();
    for (tile_x, tile_y) in tiles {
        let nodes = plan(inputs, &[layer], &channels, tile_x, tile_y, 0, Some(&fields));
        let full_stack = if set.stack.as_slice() == [layer] { Vec::new() } else { plan(inputs, &set.stack, &channels, tile_x, tile_y, 0, Some(&fields)) };
        for (x, y, offset) in tile_texels(inputs.gbuf, tile_x, tile_y) {
            let mut texel = Texel::new(inputs, x, y, offset, 0.0);
            texel.anchors_below(&full_stack, (layer, 0));
            let mut values = Channel::ALL.map(Channel::default_value);
            if let Some(mut backdrop) = texel.below {
                // Evaluate masks against the real backdrop, then show only this folder's contribution
                texel.tape = Tape::Record(std::mem::take(&mut recorded));
                texel.eval(&nodes, &mut backdrop);
                if let Tape::Record(samples) = texel.tape {
                    recorded = samples;
                }

                let mut isolated = Texel::new(inputs, x, y, offset, 0.0);
                isolated.tape = Tape::Replay(&recorded, 0);
                isolated.eval(&nodes, &mut values);
                recorded.clear();
            } else {
                texel.eval(&nodes, &mut values);
            }

            let value = values.get(channel.index()).copied().unwrap_or_else(|| channel.default_value());
            let encoded = value.map(|component| if channel.kind() == ChannelKind::Signed { to_u16_signed(component) } else { to_u16(component) });
            output.set(x, y, &encoded[..channel.components()]);
        }
    }

    Some(composite)
}
