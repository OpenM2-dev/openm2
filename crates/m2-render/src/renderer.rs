//! The pipelines, what is uploaded to the GPU, and offscreen targets.
//!
//! Three pipelines share bind group 0, the frame (camera, light, fog):
//! - models: group 1 a part's palette of skinning matrices, group 2 a
//!   texture and its sampler; drawn from both sides with an alpha test;
//! - terrain: group 1 the texture set as a texture array, group 2 an area
//!   (tile map, baked shadows);
//! - water: flat cells blended over what is behind them;
//! - effects (`effect_pipeline.rs`): group 1 a texture; world-space
//!   triangles blended over the rest, depth tested but not written.
//!
//! The sky (`sky.rs`) has a pipeline and group of its own. A frame draws the
//! sky, then the terrain, then the models, then the water, then the
//! effects.

use std::borrow::Cow;
use std::sync::mpsc;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use m2_formats::RgbaImage;
use m2_formats::map::grid::TILE_SIDE;
use wgpu::util::DeviceExt as _;

use crate::RenderError;
use crate::camera::Camera;
use crate::effect::EffectGeometry;
use crate::effect_pipeline::{EffectPipelines, GpuEffects, GpuTexture};
use crate::gpu::Gpu;
use crate::model::{PALETTE_LEN, Part, TextureData, Vertex};
use crate::sky::{GpuSky, Sky, SkyPipeline};
use crate::terrain::{AreaMesh, TerrainLayer, TerrainVertex, layer_side, mip_chain, resample};

/// Colour format of offscreen targets: 8-bit RGBA without sRGB conversion.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Depth format of every target.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const SHADER: &str = include_str!("shaders/model.wgsl");
const TERRAIN_SHADER: &str = include_str!("shaders/terrain.wgsl");

/// Texture numbers a texture set can have (tile values are bytes).
const MAX_LAYERS: usize = 256;

/// The frame uniform, as `Frame` in the shaders.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    light: [f32; 4],
    ambient: [f32; 4],
    diffuse: [f32; 4],
    fog: [f32; 4],
    fog_colour: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    shine: [f32; 4],
}

/// How far the shine maps scroll a second, along both of their axes: the
/// client's step of 0.007 a frame at its 60 frames a second.
pub const SHINE_SCROLL: f32 = 0.42;

/// A part's look, as `PartLook` in `shaders/model.wgsl`: how much it
/// shines (0 to 1).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PartLookUniform {
    shine: [f32; 4],
}

/// A texture's look, as `TextureLook` in `shaders/model.wgsl`: 1 when its
/// alpha is coverage, and 1 when it shines.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct TextureLookUniform {
    look: [f32; 4],
}

/// The texture set uniform, as `Layers` in `shaders/terrain.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct LayersUniform {
    info: [f32; 4],
    layers: [[f32; 4]; MAX_LAYERS],
}

/// Fog: colours fade to `colour` from `start` to `end` centimetres from the
/// camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fog {
    /// Where the fog starts.
    pub start: f32,
    /// Where everything is fog colour.
    pub end: f32,
    /// The fog's colour, RGB.
    pub colour: [f32; 3],
}

/// What to draw a frame with: camera, light, background.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scene {
    /// The camera.
    pub camera: Camera,
    /// Clear colour, RGBA from 0 to 1.
    pub background: [f64; 4],
    /// Ambient light, RGB.
    pub ambient: [f32; 3],
    /// Directional light, RGB.
    pub diffuse: [f32; 3],
    /// Direction towards the light; `None` takes it from the camera
    /// ([`Camera::light_direction`]).
    pub light: Option<Vec3>,
    /// Fog, if any.
    pub fog: Option<Fog>,
    /// Distance of the far plane, for scenes larger than what the camera
    /// looks at; `None` uses the camera's ([`Camera::view_proj`]).
    pub far: Option<f32>,
    /// Texels with less alpha are not drawn (0 draws everything).
    pub alpha_cutoff: f32,
}

