//! A window's surface: the application makes the window and the
//! `wgpu::Surface` (with winit or anything else), and this configures it
//! and hands out its frames, drawn in gamma space like offscreen targets.

use crate::RenderError;
use crate::gpu::Gpu;

/// A configured window surface.
pub struct WindowSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The format frames are drawn in: the surface's, without sRGB.
    format: wgpu::TextureFormat,
}

impl WindowSurface {
    /// Configures `surface`, which `gpu`'s instance made, for a window of
    /// `width` × `height` pixels: the adapter's preferred format seen
    /// through a non-sRGB view (textures and lighting stay in gamma space),
    /// with vsync.
    pub fn new(
        gpu: &Gpu,
        surface: wgpu::Surface<'static>,
        (width, height): (u32, u32),
    ) -> Result<Self, RenderError> {
        let mut config = surface
            .get_default_config(&gpu.adapter, width.max(1), height.max(1))
            .ok_or_else(|| {
                RenderError::Surface("the adapter cannot draw to this window".to_owned())
            })?;
        let format = config.format.remove_srgb_suffix();
        if format != config.format {
            config.view_formats = vec![format];
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&gpu.device, &config);
        Ok(Self {
            surface,
            config,
            format,
        })
    }

    /// The format frames are drawn in (pipelines are made for it).
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Width and height in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Configures it for a new size. False, changing nothing, for a size
    /// with no pixels (a minimised window).
    pub fn resize(&mut self, gpu: &Gpu, (width, height): (u32, u32)) -> bool {
        if width == 0 || height == 0 {
            return false;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&gpu.device, &self.config);
        true
    }

    /// The next frame to draw into and a view of it in [`Self::format`];
    /// `None` when there is nothing to draw this time (the window is
    /// hidden, or the surface was just configured again).
    pub fn frame(
        &mut self,
        gpu: &Gpu,
    ) -> Result<Option<(wgpu::SurfaceTexture, wgpu::TextureView)>, RenderError> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&gpu.device, &self.config);
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                gpu.check("getting the next frame")?;
                return Ok(None);
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.format),
            ..Default::default()
        });
        Ok(Some((frame, view)))
    }

    /// Shows a frame drawn into. A winit application tells its window first
    /// (`pre_present_notify`).
    pub fn present(&self, gpu: &Gpu, frame: wgpu::SurfaceTexture) {
        gpu.queue.present(frame);
    }
}
