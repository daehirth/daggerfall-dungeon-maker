//! Draws a [`Scene`] with wgpu into an offscreen texture that egui displays.
//!
//! Rendering happens in three passes over one render pass:
//! 1. textured models, batched with instancing (one draw per model per texture),
//! 2. camera-facing billboards for flats and markers, batched per texture,
//! 3. helper lines: the floor grid, room outlines, lights and the selection box.
//!
//! One [`SceneRenderer`] is shared by every window. GPU copies of models and
//! textures are created lazily from the [`Arena2Library`] the first time they
//! are drawn and kept until the renderer is dropped. Each viewport owns a
//! [`ViewportTarget`] that holds the image it is drawn into.

mod gpu_types;
mod resources;
mod thumbnails;

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::rc::Rc;

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use glam::{Vec2, Vec3, Vec4};

use crate::arena2::{Arena2Library, TextureKey, RDB_BLOCK_SIDE};
use crate::camera::OrbitCamera;
use crate::picking::LIGHT_PICK_RADIUS;
use crate::scene::{box_corners, Scene, SceneObject, SceneObjectKind};
use gpu_types::{BillboardInstance, CameraUniform, LineBatch, LineVertex, ModelInstance, ModelVertex};
use resources::{GpuModel, GpuTexture, GrowableBuffer};
use thumbnails::ThumbnailCache;
pub use thumbnails::ThumbnailStatus;

/// Colour format of the offscreen viewport image.
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Depth buffer format.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Multisample count used for anti-aliasing.
const SAMPLE_COUNT: u32 = 4;
/// Background colour of the viewport.
const BACKGROUND_COLOR: wgpu::Color = wgpu::Color { r: 0.035, g: 0.04, b: 0.05, a: 1.0 };

/// Highlight applied to the selected object (rgb colour, alpha strength).
const SELECTION_HIGHLIGHT: [f32; 4] = [1.0, 0.55, 0.1, 0.35];
/// Highlight applied to the object under the mouse.
const HOVER_HIGHLIGHT: [f32; 4] = [0.4, 0.7, 1.0, 0.18];
/// Highlight applied to every object of the room under the mouse in the dungeon view.
const ROOM_HIGHLIGHT: [f32; 4] = [0.35, 0.7, 1.0, 0.3];
/// Highlight applied to the ghost preview of an object about to be placed.
const PLACEMENT_PREVIEW_HIGHLIGHT: [f32; 4] = [0.35, 1.0, 0.55, 0.4];
/// Colour of a room outline, and of the outline of the highlighted room.
const ROOM_OUTLINE_COLOR: Vec4 = Vec4::new(0.75, 0.6, 0.2, 1.0);
const HIGHLIGHTED_ROOM_OUTLINE_COLOR: Vec4 = Vec4::new(0.45, 0.8, 1.0, 1.0);
/// Colour of the lines drawn for a light about to be placed.
const PLACEMENT_PREVIEW_LINE_COLOR: Vec4 = Vec4::new(0.35, 1.0, 0.55, 1.0);
/// Spacing of the floor grid, in world units (128 native units).
const GRID_SPACING: f32 = RDB_BLOCK_SIDE / 16.0;
/// How far the grid extends beyond the room on every side, in grid cells.
const GRID_MARGIN_CELLS: i32 = 8;

/// Which parts of the scene the viewport draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewSettings {
    /// Hide faces pointing away from the camera, so rooms can be seen into from outside.
    pub cull_back_faces: bool,
    pub show_grid: bool,
    pub show_room_outline: bool,
    pub show_models: bool,
    pub show_flats: bool,
    pub show_markers: bool,
    pub show_lights: bool,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            cull_back_faces: true,
            show_grid: true,
            show_room_outline: true,
            show_models: true,
            show_flats: true,
            show_markers: true,
            show_lights: true,
        }
    }
}

