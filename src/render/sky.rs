//! The skybox drawn behind a dungeon: a panorama image of the user's own.
//!
//! Images are read from disk the first time they are drawn and kept on the
//! GPU, keyed by path and by the file's size and modification time, so an
//! image changed on disk is read again. One that cannot be read is
//! remembered with its error (see [`SkyRenderer::error`]) and nothing is drawn.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use eframe::egui_wgpu::wgpu;
use glam::Mat4;

use super::{COLOR_FORMAT, DEPTH_FORMAT, SAMPLE_COUNT};

/// Which version of an image file a cached sky was read from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SkyKey {
    path: PathBuf,
    modified: Option<SystemTime>,
    length: u64,
}

impl SkyKey {
    fn of(path: &Path) -> Self {
        let metadata = std::fs::metadata(path).ok();
        Self {
            path: path.to_path_buf(),
            modified: metadata.as_ref().and_then(|metadata| metadata.modified().ok()),
            length: metadata.map_or(0, |metadata| metadata.len()),
        }
    }
}

/// Draws skyboxes, and keeps the images it has read.
pub struct SkyRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// Each image read, as a bind group to draw it with, or why it could not be read.
    skies: HashMap<SkyKey, Result<wgpu::BindGroup, String>>,
}

impl SkyRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sky.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        // Drawn first, behind everything, without touching the depth buffer.
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_sky",
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_sky",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState { count: SAMPLE_COUNT, ..Default::default() },
            multiview: None,
            cache: None,
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky uniform"),
            size: std::mem::size_of::<[[f32; 4]; 4]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Wraps around sideways; smooth, as Unity draws it.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { pipeline, layout, uniform_buffer, sampler, skies: HashMap::new() }
    }

    /// Gets the sky in the image at `path` ready to draw for a camera with
    /// `view_projection`, reading the image the first time. Returns `false` if
    /// it cannot be drawn (see [`Self::error`]).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &Path,
        view_projection: Mat4,
    ) -> bool {
        let key = SkyKey::of(path);
        if !self.skies.contains_key(&key) {
            // Older versions of this file are not needed any more.
            self.skies.retain(|cached, _| cached.path != key.path);
            let sky = read_sky_image(path, device.limits().max_texture_dimension_2d)
                .map(|image| self.upload(device, queue, path, &image));
            if let Err(error) = &sky {
                log::warn!("skybox {}: {error}", path.display());
            }
            self.skies.insert(key.clone(), sky);
        }
        let ready = matches!(self.skies.get(&key), Some(Ok(_)));
        if ready {
            let inverse = view_projection.inverse().to_cols_array_2d();
            queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&inverse));
        }
        ready
    }

    /// Draws the sky prepared last for `path` (see [`Self::prepare`]).
    pub fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>, path: &Path) {
        let Some(Ok(bind_group)) = self.skies.get(&SkyKey::of(path)) else { return };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Why the image at `path` could not be shown, if it could not.
    pub fn error(&self, path: &Path) -> Option<&str> {
        match self.skies.get(&SkyKey::of(path)) {
            Some(Err(error)) => Some(error),
            _ => None,
        }
    }

    fn upload(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &Path,
        image: &image::RgbaImage,
    ) -> wgpu::BindGroup {
        let size = wgpu::Extent3d { width: image.width(), height: image.height(), depth_or_array_layers: 1 };
        let label = format!("sky {}", path.display());
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            image.as_raw(),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * image.width()),
                rows_per_image: Some(image.height()),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&label),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        })
    }
}

/// Reads the image at `path` as RGBA, made smaller if a side is longer than
/// `max_side` (the largest texture the GPU takes).
fn read_sky_image(path: &Path, max_side: u32) -> Result<image::RgbaImage, String> {
    let image = image::open(path).map_err(|error| format!("could not read the image: {error}"))?;
    let image = if image.width() > max_side || image.height() > max_side {
        image.resize(max_side, max_side, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    Ok(image.to_rgba8())
}

#[cfg(test)]
mod tests {
    /// The shaders are only compiled when the program starts, so they are
    /// checked here as wgpu checks them.
    #[test]
    fn shaders_are_valid() {
        for (name, source) in
            [("sky.wgsl", include_str!("sky.wgsl")), ("shaders.wgsl", include_str!("shaders.wgsl"))]
        {
            let module =
                naga::front::wgsl::parse_str(source).unwrap_or_else(|error| panic!("{name}: {error}"));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        }
    }
}
