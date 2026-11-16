//! Drawing the window's interface: into the window ([`Painter`], with the
//! adapter that uses less power, else a software one), or offscreen into a
//! PNG for screenshots and tests ([`render`]).

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use m2_render::renderer::OFFSCREEN_FORMAT;
use m2_render::{Gpu, GpuOptions, Offscreen, UiFilter, UiRenderer, WindowSurface};
use m2_ui::{ImageId, Rect, Sprite, Ui};
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use super::screen::{Look, PICTURE, SIZE};

/// The number the picture is uploaded under.
const PICTURE_IMAGE: ImageId = ImageId(1);

/// The window and what draws into it.
pub struct Painter {
    /// The window.
    pub window: Arc<Window>,
    surface: WindowSurface,
    gpu: Gpu,
    renderer: UiRenderer,
    clear: [f64; 4],
}

impl Painter {
    /// Opens the window, hidden until [`Painter::show`]. Its device is the
    /// adapter that uses less power, or with `software` (or when that one
    /// fails) a software one. `note` hears why the first one failed.
    pub fn open(
        event_loop: &ActiveEventLoop,
        look: &Look,
        ui: &mut Ui,
        software: bool,
        note: &mut dyn FnMut(&str),
    ) -> Result<Self> {
        let attributes = Window::default_attributes()
            .with_title(look.name.clone())
            .with_inner_size(LogicalSize::new(SIZE.0, SIZE.1))
            .with_resizable(false)
            .with_visible(false);
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .context("opening the window")?,
        );
        let device = |software: bool| -> Result<(Gpu, WindowSurface)> {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                    Box::new(event_loop.owned_display_handle()),
                ));
            let surface = instance
                .create_surface(Arc::clone(&window))
                .map_err(|error| anyhow!("creating the window surface: {error}"))?;
            let options = GpuOptions {
                software,
                low_power: true,
            };
            let gpu = Gpu::for_surface(instance, &surface, options)?;
            let PhysicalSize { width, height } = window.inner_size();
            let surface = WindowSurface::new(&gpu, surface, (width, height))?;
            Ok((gpu, surface))
        };
        let (gpu, surface) = match device(software) {
            Ok(opened) => opened,
            Err(error) if !software => {
                note(&format!(
                    "drawing in software: the graphics adapter failed ({error:#})"
                ));
                device(true)?
            }
            Err(error) => return Err(error),
        };
        note(&format!("drawing with {}", gpu.adapter_name()));
        let renderer = new_renderer(&gpu, surface.format(), look, ui)?;
        ui.set_scale(window.scale_factor() as f32);
        Ok(Self {
            window,
            surface,
            gpu,
            renderer,
            clear: look.clear_colour(),
        })
    }

    /// Shows the window and gives it the keyboard.
    pub fn show(&self) {
        self.window.set_visible(true);
        self.window.focus_window();
    }

    /// Configures it for the window's new size.
    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.surface.resize(&self.gpu, (size.width, size.height));
    }

    /// Draws `ui` into the window. False when there was nothing to draw
    /// into this time.
    pub fn draw(&mut self, ui: &mut Ui) -> Result<bool> {
        let Some((frame, view)) = self.surface.frame(&self.gpu)? else {
            return Ok(false);
        };
        let (width, height) = self.surface.size();
        self.renderer.clear(&self.gpu, &view, self.clear)?;
        let (list, pages) = ui.draw(width as f32, height as f32);
        self.renderer
            .draw(&self.gpu, &view, (width, height), list, pages)?;
        self.window.pre_present_notify();
        self.surface.present(&self.gpu, frame);
        Ok(true)
    }
}

/// The interface pipeline for targets of `format`, with the look's picture
/// uploaded and named in `ui`.
fn new_renderer(
    gpu: &Gpu,
    format: wgpu::TextureFormat,
    look: &Look,
    ui: &mut Ui,
) -> Result<UiRenderer> {
    // Smooth: the picture is drawn at half its size at the usual scale.
    let mut renderer = UiRenderer::new(gpu, format, UiFilter::Smooth)?;
    if let Some(bytes) = &look.picture {
        let (image, _) = m2_render::png::decode_any(bytes)?;
        let size = (image.width() as f32, image.height() as f32);
        renderer.set_image(gpu, PICTURE_IMAGE, &image)?;
        ui.set_sprite(
            PICTURE,
            Sprite {
                image: PICTURE_IMAGE,
                uv: Rect::new(0.0, 0.0, 1.0, 1.0),
                size,
            },
        );
    }
    Ok(renderer)
}

/// Draws `ui` in `look` offscreen at `scale` times the window's size, and
/// encodes it as PNG; the adapter's name with it.
pub fn render(ui: &mut Ui, look: &Look, scale: f32, software: bool) -> Result<(Vec<u8>, String)> {
    let gpu = Gpu::headless(GpuOptions {
        software,
        low_power: true,
    })?;
    let mut renderer = new_renderer(&gpu, OFFSCREEN_FORMAT, look, ui)?;
    ui.set_scale(scale);
    let (width, height) = (
        (SIZE.0 as f32 * ui.scale()).round() as u32,
        (SIZE.1 as f32 * ui.scale()).round() as u32,
    );
    let target = Offscreen::for_interface(&gpu, width, height)?;
    renderer.clear(&gpu, target.view(), look.clear_colour())?;
    let (list, pages) = ui.draw(width as f32, height as f32);
    renderer.draw(&gpu, target.view(), (width, height), list, pages)?;
    let image = target.read(&gpu)?;
    Ok((m2_render::png::encode(&image)?, gpu.adapter_name()))
}
