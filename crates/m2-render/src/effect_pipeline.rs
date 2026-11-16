//! The effect pipeline (`shaders/effect.wgsl`): the world-space triangles of
//! an [`EffectGeometry`], textured and blended. Group 0 is the frame, group 1
//! a texture. There is one pipeline per blending, made the first time a
//! draw asks for it.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Mutex;

use wgpu::util::DeviceExt as _;

use crate::RenderError;
use crate::effect::{EffectGeometry, EffectVertex};
use crate::gpu::Gpu;

const SHADER: &str = include_str!("shaders/effect.wgsl");

/// A texture on the GPU, for effects.
pub struct GpuTexture {
    pub(crate) group: wgpu::BindGroup,
}

/// One frame's effect triangles on the GPU.
pub struct GpuEffects {
    vertices: Option<wgpu::Buffer>,
    draws: Vec<GpuEffectDraw>,
}

struct GpuEffectDraw {
    first: u32,
    count: u32,
    texture: Option<u32>,
    pipeline: wgpu::RenderPipeline,
}

/// A Direct3D blend factor number (`D3DBLEND`, from Microsoft's Direct3D 9
/// documentation) as wgpu's. Unknown numbers count as one.
fn factor(number: u32) -> wgpu::BlendFactor {
    use wgpu::BlendFactor as F;
    match number {
        1 => F::Zero,
        3 => F::Src,
        4 => F::OneMinusSrc,
        5 => F::SrcAlpha,
        6 => F::OneMinusSrcAlpha,
        7 => F::DstAlpha,
        8 => F::OneMinusDstAlpha,
        9 => F::Dst,
        10 => F::OneMinusDst,
        11 => F::SrcAlphaSaturated,
        _ => F::One,
    }
}

/// The blending for a source and destination blend factor pair as the
/// effect scripts number them. 12 (both source alpha) and 13 (both inverse
/// source alpha) set both factors from the source. The target's alpha is
/// kept.
pub fn blend_state(src: u32, dest: u32) -> wgpu::BlendState {
    use wgpu::BlendFactor as F;
    let (src, dst) = match src {
        12 => (F::SrcAlpha, F::OneMinusSrcAlpha),
        13 => (F::OneMinusSrcAlpha, F::SrcAlpha),
        _ => (factor(src), factor(dest)),
    };
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: src,
            dst_factor: dst,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: F::Zero,
            dst_factor: F::One,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

/// The effect pipelines, by blending.
pub(crate) struct EffectPipelines {
    layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    multisample: wgpu::MultisampleState,
    made: Mutex<BTreeMap<Option<(u32, u32)>, wgpu::RenderPipeline>>,
}

impl EffectPipelines {
    pub(crate) fn new(
        gpu: &Gpu,
        frame_layout: &wgpu::BindGroupLayout,
        texture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        multisample: wgpu::MultisampleState,
    ) -> Self {
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("effect"),
                bind_group_layouts: &[Some(frame_layout), Some(texture_layout)],
                immediate_size: 0,
            });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("effect"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
            });
        Self {
            layout,
            shader,
            format,
            multisample,
            made: Mutex::new(BTreeMap::new()),
        }
    }

    /// The pipeline for `blend` (`None`: no blending), made if needed.
    fn pipeline(
        &self,
        gpu: &Gpu,
        blend: Option<(u32, u32)>,
    ) -> Result<wgpu::RenderPipeline, RenderError> {
        let mut made = self.made.lock().map_err(|_| RenderError::Gpu {
            what: "making an effect pipeline",
            detail: "another thread failed while making one".to_owned(),
        })?;
        if let Some(pipeline) = made.get(&blend) {
            return Ok(pipeline.clone());
        }
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x3,
            1 => Float32x2,
            2 => Float32x4,
            3 => Uint32,
        ];
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("effect"),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<EffectVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: crate::renderer::DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: self.multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.format,
                        blend: blend.map(|(src, dest)| blend_state(src, dest)),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        gpu.check("making an effect pipeline")?;
        made.insert(blend, pipeline.clone());
        Ok(pipeline)
    }

    /// Uploads a frame's triangles.
    pub(crate) fn upload(
        &self,
        gpu: &Gpu,
        geometry: &EffectGeometry,
    ) -> Result<GpuEffects, RenderError> {
        let vertex_count = geometry.vertices.len() as u64;
        let mut draws = Vec::with_capacity(geometry.draws.len());
        for draw in &geometry.draws {
            if draw.count == 0 || u64::from(draw.first) + u64::from(draw.count) > vertex_count {
                continue;
            }
            draws.push(GpuEffectDraw {
                first: draw.first,
                count: draw.count,
                texture: draw.texture,
                pipeline: self.pipeline(gpu, draw.blend)?,
            });
        }
        let vertices = (!geometry.vertices.is_empty()).then(|| {
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("effect vertices"),
                    contents: bytemuck::cast_slice(&geometry.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        });
        gpu.check("uploading effects")?;
        Ok(GpuEffects { vertices, draws })
    }

    /// Draws `effects` into a pass whose group 0 is already the frame.
    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        effects: &GpuEffects,
        textures: &[GpuTexture],
        white: &wgpu::BindGroup,
    ) {
        let Some(vertices) = &effects.vertices else {
            return;
        };
        pass.set_vertex_buffer(0, vertices.slice(..));
        for draw in &effects.draws {
            pass.set_pipeline(&draw.pipeline);
            let group = draw
                .texture
                .and_then(|t| textures.get(t as usize))
                .map_or(white, |t| &t.group);
            pass.set_bind_group(1, group, &[]);
            pass.draw(draw.first..draw.first + draw.count, 0..1);
        }
    }
}
