//! The interface pipeline: draws an `m2_ui::DrawList` over a target, after
//! the world. Quads are alpha-blended in order, each batch with its texture
//! and clip rectangle (`docs/specs/render.md`, "Interface").
//!
//! Images are uploaded by the number the application gave them, once or
//! again over the same texture when they change at the same size (a
//! picture the application draws, as the minimap); the glyph atlas pages
//! are uploaded again whenever their version changes. A
//! picture the renderer draws (an [`crate::Offscreen`] target) can be shown
//! as an image too; it is blended as premultiplied, as its anti-aliased
//! edges are.
//!
//! Images drawn larger than they are (a scaled interface) are sampled as
//! [`UiFilter`] says, never outside their quad's part of the texture.

use std::borrow::Cow;
use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use m2_formats::RgbaImage;
use m2_ui::{DrawList, GlyphPage, ImageId, PAGE_SIZE, TextureRef, Vertex};

use crate::RenderError;
use crate::gpu::Gpu;

const SHADER: &str = include_str!("shaders/ui.wgsl");

/// The screen uniform, as `Screen` in `shaders/ui.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct ScreenUniform {
    size: [f32; 4],
}

/// A glyph page on the GPU and the version uploaded.
struct GpuPage {
    version: u64,
    texture: wgpu::Texture,
    group: wgpu::BindGroup,
}

/// How images are sampled when drawn at another size than their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiFilter {
    /// The nearest texel: hard pixels, even only at whole scales (2, 3).
    Sharp,
    /// Blended between the nearest texels: soft, even at any scale.
    Smooth,
}

/// An image on the GPU and how it blends.
struct GpuImage {
    group: wgpu::BindGroup,
    /// Colours already multiplied by alpha (pictures the renderer drew).
    premultiplied: bool,
    /// The texture uploaded and its size; none for pictures the renderer
    /// draws.
    texture: Option<(wgpu::Texture, u32, u32)>,
}

/// Draws interface draw lists.
pub struct UiRenderer {
    pipeline: wgpu::RenderPipeline,
    premultiplied: wgpu::RenderPipeline,
    screen_buffer: wgpu::Buffer,
    screen_group: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    /// For glyphs, flat colour and pictures, drawn at their own size.
    sampler: wgpu::Sampler,
    /// For images, as the filter says.
    image_sampler: wgpu::Sampler,
    white: wgpu::BindGroup,
    images: HashMap<ImageId, GpuImage>,
    pages: Vec<GpuPage>,
    vertices: Option<(wgpu::Buffer, u64)>,
}