impl Scene {
    /// A scene with the default light and background.
    pub fn new(camera: Camera) -> Self {
        Self {
            camera,
            background: [0.16, 0.18, 0.22, 1.0],
            ambient: [0.55, 0.55, 0.55],
            diffuse: [0.6, 0.6, 0.6],
            light: None,
            fog: None,
            far: None,
            alpha_cutoff: 0.5,
        }
    }
}

/// The pipelines and what every draw shares.
pub struct Renderer {
    format: wgpu::TextureFormat,
    samples: u32,
    pipeline: wgpu::RenderPipeline,
    frame_buffer: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    palette_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    white: wgpu::BindGroup,
    /// What parts reflect until they are given a shine map: black.
    no_shine: wgpu::TextureView,
    shine_sampler: wgpu::Sampler,
    terrain: TerrainPipelines,
    effects: EffectPipelines,
    sky: SkyPipeline,
}

/// The terrain and water pipelines and their layouts.
struct TerrainPipelines {
    terrain: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    set_layout: wgpu::BindGroupLayout,
    area_layout: wgpu::BindGroupLayout,
    layer_sampler: wgpu::Sampler,
    shadow_sampler: wgpu::Sampler,
}

/// A part on the GPU: buffers, textures and its palette; how much it
/// shines and what it reflects, shared with its instances.
pub struct GpuPart {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    draws: Vec<(u32, u32, Option<usize>)>,
    textures: Vec<wgpu::BindGroup>,
    palette: wgpu::Buffer,
    look: wgpu::Buffer,
    shine_map: wgpu::TextureView,
    palette_group: wgpu::BindGroup,
}

/// A shine map on the GPU: the picture shining parts reflect
/// ([`Renderer::set_shine`]).
pub struct GpuShineMap {
    view: wgpu::TextureView,
}

/// A texture set on the GPU.
pub struct GpuTerrainSet {
    group: wgpu::BindGroup,
}

/// An area on the GPU.
pub struct GpuArea {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    water: Option<(wgpu::Buffer, u32)>,
    group: wgpu::BindGroup,
}

/// What a frame draws.
#[derive(Default, Clone, Copy)]
pub struct World<'a> {
    /// The texture set the areas are drawn with; without one no terrain is
    /// drawn.
    pub terrain: Option<&'a GpuTerrainSet>,
    /// The areas.
    pub areas: &'a [&'a GpuArea],
    /// The models.
    pub parts: &'a [&'a GpuPart],
    /// The effects' triangles, drawn last.
    pub effects: Option<&'a GpuEffects>,
    /// The textures the effects' draws name by number.
    pub effect_textures: &'a [GpuTexture],
    /// The sky, drawn first, behind everything.
    pub sky: Option<&'a GpuSky>,
    /// The time in seconds, for what moves by itself (the clouds).
    pub seconds: f32,
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
    view_dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn depth_state(write: bool) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(wgpu::CompareFunction::Less),
        stencil: Default::default(),
        bias: Default::default(),
    }
}

