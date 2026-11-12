// MIT License
//
// Copyright (c) 2026 Noman Ijaz - iamnoman.com
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

// Fragment arithmetic translated from the locked Glimm 0.3.0 composer shader.
// `uv` uses the WebGL orientation: (0, 0) is bottom-left and (1, 1) top-right.
// The caller owns clipping, target color encoding, palette fitting, and timing.
const PI: f32 = 3.14159265359;

fn glimm(
    uv: vec2<f32>,
    elapsed: f32,
    progress: f32,
    alpha: f32,
    pal_a: vec3<f32>,
    pal_b: vec3<f32>,
    pal_c: vec3<f32>,
    pal_d: vec3<f32>,
) -> vec4<f32> {
    let axis = uv.x;
    let cross = uv.y;

    let pos = -0.2 + progress * (1.2 - -0.2);
    let tw = elapsed * 1.8;
    let d = axis - pos;
    let band = exp(-d * d * 10.0);

    let dh_daxis = -2.0 * d * 10.0 * band;
    let slope_x = dh_daxis;
    let n = normalize(vec3<f32>(-slope_x * 0.18, 0.0, 1.0));

    let trail = pow(clamp(0.5 - d * 1.3, 0.0, 1.0), 2.5) * 0.30;
    let midpoint_focus = 4.0 * progress * (1.0 - progress);
    let halo = exp(-d * d * 2.5) * 0.12 * midpoint_focus;
    let intensity = clamp(max(band, trail) + halo * (1.0 - band), 0.0, 1.0);

    let vfade = smoothstep(0.0, 0.015, cross)
        * (1.0 - smoothstep(0.985, 1.0, cross));

    let ripple = sin(cross * 12.0 + axis * 3.0 + tw * 0.40) * 0.015;
    let t = n.x * 0.12
        + axis * 0.90
        + cross * 0.16
        + ripple
        + elapsed * 0.04;
    let col = (glimm_pal(t, pal_a, pal_b, pal_c, pal_d) * 0.50
        + glimm_pal(t - 0.18, pal_a, pal_b, pal_c, pal_d) * 0.25
        + glimm_pal(t + 0.18, pal_a, pal_b, pal_c, pal_d) * 0.25) * 1.4;

    let v = vec3<f32>(0.0, 0.0, 1.0);
    let l = normalize(vec3<f32>(0.35, 0.55, 0.9));
    let h = normalize(l + v);
    let ndoth = clamp(dot(n, h), 0.0, 1.0);
    let ndotv = clamp(dot(n, v), 0.0, 1.0);
    let fresnel = pow(1.0 - ndotv, 3.0);
    let spec = pow(ndoth, 80.0);

    let entry_fade = mix(0.2, 1.0, 4.0 * progress * (1.0 - progress));
    let body_a = intensity * vfade * alpha * entry_fade;
    let body_pm = col * body_a;

    let high_mask = band * vfade * alpha * entry_fade;
    let high_emit = (col * fresnel * 0.55 + vec3<f32>(spec) * 1.1) * high_mask;
    let high_a = (fresnel * 0.4 + spec * 0.9) * high_mask;

    return vec4<f32>(body_pm + high_emit, min(body_a + high_a, 1.0));
}

fn glimm_pal(t: f32, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return a + b * cos(2.0 * PI * (c * t + d));
}
