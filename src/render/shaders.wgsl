// Shaders for the 3D viewport.
//
// Three pipelines share the camera uniform:
//   * models    - textured, lit geometry drawn with per-instance transforms
//   * billboards - upright camera-facing sprites for flats and markers
//   * lines     - coloured helper lines (grid, block outlines, lights, selection)

struct Camera {
    view_projection: mat4x4<f32>,
    // xyz: eye position in world space.
    eye_position: vec4<f32>,
    // xyz: horizontal right vector used to orient billboards.
    billboard_right: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var surface_texture: texture_2d<f32>;
@group(1) @binding(1) var surface_sampler: sampler;

// Fraction of light every surface receives regardless of orientation.
const AMBIENT_LIGHT: f32 = 0.45;

struct SurfaceVaryings {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) brightness: f32,
    // rgb: highlight colour, a: highlight strength.
    @location(2) highlight: vec4<f32>,
};

// ---------------------------------------------------------------- models

struct ModelVertex {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct ModelInstance {
    @location(3) model_column_0: vec4<f32>,
    @location(4) model_column_1: vec4<f32>,
    @location(5) model_column_2: vec4<f32>,
    @location(6) model_column_3: vec4<f32>,
    @location(7) highlight: vec4<f32>,
};

@vertex
fn vs_model(vertex: ModelVertex, instance: ModelInstance) -> SurfaceVaryings {
    let model = mat4x4<f32>(
        instance.model_column_0,
        instance.model_column_1,
        instance.model_column_2,
        instance.model_column_3,
    );
    let world_position = model * vec4<f32>(vertex.position, 1.0);
    let world_normal = normalize((model * vec4<f32>(vertex.normal, 0.0)).xyz);

    // A "headlight" keeps every surface readable no matter where the camera is.
    let to_eye = normalize(camera.eye_position.xyz - world_position.xyz);
    let facing = abs(dot(world_normal, to_eye));

    var output: SurfaceVaryings;
    output.clip_position = camera.view_projection * world_position;
    output.uv = vertex.uv;
    output.brightness = AMBIENT_LIGHT + (1.0 - AMBIENT_LIGHT) * facing;
    output.highlight = instance.highlight;
    return output;
}

// ------------------------------------------------------------ billboards

struct BillboardInstance {
    @location(0) center: vec3<f32>,
    @location(1) size: vec2<f32>,
    @location(2) highlight: vec4<f32>,
};

@vertex
fn vs_billboard(@builtin(vertex_index) vertex_index: u32, instance: BillboardInstance) -> SurfaceVaryings {
    // Two triangles forming a unit quad centred on the origin.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, -0.5),
        vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, 0.5),
    );
    let corner = corners[vertex_index];
    let right = camera.billboard_right.xyz;
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let world_position = instance.center
        + right * corner.x * instance.size.x
        + up * corner.y * instance.size.y;

    var output: SurfaceVaryings;
    output.clip_position = camera.view_projection * vec4<f32>(world_position, 1.0);
    output.uv = vec2<f32>(corner.x + 0.5, 0.5 - corner.y);
    output.brightness = 1.0;
    output.highlight = instance.highlight;
    return output;
}

// --------------------------------------------------- shared surface shading

@fragment
fn fs_surface(input: SurfaceVaryings) -> @location(0) vec4<f32> {
    let texel = textureSample(surface_texture, surface_sampler, input.uv);
    // Palette index 0 is transparent; use alpha testing so depth stays correct.
    if texel.a < 0.5 {
        discard;
    }
    let lit_color = texel.rgb * input.brightness;
    return vec4<f32>(mix(lit_color, input.highlight.rgb, input.highlight.a), 1.0);
}

// ------------------------------------------------------------------ lines

struct LineVertex {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct LineVaryings {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_line(vertex: LineVertex) -> LineVaryings {
    var output: LineVaryings;
    output.clip_position = camera.view_projection * vec4<f32>(vertex.position, 1.0);
    output.color = vertex.color;
    return output;
}

@fragment
fn fs_line(input: LineVaryings) -> @location(0) vec4<f32> {
    return input.color;
}
