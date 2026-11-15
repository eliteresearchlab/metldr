struct Geometry {
    rect: vec4<f32>,
    clip: vec4<f32>,
    fade: vec4<u32>,
    viewport: vec4<f32>,
    // Cell pitch and dot radius in device pixels, then the dot's opacity.
    params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> geometry: Geometry;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) window: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) index: u32) -> Vertex {
    let corners = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0));
    let window = geometry.rect.xy + corners[index] * geometry.rect.zw;
    let local = (window - geometry.viewport.xy) / geometry.viewport.zw;
    return Vertex(vec4(local.x * 2.0 - 1.0, 1.0 - local.y * 2.0, 0.0, 1.0), window);
}

@fragment
fn fs(vertex: Vertex) -> @location(0) vec4<f32> {
    let pitch = geometry.params.x;
    let cell = (vertex.window - geometry.rect.xy) - floor((vertex.window - geometry.rect.xy) / pitch) * pitch;
    let d = distance(cell, vec2(pitch * 0.5));
    let dot = 1.0 - smoothstep(geometry.params.y - 0.5, geometry.params.y + 0.5, d);
    let a = dot * geometry.params.z * clip_alpha(vertex.window, geometry.clip, geometry.fade);
    return vec4(0.0, 0.0, 0.0, a);
}
