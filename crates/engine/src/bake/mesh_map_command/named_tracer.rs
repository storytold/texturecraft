use texturecraft_bake::tracing::Tracer;

pub(super) struct NamedTracer;

impl Tracer for NamedTracer {
    fn name(&self) -> String {
        "gpu".into()
    }

    fn scene<'a>(
        &'a self,
        _: &[texturecraft_bake::tracing::TraceMesh<'a>],
    ) -> std::result::Result<Box<dyn texturecraft_bake::tracing::RayScene + 'a>, texturecraft_bake::BakeError> {
        Err(texturecraft_bake::BakeError::Tracer("test tracer".into()))
    }
}
