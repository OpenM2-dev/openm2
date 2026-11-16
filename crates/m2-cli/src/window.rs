//! The viewer windows: `openm2 view model` and `view map` without
//! `--screenshot`, and the [`Surface`] that `openm2 play` draws into too.
//!
//! Drag with the left mouse button to turn the camera, scroll to zoom, R
//! resets the camera, Escape closes the window. Models: Space pauses the
//! animation, the arrow keys step it while paused. Maps: WASD or the arrow
//! keys move over the ground (Shift for faster), F switches the fog.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use glam::Vec3;
use m2_render::renderer::Attachments;
use m2_render::{
    Bounds, Camera, EffectGeometry, Gpu, GpuArea, GpuOptions, GpuPart, GpuTexture, Renderer, Scene,
    WindowSurface, World,
};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::effects::{EffectLibrary, PlacedEffects, upload_pending};
use crate::mapview::{self, GpuMap, LoadedMap, MapArgs};
use crate::view::{Loaded, ModelArgs, angles};

/// Animation step for the arrow keys, in seconds.
const STEP: f32 = 1.0 / 30.0;
/// Radians the camera turns per pixel dragged.
const TURN: f32 = 0.01;

/// Opens a window showing the models until it is closed.
pub(crate) fn run(args: &ModelArgs, loaded: Loaded, options: GpuOptions) -> Result<()> {
    // Frame the whole animation (or the rest pose) once, so the camera stays
    // still while the model moves.
    let samples = 16;
    let times: Vec<f32> = (0..=samples)
        .map(|k| loaded.duration() * k as f32 / samples as f32)
        .collect();
    let bounds = loaded
        .bounds(&times)
        .context("the models have no vertices")?;
    let (yaw, pitch) = angles(args);
    let title = format!("openm2 view: {}", args.path);
    let mut viewer = Viewer {
        title,
        size: args.size,
        loaded,
        options,
        bounds,
        start: (yaw, pitch),
        yaw,
        pitch,
        zoom: 1.0,
        time: args.time,
        paused: false,
        last_frame: None,
        cursor: None,
        dragging: false,
        state: None,
        error: None,
    };
    let event_loop = EventLoop::new().context("opening a window")?;
    event_loop
        .run_app(&mut viewer)
        .context("running the window")?;
    match viewer.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// A window and what draws into it.
pub(crate) struct Surface {
    pub window: Arc<Window>,
    surface: WindowSurface,
    pub gpu: Gpu,
    pub renderer: Renderer,
    pub attachments: Attachments,
}

impl Surface {
    pub fn open(
        event_loop: &ActiveEventLoop,
        title: &str,
        size: (u32, u32),
        options: GpuOptions,
    ) -> Result<Self> {
        let attributes = Window::default_attributes()
            .with_title(title.to_owned())
            .with_inner_size(LogicalSize::new(size.0, size.1));
        let window = Arc::new(event_loop.create_window(attributes)?);
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let surface = instance
            .create_surface(Arc::clone(&window))
            .map_err(|e| anyhow!("creating the window surface: {e}"))?;
        let gpu = Gpu::for_surface(instance, &surface, options)?;
        let PhysicalSize { width, height } = window.inner_size();
        // Drawn in gamma space like offscreen targets.
        let surface = WindowSurface::new(&gpu, surface, (width, height))?;
        let format = surface.format();
        let renderer = Renderer::new(&gpu, format, gpu.sample_count(format))?;
        let (width, height) = surface.size();
        let attachments = Attachments::new(&gpu, &renderer, width, height);
        eprintln!("drawing with {}", gpu.adapter_name());
        Ok(Self {
            window,
            surface,
            gpu,
            renderer,
            attachments,
        })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if !self.surface.resize(&self.gpu, (size.width, size.height)) {
            return;
        }
        self.attachments = Attachments::new(&self.gpu, &self.renderer, size.width, size.height);
    }

    pub fn aspect(&self) -> f32 {
        let (width, height) = self.attachments.size();
        width as f32 / height.max(1) as f32
    }

    /// The next frame to draw into and a view of it; `None` when there is
    /// nothing to draw this time.
    pub fn frame(&mut self) -> Result<Option<(wgpu::SurfaceTexture, wgpu::TextureView)>> {
        Ok(self.surface.frame(&self.gpu)?)
    }

    pub fn present(&self, frame: wgpu::SurfaceTexture) {
        self.window.pre_present_notify();
        self.surface.present(&self.gpu, frame);
    }
}

/// What exists once the model window is open.
struct State {
    surface: Surface,
    parts: Vec<GpuPart>,
}

struct Viewer {
    title: String,
    size: (u32, u32),
    loaded: Loaded,
    options: GpuOptions,
    bounds: Bounds,
    start: (f32, f32),
    yaw: f32,
    pitch: f32,
    zoom: f32,
    time: f32,
    paused: bool,
    last_frame: Option<Instant>,
    cursor: Option<PhysicalPosition<f64>>,
    dragging: bool,
    state: Option<State>,
    error: Option<anyhow::Error>,
}

impl Viewer {
    fn open(&self, event_loop: &ActiveEventLoop) -> Result<State> {
        let surface = Surface::open(event_loop, &self.title, self.size, self.options)?;
        let parts = self
            .loaded
            .parts
            .iter()
            .map(|p| surface.renderer.upload(&surface.gpu, &p.part, &p.textures))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(State { surface, parts })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if let Some(state) = &mut self.state {
            state.surface.resize(size);
        }
    }

    fn redraw(&mut self) -> Result<()> {
        let now = Instant::now();
        let elapsed = self
            .last_frame
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        self.last_frame = Some(now);
        let duration = self.loaded.duration();
        if !self.paused && duration > 0.0 {
            self.time = (self.time + elapsed) % duration;
        }
        let Some(state) = &mut self.state else {
            return Ok(());
        };
        let surface = &mut state.surface;
        for (part, palette) in state.parts.iter().zip(self.loaded.palettes(self.time)) {
            surface.renderer.set_palette(&surface.gpu, part, &palette);
        }
        let mut camera = Camera::framing(self.bounds, self.yaw, self.pitch, surface.aspect());
        camera.distance *= self.zoom;
        let Some((frame, view)) = surface.frame()? else {
            return Ok(());
        };
        let parts: Vec<&GpuPart> = state.parts.iter().collect();
        surface.renderer.draw(
            &surface.gpu,
            &surface.attachments,
            &view,
            &Scene::new(camera),
            &parts,
        )?;
        surface.present(frame);
        Ok(())
    }

    fn key(&mut self, event_loop: &ActiveEventLoop, key: &Key) {
        let duration = self.loaded.duration();
        match key {
            Key::Named(NamedKey::Escape) => event_loop.exit(),
            Key::Named(NamedKey::Space) => self.paused = !self.paused,
            Key::Named(NamedKey::ArrowRight) if duration > 0.0 => {
                self.paused = true;
                self.time = (self.time + STEP) % duration;
            }
            Key::Named(NamedKey::ArrowLeft) if duration > 0.0 => {
                self.paused = true;
                self.time = (self.time - STEP).rem_euclid(duration);
            }
            Key::Character(c) if c.eq_ignore_ascii_case("r") => {
                (self.yaw, self.pitch) = self.start;
                self.zoom = 1.0;
            }
            _ => {}
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for Viewer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match self.open(event_loop) {
            Ok(state) => {
                state.surface.window.request_redraw();
                self.state = Some(state);
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => self.resize(size),
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.redraw() {
                    self.fail(event_loop, error);
                    return;
                }
                if let Some(state) = &self.state {
                    state.surface.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => self.key(event_loop, &logical_key),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => self.dragging = state == ElementState::Pressed,
            WindowEvent::CursorMoved { position, .. } => {
                if self.dragging
                    && let Some(last) = self.cursor
                {
                    self.yaw -= (position.x - last.x) as f32 * TURN;
                    self.pitch =
                        (self.pitch + (position.y - last.y) as f32 * TURN).clamp(-1.5, 1.5);
                }
                self.cursor = Some(position);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                self.zoom = (self.zoom * 0.9f32.powf(lines)).clamp(0.05, 20.0);
            }
            _ => {}
        }
    }
}

/// Opens a window showing a map until it is closed.
pub(crate) fn run_map(
    args: &MapArgs,
    map: LoadedMap,
    library: EffectLibrary,
    options: GpuOptions,
) -> Result<()> {
    let aspect = args.size.0 as f32 / args.size.1.max(1) as f32;
    let camera = mapview::camera(args, &map, aspect)?;
    let effects = PlacedEffects::new(&map.effects, 1.0);
    let mut viewer = MapViewer {
        library,
        effects,
        geometry: EffectGeometry::default(),
        title: format!("openm2 view: {}", args.map),
        size: args.size,
        options,
        span: args.texture_span,
        map,
        start: camera,
        camera,
        fog: args.fog,
        held: Held::default(),
        last_frame: None,
        seconds: 0.0,
        cursor: None,
        dragging: false,
        state: None,
        error: None,
    };
    let event_loop = EventLoop::new().context("opening a window")?;
    event_loop
        .run_app(&mut viewer)
        .context("running the window")?;
    match viewer.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The movement keys held down.
#[derive(Default)]
struct Held {
    forward: bool,
    back: bool,
    left: bool,
    right: bool,
    fast: bool,
}

/// What exists once the map window is open.
struct MapState {
    surface: Surface,
    map: GpuMap,
    textures: Vec<GpuTexture>,
}

/// How far from the point looked at effects play, in centimetres.
const EFFECT_REACH: f32 = 10_000.0;

struct MapViewer {
    library: EffectLibrary,
    effects: PlacedEffects,
    geometry: EffectGeometry,
    title: String,
    size: (u32, u32),
    options: GpuOptions,
    span: f32,
    map: LoadedMap,
    start: Camera,
    camera: Camera,
    fog: bool,
    held: Held,
    last_frame: Option<Instant>,
    /// Seconds since the window opened, for the clouds.
    seconds: f32,
    cursor: Option<PhysicalPosition<f64>>,
    dragging: bool,
    state: Option<MapState>,
    error: Option<anyhow::Error>,
}

impl MapViewer {
    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<MapState> {
        let surface = Surface::open(event_loop, &self.title, self.size, self.options)?;
        let map = GpuMap::upload(&surface.gpu, &surface.renderer, &self.map, self.span)?;
        let mut textures = Vec::new();
        upload_pending(
            &surface.gpu,
            &surface.renderer,
            &mut self.library,
            &mut textures,
        )?;
        Ok(MapState {
            surface,
            map,
            textures,
        })
    }

    /// Moves the point looked at over the ground, as the keys say.
    fn walk(&mut self, seconds: f32) {
        let held = &self.held;
        let ahead = f32::from(u8::from(held.forward)) - f32::from(u8::from(held.back));
        let side = f32::from(u8::from(held.right)) - f32::from(u8::from(held.left));
        if ahead == 0.0 && side == 0.0 {
            return;
        }
        let (sin, cos) = self.camera.yaw.sin_cos();
        let forward = Vec3::new(-sin, cos, 0.0);
        let right = Vec3::new(cos, sin, 0.0);
        let speed = self.camera.distance.max(500.0) * if held.fast { 2.0 } else { 0.5 };
        let mut target = self.camera.target + (forward * ahead + right * side) * speed * seconds;
        if let Some(z) = self.map.ground_at(target.x, target.y) {
            target.z = z;
        }
        self.camera.target = target;
    }

    fn redraw(&mut self) -> Result<()> {
        let now = Instant::now();
        let elapsed = self
            .last_frame
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32().min(0.1));
        self.last_frame = Some(now);
        self.seconds += elapsed;
        self.walk(elapsed);
        let Some(state) = &mut self.state else {
            return Ok(());
        };
        let surface = &mut state.surface;
        let scene = mapview::scene(&self.map, self.camera, self.fog);
        let Some((frame, view)) = surface.frame()? else {
            return Ok(());
        };
        let target = self.camera.target;
        self.effects.advance(elapsed, target, EFFECT_REACH);
        self.geometry.clear();
        self.effects.draw(
            &self.camera.view(),
            target,
            EFFECT_REACH,
            &mut self.geometry,
        );
        let effects = surface
            .renderer
            .upload_effects(&surface.gpu, &self.geometry)?;
        let areas: Vec<&GpuArea> = state.map.areas.iter().collect();
        let parts: Vec<&GpuPart> = state.map.parts.iter().collect();
        surface.renderer.draw_world(
            &surface.gpu,
            &surface.attachments,
            &view,
            &scene,
            &World {
                effects: Some(&effects),
                effect_textures: &state.textures,
                seconds: self.seconds,
                ..state.map.world(&areas, &parts)
            },
        )?;
        surface.present(frame);
        Ok(())
    }

    fn key(&mut self, event_loop: &ActiveEventLoop, key: &Key, pressed: bool) {
        let is = |c: &str| matches!(key, Key::Character(k) if k.eq_ignore_ascii_case(c));
        match key {
            Key::Named(NamedKey::Escape) if pressed => event_loop.exit(),
            Key::Named(NamedKey::ArrowUp) => self.held.forward = pressed,
            Key::Named(NamedKey::ArrowDown) => self.held.back = pressed,
            Key::Named(NamedKey::ArrowLeft) => self.held.left = pressed,
            Key::Named(NamedKey::ArrowRight) => self.held.right = pressed,
            Key::Named(NamedKey::Shift) => self.held.fast = pressed,
            _ if is("w") => self.held.forward = pressed,
            _ if is("s") => self.held.back = pressed,
            _ if is("a") => self.held.left = pressed,
            _ if is("d") => self.held.right = pressed,
            _ if is("f") && pressed => self.fog = !self.fog,
            _ if is("r") && pressed => self.camera = self.start,
            _ => {}
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for MapViewer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match self.open(event_loop) {
            Ok(state) => {
                state.surface.window.request_redraw();
                self.state = Some(state);
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state {
                    state.surface.resize(size);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.redraw() {
                    self.fail(event_loop, error);
                    return;
                }
                if let Some(state) = &self.state {
                    state.surface.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    logical_key, state, ..
                },
                ..
            } => self.key(event_loop, &logical_key, state == ElementState::Pressed),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => self.dragging = state == ElementState::Pressed,
            WindowEvent::CursorMoved { position, .. } => {
                if self.dragging
                    && let Some(last) = self.cursor
                {
                    self.camera.yaw -= (position.x - last.x) as f32 * TURN;
                    self.camera.pitch =
                        (self.camera.pitch + (position.y - last.y) as f32 * TURN).clamp(0.05, 1.55);
                }
                self.cursor = Some(position);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                self.camera.distance =
                    (self.camera.distance * 0.9f32.powf(lines)).clamp(200.0, 500_000.0);
            }
            _ => {}
        }
    }
}
