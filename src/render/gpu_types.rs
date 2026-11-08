//! Plain-old-data structures shared with the shaders, and their vertex layouts.

use bytemuck::{Pod, Zeroable};
use eframe::egui_wgpu::wgpu;
use glam::{Mat4, Vec3, Vec4};

/// Uniform block bound at group 0, binding 0 (`Camera` in the shader).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct CameraUniform {
    /// World to clip space transform.
    pub view_projection: [[f32; 4]; 4],
    /// Eye position (w unused).
    pub eye_position: [f32; 4],
    /// Horizontal right vector for billboards (w unused).
    pub billboard_right: [f32; 4],
}

impl CameraUniform {
    /// Builds the uniform from camera matrices and vectors.
    pub fn new(view_projection: Mat4, eye_position: Vec3, billboard_right: Vec3) -> Self {
        Self {
            view_projection: view_projection.to_cols_array_2d(),
            eye_position: eye_position.extend(1.0).to_array(),
            billboard_right: billboard_right.extend(0.0).to_array(),
        }
    }
}

/// One vertex of a model mesh.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Normalised texture coordinate.
    pub uv: [f32; 2],
}

impl ModelVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];

    /// Vertex buffer layout for slot 0 of the model pipeline.
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// Per-instance data for a model: its transform and a highlight colour.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ModelInstance {
    pub model_matrix: [[f32; 4]; 4],
    /// RGB highlight colour and blend strength in alpha.
    pub highlight: [f32; 4],
}

impl ModelInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4
    ];

    /// Vertex buffer layout for slot 1 of the model pipeline.
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// Per-instance data for a billboard sprite.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct BillboardInstance {
    pub center: [f32; 3],
    /// Width and height in world units.
    pub size: [f32; 2],
    /// RGB highlight colour and blend strength in alpha.
    pub highlight: [f32; 4],
}

impl BillboardInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];

    /// Vertex buffer layout for slot 0 of the billboard pipeline.
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// One end of a coloured line segment.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

impl LineVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];

    /// Vertex buffer layout for the line pipeline.
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// Accumulates line segments for one frame.
#[derive(Default)]
pub struct LineBatch {
    pub vertices: Vec<LineVertex>,
}

impl LineBatch {
    /// Adds a segment from `start` to `end`.
    pub fn add_line(&mut self, start: Vec3, end: Vec3, color: Vec4) {
        let color = color.to_array();
        self.vertices.push(LineVertex { position: start.to_array(), color });
        self.vertices.push(LineVertex { position: end.to_array(), color });
    }

    /// Adds the twelve edges of an oriented box given its eight corners
    /// (in the order produced by [`crate::scene::box_corners`]).
    pub fn add_box(&mut self, corners: &[Vec3; 8], color: Vec4) {
        const EDGES: [(usize, usize); 12] =
            [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
        for (start, end) in EDGES {
            self.add_line(corners[start], corners[end], color);
        }
    }

    /// Adds a circle around `center` in the plane spanned by `axis_a` and `axis_b`.
    pub fn add_circle(&mut self, center: Vec3, radius: f32, axis_a: Vec3, axis_b: Vec3, color: Vec4) {
        const SEGMENTS: usize = 32;
        let point = |step: usize| {
            let angle = step as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            center + (axis_a * angle.cos() + axis_b * angle.sin()) * radius
        };
        for step in 0..SEGMENTS {
            self.add_line(point(step), point(step + 1), color);
        }
    }
}