impl Renderer {
    /// The pipelines for targets of `format` with `samples` MSAA samples
    /// (1 or 4, see [`Gpu::sample_count`]).
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, samples: u32) -> Result<Self, RenderError> {
        let device = &gpu.device;
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let palette_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("palette"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX),
                uniform_entry(1, wgpu::ShaderStages::FRAGMENT),
                texture_entry(
                    2,
                    wgpu::TextureSampleType::Float { filterable: true },
                    wgpu::TextureViewDimension::D2,
                ),
                sampler_entry(3),
            ],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                texture_entry(
                    0,
                    wgpu::TextureSampleType::Float { filterable: true },
                    wgpu::TextureViewDimension::D2,
                ),
                sampler_entry(1),
                uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("model"),
            bind_group_layouts: &[
                Some(&frame_layout),
                Some(&palette_layout),
                Some(&texture_layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("model"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x3,
            1 => Float32x3,
            2 => Float32x2,
            3 => Uint16x4,
            4 => Float32x4,
        ];
        let multisample = wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("model"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state(true)),
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
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("texture"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let white = TextureData::new(vec![RgbaImage::filled(1, 1, [255; 4])?])?;
        let white = texture_group(gpu, &texture_layout, &sampler, &white)?;
        let black = TextureData::new(vec![RgbaImage::filled(1, 1, [0, 0, 0, 255])?])?;
        let no_shine = texture_view(gpu, &black)?;
        let shine_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shine"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let terrain = TerrainPipelines::new(gpu, &frame_layout, format, multisample);
        let effects =
            EffectPipelines::new(gpu, &frame_layout, &texture_layout, format, multisample);
        let sky = SkyPipeline::new(gpu, format, multisample);
        gpu.check("creating the pipelines")?;
        Ok(Self {
            format,
            samples,
            pipeline,
            frame_buffer,
            frame_group,
            palette_layout,
            texture_layout,
            sampler,
            white,
            no_shine,
            shine_sampler,
            terrain,
            effects,
            sky,
        })
    }

    /// The colour format this renderer draws to.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// MSAA samples per pixel.
    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// Uploads a part. `textures` goes with [`Part::textures`]; a missing
    /// texture draws white.
    pub fn upload(
        &self,
        gpu: &Gpu,
        part: &Part,
        textures: &[Option<TextureData>],
    ) -> Result<GpuPart, RenderError> {
        let device = &gpu.device;
        let mut vertex_bytes: &[u8] = bytemuck::cast_slice(&part.vertices);
        let zero_vertex = [0u8; size_of::<Vertex>()];
        if vertex_bytes.is_empty() {
            vertex_bytes = &zero_vertex;
        }
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vertices"),
            contents: vertex_bytes,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_list: &[u32] = if part.indices.is_empty() {
            &[0]
        } else {
            &part.indices
        };
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("indices"),
            contents: bytemuck::cast_slice(index_list),
            usage: wgpu::BufferUsages::INDEX,
        });
        let look = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("part look"),
            contents: bytemuck::bytes_of(&PartLookUniform { shine: [0.0; 4] }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shine_map = self.no_shine.clone();
        let (palette, palette_group) = self.palette(gpu, &look, &shine_map);
        let mut groups = Vec::new();
        let mut slot_of = Vec::new();
        for slot in 0..part.textures.len() {
            match textures.get(slot).and_then(Option::as_ref) {
                Some(data) => {
                    slot_of.push(Some(groups.len()));
                    groups.push(texture_group(
                        gpu,
                        &self.texture_layout,
                        &self.sampler,
                        data,
                    )?);
                }
                None => slot_of.push(None),
            }
        }
        let vertex_count = part.vertices.len();
        let index_count = part.indices.len() as u64;
        let draws = part
            .draws
            .iter()
            .filter(|d| d.count > 0 && u64::from(d.first) + u64::from(d.count) <= index_count)
            .filter(|_| vertex_count > 0)
            .map(|d| {
                let texture = d.texture.and_then(|t| slot_of.get(t).copied().flatten());
                (d.first, d.count, texture)
            })
            .collect();
        gpu.check("uploading a model")?;
        Ok(GpuPart {
            vertices,
            indices,
            draws,
            textures: groups,
            palette,
            look,
            shine_map,
            palette_group,
        })
    }

    /// Uploads a shine map: the picture shining parts reflect, by where
    /// the camera's view reflects off them, scrolling (`docs/specs/render.md`,
    /// "Shine").
    pub fn upload_shine_map(
        &self,
        gpu: &Gpu,
        data: &TextureData,
    ) -> Result<GpuShineMap, RenderError> {
        let view = texture_view(gpu, data)?;
        gpu.check("uploading a shine map")?;
        Ok(GpuShineMap { view })
    }

    /// Makes `part` shine: `map` added to the colour of its shining
    /// textures ([`TextureData::with_shine`]), times their alpha and `shine`
    /// (0 none to 1). Its instances made after this shine too.
    pub fn set_shine(&self, gpu: &Gpu, part: &mut GpuPart, shine: f32, map: &GpuShineMap) {
        let look = PartLookUniform {
            shine: [shine.clamp(0.0, 1.0), 0.0, 0.0, 0.0],
        };
        gpu.queue
            .write_buffer(&part.look, 0, bytemuck::bytes_of(&look));
        part.shine_map = map.view.clone();
        part.palette_group = self.palette_group(gpu, &part.palette, &part.look, &part.shine_map);
    }

    /// Uploads a texture for effects, its alpha kept as it is.
    pub fn upload_texture(&self, gpu: &Gpu, data: &TextureData) -> Result<GpuTexture, RenderError> {
        let data = data.clone().with_coverage(true);
        let group = texture_group(gpu, &self.texture_layout, &self.sampler, &data)?;
        gpu.check("uploading a texture")?;
        Ok(GpuTexture { group })
    }

    /// Uploads a sky; [`World::sky`] draws it.
    pub fn upload_sky(&self, gpu: &Gpu, sky: &Sky) -> Result<GpuSky, RenderError> {
        self.sky.upload(gpu, sky)
    }

    /// Uploads a frame's effect triangles; [`World::effects`] draws them.
    pub fn upload_effects(
        &self,
        gpu: &Gpu,
        geometry: &EffectGeometry,
    ) -> Result<GpuEffects, RenderError> {
        self.effects.upload(gpu, geometry)
    }

    /// A new palette, every matrix the identity, bound with a part's look.
    fn palette(
        &self,
        gpu: &Gpu,
        look: &wgpu::Buffer,
        shine_map: &wgpu::TextureView,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let identity = vec![Mat4::IDENTITY; PALETTE_LEN];
        let palette = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("palette"),
                contents: bytemuck::cast_slice(&identity),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let group = self.palette_group(gpu, &palette, look, shine_map);
        (palette, group)
    }

    /// A part's palette, look and shine map, bound.
    fn palette_group(
        &self,
        gpu: &Gpu,
        palette: &wgpu::Buffer,
        look: &wgpu::Buffer,
        shine_map: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("palette"),
            layout: &self.palette_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: palette.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: look.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(shine_map),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.shine_sampler),
                },
            ],
        })
    }

    /// Another copy of an uploaded part with its own palette: the vertices,
    /// indexes and textures are shared. For drawing one model in many
    /// places, like a map's buildings.
    pub fn instance(&self, gpu: &Gpu, part: &GpuPart) -> GpuPart {
        let (palette, palette_group) = self.palette(gpu, &part.look, &part.shine_map);
        GpuPart {
            vertices: part.vertices.clone(),
            indices: part.indices.clone(),
            draws: part.draws.clone(),
            textures: part.textures.clone(),
            palette,
            look: part.look.clone(),
            shine_map: part.shine_map.clone(),
            palette_group,
        }
    }

    /// Sets a part's skinning matrices (see [`Pose::palette`](crate::Pose::palette));
    /// entries past [`PALETTE_LEN`] are ignored.
    pub fn set_palette(&self, gpu: &Gpu, part: &GpuPart, palette: &[Mat4]) {
        let used = palette
            .get(..palette.len().min(PALETTE_LEN))
            .unwrap_or_default();
        gpu.queue
            .write_buffer(&part.palette, 0, bytemuck::cast_slice(used));
    }

    /// Uploads a texture set: `layers[n - 1]` is texture number `n`, `None`
    /// where the set has none. Every layer is scaled to one size
    /// ([`layer_side`]). `span` is the centimetres one repeat covers at
    /// scale 1 ([`crate::terrain::TEXTURE_SPAN`]).
    pub fn upload_layers(
        &self,
        gpu: &Gpu,
        layers: &[Option<TerrainLayer>],
        span: f32,
    ) -> Result<GpuTerrainSet, RenderError> {
        if layers.len() > MAX_LAYERS {
            return Err(RenderError::Texture(format!(
                "{} terrain textures (at most {MAX_LAYERS})",
                layers.len()
            )));
        }
        let side = layer_side(layers);
        let count = layers.len().max(1) as u32;
        let max_layers = gpu.device.limits().max_texture_array_layers;
        if count > max_layers {
            return Err(RenderError::Texture(format!(
                "{count} terrain textures; the device takes {max_layers}"
            )));
        }
        let grey = RgbaImage::filled(side, side, [128, 128, 128, 255])?;
        let level_count = mip_chain(grey.clone())?.len() as u32;
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("terrain textures"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: count,
            },
            mip_level_count: level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut uniform = LayersUniform {
            info: [1.0 / span.max(1.0), 0.0, 0.0, 0.0],
            layers: [[0.0; 4]; MAX_LAYERS],
        };
        for layer in 0..count {
            let input = layers.get(layer as usize).and_then(Option::as_ref);
            let image = match input {
                Some(input) => resample(&input.image, side)?,
                None => grey.clone(),
            };
            if let (Some(input), Some(slot)) = (input, uniform.layers.get_mut(layer as usize)) {
                *slot = [input.scale[0], input.scale[1], 1.0, 0.0];
            }
            for (level, image) in (0..).zip(mip_chain(image)?) {
                gpu.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
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
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("terrain layers"),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain set"),
            layout: &self.terrain.set_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.terrain.layer_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        gpu.check("uploading terrain textures")?;
        Ok(GpuTerrainSet { group })
    }

    /// Uploads an area.
    pub fn upload_area(&self, gpu: &Gpu, area: &AreaMesh) -> Result<GpuArea, RenderError> {
        let device = &gpu.device;
        if area.tiles.len() != (TILE_SIDE * TILE_SIDE) as usize {
            return Err(RenderError::Model(format!(
                "a tile map of {} bytes (expected {})",
                area.tiles.len(),
                TILE_SIDE * TILE_SIDE
            )));
        }
        let mut vertex_bytes: &[u8] = bytemuck::cast_slice(&area.vertices);
        let zero = [0u8; size_of::<TerrainVertex>()];
        if vertex_bytes.is_empty() {
            vertex_bytes = &zero;
        }
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain vertices"),
            contents: vertex_bytes,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_list: &[u32] = if area.indices.is_empty() {
            &[0]
        } else {
            &area.indices
        };
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain indices"),
            contents: bytemuck::cast_slice(index_list),
            usage: wgpu::BufferUsages::INDEX,
        });
        let in_range = area
            .indices
            .iter()
            .all(|i| (*i as usize) < area.vertices.len());
        let index_count = if in_range {
            u32::try_from(area.indices.len()).unwrap_or(0)
        } else {
            0
        };
        let water = (!area.water.is_empty()).then(|| {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("water"),
                contents: bytemuck::cast_slice(&area.water),
                usage: wgpu::BufferUsages::VERTEX,
            });
            (buffer, u32::try_from(area.water.len()).unwrap_or(0))
        });
        let tiles = device.create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("tiles"),
                size: wgpu::Extent3d {
                    width: TILE_SIDE,
                    height: TILE_SIDE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &area.tiles,
        );
        let white = RgbaImage::filled(1, 1, [255; 4])?;
        let shadow_image = area.shadows.as_ref().unwrap_or(&white);
        let shadows = device.create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("shadows"),
                size: wgpu::Extent3d {
                    width: shadow_image.width(),
                    height: shadow_image.height(),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            shadow_image.pixels(),
        );
        let area_side = area.cell * m2_formats::map::AREA_CELLS as f32;
        let place: [f32; 4] = [
            area.origin[0],
            area.origin[1],
            area.cell,
            1.0 / area_side.max(1.0),
        ];
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("area"),
            contents: bytemuck::cast_slice(&place),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let tiles_view = tiles.create_view(&wgpu::TextureViewDescriptor::default());
        let shadows_view = shadows.create_view(&wgpu::TextureViewDescriptor::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("area"),
            layout: &self.terrain.area_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&tiles_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadows_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.terrain.shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        gpu.check("uploading an area")?;
        Ok(GpuArea {
            vertices,
            indices,
            index_count,
            water,
            group,
        })
    }

    /// Draws `parts` into `output`, a view of a texture of this renderer's
    /// format, using `attachments` of the same size.
    pub fn draw(
        &self,
        gpu: &Gpu,
        attachments: &Attachments,
        output: &wgpu::TextureView,
        scene: &Scene,
        parts: &[&GpuPart],
    ) -> Result<(), RenderError> {
        self.draw_world(
            gpu,
            attachments,
            output,
            scene,
            &World {
                parts,
                ..World::default()
            },
        )
    }

    /// Draws the sky, terrain, models, water and effects into `output`, a
    /// view of a texture of this renderer's format, using `attachments` of
    /// the same size.
    pub fn draw_world(
        &self,
        gpu: &Gpu,
        attachments: &Attachments,
        output: &wgpu::TextureView,
        scene: &Scene,
        world: &World<'_>,
    ) -> Result<(), RenderError> {
        let (width, height) = attachments.size;
        let aspect = width as f32 / height.max(1) as f32;
        let eye = scene.camera.eye();
        let view = scene.camera.view();
        let view_proj = match scene.far {
            Some(far) => scene.camera.view_proj_to(aspect, far),
            None => scene.camera.view_proj(aspect),
        };
        let light = scene
            .light
            .and_then(|l| l.try_normalize())
            .unwrap_or_else(|| scene.camera.light_direction());
        let (fog, fog_colour) = match scene.fog {
            Some(fog) => (
                [fog.start, fog.end, 0.0, 0.0],
                [fog.colour[0], fog.colour[1], fog.colour[2], 1.0],
            ),
            None => ([0.0; 4], [0.0; 4]),
        };
        let frame = FrameUniform {
            view_proj: view_proj.to_cols_array_2d(),
            eye: eye.extend(1.0).to_array(),
            light: light.extend(0.0).to_array(),
            ambient: [scene.ambient[0], scene.ambient[1], scene.ambient[2], 1.0],
            diffuse: [
                scene.diffuse[0],
                scene.diffuse[1],
                scene.diffuse[2],
                scene.alpha_cutoff,
            ],
            fog,
            fog_colour,
            right: view.right.extend(0.0).to_array(),
            up: view.up.extend(0.0).to_array(),
            shine: [(world.seconds * SHINE_SCROLL).fract(), 0.0, 0.0, 0.0],
        };
        gpu.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));
        if let Some(sky) = world.sky {
            self.sky
                .prepare(gpu, sky, &scene.camera, aspect, world.seconds);
        }

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let (view, resolve_target) = match &attachments.msaa {
                Some(msaa) => (msaa, Some(output)),
                None => (output, None),
            };
            let [r, g, b, a] = scene.background;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &attachments.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(sky) = world.sky {
                self.sky.draw(&mut pass, sky);
            }
            if let Some(set) = world.terrain
                && !world.areas.is_empty()
            {
                pass.set_pipeline(&self.terrain.terrain);
                pass.set_bind_group(0, &self.frame_group, &[]);
                pass.set_bind_group(1, &set.group, &[]);
                for area in world.areas {
                    pass.set_bind_group(2, &area.group, &[]);
                    pass.set_vertex_buffer(0, area.vertices.slice(..));
                    pass.set_index_buffer(area.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..area.index_count, 0, 0..1);
                }
            }
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.frame_group, &[]);
            for part in world.parts {
                pass.set_bind_group(1, &part.palette_group, &[]);
                pass.set_vertex_buffer(0, part.vertices.slice(..));
                pass.set_index_buffer(part.indices.slice(..), wgpu::IndexFormat::Uint32);
                for &(first, count, texture) in &part.draws {
                    let group = texture
                        .and_then(|t| part.textures.get(t))
                        .unwrap_or(&self.white);
                    pass.set_bind_group(2, group, &[]);
                    pass.draw_indexed(first..first + count, 0, 0..1);
                }
            }
            if world.areas.iter().any(|a| a.water.is_some()) {
                pass.set_pipeline(&self.terrain.water);
                pass.set_bind_group(0, &self.frame_group, &[]);
                for (buffer, count) in world.areas.iter().filter_map(|a| a.water.as_ref()) {
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw(0..*count, 0..1);
                }
            }
            if let Some(effects) = world.effects {
                pass.set_bind_group(0, &self.frame_group, &[]);
                self.effects
                    .draw(&mut pass, effects, world.effect_textures, &self.white);
            }
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.check("drawing")
    }
}

