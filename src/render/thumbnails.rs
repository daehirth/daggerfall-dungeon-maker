//! Small rendered previews of models, shown in the asset palette.
//!
//! Each thumbnail is rendered once into its own texture, registered with
//! egui, and cached. Rendering is spread over several frames by the caller
//! (see `allow_render` on [`SceneRenderer::model_thumbnail`]) so scrolling
//! through thousands of models never stalls the user interface.

use std::collections::HashMap;

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu, wgpu::util::DeviceExt};
use glam::Mat4;

use super::gpu_types::{CameraUniform, ModelInstance};
use super::{SceneRenderer, COLOR_FORMAT, DEPTH_FORMAT, SAMPLE_COUNT};
use crate::arena2::Arena2Library;
use crate::camera::OrbitCamera;

/// Width and height of every thumbnail, in pixels.
pub const THUMBNAIL_SIZE: u32 = 96;
/// Once this many thumbnails exist they are all released and re-rendered on demand.
const MAX_CACHED_THUMBNAILS: usize = 1024;
/// Background colour behind thumbnail models.
const THUMBNAIL_BACKGROUND: wgpu::Color = wgpu::Color { r: 0.07, g: 0.08, b: 0.1, a: 1.0 };
/// Viewing angles used for every thumbnail, in radians.
const THUMBNAIL_YAW: f32 = 0.8;
const THUMBNAIL_PITCH: f32 = -0.5;
/// Field of view of the thumbnail camera, in degrees.
const THUMBNAIL_FIELD_OF_VIEW_DEGREES: f32 = 40.0;

/// Result of asking for a thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailStatus {
    /// The thumbnail is ready to draw.
    Ready(egui::TextureId),
    /// The thumbnail has not been rendered yet; ask again next frame.
    Pending,
    /// The model could not be loaded, so there will never be a thumbnail.
    Unavailable,
}

/// One rendered thumbnail.
struct Thumbnail {
    texture_id: egui::TextureId,
}

/// Cached thumbnails and the scratch buffers used to render them.
pub(super) struct ThumbnailCache {
    multisampled_color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    /// `None` marks models that failed to load.
    thumbnails: HashMap<u32, Option<Thumbnail>>,
}

impl ThumbnailCache {
    /// Creates the shared multisampled colour and depth buffers.
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let create_view = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: thumbnail_extent(),
                    mip_level_count: 1,
                    sample_count: SAMPLE_COUNT,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        Self {
            multisampled_color_view: create_view("thumbnail colour (multisampled)", COLOR_FORMAT),
            depth_view: create_view("thumbnail depth", DEPTH_FORMAT),
            thumbnails: HashMap::new(),
        }
    }

    /// Releases every thumbnail texture.
    pub(super) fn clear(&mut self, render_state: &egui_wgpu::RenderState) {
        let mut egui_renderer = render_state.renderer.write();
        for thumbnail in self.thumbnails.drain().filter_map(|(_, thumbnail)| thumbnail) {
            egui_renderer.free_texture(&thumbnail.texture_id);
        }
    }
}

impl SceneRenderer {
    /// Whether a thumbnail for `model_id` has already been rendered (or found to be unavailable).
    pub fn has_cached_thumbnail(&self, model_id: u32) -> bool {
        self.thumbnails.thumbnails.contains_key(&model_id)
    }

    /// Returns the thumbnail for `model_id`.
    ///
    /// If it has not been rendered yet it is rendered now when `allow_render`
    /// is true; otherwise [`ThumbnailStatus::Pending`] is returned so the
    /// caller can spread the work over several frames.
    pub fn model_thumbnail(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        library: &mut Arena2Library,
        model_id: u32,
        allow_render: bool,
    ) -> ThumbnailStatus {
        match self.thumbnails.thumbnails.get(&model_id) {
            Some(Some(thumbnail)) => return ThumbnailStatus::Ready(thumbnail.texture_id),
            Some(None) => return ThumbnailStatus::Unavailable,
            None if !allow_render => return ThumbnailStatus::Pending,
            None => {}
        }

        if self.thumbnails.thumbnails.len() >= MAX_CACHED_THUMBNAILS {
            self.thumbnails.clear(render_state);
        }

        let thumbnail = self.render_model_thumbnail(render_state, library, model_id);
        let status = thumbnail
            .as_ref()
            .map_or(ThumbnailStatus::Unavailable, |thumbnail| ThumbnailStatus::Ready(thumbnail.texture_id));
        self.thumbnails.thumbnails.insert(model_id, thumbnail);
        status
    }

    /// Renders one model into a new texture and registers it with egui.
    fn render_model_thumbnail(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        library: &mut Arena2Library,
        model_id: u32,
    ) -> Option<Thumbnail> {
        let device = &render_state.device;
        let queue = &render_state.queue;
        if !self.ensure_model(device, queue, library, model_id) {
            return None;
        }
        let mesh = library.model(model_id)?;

        // Fit the model's bounding sphere into the view.
        let vertical_fov = THUMBNAIL_FIELD_OF_VIEW_DEGREES.to_radians();
        let camera = OrbitCamera {
            target: mesh.bounds_center(),
            yaw: THUMBNAIL_YAW,
            pitch: THUMBNAIL_PITCH,
            distance: mesh.bounding_radius().max(0.1) / (vertical_fov * 0.5).sin() * 1.05,
            vertical_fov,
        };
        let camera_uniform =
            CameraUniform::new(camera.view_projection(1.0), camera.eye_position(), camera.right());
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("thumbnail camera"),
            contents: bytemuck::bytes_of(&camera_uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("thumbnail camera"),
            layout: &self.camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
        });
        let instance = ModelInstance { model_matrix: Mat4::IDENTITY.to_cols_array_2d(), highlight: [0.0; 4] };
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("thumbnail instance"),
            contents: bytemuck::bytes_of(&instance),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let resolved_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("model thumbnail"),
                size: thumbnail_extent(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: COLOR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());

        let model = self.models.get(&model_id)?.as_ref()?;
        let mut encoder = device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("thumbnail encoder") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("thumbnail pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.thumbnails.multisampled_color_view,
                    resolve_target: Some(&resolved_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(THUMBNAIL_BACKGROUND),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.thumbnails.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.model_pipeline_double_sided);
            pass.set_bind_group(0, &camera_bind_group, &[]);
            pass.set_vertex_buffer(0, model.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, instance_buffer.slice(..));
            pass.set_index_buffer(model.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for submesh in &model.submeshes {
                pass.set_bind_group(1, &submesh.texture.bind_group, &[]);
                pass.draw_indexed(submesh.index_range.clone(), 0, 0..1);
            }
        }
        queue.submit(Some(encoder.finish()));

        let texture_id = render_state.renderer.write().register_native_texture(
            device,
            &resolved_view,
            wgpu::FilterMode::Linear,
        );
        Some(Thumbnail { texture_id })
    }
}

/// Size of every thumbnail texture.
fn thumbnail_extent() -> wgpu::Extent3d {
    wgpu::Extent3d { width: THUMBNAIL_SIZE, height: THUMBNAIL_SIZE, depth_or_array_layers: 1 }
}
