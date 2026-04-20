//! Cube mesh geometry and instance data for instanced rendering

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// A single cube instance sent to GPU
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CubeInstance {
    pub model: [[f32; 4]; 4],  // 64B: model matrix (4 columns)
    pub color: [f32; 4],       // 16B: RGBA
    pub hash: u32,             //  4B: name hash for effects
    pub object_id: u32,        //  4B: unique ID for picking
    pub _padding: [u32; 2],    //  8B: align to 16
}                              // Total: 96B

impl CubeInstance {
    pub fn new(model: Mat4, color: [f32; 4], hash: u32, object_id: u32) -> Self {
        Self {
            model: model.to_cols_array_2d(),
            color,
            hash,
            object_id,
            _padding: [0; 2],
        }
    }

    const ATTRIBS: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
        2 => Float32x4, // model col 0
        3 => Float32x4, // model col 1
        4 => Float32x4, // model col 2
        5 => Float32x4, // model col 3
        6 => Float32x4, // color
        7 => Uint32,     // hash
        8 => Uint32,     // object_id
    ];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Cube mesh vertex
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

impl Vertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
    ];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Shared vertex buffer layout for all cube pipelines
pub fn cube_vertex_layouts() -> [wgpu::VertexBufferLayout<'static>; 2] {
    [Vertex::desc(), CubeInstance::desc()]
}

/// Unit cube vertices (centered at origin, 1x1x1)
pub const CUBE_VERTICES: &[Vertex] = &[
    // Front (+Z)
    Vertex { position: [-0.5, -0.5,  0.5], normal: [0.0, 0.0, 1.0] },
    Vertex { position: [ 0.5, -0.5,  0.5], normal: [0.0, 0.0, 1.0] },
    Vertex { position: [ 0.5,  0.5,  0.5], normal: [0.0, 0.0, 1.0] },
    Vertex { position: [-0.5,  0.5,  0.5], normal: [0.0, 0.0, 1.0] },
    // Back (-Z)
    Vertex { position: [ 0.5, -0.5, -0.5], normal: [0.0, 0.0, -1.0] },
    Vertex { position: [-0.5, -0.5, -0.5], normal: [0.0, 0.0, -1.0] },
    Vertex { position: [-0.5,  0.5, -0.5], normal: [0.0, 0.0, -1.0] },
    Vertex { position: [ 0.5,  0.5, -0.5], normal: [0.0, 0.0, -1.0] },
    // Top (+Y)
    Vertex { position: [-0.5,  0.5,  0.5], normal: [0.0, 1.0, 0.0] },
    Vertex { position: [ 0.5,  0.5,  0.5], normal: [0.0, 1.0, 0.0] },
    Vertex { position: [ 0.5,  0.5, -0.5], normal: [0.0, 1.0, 0.0] },
    Vertex { position: [-0.5,  0.5, -0.5], normal: [0.0, 1.0, 0.0] },
    // Bottom (-Y)
    Vertex { position: [-0.5, -0.5, -0.5], normal: [0.0, -1.0, 0.0] },
    Vertex { position: [ 0.5, -0.5, -0.5], normal: [0.0, -1.0, 0.0] },
    Vertex { position: [ 0.5, -0.5,  0.5], normal: [0.0, -1.0, 0.0] },
    Vertex { position: [-0.5, -0.5,  0.5], normal: [0.0, -1.0, 0.0] },
    // Right (+X)
    Vertex { position: [ 0.5, -0.5,  0.5], normal: [1.0, 0.0, 0.0] },
    Vertex { position: [ 0.5, -0.5, -0.5], normal: [1.0, 0.0, 0.0] },
    Vertex { position: [ 0.5,  0.5, -0.5], normal: [1.0, 0.0, 0.0] },
    Vertex { position: [ 0.5,  0.5,  0.5], normal: [1.0, 0.0, 0.0] },
    // Left (-X)
    Vertex { position: [-0.5, -0.5, -0.5], normal: [-1.0, 0.0, 0.0] },
    Vertex { position: [-0.5, -0.5,  0.5], normal: [-1.0, 0.0, 0.0] },
    Vertex { position: [-0.5,  0.5,  0.5], normal: [-1.0, 0.0, 0.0] },
    Vertex { position: [-0.5,  0.5, -0.5], normal: [-1.0, 0.0, 0.0] },
];

pub const CUBE_INDICES: &[u16] = &[
    0,  1,  2,  0,  2,  3,   // Front
    4,  5,  6,  4,  6,  7,   // Back
    8,  9,  10, 8,  10, 11,  // Top
    12, 13, 14, 12, 14, 15,  // Bottom
    16, 17, 18, 16, 18, 19,  // Right
    20, 21, 22, 20, 22, 23,  // Left
];

pub const NUM_INDICES: u32 = 36;
