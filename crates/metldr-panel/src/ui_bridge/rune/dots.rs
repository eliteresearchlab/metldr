//! The molded theme's ground: a grid of small dots, one every 16 points, painted by the GPU in
//! one pass over its slot instead of as hundreds of elements.

use ::rune::render::{CLIP_WGSL, EffectSite};
use std::borrow::Cow;

/// The `El::effect` mark of the dot ground.
pub(super) const MARK: u32 = 0x444f5453;
const PITCH: f32 = 16.0;
const RADIUS: f32 = 1.0;
const OPACITY: f32 = 0.14;
const WORDS: usize = 20;

/// The slot to put under a molded theme's canvas: it fills its parent and draws the dots.
pub(super) fn ground() -> ::rune::El {
    ::rune::effect(MARK)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .a11y_hidden()
}

pub(super) struct Dots {
    geometry: wgpu::Buffer,
    bind: wgpu::BindGroup,
    layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl Dots {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let geometry = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dot ground"),
            size: (WORDS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[super::glimm::uniform(0)],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: geometry.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dot ground"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!(
                "{CLIP_WGSL}\n{}",
                include_str!("dots.wgsl")
            ))),
        });
        Self {
            geometry,
            bind,
            layout,
            shader,
            pipelines: Vec::with_capacity(2),
        }
    }

    pub(super) fn draw(
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
                let pipeline = super::glimm::pipeline(
                    device,
                    &self.layout,
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
        let mut words = [0u32; WORDS];
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
        words[16..20].copy_from_slice(
            &[PITCH * site.scale, RADIUS * site.scale, OPACITY, 0.0].map(f32::to_bits),
        );
        queue.write_buffer(
            &self.geometry,
            0,
            words.map(u32::to_le_bytes).as_flattened(),
        );
        pass.set_pipeline(&self.pipelines[index].1);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.draw(0..6, 0..1);
    }
}