impl UiRenderer {
    /// The pipeline for targets of `format` (no multisampling: the interface
    /// is drawn on the resolved picture), sampling images with `filter`.
    pub fn new(
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        filter: UiFilter,
    ) -> Result<Self, RenderError> {
        let device = &gpu.device;
        let screen_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui screen"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui"),
            bind_group_layouts: &[Some(&screen_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x2,
            1 => Float32x2,
            2 => Unorm8x4,
            3 => Float32x4,
        ];
        let blend = |colour_factor| wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: colour_factor,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let make = |blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ui"),
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
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make(blend(wgpu::BlendFactor::SrcAlpha));
        let premultiplied = make(blend(wgpu::BlendFactor::One));
        let screen_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui screen"),
            size: size_of::<ScreenUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let screen_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui screen"),
            layout: &screen_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen_buffer.as_entire_binding(),
            }],
        });
        // Glyphs and pictures are drawn at their own size on whole pixels:
        // nearest sampling keeps them sharp. Images too, unless the filter
        // is smooth; the shader keeps either inside each quad's part of an
        // image.
        let make_sampler = |filter: wgpu::FilterMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("ui"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            })
        };
        let sampler = make_sampler(wgpu::FilterMode::Nearest);
        let image_sampler = make_sampler(match filter {
            UiFilter::Sharp => wgpu::FilterMode::Nearest,
            UiFilter::Smooth => wgpu::FilterMode::Linear,
        });
        let (_, white) = upload(
            gpu,
            &texture_layout,
            &sampler,
            &RgbaImage::filled(1, 1, [255; 4])?,
            "ui white",
        )?;
        gpu.check("creating the interface pipeline")?;
        Ok(Self {
            pipeline,
            premultiplied,
            screen_buffer,
            screen_group,
            texture_layout,
            sampler,
            image_sampler,
            white,
            images: HashMap::new(),
            pages: Vec::new(),
            vertices: None,
        })
    }

    /// Uploads an image under the number draw lists use for it, replacing
    /// any image there: into the same texture if that was an image of the
    /// same size.
    pub fn set_image(
        &mut self,
        gpu: &Gpu,
        id: ImageId,
        image: &RgbaImage,
    ) -> Result<(), RenderError> {
        let (width, height) = (image.width(), image.height());
        if let Some(GpuImage {
            texture: Some((texture, w, h)),
            ..
        }) = self.images.get(&id)
            && (*w, *h) == (width, height)
        {
            write_pixels(gpu, texture, width, height, image.pixels());
            return Ok(());
        }
        let (texture, group) = upload(
            gpu,
            &self.texture_layout,
            &self.image_sampler,
            image,
            "ui image",
        )?;
        let image = GpuImage {
            group,
            premultiplied: false,
            texture: Some((texture, width, height)),
        };
        self.images.insert(id, image);
        Ok(())
    }

    /// Shows a picture the renderer draws under the number draw lists use
    /// for it, replacing any image there: `view` is an
    /// [`crate::Offscreen`] target's. Set it again when the target is made
    /// anew (a new size).
    pub fn set_picture(&mut self, gpu: &Gpu, id: ImageId, view: &wgpu::TextureView) {
        let group = bind_view(gpu, &self.texture_layout, &self.sampler, view, "ui picture");
        let image = GpuImage {
            group,
            premultiplied: true,
            texture: None,
        };
        self.images.insert(id, image);
    }

    /// True if an image was uploaded under this number.
    pub fn has_image(&self, id: ImageId) -> bool {
        self.images.contains_key(&id)
    }

    /// Brings the glyph pages on the GPU up to date.
    fn update_pages(&mut self, gpu: &Gpu, pages: &[GlyphPage]) {
        for (index, page) in pages.iter().enumerate() {
            match self.pages.get_mut(index) {
                Some(uploaded) if uploaded.version == page.version => {}
                Some(uploaded) => {
                    write_pixels(gpu, &uploaded.texture, PAGE_SIZE, PAGE_SIZE, &page.pixels);
                    uploaded.version = page.version;
                }
                None => {
                    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("glyph page"),
                        size: wgpu::Extent3d {
                            width: PAGE_SIZE,
                            height: PAGE_SIZE,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    write_pixels(gpu, &texture, PAGE_SIZE, PAGE_SIZE, &page.pixels);
                    let group = bind(
                        gpu,
                        &self.texture_layout,
                        &self.sampler,
                        &texture,
                        "glyph page",
                    );
                    self.pages.push(GpuPage {
                        version: page.version,
                        texture,
                        group,
                    });
                }
            }
        }
    }

    /// Fills `output` with one colour (RGBA, 0 to 1), for an interface drawn
    /// with no world under it.
    pub fn clear(
        &self,
        gpu: &Gpu,
        output: &wgpu::TextureView,
        [r, g, b, a]: [f64; 4],
    ) -> Result<(), RenderError> {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ui clear"),
            });
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        gpu.queue.submit([encoder.finish()]);
        gpu.check("clearing under the interface")
    }

    /// Draws `list` over `output`, a `width` × `height` view of a texture of
    /// the pipeline's format, keeping what is there. Batches whose image was
    /// never uploaded are skipped.
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        output: &wgpu::TextureView,
        (width, height): (u32, u32),
        list: &DrawList,
        pages: &[GlyphPage],
    ) -> Result<(), RenderError> {
        if list.vertices.is_empty() || width == 0 || height == 0 {
            return Ok(());
        }
        self.update_pages(gpu, pages);
        let screen = ScreenUniform {
            size: [width as f32, height as f32, 0.0, 0.0],
        };
        gpu.queue
            .write_buffer(&self.screen_buffer, 0, bytemuck::bytes_of(&screen));
        let bytes: &[u8] = bytemuck::cast_slice(&list.vertices);
        let needed = bytes.len() as u64;
        if self
            .vertices
            .as_ref()
            .is_none_or(|(_, capacity)| *capacity < needed)
        {
            let capacity = needed.next_power_of_two().max(64 * 1024);
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui vertices"),
                size: capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.vertices = Some((buffer, capacity));
        }
        let Some((buffer, _)) = &self.vertices else {
            return Ok(());
        };
        gpu.queue.write_buffer(buffer, 0, bytes);

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.screen_group, &[]);
            pass.set_vertex_buffer(0, buffer.slice(..));
            let mut premultiplied = false;
            for batch in &list.batches {
                let found = match batch.texture {
                    TextureRef::White => Some((&self.white, false)),
                    TextureRef::Image(id) => {
                        self.images.get(&id).map(|i| (&i.group, i.premultiplied))
                    }
                    TextureRef::Glyphs(page) => {
                        self.pages.get(page as usize).map(|p| (&p.group, false))
                    }
                };
                let Some((group, wants_premultiplied)) = found else {
                    continue;
                };
                let Some((x, y, w, h)) = scissor(batch.clip, width, height) else {
                    continue;
                };
                if wants_premultiplied != premultiplied {
                    premultiplied = wants_premultiplied;
                    pass.set_pipeline(if premultiplied {
                        &self.premultiplied
                    } else {
                        &self.pipeline
                    });
                }
                pass.set_scissor_rect(x, y, w, h);
                pass.set_bind_group(1, group, &[]);
                pass.draw(batch.vertices.clone(), 0..1);
            }
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.check("drawing the interface")
    }
}

