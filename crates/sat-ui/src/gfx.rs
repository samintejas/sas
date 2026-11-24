//! GPU setup and offscreen rendering.
//!
//! The app renders to a window surface; tests and the screenshot tool render
//! the same scene into an offscreen texture and read the pixels back. Without
//! a hardware GPU, a software Vulkan driver (Mesa lavapipe) works.

use vello::kurbo::Affine;
use vello::peniko::Color;
use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

/// Why rendering could not start or finish.
#[derive(Debug)]
pub enum GfxError {
    /// No GPU adapter (hardware or software) is available.
    NoAdapter(String),
    /// The device could not be created.
    Device(String),
    /// Vello failed.
    Render(String),
}

impl core::fmt::Display for GfxError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GfxError::NoAdapter(e) => write!(f, "no GPU adapter: {e}"),
            GfxError::Device(e) => write!(f, "device: {e}"),
            GfxError::Render(e) => write!(f, "render: {e}"),
        }
    }
}

impl std::error::Error for GfxError {}

/// A GPU device with a vello renderer.
pub struct Gpu {
    /// The wgpu device.
    pub device: wgpu::Device,
    /// Its queue.
    pub queue: wgpu::Queue,
    /// The vello renderer.
    pub renderer: Renderer,
    /// Adapter description, for logs.
    pub adapter_name: String,
}

impl Gpu {
    /// Creates a device without a window.
    pub fn headless() -> Result<Gpu, GfxError> {
        Gpu::from_adapter(&Gpu::headless_adapter()?)
    }

    /// Picks the adapter [`Gpu::headless`] would use, without creating a device,
    /// so callers can inspect it first (for example to skip software rasterisers).
    pub fn headless_adapter() -> Result<wgpu::Adapter, GfxError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|e| GfxError::NoAdapter(e.to_string()))
    }

    /// Creates a device and renderer on `adapter`.
    pub fn from_adapter(adapter: &wgpu::Adapter) -> Result<Gpu, GfxError> {
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|e| GfxError::Device(e.to_string()))?;
        let renderer = Renderer::new(
            &device,
            RendererOptions { antialiasing_support: AaSupport::area_only(), ..Default::default() },
        )
        .map_err(|e| GfxError::Render(e.to_string()))?;
        Ok(Gpu { device, queue, renderer, adapter_name: format!("{} ({:?})", info.name, info.backend) })
    }

    /// Renders `scene` into a new `width`×`height` RGBA8 image.
    pub fn render_rgba(&mut self, scene: &Scene, width: u32, height: u32, base: Color) -> Result<Image, GfxError> {
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let params = RenderParams { base_color: base, width, height, antialiasing_method: AaConfig::Area };
        self.renderer
            .render_to_texture(&self.device, &self.queue, scene, &view, &params)
            .map_err(|e| GfxError::Render(e.to_string()))?;
        let padded = (width * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None },
            },
            size,
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| GfxError::Render(e.to_string()))?;
        let data = slice.get_mapped_range().map_err(|e| GfxError::Render(e.to_string()))?;
        let mut px = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let s = (row * padded) as usize;
            px.extend_from_slice(&data[s..s + (width * 4) as usize]);
        }
        Ok(Image { width, height, rgba: px })
    }
}

/// An RGBA8 image read back from the GPU.
#[derive(Debug, Clone)]
pub struct Image {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixels, row major, 4 bytes each.
    pub rgba: Vec<u8>,
}

impl Image {
    /// The pixel at (x, y) as [r, g, b, a].
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    /// Writes a PNG.
    pub fn save_png(&self, path: &std::path::Path) -> std::io::Result<()> {
        let f = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut e = png::Encoder::new(f, self.width, self.height);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header().map_err(std::io::Error::other)?;
        w.write_image_data(&self.rgba).map_err(std::io::Error::other)
    }
}

/// Wraps a scene in a scale transform for HiDPI output.
pub fn scaled(scene: &Scene, scale: f64) -> Scene {
    let mut s = Scene::new();
    s.append(scene, Some(Affine::scale(scale)));
    s
}
