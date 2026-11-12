use super::State;
use ::rune::{
    compose::{Compositor, DeviceKey, Frame},
    render::{CLIP_WGSL, EffectSite},
};
use std::borrow::Cow;

pub(super) const MARK: u32 = 0x4d45544c;
const SWEEP_WORDS: usize = 8;
const GEOMETRY_WORDS: usize = 20;

#[derive(Default)]
pub(super) struct Glimm {
    epoch: u32,
    elapsed: Option<f64>,
    gpu: Option<Gpu>,
}

fn phase(elapsed: f64) -> Option<(f32, f32)> {
    if elapsed >= 0.65 {
        return None;
    }
    if elapsed <= 0.57 {
        let raw = elapsed / 0.57;
        let progress = if raw == 1.0 {
            1.0
        } else {
            1.0 - 2_f64.powf(-10.0 * raw)
        };
        Some((progress as f32, 1.3))
    } else {
        let raw = (elapsed - 0.57) / 0.08;
        let ease = if raw < 0.5 {
            4.0 * raw.powi(3)
        } else {
            1.0 - (-2.0 * raw + 2.0).powi(3) / 2.0
        };
        Some((1.0, (1.3 * (1.0 - ease)) as f32))
    }
}

fn extent(rect: [f32; 4], scale: f32) -> [u32; 2] {
    let ratio = scale.min(2.0) / scale;
    [
        (rect[2] * ratio).round().max(1.0) as u32,
        (rect[3] * ratio).round().max(1.0) as u32,
    ]
}

