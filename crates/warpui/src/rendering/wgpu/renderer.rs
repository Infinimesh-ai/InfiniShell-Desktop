mod frame;
mod glyph;
mod image;
mod rect;
mod util;

use std::sync::atomic::Ordering;

use frame::Frame;
use pathfinder_geometry::vector::Vector2F;
use util::with_error_scope;
use warpui_core::platform::CapturedFrame;
use wgpu::wgc::device::DeviceError;
use wgpu::wgc::present::SurfaceError;

pub use super::resources::{GetSurfaceTextureError, SurfaceConfigureError};
use crate::Scene;
use crate::r#async::block_on;
use crate::rendering::wgpu::Resources;
use crate::rendering::{GlyphConfig, GlyphRasterBoundsFn, RasterizeGlyphFn};

const ENCODER_DESCRIPTOR: wgpu::CommandEncoderDescriptor = wgpu::CommandEncoderDescriptor {
    label: Some("Command encoder"),
};

pub struct Renderer {
    rect_pipeline: rect::Pipeline,
    glyph_pipeline: glyph::Pipeline,
    image_pipeline: image::Pipeline,
}

impl Renderer {
    pub fn new(resources: &Resources, glyph_config: GlyphConfig) -> Self {
        let Resources { device, .. } = resources;

        let format = resources.surface_config.borrow().format;
        let color_target = wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::all(),
        };

        let rect_pipeline = rect::Pipeline::new(
            resources.uniform_bind_group_layout(),
            device,
            color_target.clone(),
        );

        let glyph_pipeline = glyph::Pipeline::new(
            resources.uniform_bind_group_layout(),
            device,
            color_target.clone(),
            glyph_config,
        );

        let image_pipeline =
            image::Pipeline::new(resources.uniform_bind_group_layout(), device, color_target);

        Self {
            rect_pipeline,
            glyph_pipeline,
            image_pipeline,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render<'a>(
        &mut self,
        scene: &Scene,
        resources: &Resources,
        rasterize_glyph_fn: &RasterizeGlyphFn,
        glyph_raster_bounds_fn: &GlyphRasterBoundsFn,
        window_size: Vector2F,
        pre_present_callback: Option<Box<dyn FnOnce() + 'a>>,
        capture_callback: Option<Box<dyn FnOnce(CapturedFrame) + Send + 'static>>,
    ) -> Result<(), Error> {
        let Resources { device, queue, .. } = resources;

        // Don't initiate the render if we are trying to render into a
        // zero-sized window.
        if window_size.is_zero() {
            return Ok(());
        }

        // Surface acquisition can degrade to persistent validation errors after the device-lost
        // callback fires, which bypasses the event loop's renderer-recovery branch.
        if resources.device_lost.load(Ordering::SeqCst) {
            return Err(Error::DeviceLost);
        }

        let mut ctx = WGPUContext {
            resources,
            rasterize_glyph_fn,
            glyph_raster_bounds_fn,
        };

        let frame = match with_error_scope(device, || {
            Frame::new(
                scene,
                &mut ctx,
                &self.rect_pipeline,
                &mut self.glyph_pipeline,
                &mut self.image_pipeline,
            )
        }) {
            (_, Some(error)) => return Err(error),
            (frame, _) => frame,
        };

        let surface_texture = resources.get_surface_texture()?;

        let mut encoder = device.create_command_encoder(&ENCODER_DESCRIPTOR);
        let (backing_texture, error) = with_error_scope(device, || {
            // 部分 GL surface 只支持渲染；截图时先渲染到可读纹理，再把同一帧画到真实窗口。
            let backing_texture = (capture_callback.is_some()
                && !surface_texture
                    .texture
                    .usage()
                    .contains(wgpu::TextureUsages::COPY_SRC))
            .then(|| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Frame capture backing texture"),
                    size: surface_texture.texture.size(),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: surface_texture.texture.format(),
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            });
            let render_target = backing_texture.as_ref().unwrap_or(&surface_texture.texture);
            frame.draw(resources, &mut encoder, render_target);
            if let Some(texture) = &backing_texture {
                // 默认最近邻且不混合，保留截图纹理的颜色与透明度；不要求 surface 支持 COPY_DST。
                wgpu::util::TextureBlitter::new(device, surface_texture.texture.format()).copy(
                    device,
                    &mut encoder,
                    &texture.create_view(&Default::default()),
                    &surface_texture.texture.create_view(&Default::default()),
                );
            }
            queue.submit(Some(encoder.finish()));
            backing_texture
        });