impl TerrainPipelines {
    fn new(
        gpu: &Gpu,
        frame_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        multisample: wgpu::MultisampleState,
    ) -> Self {
        let device = &gpu.device;
        let set_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain set"),
            entries: &[
                texture_entry(
                    0,
                    wgpu::TextureSampleType::Float { filterable: true },
                    wgpu::TextureViewDimension::D2Array,
                ),
                sampler_entry(1),
                uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let area_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("area"),
            entries: &[
                texture_entry(
                    0,
                    wgpu::TextureSampleType::Uint,
                    wgpu::TextureViewDimension::D2,
                ),
                texture_entry(
                    1,
                    wgpu::TextureSampleType::Float { filterable: true },
                    wgpu::TextureViewDimension::D2,
                ),
                sampler_entry(2),
                uniform_entry(3, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(TERRAIN_SHADER)),
        });
        let terrain_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(frame_layout), Some(&set_layout), Some(&area_layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let target = |blend| {
            [Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })]
        };
        let opaque = target(None);
        let terrain = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&terrain_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_terrain"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<TerrainVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state(true)),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_terrain"),
                compilation_options: Default::default(),
                targets: &opaque,
            }),
            multiview_mask: None,
            cache: None,
        });
        let water_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water"),
            bind_group_layouts: &[Some(frame_layout)],
            immediate_size: 0,
        });
        let water_attributes = wgpu::vertex_attr_array![0 => Float32x3];
        let blended = target(Some(wgpu::BlendState::ALPHA_BLENDING));
        let water = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water"),
            layout: Some(&water_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_water"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<[f32; 3]>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &water_attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state(false)),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_water"),
                compilation_options: Default::default(),
                targets: &blended,
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = |label, address_mode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: address_mode,
                address_mode_v: address_mode,
                address_mode_w: address_mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            })
        };
        Self {
            terrain,
            water,
            set_layout,
            area_layout,
            layer_sampler: sampler("terrain textures", wgpu::AddressMode::Repeat),
            shadow_sampler: sampler("shadows", wgpu::AddressMode::ClampToEdge),
        }
    }
}