/// The clip rectangle in whole pixels inside the target; `None` if nothing
/// of it is inside.
fn scissor(clip: Option<m2_ui::Rect>, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let Some(clip) = clip else {
        return Some((0, 0, width, height));
    };
    let x0 = clip.x.floor().clamp(0.0, width as f32) as u32;
    let y0 = clip.y.floor().clamp(0.0, height as f32) as u32;
    let x1 = clip.right().ceil().clamp(0.0, width as f32) as u32;
    let y1 = clip.bottom().ceil().clamp(0.0, height as f32) as u32;
    (x1 > x0 && y1 > y0).then(|| (x0, y0, x1 - x0, y1 - y0))
}

fn write_pixels(gpu: &Gpu, texture: &wgpu::Texture, width: u32, height: u32, pixels: &[u8]) {
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

/// Uploads an image and binds it with the sampler.
fn upload(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    image: &RgbaImage,
    label: &'static str,
) -> Result<(wgpu::Texture, wgpu::BindGroup), RenderError> {
    let max = gpu.device.limits().max_texture_dimension_2d;
    if image.width() > max || image.height() > max {
        return Err(RenderError::Texture(format!(
            "{}×{} is over the device's limit of {max}",
            image.width(),
            image.height()
        )));
    }
    let size = wgpu::Extent3d {
        width: image.width(),
        height: image.height(),
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_pixels(gpu, &texture, image.width(), image.height(), image.pixels());
    let group = bind(gpu, layout, sampler, &texture, label);
    Ok((texture, group))
}

/// A texture and the sampler as a bind group of `layout`.
fn bind(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    texture: &wgpu::Texture,
    label: &'static str,
) -> wgpu::BindGroup {
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    bind_view(gpu, layout, sampler, &view, label)
}

/// A texture view and the sampler as a bind group of `layout`.
fn bind_view(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
    label: &'static str,
) -> wgpu::BindGroup {
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_rectangles_become_scissors_inside_the_target() {
        assert_eq!(scissor(None, 800, 600), Some((0, 0, 800, 600)));
        let clip = |x, y, w, h| Some(m2_ui::Rect::new(x, y, w, h));
        assert_eq!(
            scissor(clip(10.5, 20.0, 100.0, 50.2), 800, 600),
            Some((10, 20, 101, 51))
        );
        assert_eq!(
            scissor(clip(-10.0, -10.0, 30.0, 30.0), 800, 600),
            Some((0, 0, 20, 20))
        );
        assert_eq!(
            scissor(clip(790.0, 0.0, 30.0, 30.0), 800, 600),
            Some((790, 0, 10, 30))
        );
        assert_eq!(scissor(clip(900.0, 0.0, 30.0, 30.0), 800, 600), None);
    }
}