        if error.is_none()
            && let Some(callback) = capture_callback
            && let Err(err) = capture_texture(
                device,
                queue,
                backing_texture.as_ref().unwrap_or(&surface_texture.texture),
                callback,
            )
        {
            log::warn!("Frame capture failed: {err}");
        }

        if let Some(callback) = pre_present_callback {
            callback();
        }

        match error {
            Some(error) => Err(error),
            None => {
                // Only present the surface if there were no errors, otherwise
                // wgpu will print out an error that we attempted to present a
                // texture without submitting any work to the GPU.
                match with_error_scope(device, || {
                    queue.present(surface_texture);
                }) {
                    (_, None) => Ok(()),
                    (_, Some(error)) => Err(error),
                }
            }
        }
    }
}

/// Errors that can occur while rendering a scene.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Device was lost")]
    DeviceLost,
    #[error("Failed to map buffer range: {0}")]
    BufferMap(#[from] wgpu::MapRangeError),
    #[error("Failed to acquire surface texture: {0:#}")]
    SurfaceError(#[from] GetSurfaceTextureError),
    #[error("Failed to configure surface: {0:#}")]
    SurfaceConfigureError(#[from] SurfaceConfigureError),
    #[error("{0:#}")]
    Unknown(#[source] wgpu::Error),
}

impl From<wgpu::Error> for Error {
    fn from(value: wgpu::Error) -> Self {
        for error in anyhow::Chain::new(&value) {
            if let Some(DeviceError::Lost) = error.downcast_ref::<DeviceError>() {
                return Error::DeviceLost;
            }

            // The use of `#[transparent]` for many nested device errors breaks
            // error chaining - the call to `source()` gets forwarded to the
            // DeviceError::Lost, which returns None (it doesn't wrap an error).
            // Ideally, these wrapped errors should use `#[from]` instead, but
            // until then, we need to do this to properly catch DeviceError::Lost
            // from within a call to present().
            if let Some(SurfaceError::Device(DeviceError::Lost)) =
                error.downcast_ref::<SurfaceError>()
            {
                return Error::DeviceLost;
            }
        }
        Error::Unknown(value)
    }
}

/// 读取真实渲染纹理并同步回调；回调应尽快返回，避免阻塞窗口呈现。
fn capture_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    callback: Box<dyn FnOnce(CapturedFrame) + Send + 'static>,
) -> Result<(), String> {
    let width = texture.width();
    let height = texture.height();

    if width == 0 || height == 0 {
        return Err(format!("Invalid texture dimensions: {width}x{height}"));
    }

    let format = texture.format();
    let bytes_per_pixel = 4u32;
    let unpadded_bytes_per_row = width * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
    let buffer_size = (padded_bytes_per_row * height) as u64;

    let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Frame capture staging buffer"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Frame capture encoder"),
    });

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    let buffer_slice = staging_buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });

    block_on(async {
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    });

    let map_result = receiver
        .recv()
        .map_err(|e| format!("Failed to receive map result: {e}"))?
        .map_err(|e| format!("Buffer mapping failed: {e}"));

    map_result?;

    let data = buffer_slice
        .get_mapped_range()
        .map_err(|e| format!("Failed to get mapped range: {e}"))?;
    let mut rgba_data = Vec::with_capacity((width * height * bytes_per_pixel) as usize);
    for row in 0..height {
        let start = (row * padded_bytes_per_row) as usize;
        let end = start + unpadded_bytes_per_row as usize;
        rgba_data.extend_from_slice(&data[start..end]);
    }
    drop(data);
    staging_buffer.unmap();

    if format == wgpu::TextureFormat::Bgra8Unorm || format == wgpu::TextureFormat::Bgra8UnormSrgb {
        for chunk in rgba_data.chunks_exact_mut(4) {
            chunk.swap(0, 2);
        }
    }

    callback(CapturedFrame::new(width, height, rgba_data));
    Ok(())
}

struct WGPUContext<'a> {
    resources: &'a Resources,
    rasterize_glyph_fn: &'a RasterizeGlyphFn<'a>,
    glyph_raster_bounds_fn: &'a GlyphRasterBoundsFn<'a>,
}
