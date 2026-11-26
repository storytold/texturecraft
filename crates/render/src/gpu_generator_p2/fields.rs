use super::field_buffers::FieldBuffers;

/// The fields of one block size's passes, with the request's slots
pub(super) struct Fields<'a> {
    pub(super) slots: &'a wgpu::Buffer,
    pub(super) fields: [wgpu::Buffer; 6],
}

impl FieldBuffers for Fields<'_> {
    fn buffers(&self) -> [&wgpu::Buffer; 7] {
        let [heights, first, second, detail, normals, tilted] = &self.fields;
        [self.slots, heights, first, second, detail, normals, tilted]
    }
}
