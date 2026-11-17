//! The welcome stage's moving sky: Paper Shaders' mesh gradient on WebGL2. It draws at half
//! resolution, runs only while the hero is on screen and the tab is visible, and draws one still
//! frame when the user prefers reduced motion. Without WebGL the hero keeps its CSS gradient.

use std::{cell::RefCell, rc::Rc};

use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{
    Document, Event, HtmlCanvasElement, IntersectionObserver, IntersectionObserverEntry,
    WebGl2RenderingContext as Gl, WebGlProgram, WebGlShader, WebGlUniformLocation,
};

use crate::dom;

const VERTEX: &str = r#"#version 300 es
uniform vec2 u_resolution;
out vec2 v_objectUV;
void main() {
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2)) * 2.0 - 1.0;
  v_objectUV = 0.5 * p * u_resolution / max(u_resolution.x, u_resolution.y);
  gl_Position = vec4(p, 0.0, 1.0);
}
"#;

// Paper Shaders mesh gradient (Apache-2.0), fixed to five colour spots with a light grain.
const FRAGMENT: &str = r#"#version 300 es
precision mediump float;
uniform float u_time;
uniform vec4 u_colors[5];
in vec2 v_objectUV;
out vec4 fragColor;
const float u_distortion = 0.8;
const float u_swirl = 0.12;
const float u_grainOverlay = 0.12;
vec2 rotate(vec2 uv, float th) { return mat2(cos(th), sin(th), -sin(th), cos(th)) * uv; }
float hash21(vec2 p) {
  p = fract(p * vec2(0.3183099, 0.3678794)) + 0.1;
  p += dot(p, p + 19.19);
  return fract(p.x * p.y);
}
float valueNoise(vec2 st) {
  vec2 i = floor(st);
  vec2 f = fract(st);
  float a = hash21(i);
  float b = hash21(i + vec2(1.0, 0.0));
  float c = hash21(i + vec2(0.0, 1.0));
  float d = hash21(i + vec2(1.0, 1.0));
  vec2 u = f * f * (3.0 - 2.0 * f);
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
vec2 getPosition(int i, float t) {
  float a = float(i) * .37;
  float b = .6 + fract(float(i) / 3.) * .9;
  float c = .8 + fract(float(i + 1) / 4.);
  return .5 + .5 * vec2(sin(t * b + a), cos(t * c + a * 1.5));
}
void main() {
  vec2 uv = v_objectUV + .5;
  vec2 grainUV = gl_FragCoord.xy * 2.;
  float t = .5 * (u_time + 41.5);
  float radius = smoothstep(0., 1., length(uv - .5));
  float center = 1. - radius;
  for (float i = 1.; i <= 2.; i++) {
    uv.x += u_distortion * center / i * sin(t + i * .4 * smoothstep(.0, 1., uv.y)) * cos(.2 * t + i * 2.4 * smoothstep(.0, 1., uv.y));
    uv.y += u_distortion * center / i * cos(t + i * 2. * smoothstep(.0, 1., uv.x));
  }
  vec2 uvRotated = rotate(uv - .5, -3. * u_swirl * radius) + .5;
  vec3 color = vec3(0.);
  float totalWeight = 0.;
  for (int i = 0; i < 5; i++) {
    float dist = pow(length(uvRotated - getPosition(i, t)), 3.5);
    float weight = 1. / (dist + 1e-3);
    color += u_colors[i].rgb * weight;
    totalWeight += weight;
  }
  color /= max(1e-4, totalWeight);
  float grain = mix(valueNoise(rotate(grainUV, 1.) + vec2(3.)), valueNoise(rotate(grainUV, 2.) + vec2(-1.)), .5);
  grain = pow(grain, 1.3) * 2. - 1.;
  color = mix(color, vec3(step(0., grain)), .35 * pow(u_grainOverlay * abs(grain), .8));
  fragColor = vec4(color, 1.);
}
"#;

/// A clear sky as sRGB 0 to 1: cobalt, open sky, cloud white, deep cobalt and a pale blue.
const SKY: [f32; 20] = [
    0.16, 0.36, 0.84, 1.0, //
    0.42, 0.64, 0.96, 1.0, //
    0.95, 0.96, 0.99, 1.0, //
    0.09, 0.22, 0.66, 1.0, //
    0.68, 0.80, 0.98, 1.0,
];

/// Seconds of shader time per second of wall time: slow enough to read calmly over.
const SPEED: f64 = 0.25;

/// The shortest gap between two drawn frames, in milliseconds.
const FRAME_MS: f64 = 1000.0 / 30.0 - 2.0;

struct Field {
    canvas: HtmlCanvasElement,
    gl: Gl,
    resolution: WebGlUniformLocation,
    time: WebGlUniformLocation,
    visible: bool,
    still: bool,
    running: bool,
    drawn: bool,
    last: f64,
    frame: Option<Closure<dyn FnMut(f64)>>,
}

pub fn mount(document: &Document) -> Result<(), JsValue> {
    let canvas = dom::element(document, "backdrop")?.dyn_into::<HtmlCanvasElement>()?;
    let Some(gl) = canvas
        .get_context("webgl2")?
        .and_then(|context| context.dyn_into::<Gl>().ok())
    else {
        return Ok(());
    };
    let program = link(&gl)?;
    gl.use_program(Some(&program));
    let uniform = |name: &str| {
        gl.get_uniform_location(&program, name)
            .ok_or_else(|| JsValue::from_str("Backdrop uniform missing"))
    };
    let resolution = uniform("u_resolution")?;
    let time = uniform("u_time")?;
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("Window unavailable"))?;
    gl.uniform4fv_with_f32_array(Some(&uniform("u_colors")?), &SKY);
    let still = window
        .match_media("(prefers-reduced-motion: reduce)")?
        .is_some_and(|query| query.matches());
    let field = Rc::new(RefCell::new(Field {
        canvas,
        gl,
        resolution,
        time,
        visible: false,
        still,
        running: false,
        drawn: false,
        last: f64::NEG_INFINITY,
        frame: None,
    }));
    let looping = field.clone();
    field.borrow_mut().frame = Some(Closure::new(move |now: f64| {
        let mut field = looping.borrow_mut();
        field.running = false;
        // The sky moves slowly, so 30 frames a second look the same as 60 at half the work.
        if now - field.last >= FRAME_MS {
            field.last = now;
            field.draw(now / 1000.0 * SPEED);
        }
        field.schedule();
    }));

    let observed = field.clone();
    let intersect = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
        let mut field = observed.borrow_mut();
        for entry in entries
            .iter()
            .filter_map(|entry| entry.dyn_into::<IntersectionObserverEntry>().ok())
        {
            field.visible = entry.is_intersecting();
        }
        field.schedule();
    });
    let observer = IntersectionObserver::new(intersect.as_ref().unchecked_ref())?;
    observer.observe(&field.borrow().canvas);
    intersect.forget();

    let shown = field.clone();
    let visibility = Closure::<dyn FnMut(Event)>::new(move |_| shown.borrow_mut().schedule());
    document.add_event_listener_with_callback(
        "visibilitychange",
        visibility.as_ref().unchecked_ref(),
    )?;
    visibility.forget();

    let leaving = Closure::<dyn FnMut(Event)>::new(move |_| {
        observer.disconnect();
        field.borrow_mut().visible = false;
    });
    window.add_event_listener_with_callback("pagehide", leaving.as_ref().unchecked_ref())?;
    leaving.forget();
    Ok(())
}