/// The square footprint of one RDB block, drawn as an outline at grid height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoomOutline {
    /// World position of the block's minimum corner.
    pub origin: Vec3,
    /// Draw the outline in the highlight colour.
    pub highlighted: bool,
}

/// Everything needed to draw one frame of a viewport.
pub struct FrameRequest<'frame> {
    pub scene: Option<&'frame Scene>,
    pub camera: &'frame OrbitCamera,
    pub selected_object: Option<usize>,
    pub hovered_object: Option<usize>,
    /// Every object whose group index matches is highlighted (used for whole rooms in the dungeon view).
    pub highlighted_group: Option<usize>,
    /// Object being placed from the asset palette, drawn as a ghost.
    pub placement_preview: Option<&'frame SceneObject>,
    /// Block footprints to outline; the grid is sized to cover them.
    pub room_outlines: &'frame [RoomOutline],
    pub settings: &'frame ViewSettings,
    /// Height of the floor grid in world units.
    pub grid_height: f32,
    /// Viewport size in physical pixels.
    pub size_in_pixels: [u32; 2],
}

/// The image one viewport is rendered into, plus the egui texture that displays it.
///
/// Every window with a 3D view owns one of these. Call [`ViewportTarget::release`]
/// when the window closes so the egui texture is freed.
#[derive(Default)]
pub struct ViewportTarget {
    textures: Option<RenderTarget>,
    egui_texture_id: Option<egui::TextureId>,
}

impl ViewportTarget {
    /// Frees the egui texture and GPU images.
    pub fn release(&mut self, render_state: &egui_wgpu::RenderState) {
        if let Some(texture_id) = self.egui_texture_id.take() {
            render_state.renderer.write().free_texture(&texture_id);
        }
        self.textures = None;
    }

    /// Creates or resizes the offscreen images and returns the egui texture id that shows them.
    fn prepare(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        size_in_pixels: [u32; 2],
    ) -> egui::TextureId {
        let size_in_pixels = [size_in_pixels[0].max(1), size_in_pixels[1].max(1)];
        if let (Some(textures), Some(texture_id)) = (&self.textures, self.egui_texture_id) {
            if textures.size_in_pixels == size_in_pixels {
                return texture_id;
            }
        }

        let device = &render_state.device;
        let textures = RenderTarget::new(device, size_in_pixels);
        let mut egui_renderer = render_state.renderer.write();
        let texture_id = match self.egui_texture_id {
            Some(texture_id) => {
                egui_renderer.update_egui_texture_from_wgpu_texture(
                    device,
                    &textures.resolved_color_view,
                    wgpu::FilterMode::Linear,
                    texture_id,
                );
                texture_id
            }
            None => egui_renderer.register_native_texture(
                device,
                &textures.resolved_color_view,
                wgpu::FilterMode::Linear,
            ),
        };
        self.egui_texture_id = Some(texture_id);
        self.textures = Some(textures);
        texture_id
    }
}

/// The offscreen textures a viewport renders into.
struct RenderTarget {
    size_in_pixels: [u32; 2],
    /// Multisampled colour buffer that is resolved into the egui texture.
    multisampled_color_view: wgpu::TextureView,
    /// Single-sample texture shown by egui.
    resolved_color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

impl RenderTarget {
    /// Allocates the colour, resolve and depth images for a viewport of the given size.
    fn new(device: &wgpu::Device, size_in_pixels: [u32; 2]) -> Self {
        let extent =
            wgpu::Extent3d { width: size_in_pixels[0], height: size_in_pixels[1], depth_or_array_layers: 1 };
        let create_view = |label: &str, format, sample_count, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };

