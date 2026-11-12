struct Sweep { values: vec4<f32>, size: vec4<f32> }
@group(0) @binding(0) var<uniform> sweep: Sweep;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corners = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0));
    return vec4(corners[index] * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = vec2(position.x / sweep.size.x, 1.0 - position.y / sweep.size.y);
    return glimm(uv, sweep.values.x, sweep.values.y, sweep.values.z,
        vec3(0.8648670315742493, 0.1669161021709442, 0.7479681372642517),
        vec3(0.5743485093116760, 0.7505569458007812, 0.6149550676345825),
        vec3(0.5),
        vec3(0.1257428377866745, 0.7517492175102234, 0.39824193716049194));
}