impl Field {
    /// Asks for the next frame while the hero is on screen and the tab is shown; a still
    /// field stops after its first frame.
    fn schedule(&mut self) {
        let hidden = self.canvas.owner_document().is_some_and(|d| d.hidden());
        if self.running || !self.visible || hidden || (self.still && self.drawn) {
            return;
        }
        if let (Some(window), Some(frame)) = (web_sys::window(), &self.frame) {
            self.running = window
                .request_animation_frame(frame.as_ref().unchecked_ref())
                .is_ok();
        }
    }

    fn draw(&mut self, seconds: f64) {
        let width = (self.canvas.client_width().max(1) as f64 / 2.0).ceil() as u32;
        let height = (self.canvas.client_height().max(1) as f64 / 2.0).ceil() as u32;
        if self.canvas.width() != width || self.canvas.height() != height {
            self.canvas.set_width(width);
            self.canvas.set_height(height);
        }
        self.gl.viewport(0, 0, width as i32, height as i32);
        self.gl
            .uniform2f(Some(&self.resolution), width as f32, height as f32);
        let seconds = if self.still { 0.0 } else { seconds };
        self.gl.uniform1f(Some(&self.time), (seconds % 3600.0) as f32);
        self.gl.draw_arrays(Gl::TRIANGLES, 0, 3);
        if !self.drawn {
            self.drawn = true;
            let _ = self.canvas.set_attribute("data-drawn", "");
        }
    }
}

fn link(gl: &Gl) -> Result<WebGlProgram, JsValue> {
    let program = gl
        .create_program()
        .ok_or_else(|| JsValue::from_str("Backdrop program unavailable"))?;
    gl.attach_shader(&program, &compile(gl, Gl::VERTEX_SHADER, VERTEX)?);
    gl.attach_shader(&program, &compile(gl, Gl::FRAGMENT_SHADER, FRAGMENT)?);
    gl.link_program(&program);
    if gl
        .get_program_parameter(&program, Gl::LINK_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        Ok(program)
    } else {
        Err(JsValue::from_str("Backdrop program did not link"))
    }
}

fn compile(gl: &Gl, kind: u32, source: &str) -> Result<WebGlShader, JsValue> {
    let shader = gl
        .create_shader(kind)
        .ok_or_else(|| JsValue::from_str("Backdrop shader unavailable"))?;
    gl.shader_source(&shader, source);
    gl.compile_shader(&shader);
    if gl
        .get_shader_parameter(&shader, Gl::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        Ok(shader)
    } else {
        Err(JsValue::from_str("Backdrop shader did not compile"))
    }
}
