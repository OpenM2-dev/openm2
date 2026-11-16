//! The sky (`shaders/sky.wgsl`): drawn behind everything else, from colour
//! bands by height or the six pictures of a box, with a layer of drifting
//! clouds (`docs/specs/render.md`, "Maps: the sky").

use std::borrow::Cow;

use bytemuck::{Pod, Zeroable};
use m2_formats::RgbaImage;
use wgpu::util::DeviceExt as _;

use crate::RenderError;
use crate::camera::Camera;
use crate::gpu::Gpu;
use crate::renderer::DEPTH_FORMAT;
use crate::terrain::{mip_chain, resample};

const SHADER: &str = include_str!("shaders/sky.wgsl");

/// Most colour bands a sky draws, above and below the horizon together.
pub const MAX_BANDS: usize = 16;

/// Largest side a sky picture is drawn at; larger ones are scaled down.
pub const MAX_SIDE: u32 = 2048;

/// What is drawn behind the world.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sky {
    /// The bands above the horizon, from the highest down: each its top and
    /// its bottom colour, RGB.
    pub upper: Vec<[[f32; 3]; 2]>,
    /// The bands below the horizon, from the horizon down.
    pub lower: Vec<[[f32; 3]; 2]>,
    /// The box's pictures instead of the bands: front (facing south), back,
    /// left (east), right, top, bottom. Where one is missing the bands show.
    pub faces: Option<[Option<RgbaImage>; 6]>,
    /// The cloud layer.
    pub clouds: Option<Clouds>,
}

/// A layer of clouds over the camera, added to the sky behind it.
#[derive(Debug, Clone, PartialEq)]
pub struct Clouds {
    /// Its texture: colour times alpha is added.
    pub image: RgbaImage,
    /// How high over the camera it is, in centimetres.
    pub height: f32,
    /// How far it reaches from the camera each way, in centimetres; it
    /// fades out over the outer part.
    pub reach: [f32; 2],
    /// How many times the texture repeats over its reach.
    pub repeats: [f32; 2],
    /// How fast it drifts, in repeats of the texture a second.
    pub speed: [f32; 2],
}

/// The sky uniform, as `Sky` in `shaders/sky.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct SkyUniform {
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    info: [f32; 4],
    clouds: [f32; 4],
    cloud_uv: [f32; 4],
    faces: [[f32; 4]; 2],
    bands: [[f32; 4]; 2 * MAX_BANDS],
}

/// A sky on the GPU; [`World::sky`](crate::World::sky) draws it.
pub struct GpuSky {
    uniform: SkyUniform,
    /// Drift in repeats a second.
    speed: [f32; 2],
    buffer: wgpu::Buffer,
    group: wgpu::BindGroup,
}

/// The sky pipeline and its layout.
pub(crate) struct SkyPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    face_sampler: wgpu::Sampler,
    cloud_sampler: wgpu::Sampler,
}

