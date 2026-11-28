//! Shader-picker spheres, rendered serially on the app's existing GPU device.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use texturecraft_engine::compose::{ImageRes, ImageStore};
use texturecraft_engine::core::custom_shader::CustomShader;
use texturecraft_engine::geom::{OrbitCamera, primitives};
use texturecraft_render::{Frame, Renderer, Shade, ViewKind, wgpu};

struct Cached {
    frame: Frame,
    images: Vec<Option<Weak<ImageRes>>>,
    failed: bool,
}

#[derive(Clone)]
struct PreviewError {
    source: Weak<str>,
    message: String,
}

impl PreviewError {
    fn matches(&self, shader: &CustomShader) -> bool {
        self.source.upgrade().is_some_and(|source| source == shader.source)
    }
}

impl Cached {
    fn matches(&self, frame: &Frame, images: &ImageStore) -> bool {
        let current = image_keys(frame, images);
        self.frame == *frame
            && self.images.len() == current.len()
            && self.images.iter().zip(current.iter()).all(|(old, new)| old.as_ref().map(Weak::as_ptr) == new.as_ref().map(Weak::as_ptr))
    }
}

fn image_keys(frame: &Frame, images: &ImageStore) -> Vec<Option<Weak<ImageRes>>> {
    frame.shade.custom_shader.as_ref().map_or_else(Vec::new, |shader| {
        shader.textures.iter().map(|input| input.resource.as_ref().and_then(|name| images.get(name)).map(Arc::downgrade)).collect()
    })
}

struct Previews {
    project: u64,
    renderer: Renderer,
    cached: HashMap<u64, Cached>,
    rendered_pass: Option<u64>,
}

impl Previews {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, project: u64) -> Self {
        let mut renderer = Renderer::new(device, queue, format);
        let sphere = primitives::sphere(1.0, 32, 16);
        renderer.set_mesh(device, queue, &sphere, 1);
        renderer.set_texture_sets(device, queue, &[((64, 64), 0)], 1);
        Self { project, renderer, cached: HashMap::new(), rendered_pass: None }
    }
}

struct PreviewCallback {
    project: u64,
    format: wgpu::TextureFormat,
    frame: Frame,
    images: ImageStore,
    rect: egui::Rect,
    ctx: egui::Context,
}

impl CallbackTrait for PreviewCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if resources
            .get::<Previews>()
            .is_none_or(|previews| previews.project != self.project || (previews.cached.len() >= 17 && !previews.cached.contains_key(&self.frame.view_id)))
        {
            resources.insert(Previews::new(device, queue, self.format, self.project));
        }
        let Some(previews) = resources.get_mut::<Previews>() else { return Vec::new() };
        let pass = self.ctx.cumulative_pass_nr();
        let id = self.frame.view_id;
        if !previews.cached.get(&id).is_some_and(|cached| cached.matches(&self.frame, &self.images)) {
            if previews.rendered_pass == Some(pass) {
                self.ctx.request_repaint();
                return Vec::new();
            }
            previews.rendered_pass = Some(pass);
            if let Some(shader) = &self.frame.shade.custom_shader {
                previews.renderer.upload_shader_inputs(device, queue, shader, &self.images);
            }
            previews.renderer.prepare(device, queue, encoder, &self.frame);
            if self.frame.shade.custom_shader.is_none() || previews.renderer.shader_ready() || previews.renderer.shader_error().is_some() {
                previews.cached.insert(
                    id,
                    Cached {
                        frame: self.frame.clone(),
                        images: image_keys(&self.frame, &self.images),
                        failed: self.frame.shade.custom_shader.is_some() && previews.renderer.shader_error().is_some(),
                    },
                );
            }
            if let Some(error) = previews.renderer.take_shader_error()
                && let Some(shader) = &self.frame.shade.custom_shader
            {
                self.ctx.data_mut(|data| {
                    data.insert_temp(
                        egui::Id::unique(("shader_preview_error", self.project, id)),
                        PreviewError { source: Arc::downgrade(&shader.source), message: error },
                    );
                });
            }
            self.ctx.request_repaint();
        }
        let scale = screen.pixels_per_point;
        previews.renderer.set_blit_rect(
            queue,
            id,
            [self.rect.min.x * scale, self.rect.min.y * scale, self.rect.width() * scale, self.rect.height() * scale],
            screen.size_in_pixels,
        );
        Vec::new()
    }

    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &CallbackResources) {
        let Some(previews) = resources.get::<Previews>() else { return };
        if !previews.cached.get(&self.frame.view_id).is_some_and(|cached| !cached.failed && cached.matches(&self.frame, &self.images)) {
            return;
        }
        let [width, height] = info.screen_size_px;
        let clip = info.clip_rect_in_pixels();
        if clip.width_px <= 0 || clip.height_px <= 0 || width == 0 || height == 0 {
            return;
        }
        pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
        pass.set_scissor_rect(clip.left_px.max(0) as u32, clip.top_px.max(0) as u32, clip.width_px as u32, clip.height_px as u32);
        previews.renderer.blit_prepared(pass, self.frame.view_id);
    }
}

