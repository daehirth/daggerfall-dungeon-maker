//! GPU copies of textures and models, created on demand from the asset library.

use std::ops::Range;
use std::rc::Rc;

use eframe::egui_wgpu::wgpu;
use eframe::egui_wgpu::wgpu::util::DeviceExt;

use super::gpu_types::ModelVertex;
use crate::arena2::{ModelMesh, RgbaImage, TextureKey};

/// Texture format used for every surface texture (colours are stored in sRGB).
const SURFACE_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Size of the checkerboard used when a texture cannot be loaded.
const FALLBACK_TEXTURE_SIZE: u32 = 64;

/// A texture uploaded to the GPU together with the bind group that samples it.
pub struct GpuTexture {
    /// Bind group for group 1 of the surface shaders.
    pub bind_group: wgpu::BindGroup,
    /// Width in pixels of the source image.
    pub width: u32,
    /// Height in pixels of the source image.
    pub height: u32,
}

/// The part of a [`GpuModel`] drawn with one texture.
pub struct GpuSubMesh {
    /// Texture used by this range of indices.
    pub texture: Rc<GpuTexture>,
    /// Range within the model's index buffer.
    pub index_range: Range<u32>,
}

/// A model uploaded to the GPU.
pub struct GpuModel {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub submeshes: Vec<GpuSubMesh>,
}

/// Creates the bind group layout used by every surface texture.
pub fn create_texture_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("surface texture layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// Creates the sampler shared by every surface texture.
///
/// Magnification uses nearest filtering to keep the crisp pixel art look,
/// while minification blends mip levels to avoid shimmering in the distance.
pub fn create_surface_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("surface sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

/// Uploads `image` (and a generated mip chain) to the GPU.
pub fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    label: &str,
    image: &RgbaImage,
) -> GpuTexture {
    let mip_chain = build_mip_chain(image);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: image.width.max(1),
            height: image.height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: mip_chain.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SURFACE_TEXTURE_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    for (mip_level, mip_image) in mip_chain.iter().enumerate() {
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: mip_level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &mip_image.pixels,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * mip_image.width),
                rows_per_image: Some(mip_image.height),
            },
            wgpu::Extent3d { width: mip_image.width, height: mip_image.height, depth_or_array_layers: 1 },
        );
    }

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });

    GpuTexture { bind_group, width: image.width.max(1), height: image.height.max(1) }
}

/// Builds a magenta and black checkerboard used in place of missing textures.
pub fn fallback_image() -> RgbaImage {
    const CHECK_SIZE: u32 = 8;
    let size = FALLBACK_TEXTURE_SIZE;
    let pixels = (0..size * size)
        .flat_map(|pixel_index| {
            let (x, y) = (pixel_index % size, pixel_index / size);
            if (x / CHECK_SIZE + y / CHECK_SIZE).is_multiple_of(2) {
                [255, 0, 255, 255]
            } else {
                [24, 24, 24, 255]
            }
        })
        .collect();
    RgbaImage { width: size, height: size, pixels }
}

/// Generates successively halved copies of `image` using a 2x2 box filter.
fn build_mip_chain(image: &RgbaImage) -> Vec<RgbaImage> {
    let mut chain = vec![image.clone()];
    while let Some(previous) = chain.last() {
        if previous.width == 1 && previous.height == 1 {
            break;
        }
        let width = (previous.width / 2).max(1);
        let height = (previous.height / 2).max(1);
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);

        for y in 0..height {
            for x in 0..width {
                let mut sum = [0u32; 4];
                for (offset_x, offset_y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let source_x = (x * 2 + offset_x).min(previous.width - 1);
                    let source_y = (y * 2 + offset_y).min(previous.height - 1);
                    let source_index = ((source_y * previous.width + source_x) * 4) as usize;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += u32::from(previous.pixels[source_index + channel]);
                    }
                }
                pixels.extend(sum.map(|total| (total / 4) as u8));
            }
        }
        chain.push(RgbaImage { width, height, pixels });
    }
    chain
}

/// Uploads a model, normalising texture coordinates with each sub-mesh's texture size.
///
/// `texture_for` resolves a texture key to an uploaded texture (or the fallback).
pub fn upload_model(
    device: &wgpu::Device,
    mesh: &ModelMesh,
    mut texture_for: impl FnMut(TextureKey) -> Rc<GpuTexture>,
) -> GpuModel {
    let mut vertices: Vec<ModelVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut submeshes = Vec::with_capacity(mesh.submeshes.len());

    for submesh in &mesh.submeshes {
        let texture = texture_for(submesh.texture);
        let texture_size = glam::Vec2::new(texture.width as f32, texture.height as f32);
        let first_vertex = vertices.len() as u32;
        let first_index = indices.len() as u32;

        vertices.extend(submesh.vertices.iter().map(|vertex| ModelVertex {
            position: vertex.position.to_array(),
            normal: vertex.normal.to_array(),
            uv: (vertex.uv_texels / texture_size).to_array(),
        }));
        indices.extend(submesh.indices.iter().map(|index| index + first_vertex));

        submeshes.push(GpuSubMesh { texture, index_range: first_index..indices.len() as u32 });
    }

    let label = format!("model {}", mesh.model_id);
    GpuModel {
        vertex_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&label),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        index_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&label),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        }),
        submeshes,
    }
}

/// A GPU buffer that grows to fit whatever is written to it each frame.
pub struct GrowableBuffer {
    buffer: wgpu::Buffer,
    capacity: u64,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl GrowableBuffer {
    /// Creates a buffer with a small initial capacity.
    pub fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        const INITIAL_CAPACITY: u64 = 64 * 1024;
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        Self {
            buffer: Self::allocate(device, label, usage, INITIAL_CAPACITY),
            capacity: INITIAL_CAPACITY,
            usage,
            label,
        }
    }

    fn allocate(device: &wgpu::Device, label: &str, usage: wgpu::BufferUsages, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    }

    /// Writes `bytes` to the start of the buffer, reallocating if it is too small.
    pub fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) {
        let required = bytes.len() as u64;
        if required > self.capacity {
            self.capacity = required.next_power_of_two();
            self.buffer = Self::allocate(device, self.label, self.usage, self.capacity);
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytes);
        }
    }

    /// The underlying wgpu buffer.
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}