impl SkyPipeline {
    pub(crate) fn new(
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        multisample: wgpu::MultisampleState,
    ) -> Self {
        let device = &gpu.device;
        let texture = |binding, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1, wgpu::TextureViewDimension::D2Array),
                texture(2, wgpu::TextureViewDimension::D2),
                sampler(3),
                sampler(4),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            // Behind everything: depth neither tested nor written.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let filtered = |address_mode| wgpu::SamplerDescriptor {
            label: Some("sky"),
            address_mode_u: address_mode,
            address_mode_v: address_mode,
            address_mode_w: address_mode,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        };
        Self {
            pipeline,
            layout,
            face_sampler: device.create_sampler(&filtered(wgpu::AddressMode::ClampToEdge)),
            cloud_sampler: device.create_sampler(&filtered(wgpu::AddressMode::Repeat)),
        }
    }

    pub(crate) fn upload(&self, gpu: &Gpu, sky: &Sky) -> Result<GpuSky, RenderError> {
        let device = &gpu.device;
        let mut uniform = SkyUniform::zeroed();
        let upper = sky.upper.len().min(MAX_BANDS);
        let lower = sky.lower.len().min(MAX_BANDS - upper);
        let bands = sky
            .upper
            .iter()
            .take(upper)
            .chain(sky.lower.iter().take(lower));
        for (slots, band) in uniform.bands.as_chunks_mut::<2>().0.iter_mut().zip(bands) {
            for (slot, [r, g, b]) in slots.iter_mut().zip(band) {
                *slot = [*r, *g, *b, 1.0];
            }
        }
        uniform.info = [upper as f32, lower as f32, 0.0, 0.0];

        let faces = sky.faces.as_ref();
        let side = faces
            .into_iter()
            .flatten()
            .flatten()
            .map(|image| image.width().max(image.height()))
            .max()
            .unwrap_or(1)
            .checked_next_power_of_two()
            .unwrap_or(MAX_SIDE)
            .clamp(1, MAX_SIDE);
        let black = RgbaImage::filled(side, side, [0, 0, 0, 255])?;
        let mut layers = Vec::with_capacity(6);
        for (face, slot) in (0..6).zip(uniform.faces.as_flattened_mut()) {
            match faces
                .and_then(|faces| faces.get(face))
                .and_then(Option::as_ref)
            {
                Some(image) => {
                    layers.push(resample(image, side)?);
                    *slot = 1.0;
                }
                None => layers.push(black.clone()),
            }
        }
        if faces.is_some() {
            uniform.info[2] = 1.0;
        }
        let face_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky faces"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, image) in (0..).zip(&layers) {
            write_level(gpu, &face_texture, 0, layer, image);
        }

        let mut speed = [0.0; 2];
        let cloud_image = match &sky.clouds {
            Some(clouds) => {
                let [x, y] = clouds.reach.map(|r| r.max(1.0));
                uniform.info[3] = 1.0;
                let height = clouds.height.max(1.0);
                uniform.clouds = [height, 1.0 / x, 1.0 / y, height / x.min(y)];
                uniform.cloud_uv = [clouds.repeats[0] / x, clouds.repeats[1] / y, 0.0, 0.0];
                speed = clouds.speed;
                let side = clouds
                    .image
                    .width()
                    .max(clouds.image.height())
                    .checked_next_power_of_two()
                    .unwrap_or(MAX_SIDE)
                    .clamp(1, MAX_SIDE);
                resample(&clouds.image, side)?
            }
            None => RgbaImage::filled(1, 1, [0, 0, 0, 0])?,
        };
        let levels = mip_chain(cloud_image)?;
        let first = levels
            .first()
            .ok_or_else(|| RenderError::Texture("no levels".to_owned()))?;
        let cloud_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("clouds"),
            size: wgpu::Extent3d {
                width: first.width(),
                height: first.height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, image) in (0..).zip(&levels) {
            write_level(gpu, &cloud_texture, level, 0, image);
        }

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sky"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let face_view = face_texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let cloud_view = cloud_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&face_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&cloud_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.face_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.cloud_sampler),
                },
            ],
        });
        gpu.check("uploading the sky")?;
        Ok(GpuSky {
            uniform,
            speed,
            buffer,
            group,
        })
    }

    /// Sets the frame's view rays for `camera` and a view of `aspect`
    /// (width / height), and the clouds' drift at `seconds`, before the pass
    /// that draws the sky. The rays come from the camera's axes, as
    /// [`Camera::ray`]'s do: an inverted view matrix is off by more than the
    /// clouds near the horizon can hide, and by a different amount at every
    /// step the camera takes.
    pub(crate) fn prepare(
        &self,
        gpu: &Gpu,
        sky: &GpuSky,
        camera: &Camera,
        aspect: f32,
        seconds: f32,
    ) {
        let mut uniform = sky.uniform;
        let (_, forward) = camera.ray(aspect, 0.0, 0.0);
        let (_, right) = camera.ray(aspect, 1.0, 0.0);
        let (_, up) = camera.ray(aspect, 0.0, 1.0);
        // Back from unit rays to the steps across the picture.
        let tan_x = right.dot(forward).recip() * right - forward;
        let tan_y = up.dot(forward).recip() * up - forward;
        uniform.forward = forward.extend(0.0).to_array();
        uniform.right = tan_x.extend(0.0).to_array();
        uniform.up = tan_y.extend(0.0).to_array();
        // Only the fraction matters to a repeating texture, and keeps the
        // numbers small however long the game runs.
        let drift = sky
            .speed
            .map(|s| (f64::from(s) * f64::from(seconds)).fract() as f32);
        uniform.cloud_uv[2] = drift[0];
        uniform.cloud_uv[3] = drift[1];
        gpu.queue
            .write_buffer(&sky.buffer, 0, bytemuck::bytes_of(&uniform));
    }

    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, sky: &GpuSky) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &sky.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn write_level(gpu: &Gpu, texture: &wgpu::Texture, level: u32, layer: u32, image: &RgbaImage) {
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        image.pixels(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width() * 4),
            rows_per_image: Some(image.height()),
        },
        wgpu::Extent3d {
            width: image.width(),
            height: image.height(),
            depth_or_array_layers: 1,
        },
    );
}