        Self {
            size_in_pixels,
            multisampled_color_view: create_view(
                "viewport colour (multisampled)",
                COLOR_FORMAT,
                SAMPLE_COUNT,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
            resolved_color_view: create_view(
                "viewport colour",
                COLOR_FORMAT,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
            depth_view: create_view(
                "viewport depth",
                DEPTH_FORMAT,
                SAMPLE_COUNT,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
        }
    }
}

/// A run of model instances that share one model.
struct ModelBatch {
    model_id: u32,
    instances: Range<u32>,
}

/// A run of billboard instances that share one texture.
struct BillboardBatch {
    texture: Rc<GpuTexture>,
    instances: Range<u32>,
}

/// Owns every GPU resource used to draw the viewport.
pub struct SceneRenderer {
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    camera_bind_group_layout: wgpu::BindGroupLayout,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    surface_sampler: wgpu::Sampler,
    model_pipeline_culled: wgpu::RenderPipeline,
    model_pipeline_double_sided: wgpu::RenderPipeline,
    billboard_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    fallback_texture: Rc<GpuTexture>,
    textures: HashMap<TextureKey, Rc<GpuTexture>>,
    models: HashMap<u32, Option<GpuModel>>,
    model_instance_buffer: GrowableBuffer,
    billboard_instance_buffer: GrowableBuffer,
    line_vertex_buffer: GrowableBuffer,
    /// Rendered previews of models for the asset palette.
    thumbnails: ThumbnailCache,
}

impl SceneRenderer {
    /// Creates pipelines and shared resources on the device used by egui.
    pub fn new(render_state: &egui_wgpu::RenderState) -> Self {
        let device = &render_state.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport shaders"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });

        let camera_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera uniform"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera bind group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
        });

        let texture_bind_group_layout = resources::create_texture_bind_group_layout(device);
        let surface_sampler = resources::create_surface_sampler(device);

        let surface_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("surface pipeline layout"),
            bind_group_layouts: &[&camera_bind_group_layout, &texture_bind_group_layout],
            push_constant_ranges: &[],
        });
        let line_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("line pipeline layout"),
            bind_group_layouts: &[&camera_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline_builder = PipelineBuilder { device, shader: &shader };
        let model_buffers = [ModelVertex::layout(), ModelInstance::layout()];
        let model_pipeline_culled = pipeline_builder.build(PipelineSpec {
            label: "model pipeline (culled)",
            ..model_pipeline_spec(&surface_layout, &model_buffers)
        });
        let model_pipeline_double_sided = pipeline_builder.build(PipelineSpec {
            label: "model pipeline (double sided)",
            cull_mode: None,
            ..model_pipeline_spec(&surface_layout, &model_buffers)
        });
        let billboard_pipeline = pipeline_builder.build(PipelineSpec {
            label: "billboard pipeline",
            layout: &surface_layout,
            vertex_entry: "vs_billboard",
            fragment_entry: "fs_surface",
            buffers: &[BillboardInstance::layout()],
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            depth_write: true,
        });
        let line_pipeline = pipeline_builder.build(PipelineSpec {
            label: "line pipeline",
            layout: &line_layout,
            vertex_entry: "vs_line",
            fragment_entry: "fs_line",
            buffers: &[LineVertex::layout()],
            topology: wgpu::PrimitiveTopology::LineList,
            cull_mode: None,
            depth_write: false,
        });

        let fallback_texture = Rc::new(resources::upload_texture(
            device,
            &render_state.queue,
            &texture_bind_group_layout,
            &surface_sampler,
            "missing texture",
            &resources::fallback_image(),
        ));

        let thumbnails = ThumbnailCache::new(device);

        Self {
            camera_buffer,
            camera_bind_group,
            camera_bind_group_layout,
            texture_bind_group_layout,
            surface_sampler,
            model_pipeline_culled,
            model_pipeline_double_sided,
            billboard_pipeline,
            line_pipeline,
            fallback_texture,
            textures: HashMap::new(),
            models: HashMap::new(),
            model_instance_buffer: GrowableBuffer::new(device, "model instances", wgpu::BufferUsages::VERTEX),
            billboard_instance_buffer: GrowableBuffer::new(
                device,
                "billboard instances",
                wgpu::BufferUsages::VERTEX,
            ),
            line_vertex_buffer: GrowableBuffer::new(device, "line vertices", wgpu::BufferUsages::VERTEX),
            thumbnails,
        }
    }

