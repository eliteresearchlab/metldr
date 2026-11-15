//! A theme's ground under its canvas, painted by the GPU in one pass over its slot instead of as
//! hundreds of elements: a molded theme's grid of small dots, one every 16 points, a neon
//! theme's scanlines, one every 3 points, or a retro theme's still film grain.

use ::rune::render::{CLIP_WGSL, EffectSite};
use std::borrow::Cow;

/// The `El::effect` mark of a ground.
pub(super) const MARK: u32 = 0x444f5453;
const WORDS: usize = 24;

/// The slot to put under a theme's canvas: it fills its parent and draws the theme's ground, or
/// nothing on a flat theme. Its data carries the kind, the opacity and the colour.
pub(super) fn ground(p: &super::theme::Palette) -> Option<::rune::El> {
    let (kind, opacity, color) = if p.molded() {
        (0.0, 0.14, ::rune::Color::BLACK)
    } else if p.neon() {
        (1.0, 0.07, p.accent)
    } else if p.retro() {
        (2.0, 0.06, p.ink)
    } else {
        return None;
    };
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    let packed = (channel(color.r) << 16 | channel(color.g) << 8 | channel(color.b)) as f32;
    Some(
        ::rune::effect(MARK)
            .effect_data([kind, opacity, packed, 0.0])
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .a11y_hidden(),
    )
}

pub(super) struct Ground {
    geometry: wgpu::Buffer,
    bind: wgpu::BindGroup,
    layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl Ground {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let geometry = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ground"),
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
            label: Some("ground"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!(
                "{CLIP_WGSL}\n{}",
                include_str!("ground.wgsl")
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
        // Dots: 16 point cells with a 1 point radius. Scanlines: a 1 point line every 3 points.
        let [kind, opacity, packed, _] = site.data;
        // Grain: one random speck per point.
        let (pitch, size) = match kind as u32 {
            0 => (16.0, 1.0),
            1 => (3.0, 1.0),
            _ => (1.0, 1.0),
        };
        words[16..20].copy_from_slice(&[pitch * site.scale, size * site.scale, opacity, kind].map(f32::to_bits));
        let packed = packed as u32;
        let channel = |shift: u32| ((packed >> shift) & 0xff) as f32 / 255.0;
        words[20..24].copy_from_slice(&[channel(16), channel(8), channel(0), 1.0].map(f32::to_bits));
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