impl Compositor<State> for Glimm {
    fn render(&mut self, frame: &mut Frame<'_, State>) {
        let slot = frame.rt.marks.iter().position(|mark| *mark == MARK);
        let reduced = slot.is_none_or(|index| frame.rt.instances[index].uv[1] > 0.5);
        if let Some(elapsed) = &mut self.elapsed {
            *elapsed += frame.dt;
        }
        if self.epoch != frame.rt.state.celebration {
            self.epoch = frame.rt.state.celebration;
            if self.elapsed.is_none() && !reduced {
                self.elapsed = Some(0.0);
            }
        }
        if reduced {
            self.elapsed = None;
        }
        let active = self
            .elapsed
            .and_then(|elapsed| phase(elapsed).map(|(progress, alpha)| (elapsed, progress, alpha)));
        let Some((elapsed, progress, alpha)) = active else {
            self.elapsed = None;
            if let Some(gpu) = &mut self.gpu {
                gpu.image = None;
            }
            frame.draw_ui();
            return;
        };
        let rect = frame.rt.instances[slot.unwrap()].rect;
        if rect[2] <= 0.0 || rect[3] <= 0.0 {
            self.elapsed = None;
            frame.draw_ui();
            return;
        }
        let key = frame.device_key();
        if self.gpu.as_ref().is_none_or(|gpu| gpu.key != key) {
            self.gpu = Some(Gpu::new(frame.device, key));
        }
        let gpu = self.gpu.as_mut().unwrap();
        let size = extent(rect, frame.rt.env.scale);
        gpu.image(frame.device, size);
        let sweep = [
            elapsed as f32,
            progress,
            alpha,
            0.0,
            size[0] as f32,
            size[1] as f32,
            0.0,
            0.0,
        ]
        .map(f32::to_le_bytes);
        frame.write_buffer(&gpu.sweep, 0, sweep.as_flattened());
        {
            let image = gpu.image.as_ref().unwrap();
            let mut pass = frame
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("composer sweep"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &image.view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            pass.set_pipeline(&gpu.raw);
            pass.set_bind_group(0, &gpu.raw_bind, &[]);
            pass.draw(0..6, 0..1);
        }
        let clear = frame.rt.clear_color();
        let (device, queue) = (frame.device, frame.queue);
        frame.ui.prepare(
            device,
            queue,
            &mut frame.rt.text.atlas,
            &frame.rt.instances,
            frame.size,
        );
        frame
            .ui
            .draw_with(frame.encoder, frame.target, clear, &mut |pass, site| {
                if site.mark == MARK {
                    gpu.draw(device, queue, pass, site);
                }
            });
    }

    fn animating(&self) -> bool {
        self.elapsed.is_some()
    }
}

struct Image {
    size: [u32; 2],
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

struct Gpu {
    key: DeviceKey,
    raw: wgpu::RenderPipeline,
    raw_bind: wgpu::BindGroup,
    sweep: wgpu::Buffer,
    geometry: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    sampler: wgpu::Sampler,
    image: Option<Image>,
}

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("composer Glimm"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl Gpu {
    fn new(device: &wgpu::Device, key: DeviceKey) -> Self {
        let buffer = |size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("composer uniforms"),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let sweep = buffer((SWEEP_WORDS * 4) as u64);
        let geometry = buffer((GEOMETRY_WORDS * 4) as u64);
        let raw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[uniform(0)],
        });
        let raw_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&raw_layout)],
            immediate_size: 0,
        });
        let raw_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &raw_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: sweep.as_entire_binding(),
            }],
        });
        let raw_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Glimm 0.3.0"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!(
                "{}\n{}",
                include_str!("glimm.wgsl"),
                include_str!("glimm-pass.wgsl")
            ))),
        });
        let raw = pipeline(
            device,
            &raw_pipeline_layout,
            &raw_shader,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
        );
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                uniform(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composer composite"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!(
                "{CLIP_WGSL}\n{}",
                include_str!("glimm-composite.wgsl")
            ))),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: None,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            key,
            raw,
            raw_bind,
            sweep,
            geometry,
            layout,
            pipeline_layout,
            shader,
            sampler,
            image: None,
            pipelines: Vec::with_capacity(2),
        }
    }

    fn image(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.image.as_ref().is_some_and(|image| image.size == size) {
            return;
        }
        // Match the original WebGL canvas's RGBA8 storage and its DPR cap before compositing.
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("composer sweep"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.geometry.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        self.image = Some(Image {
            size,
            _texture: texture,
            view,
            bind,
        });
    }

    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        site: &EffectSite<'_>,
    ) {
        let index = self
            .pipelines
            .iter()
            .position(|(format, _)| *format == site.format)
            .unwrap_or_else(|| {
                let pipeline = pipeline(
                    device,
                    &self.pipeline_layout,
                    &self.shader,
                    site.format,
                    Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                );
                if self.pipelines.len() == 2 {
                    self.pipelines.remove(0);
                }
                self.pipelines.push((site.format, pipeline));
                self.pipelines.len() - 1
            });
        let mut words = [0u32; GEOMETRY_WORDS];
        words[0..4].copy_from_slice(&site.instance.rect.map(f32::to_bits));
        words[4..8].copy_from_slice(&site.instance.clip.map(f32::to_bits));
        words[8..12].copy_from_slice(&site.instance.fade);
        words[12..16].copy_from_slice(
            &[
                site.origin[0],
                site.origin[1],
                site.size[0] as f32,
                site.size[1] as f32,
            ]
            .map(f32::to_bits),
        );
        words[16] = (if site.srgb_blend { 1.0f32 } else { 0.0 }).to_bits();
        let bytes = words.map(u32::to_le_bytes);
        queue.write_buffer(&self.geometry, 0, bytes.as_flattened());
        pass.set_pipeline(&self.pipelines[index].1);
        pass.set_bind_group(0, &self.image.as_ref().unwrap().bind, &[]);
        pass.draw(0..6, 0..1);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn shaders_parse_and_match_our_uniform_buffers() {
        let shaders = [
            (
                format!(
                    "{}\n{}",
                    include_str!("glimm.wgsl"),
                    include_str!("glimm-pass.wgsl")
                ),
                "sweep",
                SWEEP_WORDS * 4,
                wgpu::naga::valid::Capabilities::empty(),
            ),
            (
                format!("{CLIP_WGSL}\n{}", include_str!("glimm-composite.wgsl")),
                "geometry",
                GEOMETRY_WORDS * 4,
                wgpu::naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32,
            ),
        ];
        for (source, name, bytes, capabilities) in shaders {
            let module = wgpu::naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                capabilities,
            )
            .validate(&module)
            .unwrap();
            let (_, uniform) = module
                .global_variables
                .iter()
                .find(|(_, global)| global.name.as_deref() == Some(name))
                .unwrap();
            assert!(
                matches!(module.types[uniform.ty].inner, wgpu::naga::TypeInner::Struct { span, .. } if span as usize == bytes)
            );
        }
    }

    #[test]
    fn sweep_finishes_and_uses_the_original_canvas_resolution() {
        assert_eq!(phase(0.0), Some((0.0, 1.3)));
        assert_eq!(phase(0.57), Some((1.0, 1.3)));
        let (progress, alpha) = phase(0.61).unwrap();
        assert_eq!(progress, 1.0);
        assert!((alpha - 0.65).abs() < 0.00001);
        assert!(phase(0.65).is_none());
        assert_eq!(extent([0.0, 0.0, 600.0, 300.0], 3.0), [400, 200]);
        assert_eq!(extent([0.0, 0.0, 600.0, 300.0], 2.0), [600, 300]);
        assert_eq!(extent([0.0, 0.0, 600.0, 300.0], 1.0), [600, 300]);
    }
}