    /// Drops every cached model, texture and thumbnail, for example after a different ARENA2 folder is loaded.
    pub fn clear_cache(&mut self, render_state: &egui_wgpu::RenderState) {
        self.models.clear();
        self.textures.clear();
        self.thumbnails.clear(render_state);
    }

    /// Renders a frame into `target` and returns the egui texture that holds the result.
    ///
    /// Several viewports can be rendered per frame: shared buffers are written
    /// and submitted in sequence, so each submission sees its own data.
    pub fn render(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        library: &mut Arena2Library,
        target: &mut ViewportTarget,
        request: &FrameRequest<'_>,
    ) -> egui::TextureId {
        let device = &render_state.device;
        let queue = &render_state.queue;
        let texture_id = target.prepare(render_state, request.size_in_pixels);

        let [width, height] = request.size_in_pixels;
        let aspect_ratio = width as f32 / height.max(1) as f32;
        let camera = request.camera;
        let billboard_right = Vec3::new(camera.right().x, 0.0, camera.right().z).normalize_or_zero();
        let camera_uniform =
            CameraUniform::new(camera.view_projection(aspect_ratio), camera.eye_position(), billboard_right);
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));

        let (model_batches, billboard_batches) =
            self.prepare_scene_instances(device, queue, library, request);

        let lines = build_helper_lines(library, request);
        self.line_vertex_buffer.write(device, queue, bytemuck::cast_slice(&lines.vertices));

        let mut encoder = device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("viewport encoder") });
        {
            let target = target.textures.as_ref().expect("target prepared above");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.multisampled_color_view,
                    resolve_target: Some(&target.resolved_color_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BACKGROUND_COLOR),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.camera_bind_group, &[]);

            // Models.
            let model_pipeline = if request.settings.cull_back_faces {
                &self.model_pipeline_culled
            } else {
                &self.model_pipeline_double_sided
            };
            pass.set_pipeline(model_pipeline);
            pass.set_vertex_buffer(1, self.model_instance_buffer.buffer().slice(..));
            for batch in &model_batches {
                let Some(Some(model)) = self.models.get(&batch.model_id) else {
                    continue;
                };
                pass.set_vertex_buffer(0, model.vertex_buffer.slice(..));
                pass.set_index_buffer(model.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                for submesh in &model.submeshes {
                    pass.set_bind_group(1, &submesh.texture.bind_group, &[]);
                    pass.draw_indexed(submesh.index_range.clone(), 0, batch.instances.clone());
                }
            }

            // Billboards.
            pass.set_pipeline(&self.billboard_pipeline);
            pass.set_vertex_buffer(0, self.billboard_instance_buffer.buffer().slice(..));
            for batch in &billboard_batches {
                pass.set_bind_group(1, &batch.texture.bind_group, &[]);
                pass.draw(0..6, batch.instances.clone());
            }

            // Helper lines.
            if !lines.vertices.is_empty() {
                pass.set_pipeline(&self.line_pipeline);
                pass.set_vertex_buffer(0, self.line_vertex_buffer.buffer().slice(..));
                pass.draw(0..lines.vertices.len() as u32, 0..1);
            }
        }
        queue.submit(Some(encoder.finish()));
        texture_id
    }

    /// Builds instance buffers for every visible object and uploads any missing GPU resources.
    fn prepare_scene_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &mut Arena2Library,
        request: &FrameRequest<'_>,
    ) -> (Vec<ModelBatch>, Vec<BillboardBatch>) {
        let settings = request.settings;
        let highlight_for = |index: usize, object: &SceneObject| {
            if request.selected_object == Some(index) {
                SELECTION_HIGHLIGHT
            } else if request.highlighted_group == Some(object.group_index) {
                ROOM_HIGHLIGHT
            } else if request.hovered_object == Some(index) {
                HOVER_HIGHLIGHT
            } else {
                [0.0; 4]
            }
        };

        let mut model_instances_by_id: BTreeMap<u32, Vec<ModelInstance>> = BTreeMap::new();
        let mut billboard_instances_by_texture: BTreeMap<TextureKey, Vec<BillboardInstance>> =
            BTreeMap::new();

        // Every visible scene object with its highlight, followed by the placement ghost.
        let scene_objects = request.scene.map_or(&[][..], |scene| scene.objects.as_slice());
        let drawable_objects = scene_objects
            .iter()
            .enumerate()
            .filter(|(_, object)| object.visible)
            .map(|(index, object)| (object, highlight_for(index, object)))
            .chain(request.placement_preview.map(|object| (object, PLACEMENT_PREVIEW_HIGHLIGHT)));

        for (object, highlight) in drawable_objects {
            match object.kind {
                SceneObjectKind::Model { model_id, .. } if settings.show_models => {
                    model_instances_by_id.entry(model_id).or_default().push(ModelInstance {
                        model_matrix: object.model_matrix().to_cols_array_2d(),
                        highlight,
                    });
                }
                SceneObjectKind::Flat { texture } => {
                    let shown = if object.is_marker() { settings.show_markers } else { settings.show_flats };
                    if !shown {
                        continue;
                    }
                    let size = library.billboard_world_size(texture).unwrap_or(Vec2::ONE);
                    billboard_instances_by_texture.entry(texture).or_default().push(BillboardInstance {
                        center: object.position.to_array(),
                        size: size.to_array(),
                        highlight,
                    });
                }
                _ => {}
            }
        }

        // Models: upload any new ones and concatenate instances into one buffer.
        let mut model_instances = Vec::new();
        let mut model_batches = Vec::new();
        for (model_id, instances) in model_instances_by_id {
            if !self.ensure_model(device, queue, library, model_id) {
                continue;
            }
            let start = model_instances.len() as u32;
            model_instances.extend(instances);
            model_batches.push(ModelBatch { model_id, instances: start..model_instances.len() as u32 });
        }
        self.model_instance_buffer.write(device, queue, bytemuck::cast_slice(&model_instances));

        // Billboards: the same, grouped by texture.
        let mut billboard_instances = Vec::new();
        let mut billboard_batches = Vec::new();
        for (texture_key, instances) in billboard_instances_by_texture {
            let texture = self.texture(device, queue, library, texture_key);
            let start = billboard_instances.len() as u32;
            billboard_instances.extend(instances);
            billboard_batches
                .push(BillboardBatch { texture, instances: start..billboard_instances.len() as u32 });
        }
        self.billboard_instance_buffer.write(device, queue, bytemuck::cast_slice(&billboard_instances));

        (model_batches, billboard_batches)
    }

    /// Makes sure the model `model_id` is on the GPU. Returns `false` if it cannot be loaded.
    fn ensure_model(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &mut Arena2Library,
        model_id: u32,
    ) -> bool {
        if let Some(model) = self.models.get(&model_id) {
            return model.is_some();
        }
        let gpu_model = library.model(model_id).map(|mesh| {
            resources::upload_model(device, &mesh, |texture_key| {
                self.texture(device, queue, library, texture_key)
            })
        });
        let loaded = gpu_model.is_some();
        self.models.insert(model_id, gpu_model);
        loaded
    }

    /// Returns the GPU texture for `texture_key`, uploading it on first use.
    fn texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &mut Arena2Library,
        texture_key: TextureKey,
    ) -> Rc<GpuTexture> {
        if let Some(texture) = self.textures.get(&texture_key) {
            return texture.clone();
        }
        let texture = match library.texture_image(texture_key) {
            Some(image) => Rc::new(resources::upload_texture(
                device,
                queue,
                &self.texture_bind_group_layout,
                &self.surface_sampler,
                &texture_key.to_string(),
                &image,
            )),
            None => self.fallback_texture.clone(),
        };
        self.textures.insert(texture_key, texture.clone());
        texture
    }
}