/// Uploads a texture and binds it with `sampler` and whether its alpha is
/// coverage.
fn texture_group(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    data: &TextureData,
) -> Result<wgpu::BindGroup, RenderError> {
    let view = texture_view(gpu, data)?;
    let flag = |on: bool| if on { 1.0 } else { 0.0 };
    let look = TextureLookUniform {
        look: [flag(data.coverage()), flag(data.shines()), 0.0, 0.0],
    };
    let look = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("texture look"),
            contents: bytemuck::bytes_of(&look),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    Ok(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("model texture"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: look.as_entire_binding(),
            },
        ],
    }))
}

/// Uploads a texture with its mip levels, its alpha as it is: whether it
/// is coverage or a gloss mask is the shader's to know.
fn texture_view(gpu: &Gpu, data: &TextureData) -> Result<wgpu::TextureView, RenderError> {
    let first = data
        .levels()
        .first()
        .ok_or_else(|| RenderError::Texture("no levels".to_owned()))?;
    let max = gpu.device.limits().max_texture_dimension_2d;
    if first.width() > max || first.height() > max {
        return Err(RenderError::Texture(format!(
            "{}×{} is over the device's limit of {max}",
            first.width(),
            first.height()
        )));
    }
    let size = |image: &RgbaImage| wgpu::Extent3d {
        width: image.width(),
        height: image.height(),
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("model texture"),
        size: size(first),
        mip_level_count: data.levels().len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, image) in (0..).zip(data.levels()) {
        let pixels = image.pixels();
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width() * 4),
                rows_per_image: Some(image.height()),
            },
            size(image),
        );
    }
    Ok(texture.create_view(&wgpu::TextureViewDescriptor::default()))
}

