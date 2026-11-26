fn mask_with_height(layer: Operation, texel: LayerTexel, mask: f32) -> f32 {
    let contrast = bitcast<f32>(layer.constraints.z);
    if contrast == 0.0 || mask <= 0.0 || mask >= 1.0 { return mask; }
    var height = 0.0;
    let height_slot = layer.constraints.y & ~PASSTHROUGH_STACK;
    if height_slot != 0u {
        let height_op = layers[height_slot - 1u];
        height = layer_content(height_op, texel, height_op.effects.w).x;
    }
    if !(abs(height) <= 3.402823466e38) { return mask; }
    let offset = bitcast<f32>(layer.constraints.w);
    return clamp(mask + contrast * 4.0 * mask * (1.0 - mask) * (height + offset), 0.0, 1.0);
}
