use texturecraft_compose::{BakedMaps, ImageStore, Inputs, PixelStore, mask_at};
use texturecraft_core::{Channel, Effect, EffectKind, Fill, Generator, Layer, LayerId, LayerKind, Mask, MaskBackground, Project, Source, TextureSet};
use texturecraft_geom::{Mesh, UvGBuffer};

pub(super) struct GeneratorFixture {
    pub(super) project: Project,
    pub(super) mesh: Mesh,
    pub(super) gbuf: UvGBuffer,
    pub(super) layer: LayerId,
    pub(super) baked: BakedMaps,
    pub(super) images: ImageStore,
}

impl GeneratorFixture {
    pub(super) fn new(mesh: Mesh, asset: &str, resolution: u32) -> Self {
        let gbuf = UvGBuffer::new(&mesh, 0, resolution, resolution);
        let mut project = Project::new("Generator test", asset);
        project.texture_sets.push(TextureSet::new("A", 0, resolution));

        let id = project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::BaseColor, Source::Uniform([1.0; 3]));
        let layer = project.insert_layer(0, Layer::new(id, "Fill", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();

        Self { project, mesh, gbuf, layer, baked: BakedMaps::default(), images: ImageStore::default() }
    }

    pub(super) fn sample(&mut self, generator: Generator, texels: &[(u32, u32)]) -> Vec<f32> {
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(self.project.alloc_id(), EffectKind::Generator(generator)));
        self.project.layer_mut(self.layer).unwrap().mask = Some(mask);

        let inputs = Inputs {
            project: &self.project,
            ts: 0,
            mesh: &self.mesh,
            gbuf: &self.gbuf,
            pixels: &PixelStore::default(),
            baked: &self.baked,
            block_scales: self.project.texture_sets[0].block_scales(),
            preview: None,
            images: &self.images,
            bounds: self.mesh.bounds(),
        };
        mask_at(&inputs, self.layer, texels).unwrap()
    }
}
