//! Screen thumbnail capture for the classroom overview.
//!
//! Captures the primary display, downscales to a bounded dimension, and
//! JPEG-encodes it. Thumbnails are intentionally small/low-quality: the goal
//! is an at-a-glance grid, not a high-fidelity remote view.

use anyhow::Result;
use base64::Engine;
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, RgbaImage};
use labctl_common::protocol::ScreenThumbnail;
use uuid::Uuid;

const JPEG_QUALITY: u8 = 55;

pub fn capture_thumbnail(machine_id: Uuid, max_dim: u32) -> Result<ScreenThumbnail> {
    let full = grab_primary_screen()?;
    let (w, h) = (full.width(), full.height());
    let scale = (max_dim as f32 / w.max(h) as f32).min(1.0);
    let (tw, th) = ((w as f32 * scale) as u32, (h as f32 * scale) as u32);
    let thumb = image::imageops::resize(&full, tw.max(1), th.max(1), FilterType::Triangle);

    let mut jpeg = Vec::new();
    {
        let rgb = image::DynamicImage::ImageRgba8(thumb).into_rgb8();
        JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY).encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )?;
    }

    Ok(ScreenThumbnail {
        machine_id,
        width: tw.max(1),
        height: th.max(1),
        jpeg_base64: base64::engine::general_purpose::STANDARD.encode(&jpeg),
        captured_at: chrono::Utc::now(),
    })
}

#[cfg(windows)]
fn grab_primary_screen() -> Result<RgbaImage> {
    let monitors = xcap::Monitor::all()?;
    let mon = monitors
        .into_iter()
        .find(|m| m.is_primary())
        .or_else(|| xcap::Monitor::all().ok().and_then(|v| v.into_iter().next()))
        .ok_or_else(|| anyhow::anyhow!("no monitor found"))?;
    let img = mon.capture_image()?;
    Ok(img)
}

#[cfg(not(windows))]
fn grab_primary_screen() -> Result<RgbaImage> {
    // Dev/test placeholder so the pipeline works off-Windows.
    let mut img = RgbaImage::new(640, 400);
    for (x, _y, px) in img.enumerate_pixels_mut() {
        let v = (x % 255) as u8;
        *px = image::Rgba([v, v, v, 255]);
    }
    Ok(img)
}