/// The depth buffer and, with MSAA, the multisampled colour buffer of a
/// target of one size.
pub struct Attachments {
    size: (u32, u32),
    depth: wgpu::TextureView,
    msaa: Option<wgpu::TextureView>,
}

impl Attachments {
    /// Attachments for a `width` × `height` target drawn by `renderer`.
    pub fn new(gpu: &Gpu, renderer: &Renderer, width: u32, height: u32) -> Self {
        Self::with(gpu, renderer.format, renderer.samples, width, height)
    }

    /// Attachments for a target of `format` drawn with `samples` samples.
    fn with(gpu: &Gpu, format: wgpu::TextureFormat, samples: u32, width: u32, height: u32) -> Self {
        let size = wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        };
        let texture = |label, format, samples| {
            gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let depth = texture("depth", DEPTH_FORMAT, samples);
        let msaa = (samples > 1).then(|| texture("msaa", format, samples));
        Self {
            size: (size.width, size.height),
            depth,
            msaa,
        }
    }

    /// Width and height.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
}

/// A target in memory: draw into it, then read the pixels back, or show
/// it in the interface (`UiRenderer::set_picture`).
pub struct Offscreen {
    attachments: Attachments,
    format: wgpu::TextureFormat,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Offscreen {
    /// A `width` × `height` target for `renderer`, whose format must be
    /// [`OFFSCREEN_FORMAT`] or its BGRA twin (a window's).
    pub fn new(
        gpu: &Gpu,
        renderer: &Renderer,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let format = renderer.format;
        if !matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
        ) {
            return Err(RenderError::Surface(format!(
                "offscreen targets are {OFFSCREEN_FORMAT:?} or Bgra8Unorm, the renderer draws {format:?}"
            )));
        }
        Self::with(gpu, format, renderer.samples, width, height)
    }