/// Builds the floor grid, room outlines, light gizmos and selection box.
fn build_helper_lines(library: &mut Arena2Library, request: &FrameRequest<'_>) -> LineBatch {
    let mut lines = LineBatch::default();
    let settings = request.settings;
    let grid_height = request.grid_height;

    if settings.show_grid {
        add_grid(&mut lines, request.room_outlines, grid_height);
    }

    if settings.show_room_outline {
        // Draw highlighted outlines last so they are not hidden under neighbouring ones.
        let outlines = request.room_outlines.iter().filter(|outline| !outline.highlighted);
        let highlighted = request.room_outlines.iter().filter(|outline| outline.highlighted);
        for outline in outlines.chain(highlighted) {
            let color = if outline.highlighted { HIGHLIGHTED_ROOM_OUTLINE_COLOR } else { ROOM_OUTLINE_COLOR };
            let corner = |x: f32, z: f32| outline.origin + Vec3::new(x, 0.0, z) + Vec3::Y * grid_height;
            let corners = [
                corner(0.0, 0.0),
                corner(RDB_BLOCK_SIDE, 0.0),
                corner(RDB_BLOCK_SIDE, RDB_BLOCK_SIDE),
                corner(0.0, RDB_BLOCK_SIDE),
            ];
            for side in 0..4 {
                lines.add_line(corners[side], corners[(side + 1) % 4], color);
            }
        }
    }

    if let Some(SceneObject { kind: SceneObjectKind::Light { radius }, position, .. }) =
        request.placement_preview
    {
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let offset = axis * LIGHT_PICK_RADIUS;
            lines.add_line(*position - offset, *position + offset, PLACEMENT_PREVIEW_LINE_COLOR);
        }
        lines.add_circle(*position, *radius, Vec3::X, Vec3::Z, PLACEMENT_PREVIEW_LINE_COLOR);
    }

    let Some(scene) = request.scene else {
        return lines;
    };

    if settings.show_lights {
        for (index, object) in scene.objects.iter().enumerate() {
            let SceneObjectKind::Light { radius } = object.kind else { continue };
            if !object.visible {
                continue;
            }
            let selected = request.selected_object == Some(index);
            let color = if selected { Vec4::new(1.0, 0.6, 0.1, 1.0) } else { Vec4::new(1.0, 0.9, 0.35, 1.0) };
            let center = object.position;
            let size = LIGHT_PICK_RADIUS;
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                lines.add_line(center - axis * size, center + axis * size, color);
            }
            if selected {
                lines.add_circle(center, radius, Vec3::X, Vec3::Z, color);
                lines.add_circle(center, radius, Vec3::X, Vec3::Y, color);
                lines.add_circle(center, radius, Vec3::Z, Vec3::Y, color);
            }
        }
    }

    // Bounding box around the selected model.
    if let Some(object) = request.selected_object.and_then(|index| scene.objects.get(index)) {
        if let SceneObjectKind::Model { model_id, .. } = object.kind {
            if let Some(mesh) = library.model(model_id) {
                let matrix = object.model_matrix();
                let corners = box_corners(mesh.bounds_min, mesh.bounds_max)
                    .map(|corner| matrix.transform_point3(corner));
                lines.add_box(&corners, Vec4::new(1.0, 0.55, 0.1, 1.0));
            }
        }
    }

    lines
}