/// Overlay the sphere above the tile's label. The button retains its standard focus and AccessKit node.
pub(crate) fn tile(app: &crate::TextureCraftApp, ui: &mut egui::Ui, name: &str, selected: bool, shader: Option<&CustomShader>) -> egui::Response {
    let response = ui.add_sized(crate::shader_settings::SHADER_TILE_SIZE, egui::Button::selectable(selected, ""));
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, name));
    let tokens = crate::theme::Tokens::get(ui.ctx());
    let painter = ui.painter().with_clip_rect(response.rect.shrink(4.0));
    painter.text(
        egui::pos2(response.rect.left() + 4.0, response.rect.bottom() - 4.0),
        egui::Align2::LEFT_BOTTOM,
        name,
        egui::TextStyle::Small.resolve(ui.style()),
        tokens.text,
    );
    show(app, ui, &response, shader);
    response.on_hover_text(name)
}

fn show(app: &crate::TextureCraftApp, ui: &egui::Ui, response: &egui::Response, shader: Option<&CustomShader>) {
    let Some(render_state) = &app.render_state else {
        response.clone().on_hover_text(crate::i18n::tr("Shader preview needs a GPU renderer"));
        return;
    };
    let name = shader.map_or(crate::shader_settings::CURRENT_SHADER, |shader| shader.name.as_str());
    let id = egui::Id::unique(("shader_preview", name)).value();
    let error = ui
        .data(|data| data.get_temp::<PreviewError>(egui::Id::unique(("shader_preview_error", app.session.project_id, id))))
        .filter(|error| shader.is_some_and(|shader| error.matches(shader)));
    if let Some(error) = error {
        response.clone().on_hover_text(format!("{}: {}", crate::i18n::tr("Shader preview unavailable"), error.message));
        return;
    }
    let side = (response.rect.width() - 8.0).min(42.0);
    let rect = egui::Rect::from_center_size(egui::pos2(response.rect.center().x, response.rect.top() + side * 0.5 + 4.0), egui::vec2(side, side));
    let size = (side * ui.ctx().pixels_per_point()).round().clamp(1.0, 128.0) as u32;
    let mut camera = OrbitCamera { yaw: 0.5, pitch: 0.2, ..OrbitCamera::default() };
    camera.frame(texturecraft_engine::geom::glam::Vec3::splat(-1.0), texturecraft_engine::geom::glam::Vec3::splat(1.0));
    let background = crate::theme::Tokens::get(ui.ctx()).chrome;
    let rgb = [background.r(), background.g(), background.b()].map(|v| texturecraft_engine::texel::srgb_to_linear(v as f32 / 255.0));
    let frame = Frame {
        view_id: id,
        size: [size, size],
        kind: ViewKind::Scene,
        camera,
        shade: Shade { custom_shader: shader.cloned(), taa: false, background: [rgb, rgb], ..Shade::default() },
        visible: vec![true],
        overlay: Vec::new(),
    };
    let images = shader.map_or_else(ImageStore::new, |shader| {
        shader
            .textures
            .iter()
            .filter_map(|input| input.resource.as_ref())
            .filter_map(|name| app.session.images.get(name).map(|image| (name.clone(), image.clone())))
            .collect()
    });
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(
        rect,
        PreviewCallback { project: app.session.project_id, format: render_state.target_format, frame, images, rect, ctx: ui.ctx().clone() },
    ));
}