    /// A `width` × `height` target of [`OFFSCREEN_FORMAT`] for an interface
    /// alone ([`crate::UiRenderer`], cleared with
    /// [`crate::UiRenderer::clear`]): no renderer needed.
    pub fn for_interface(gpu: &Gpu, width: u32, height: u32) -> Result<Self, RenderError> {
        Self::with(gpu, OFFSCREEN_FORMAT, 1, width, height)
    }

    fn with(
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        samples: u32,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let max = gpu.device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > max || height > max {
            return Err(RenderError::Surface(format!(
                "{width}×{height} (each side 1 to {max})"
            )));
        }
        let attachments = Attachments::with(gpu, format, samples, width, height);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(Self {
            attachments,
            format,
            texture,
            view,
        })
    }

    /// Draws `parts` and reads the image back.
    pub fn render(
        &self,
        gpu: &Gpu,
        renderer: &Renderer,
        scene: &Scene,
        parts: &[&GpuPart],
    ) -> Result<RgbaImage, RenderError> {
        renderer.draw(gpu, &self.attachments, &self.view, scene, parts)?;
        self.read(gpu)
    }

    /// Draws terrain, models and water and reads the image back.
    pub fn render_world(
        &self,
        gpu: &Gpu,
        renderer: &Renderer,
        scene: &Scene,
        world: &World<'_>,
    ) -> Result<RgbaImage, RenderError> {
        renderer.draw_world(gpu, &self.attachments, &self.view, scene, world)?;
        self.read(gpu)
    }

