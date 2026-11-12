struct Geometry {
    rect: vec4<f32>,
    clip: vec4<f32>,
    fade: vec4<u32>,
    viewport: vec4<f32>,
    color: vec4<f32>,
}
@group(0) @binding(0) var<uniform> geometry: Geometry;
@group(0) @binding(1) var filtering: sampler;
@group(0) @binding(2) var sweep_image: texture_2d<f32>;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) window: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) index: u32) -> Vertex {
    let corners = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0));
    let uv = corners[index];
    let window = geometry.rect.xy + uv * geometry.rect.zw;
    let local = (window - geometry.viewport.xy) / geometry.viewport.zw;
    return Vertex(vec4(local.x * 2.0 - 1.0, 1.0 - local.y * 2.0, 0.0, 1.0), uv, window);
}

@fragment
fn fs(vertex: Vertex) -> @location(0) vec4<f32> {
    var color = textureSample(sweep_image, filtering, vertex.uv);
    if geometry.color.x < 0.5 && color.a > 0.0 {
        let rgb = color.rgb / color.a;
        let linear = select(rgb / 12.92, pow((rgb + vec3(0.055)) / 1.055, vec3(2.4)), rgb > vec3(0.04045));
        color = vec4(linear * color.a, color.a);
    }
    return color * clip_alpha(vertex.window, geometry.clip, geometry.fade);
}
