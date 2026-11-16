//! The wgpu device: one adapter, one device, one queue.

use std::future::Future;
use std::pin::pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use crate::RenderError;

/// How to choose the adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GpuOptions {
    /// Only a software adapter (WARP on Windows, llvmpipe or lavapipe on
    /// Linux). Slower, but available without a GPU driver.
    pub software: bool,
    /// The adapter that uses less power (a laptop's integrated GPU), for
    /// a small window that leaves the fast one to the game.
    pub low_power: bool,
}

/// An open graphics device.
///
/// wgpu reports some errors through a callback instead of return values.
/// They are collected here; [`Gpu::check`] turns them into a
/// [`RenderError`], so a bug shows up as an error instead of a panic.
pub struct Gpu {
    /// The wgpu instance (creates surfaces).
    pub instance: wgpu::Instance,
    /// The adapter in use.
    pub adapter: wgpu::Adapter,
    /// The device.
    pub device: wgpu::Device,
    /// The device's queue.
    pub queue: wgpu::Queue,
    errors: Arc<Mutex<Vec<String>>>,
}

impl Gpu {
    /// A device for offscreen rendering. `WGPU_BACKEND` (e.g. `dx12`,
    /// `vulkan`, `gl`) limits the backends tried.
    pub fn headless(options: GpuOptions) -> Result<Self, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        Self::new(instance, None, options)
    }

    /// A device that can present to `surface`, which `instance` created.
    pub fn for_surface(
        instance: wgpu::Instance,
        surface: &wgpu::Surface<'_>,
        options: GpuOptions,
    ) -> Result<Self, RenderError> {
        Self::new(instance, Some(surface), options)
    }

    fn new(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        options: GpuOptions,
    ) -> Result<Self, RenderError> {
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: if options.low_power {
                wgpu::PowerPreference::LowPower
            } else {
                wgpu::PowerPreference::HighPerformance
            },
            force_fallback_adapter: options.software,
            compatible_surface: surface,
            ..Default::default()
        }))
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        let limits = wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits());
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("openm2"),
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;
        let errors: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = Arc::clone(&errors);
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(error.to_string());
        }));
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            errors,
        })
    }

    /// The adapter's name and backend, e.g. `Microsoft Basic Render Driver
    /// (Dx12, software)`.
    pub fn adapter_name(&self) -> String {
        let info = self.adapter.get_info();
        let kind = match info.device_type {
            wgpu::DeviceType::Cpu => ", software",
            _ => "",
        };
        format!("{} ({:?}{kind})", info.name, info.backend)
    }

    /// The largest MSAA sample count up to 4 that `format` and the depth
    /// format both support.
    pub fn sample_count(&self, format: wgpu::TextureFormat) -> u32 {
        let supports = |count| {
            [format, crate::renderer::DEPTH_FORMAT].iter().all(|f| {
                self.adapter
                    .get_texture_format_features(*f)
                    .flags
                    .sample_count_supported(count)
            })
        };
        if supports(4) { 4 } else { 1 }
    }

    /// Returns the errors wgpu reported since the last check, as one error.
    pub fn check(&self, what: &'static str) -> Result<(), RenderError> {
        let mut errors = self.errors.lock().unwrap_or_else(PoisonError::into_inner);
        if errors.is_empty() {
            return Ok(());
        }
        let detail = errors.join("; ");
        errors.clear();
        Err(RenderError::Gpu { what, detail })
    }

    /// Waits until the GPU has finished all submitted work.
    pub fn wait(&self) -> Result<(), RenderError> {
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map(drop)
            .map_err(|e| RenderError::Gpu {
                what: "waiting for the GPU",
                detail: e.to_string(),
            })
    }
}

/// Runs a future to completion on this thread. wgpu's futures finish on
/// their own (native backends) or after a device poll; nothing else here is
/// async, so this replaces an executor.
pub fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}