    /// The target, for drawing more over it (the interface) before
    /// reading it back, or for showing it.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Its depth and MSAA attachments.
    pub fn attachments(&self) -> &Attachments {
        &self.attachments
    }

    /// Reads the pixels back.
    pub fn read(&self, gpu: &Gpu) -> Result<RgbaImage, RenderError> {
        let (width, height) = self.attachments.size;
        let row = width * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        let (sender, receiver) = mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        gpu.wait()?;
        let mapped = receiver.recv().map_err(|e| RenderError::Gpu {
            what: "reading pixels back",
            detail: e.to_string(),
        })?;
        mapped.map_err(|e| RenderError::Gpu {
            what: "reading pixels back",
            detail: e.to_string(),
        })?;
        gpu.check("reading pixels back")?;
        let mut pixels = Vec::with_capacity(row as usize * height as usize);
        {
            let view = buffer
                .slice(..)
                .get_mapped_range()
                .map_err(|e| RenderError::Gpu {
                    what: "reading pixels back",
                    detail: e.to_string(),
                })?;
            for line in view.chunks(padded as usize).take(height as usize) {
                pixels.extend_from_slice(line.get(..row as usize).unwrap_or_default());
            }
        }
        buffer.unmap();
        if self.format == wgpu::TextureFormat::Bgra8Unorm {
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
        }
        RgbaImage::from_pixels(width, height, pixels).ok_or_else(|| RenderError::Gpu {
            what: "reading pixels back",
            detail: "the copy came back short".to_owned(),
        })
    }
}
