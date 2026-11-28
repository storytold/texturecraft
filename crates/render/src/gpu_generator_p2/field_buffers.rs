/// The seven storage buffers of a pass (bindings 1 to 7): the slots, then the fields
pub(super) trait FieldBuffers {
    fn buffers(&self) -> [&wgpu::Buffer; 7];
}