/// Adds a floor grid that covers every room outline (or one block at the origin) plus a margin.
fn add_grid(lines: &mut LineBatch, room_outlines: &[RoomOutline], grid_height: f32) {
    let grid_color = Vec4::new(0.12, 0.13, 0.15, 1.0);
    let margin = GRID_MARGIN_CELLS as f32 * GRID_SPACING;

    let (minimum_corner, maximum_corner) = room_outlines
        .iter()
        .map(|outline| (outline.origin, outline.origin + Vec3::new(RDB_BLOCK_SIDE, 0.0, RDB_BLOCK_SIDE)))
        .reduce(|(minimum, maximum), (low, high)| (minimum.min(low), maximum.max(high)))
        .unwrap_or((Vec3::ZERO, Vec3::new(RDB_BLOCK_SIDE, 0.0, RDB_BLOCK_SIDE)));
    let snap_down = |value: f32| ((value - margin) / GRID_SPACING).floor() as i32;
    let snap_up = |value: f32| ((value + margin) / GRID_SPACING).ceil() as i32;
    let (first_column, last_column) = (snap_down(minimum_corner.x), snap_up(maximum_corner.x));
    let (first_row, last_row) = (snap_down(minimum_corner.z), snap_up(maximum_corner.z));
    let (minimum_x, maximum_x) = (first_column as f32 * GRID_SPACING, last_column as f32 * GRID_SPACING);
    let (minimum_z, maximum_z) = (first_row as f32 * GRID_SPACING, last_row as f32 * GRID_SPACING);

    for column in first_column..=last_column {
        let x = column as f32 * GRID_SPACING;
        lines.add_line(
            Vec3::new(x, grid_height, minimum_z),
            Vec3::new(x, grid_height, maximum_z),
            grid_color,
        );
    }
    for row in first_row..=last_row {
        let z = row as f32 * GRID_SPACING;
        lines.add_line(
            Vec3::new(minimum_x, grid_height, z),
            Vec3::new(maximum_x, grid_height, z),
            grid_color,
        );
    }

    // World axes through the origin: X in red, Z in blue.
    lines.add_line(
        Vec3::new(minimum_x, grid_height, 0.0),
        Vec3::new(maximum_x, grid_height, 0.0),
        Vec4::new(0.55, 0.15, 0.15, 1.0),
    );
    lines.add_line(
        Vec3::new(0.0, grid_height, minimum_z),
        Vec3::new(0.0, grid_height, maximum_z),
        Vec4::new(0.15, 0.25, 0.6, 1.0),
    );
}

