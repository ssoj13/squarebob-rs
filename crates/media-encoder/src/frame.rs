use std::borrow::Cow;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use half::f16 as F16;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const RGBA_CHANNELS: usize = 4;

pub fn checked_element_count(
    width: usize,
    height: usize,
    channels: usize,
    element_size: usize,
) -> Result<usize, FrameError> {
    let elements = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or(FrameError::LayoutOverflow {
            width,
            height,
            channels,
        })?;
    elements
        .checked_mul(element_size)
        .ok_or(FrameError::LayoutOverflow {
            width,
            height,
            channels,
        })?;
    Ok(elements)
}

#[derive(Debug, Clone)]
pub enum PixelBuffer {
    U8(Vec<u8>),
    F16(Vec<F16>),
    F32(Vec<f32>),
}

impl PixelBuffer {
    fn format(&self) -> PixelFormat {
        match self {
            Self::U8(_) => PixelFormat::Rgba8,
            Self::F16(_) => PixelFormat::RgbaF16,
            Self::F32(_) => PixelFormat::RgbaF32,
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::U8(data) => data.len(),
            Self::F16(data) => data.len(),
            Self::F32(data) => data.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8,
    RgbaF16,
    RgbaF32,
}

impl PixelFormat {
    const fn element_size(self) -> usize {
        match self {
            Self::Rgba8 => size_of::<u8>(),
            Self::RgbaF16 => size_of::<F16>(),
            Self::RgbaF32 => size_of::<f32>(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelLayout {
    width: usize,
    height: usize,
    channels: usize,
    row_elements: usize,
    element_count: usize,
    byte_len: usize,
    pixel_format: PixelFormat,
}

impl PixelLayout {
    fn rgba(
        width: usize,
        height: usize,
        pixel_format: PixelFormat,
        actual_elements: usize,
    ) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::EmptyDimensions { width, height });
        }
        u32::try_from(width).map_err(|_| FrameError::DimensionOutOfRange { width, height })?;
        u32::try_from(height).map_err(|_| FrameError::DimensionOutOfRange { width, height })?;

        let row_elements = checked_element_count(width, 1, RGBA_CHANNELS, 1)?;
        let element_count =
            checked_element_count(width, height, RGBA_CHANNELS, pixel_format.element_size())?;
        let byte_len = element_count * pixel_format.element_size();

        if actual_elements != element_count {
            return Err(FrameError::BufferLength {
                expected: element_count,
                actual: actual_elements,
                width,
                height,
                pixel_format,
            });
        }

        Ok(Self {
            width,
            height,
            channels: RGBA_CHANNELS,
            row_elements,
            element_count,
            byte_len,
            pixel_format,
        })
    }

    pub fn width(self) -> usize {
        self.width
    }

    pub fn height(self) -> usize {
        self.height
    }

    pub fn width_u32(self) -> u32 {
        self.width as u32
    }

    pub fn height_u32(self) -> u32 {
        self.height as u32
    }

    pub fn channels(self) -> usize {
        self.channels
    }

    pub fn row_elements(self) -> usize {
        self.row_elements
    }

    pub fn element_count(self) -> usize {
        self.element_count
    }

    pub fn byte_len(self) -> usize {
        self.byte_len
    }

    pub fn pixel_format(self) -> PixelFormat {
        self.pixel_format
    }

    pub fn elements_for(self, channels: usize, element_size: usize) -> Result<usize, FrameError> {
        checked_element_count(self.width, self.height, channels, element_size)
    }

    fn row_range(self, row: usize, x: usize, width: usize) -> Result<Range<usize>, FrameError> {
        let row_base = row
            .checked_mul(self.row_elements)
            .ok_or(FrameError::LayoutOverflow {
                width: self.width,
                height: self.height,
                channels: self.channels,
            })?;
        let x_offset = x
            .checked_mul(self.channels)
            .ok_or(FrameError::LayoutOverflow {
                width: self.width,
                height: self.height,
                channels: self.channels,
            })?;
        let len = width
            .checked_mul(self.channels)
            .ok_or(FrameError::LayoutOverflow {
                width: self.width,
                height: self.height,
                channels: self.channels,
            })?;
        let start = row_base
            .checked_add(x_offset)
            .ok_or(FrameError::LayoutOverflow {
                width: self.width,
                height: self.height,
                channels: self.channels,
            })?;
        let end = start.checked_add(len).ok_or(FrameError::LayoutOverflow {
            width: self.width,
            height: self.height,
            channels: self.channels,
        })?;
        if end > self.element_count {
            return Err(FrameError::RegionOutOfBounds);
        }
        Ok(start..end)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FrameError {
    #[error("frame dimensions must be non-zero, got {width}x{height}")]
    EmptyDimensions { width: usize, height: usize },
    #[error("frame dimensions exceed u32 encoder limits: {width}x{height}")]
    DimensionOutOfRange { width: usize, height: usize },
    #[error("pixel layout overflow for {width}x{height} with {channels} channels")]
    LayoutOverflow {
        width: usize,
        height: usize,
        channels: usize,
    },
    #[error(
        "invalid {pixel_format:?} buffer length for {width}x{height}: expected {expected}, got {actual}"
    )]
    BufferLength {
        expected: usize,
        actual: usize,
        width: usize,
        height: usize,
        pixel_format: PixelFormat,
    },
    #[error("pixel copy region is outside the validated frame layout")]
    RegionOutOfBounds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropAlign {
    Center,
    LeftTop,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum TonemapMode {
    Clamp,
    #[default]
    ACES,
    Reinhard,
}

#[derive(Debug, Clone)]
pub struct Frame {
    buffer: Arc<PixelBuffer>,
    layout: PixelLayout,
    file: Option<PathBuf>,
    /// Present only for display-linear Rec.709 capture, never for scene radiance.
    display: Option<(video_io::export::DisplayLight, f32)>,
}

impl Frame {
    pub fn rgba8(width: usize, height: usize, data: Vec<u8>) -> Result<Self, FrameError> {
        Self::new(width, height, PixelBuffer::U8(data), None)
    }

    pub fn rgba_f16(width: usize, height: usize, data: Vec<F16>) -> Result<Self, FrameError> {
        Self::new(width, height, PixelBuffer::F16(data), None)
    }

    pub fn rgba_f32(width: usize, height: usize, data: Vec<f32>) -> Result<Self, FrameError> {
        Self::new(width, height, PixelBuffer::F32(data), None)
    }

    pub fn new(
        width: usize,
        height: usize,
        buffer: PixelBuffer,
        file: Option<PathBuf>,
    ) -> Result<Self, FrameError> {
        let pixel_format = buffer.format();
        let layout = PixelLayout::rgba(width, height, pixel_format, buffer.len())?;
        Ok(Self {
            buffer: Arc::new(buffer),
            layout,
            file,
            display: None,
        })
    }

    /// Capture linear Rec.709 display light without SDR quantization. Relative
    /// light uses `white_nits` per unit; absolute HDR views use 100 nits per unit.
    /// Reference white identifies the capture job; alpha stays linear coverage.
    pub fn display_light(
        width: usize,
        height: usize,
        light: Vec<f32>,
        kind: video_io::export::DisplayLight,
        white_nits: f32,
    ) -> Result<Self, String> {
        if !white_nits.is_finite() || white_nits <= 0.0 || !light.iter().all(|v| v.is_finite()) {
            return Err(
                "Display light and reference white must be finite; white must be positive".into(),
            );
        }
        let mut frame = Self::rgba_f32(width, height, light).map_err(|e| e.to_string())?;
        frame.display = Some((kind, white_nits));
        Ok(frame)
    }

    /// Explicit display-light contract required by the WarpBro HDR PNG writer.
    /// Generic floating-point media frames cannot silently acquire display tags.
    pub fn hdr_light(&self) -> Result<(&[f32], video_io::export::DisplayLight, f32), String> {
        let (kind, white) = self
            .display
            .ok_or("HDR export requires display-linear Rec.709 capture")?;
        match self.buffer.as_ref() {
            PixelBuffer::F32(light) => Ok((light, kind, white)),
            _ => Err("Display-light capture must retain floating-point pixels".into()),
        }
    }

    /// Thin host adapter to the shared WarpBro display-light PNG exporter.
    /// SDR format-specific compression/channels remain in the image-sequence writer.
    pub fn save_png(
        &self,
        path: &std::path::Path,
        encoding: video_io::export::PngEncoding,
        white_nits: f32,
    ) -> Result<Option<video_io::export::HdrLevels>, String> {
        if !encoding.hdr() {
            return Err(
                "Use the SDR image writer for configurable PNG channels and compression".into(),
            );
        }
        let (light, kind, source_white) = self.hdr_light()?;
        if source_white != white_nits {
            return Err("Capture reference white changed during export".into());
        }
        let light: &[[f32; 4]] = bytemuck::try_cast_slice(light)
            .map_err(|e| format!("Invalid display-light pixel layout: {e}"))?;
        let scale = video_io::export::hdr_scale(kind, light, white_nits, encoding);
        video_io::export::write_png(
            path,
            self.layout.width(),
            self.layout.height(),
            light,
            Vec::new,
            encoding,
            scale,
            true,
        )
        .map_err(|e| e.to_string())
    }

    pub fn file(&self) -> Option<&PathBuf> {
        self.file.as_ref()
    }

    pub fn buffer(&self) -> Arc<PixelBuffer> {
        Arc::clone(&self.buffer)
    }

    pub fn layout(&self) -> PixelLayout {
        self.layout
    }

    pub fn pixel_format(&self) -> PixelFormat {
        self.layout.pixel_format()
    }

    pub fn resolution(&self) -> (usize, usize) {
        (self.layout.width(), self.layout.height())
    }

    pub fn crop_copy(
        &self,
        new_w: usize,
        new_h: usize,
        align: CropAlign,
    ) -> Result<Frame, FrameError> {
        if new_w == self.layout.width() && new_h == self.layout.height() {
            return Ok(self.clone());
        }

        let dst_layout = PixelLayout::rgba(
            new_w,
            new_h,
            self.pixel_format(),
            checked_element_count(new_w, new_h, RGBA_CHANNELS, 1)?,
        )?;
        let (src_x, src_y, dst_x, dst_y, copy_w, copy_h) = crop_window(
            self.layout.width(),
            self.layout.height(),
            new_w,
            new_h,
            align,
        );

        let mut cropped = match self.buffer.as_ref() {
            PixelBuffer::U8(src) => {
                let mut dst = vec![0u8; dst_layout.element_count()];
                copy_rows(
                    src,
                    &mut dst,
                    self.layout,
                    dst_layout,
                    src_x,
                    src_y,
                    dst_x,
                    dst_y,
                    copy_w,
                    copy_h,
                )?;
                Frame::rgba8(new_w, new_h, dst)
            }
            PixelBuffer::F16(src) => {
                let mut dst = vec![F16::ZERO; dst_layout.element_count()];
                copy_rows(
                    src,
                    &mut dst,
                    self.layout,
                    dst_layout,
                    src_x,
                    src_y,
                    dst_x,
                    dst_y,
                    copy_w,
                    copy_h,
                )?;
                Frame::rgba_f16(new_w, new_h, dst)
            }
            PixelBuffer::F32(src) => {
                let mut dst = vec![0.0f32; dst_layout.element_count()];
                copy_rows(
                    src,
                    &mut dst,
                    self.layout,
                    dst_layout,
                    src_x,
                    src_y,
                    dst_x,
                    dst_y,
                    copy_w,
                    copy_h,
                )?;
                Frame::rgba_f32(new_w, new_h, dst)
            }
        }?;
        cropped.display = self.display;
        Ok(cropped)
    }
}

pub trait FrameConversion {
    /// Map RGB through the selected curve; keep alpha linear until output quantization.
    fn tonemap(&self, mode: TonemapMode, output_format: PixelFormat) -> Result<Frame, String>;
    fn to_rgb24(&self) -> Result<Vec<u8>, String>;
    fn to_rgb48(&self) -> Result<Vec<u16>, String>;
}

impl FrameConversion for Frame {
    fn tonemap(&self, mode: TonemapMode, output_format: PixelFormat) -> Result<Frame, String> {
        let (width, height) = self.resolution();
        if self.display.is_some_and(|(kind, _)| kind.hdr()) {
            return Err("Absolute HDR display light requires HDR delivery".into());
        }
        if matches!(
            (self.buffer.as_ref(), output_format),
            (PixelBuffer::U8(_), PixelFormat::Rgba8)
        ) {
            return Ok(self.clone());
        }

        let source_is_u8 = matches!(self.buffer.as_ref(), PixelBuffer::U8(_));
        let values: Cow<'_, [f32]> = match self.buffer.as_ref() {
            PixelBuffer::U8(data) => {
                Cow::Owned(data.iter().map(|&value| f32::from(value) / 255.0).collect())
            }
            PixelBuffer::F16(data) => Cow::Owned(data.iter().map(|value| value.to_f32()).collect()),
            PixelBuffer::F32(data) => Cow::Borrowed(data.as_slice()),
        };
        if values.iter().any(|value| !value.is_finite()) {
            return Err("Tone mapping requires finite RGB and alpha".into());
        }

        let map_pixel = |[r, g, b, a]: [f32; RGBA_CHANNELS]| -> [f32; RGBA_CHANNELS] {
            let rgb = if source_is_u8 {
                [r, g, b]
            } else if self.display.is_some() {
                // A display transform has already been applied. Only encode its
                // linear Rec.709 light for an SDR image; never tone map it twice.
                [
                    video_io::color::oetf(r),
                    video_io::color::oetf(g),
                    video_io::color::oetf(b),
                ]
            } else {
                tonemap_rgb([r, g, b], mode)
            };
            [rgb[0], rgb[1], rgb[2], a.clamp(0.0, 1.0)]
        };

        let (pixels, remainder) = values.as_chunks::<RGBA_CHANNELS>();
        debug_assert!(remainder.is_empty());

        match output_format {
            PixelFormat::Rgba8 => {
                let mut out = Vec::with_capacity(values.len());
                for &pixel in pixels {
                    out.extend(map_pixel(pixel).map(float_to_u8));
                }
                Frame::rgba8(width, height, out)
            }
            PixelFormat::RgbaF16 => {
                let mut out = Vec::with_capacity(values.len());
                for &pixel in pixels {
                    out.extend(map_pixel(pixel).map(F16::from_f32));
                }
                Frame::rgba_f16(width, height, out)
            }
            PixelFormat::RgbaF32 => {
                let mut out = Vec::with_capacity(values.len());
                for &pixel in pixels {
                    out.extend(map_pixel(pixel));
                }
                Frame::rgba_f32(width, height, out)
            }
        }
        .map_err(|error| error.to_string())
    }

    fn to_rgb24(&self) -> Result<Vec<u8>, String> {
        let rgba = match self.buffer.as_ref() {
            PixelBuffer::U8(data) => data.clone(),
            PixelBuffer::F16(_) | PixelBuffer::F32(_) => self
                .tonemap(TonemapMode::default(), PixelFormat::Rgba8)?
                .to_rgba8_vec()?,
        };
        let capacity = self
            .layout
            .elements_for(3, size_of::<u8>())
            .map_err(|error| error.to_string())?;
        let mut rgb = Vec::with_capacity(capacity);
        for pixel in rgba.as_chunks::<RGBA_CHANNELS>().0 {
            rgb.extend_from_slice(&pixel[0..3]);
        }
        Ok(rgb)
    }

    fn to_rgb48(&self) -> Result<Vec<u16>, String> {
        let capacity = self
            .layout
            .elements_for(3, size_of::<u16>())
            .map_err(|error| error.to_string())?;
        let mut rgb = Vec::with_capacity(capacity);
        match self.buffer.as_ref() {
            PixelBuffer::U8(data) => {
                for pixel in data.as_chunks::<RGBA_CHANNELS>().0 {
                    rgb.push((pixel[0] as u16) * 257);
                    rgb.push((pixel[1] as u16) * 257);
                    rgb.push((pixel[2] as u16) * 257);
                }
            }
            PixelBuffer::F16(data) => {
                for pixel in data.as_chunks::<RGBA_CHANNELS>().0 {
                    rgb.push(float_to_u16(pixel[0].to_f32()));
                    rgb.push(float_to_u16(pixel[1].to_f32()));
                    rgb.push(float_to_u16(pixel[2].to_f32()));
                }
            }
            PixelBuffer::F32(data) => {
                for pixel in data.as_chunks::<RGBA_CHANNELS>().0 {
                    rgb.push(float_to_u16(pixel[0]));
                    rgb.push(float_to_u16(pixel[1]));
                    rgb.push(float_to_u16(pixel[2]));
                }
            }
        }
        Ok(rgb)
    }
}

impl Frame {
    fn to_rgba8_vec(&self) -> Result<Vec<u8>, String> {
        match self.buffer.as_ref() {
            PixelBuffer::U8(data) => Ok(data.clone()),
            PixelBuffer::F16(_) | PixelBuffer::F32(_) => self
                .tonemap(TonemapMode::default(), PixelFormat::Rgba8)
                .and_then(|frame| frame.to_rgba8_vec()),
        }
    }
}

fn crop_window(
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
    align: CropAlign,
) -> (usize, usize, usize, usize, usize, usize) {
    let copy_w = src_w.min(dst_w);
    let copy_h = src_h.min(dst_h);
    match align {
        CropAlign::LeftTop => (0, 0, 0, 0, copy_w, copy_h),
        CropAlign::Center => {
            let src_x = src_w.saturating_sub(copy_w) / 2;
            let src_y = src_h.saturating_sub(copy_h) / 2;
            let dst_x = dst_w.saturating_sub(copy_w) / 2;
            let dst_y = dst_h.saturating_sub(copy_h) / 2;
            (src_x, src_y, dst_x, dst_y, copy_w, copy_h)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn copy_rows<T: Copy>(
    src: &[T],
    dst: &mut [T],
    src_layout: PixelLayout,
    dst_layout: PixelLayout,
    src_x: usize,
    src_y: usize,
    dst_x: usize,
    dst_y: usize,
    copy_w: usize,
    copy_h: usize,
) -> Result<(), FrameError> {
    for row in 0..copy_h {
        let src_row = src_y
            .checked_add(row)
            .ok_or(FrameError::RegionOutOfBounds)?;
        let dst_row = dst_y
            .checked_add(row)
            .ok_or(FrameError::RegionOutOfBounds)?;
        let src_range = src_layout.row_range(src_row, src_x, copy_w)?;
        let dst_range = dst_layout.row_range(dst_row, dst_x, copy_w)?;
        let src_slice = src.get(src_range).ok_or(FrameError::RegionOutOfBounds)?;
        let dst_slice = dst
            .get_mut(dst_range)
            .ok_or(FrameError::RegionOutOfBounds)?;
        dst_slice.copy_from_slice(src_slice);
    }
    Ok(())
}

fn float_to_u8(value: f32) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

fn float_to_u16(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65535.0).round() as u16
}

#[inline]
fn rrt_and_odt_fit_f64(v: f64) -> f64 {
    let x = v.max(0.0);
    let a = x * (x + 0.0245786) - 0.000090537;
    let b = x * (0.983729 * x + 0.4329510) + 0.238081;
    (a / b).max(0.0)
}

fn tonemap_rgb(rgb: [f32; 3], mode: TonemapMode) -> [f32; 3] {
    match mode {
        TonemapMode::Clamp => rgb.map(|v| v.clamp(0.0, 1.0)),
        TonemapMode::Reinhard => rgb.map(|v| {
            let x = v.max(0.0);
            (x / (1.0 + x)).clamp(0.0, 1.0)
        }),
        TonemapMode::ACES => {
            let c0 = f64::from(rgb[0]).max(0.0);
            let c1 = f64::from(rgb[1]).max(0.0);
            let c2 = f64::from(rgb[2]).max(0.0);
            // ACESInputMat (sRGB / Rec.709 -> AP1 + RRT saturation)
            let v0 = 0.59719 * c0 + 0.35458 * c1 + 0.04823 * c2;
            let v1 = 0.07600 * c0 + 0.90834 * c1 + 0.01566 * c2;
            let v2 = 0.02840 * c0 + 0.13383 * c1 + 0.83777 * c2;
            // RRTAndODTFit
            let u0 = rrt_and_odt_fit_f64(v0);
            let u1 = rrt_and_odt_fit_f64(v1);
            let u2 = rrt_and_odt_fit_f64(v2);
            // ACESOutputMat (AP1 + ODT saturation -> sRGB / Rec.709)
            let out0 = 1.60475 * u0 - 0.53108 * u1 - 0.07367 * u2;
            let out1 = -0.10208 * u0 + 1.10813 * u1 - 0.00605 * u2;
            let out2 = -0.00327 * u0 - 0.07276 * u1 + 1.07602 * u2;
            [
                out0.clamp(0.0, 1.0) as f32,
                out1.clamp(0.0, 1.0) as f32,
                out2.clamp(0.0, 1.0) as f32,
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_light_sdr_conversion_applies_transfer_once_and_preserves_alpha() {
        let frame = Frame::display_light(
            1,
            1,
            vec![0.18, 0.003, 4.0, 0.5],
            video_io::export::DisplayLight::Relative,
            203.0,
        )
        .unwrap();
        let coded = frame
            .tonemap(TonemapMode::ACES, PixelFormat::RgbaF32)
            .unwrap();
        let PixelBuffer::F32(values) = coded.buffer.as_ref() else {
            panic!("Float output expected")
        };
        assert_eq!(values[0], video_io::color::oetf(0.18));
        assert_eq!(values[1], video_io::color::oetf(0.003));
        assert_eq!(values[2], video_io::color::oetf(4.0));
        assert_eq!(values[3], 0.5);
        let bytes = frame
            .tonemap(TonemapMode::ACES, PixelFormat::Rgba8)
            .unwrap();
        let PixelBuffer::U8(values) = bytes.buffer.as_ref() else {
            panic!("U8 output expected")
        };
        assert_eq!(values.as_slice(), &[118, 10, 255, 128]);
    }

    #[test]
    fn tonemap_rejects_nonfinite_input_and_retains_finite_highlights() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for channel in 0..4 {
                let mut pixels = vec![1.0; 4];
                pixels[channel] = value;
                let frame = Frame::rgba_f32(1, 1, pixels).unwrap();
                assert!(
                    frame
                        .tonemap(TonemapMode::ACES, PixelFormat::Rgba8)
                        .is_err()
                );
                assert!(
                    frame
                        .tonemap(TonemapMode::Clamp, PixelFormat::RgbaF32)
                        .is_err()
                );
            }
        }
        let frame = Frame::rgba_f32(1, 1, vec![f32::MAX, f32::MAX, f32::MAX, 0.5]).unwrap();
        let mapped = frame
            .tonemap(TonemapMode::ACES, PixelFormat::RgbaF32)
            .unwrap();
        let buffer = mapped.buffer();
        let PixelBuffer::F32(pixels) = buffer.as_ref() else {
            panic!("float output");
        };
        assert_eq!(pixels, &[1.0, 1.0, 1.0, 0.5]);
    }

    #[test]
    fn aces_tonemap_desaturates_bright_saturated_highlights_toward_white() {
        let frame = Frame::rgba_f32(1, 1, vec![0.0, 45.0, 50.0, 1.0]).unwrap();
        let mapped = frame
            .tonemap(TonemapMode::ACES, PixelFormat::RgbaF32)
            .unwrap();
        let buffer = mapped.buffer();
        let PixelBuffer::F32(pixels) = buffer.as_ref() else {
            panic!("float output");
        };
        let [r, g, b, a] = [pixels[0], pixels[1], pixels[2], pixels[3]];
        assert!(
            r > 0.90 && g > 0.98 && b > 0.98,
            "expected Path-to-White desaturation on overbright cyan, got [{r}, {g}, {b}]"
        );
        assert_eq!(a, 1.0);
    }

    #[test]
    fn display_capture_keeps_highlights_alpha_and_colour_contract_after_crop() {
        let capture = Frame::display_light(
            2,
            1,
            vec![4.0, 2.0, 1.0, 0.5, 0.0, 0.0, 0.0, 1.0],
            video_io::export::DisplayLight::Relative,
            203.0,
        )
        .unwrap();
        let cropped = capture.crop_copy(1, 1, CropAlign::LeftTop).unwrap();
        let (light, kind, white) = cropped.hdr_light().unwrap();
        assert_eq!(kind, video_io::export::DisplayLight::Relative);
        assert_eq!(light, &[4.0, 2.0, 1.0, 0.5]);
        assert_eq!(white, 203.0);
        let scene = Frame::rgba_f32(1, 1, vec![4.0, 2.0, 1.0, 0.5]).unwrap();
        assert!(
            scene.hdr_light().is_err(),
            "scene radiance cannot acquire display metadata"
        );
    }

    #[test]
    fn display_capture_rejects_invalid_light_or_reference_white() {
        for white in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(
                Frame::display_light(
                    1,
                    1,
                    vec![1.0; 4],
                    video_io::export::DisplayLight::Relative,
                    white
                )
                .is_err()
            );
        }
        assert!(
            Frame::display_light(
                1,
                1,
                vec![f32::INFINITY; 4],
                video_io::export::DisplayLight::Relative,
                203.0
            )
            .is_err()
        );
        assert!(
            Frame::display_light(
                2,
                1,
                vec![1.0; 4],
                video_io::export::DisplayLight::Relative,
                203.0
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_malformed_buffer() {
        let error = Frame::rgba8(2, 2, vec![0; 15]).unwrap_err();
        assert!(matches!(
            error,
            FrameError::BufferLength {
                expected: 16,
                actual: 15,
                ..
            }
        ));
    }

    #[test]
    fn rejects_empty_dimensions() {
        assert!(matches!(
            Frame::rgba8(0, 1, Vec::new()),
            Err(FrameError::EmptyDimensions { .. })
        ));
    }

    #[test]
    fn crop_preserves_validated_layout() {
        let frame = Frame::rgba8(2, 2, (0..16).collect()).unwrap();
        let cropped = frame.crop_copy(1, 1, CropAlign::LeftTop).unwrap();
        assert_eq!(cropped.resolution(), (1, 1));
        assert_eq!(cropped.layout().element_count(), 4);
        match cropped.buffer().as_ref() {
            PixelBuffer::U8(data) => assert_eq!(data, &[0, 1, 2, 3]),
            _ => panic!("unexpected pixel format"),
        }
    }
}
