// Draws a skybox behind everything: a 360-degree panorama (latitude and
// longitude, twice as wide as it is high) mapped as Unity's Skybox/Panoramic
// shader maps it, so the sky faces the same way as in Daggerfall Unity.

struct Sky {
    // Clip space back to world space.
    inverse_view_projection: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> sky: Sky;
@group(0) @binding(1) var sky_texture: texture_2d<f32>;
@group(0) @binding(2) var sky_sampler: sampler;

struct SkyOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

// One triangle covering the whole view.
@vertex
fn vs_sky(@builtin(vertex_index) index: u32) -> SkyOutput {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    let ndc = corner * 2.0 - 1.0;
    var output: SkyOutput;
    output.position = vec4<f32>(ndc, 0.0, 1.0);
    output.ndc = ndc;
    return output;
}

const PI: f32 = 3.14159265358979;

@fragment
fn fs_sky(input: SkyOutput) -> @location(0) vec4<f32> {
    // The direction the camera looks through this pixel.
    let near = sky.inverse_view_projection * vec4<f32>(input.ndc, 0.0, 1.0);
    let far = sky.inverse_view_projection * vec4<f32>(input.ndc, 0.5, 1.0);
    let direction = normalize(far.xyz / far.w - near.xyz / near.w);
    // As Unity: u = 0.5 - longitude / 2pi, with the image's top straight up.
    let latitude = acos(clamp(direction.y, -1.0, 1.0));
    let longitude = atan2(direction.z, direction.x);
    let uv = vec2<f32>(0.5 - longitude / (2.0 * PI), latitude / PI);
    return textureSampleLevel(sky_texture, sky_sampler, uv, 0.0);
}