/// Describes one render pipeline variant.
struct PipelineSpec<'spec> {
    label: &'static str,
    layout: &'spec wgpu::PipelineLayout,
    vertex_entry: &'static str,
    fragment_entry: &'static str,
    buffers: &'spec [wgpu::VertexBufferLayout<'static>],
    topology: wgpu::PrimitiveTopology,
    cull_mode: Option<wgpu::Face>,
    depth_write: bool,
}

/// The model pipeline description shared by the culled and double sided variants.
fn model_pipeline_spec<'spec>(
    layout: &'spec wgpu::PipelineLayout,
    buffers: &'spec [wgpu::VertexBufferLayout<'static>],
) -> PipelineSpec<'spec> {
    PipelineSpec {
        label: "model pipeline",
        layout,
        vertex_entry: "vs_model",
        fragment_entry: "fs_surface",
        buffers,
        topology: wgpu::PrimitiveTopology::TriangleList,
        cull_mode: Some(wgpu::Face::Back),
        depth_write: true,
    }
}

/// Creates render pipelines that share one shader module and target format.
struct PipelineBuilder<'builder> {
    device: &'builder wgpu::Device,
    shader: &'builder wgpu::ShaderModule,
}

impl PipelineBuilder<'_> {
    fn build(&self, spec: PipelineSpec<'_>) -> wgpu::RenderPipeline {
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(spec.label),
            layout: Some(spec.layout),
            vertex: wgpu::VertexState {
                module: self.shader,
                entry_point: spec.vertex_entry,
                compilation_options: Default::default(),
                buffers: spec.buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: self.shader,
                entry_point: spec.fragment_entry,
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: spec.topology,
                // Daggerfall data uses clockwise front faces in this left-handed space.
                front_face: wgpu::FrontFace::Cw,
                cull_mode: spec.cull_mode,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: spec.depth_write,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState { count: SAMPLE_COUNT, ..Default::default() },
            multiview: None,
            cache: None,
        })
    }
}
