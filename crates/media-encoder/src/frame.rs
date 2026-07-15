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
        })
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

        match self.buffer.as_ref() {
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
        }
    }
}

pub trait FrameConversion {
    fn tonemap(&self, mode: TonemapMode) -> Result<Frame, String>;
    fn to_rgb24(&self) -> Result<Vec<u8>, String>;
    fn to_rgb48(&self) -> Result<Vec<u16>, String>;
}

impl FrameConversion for Frame {
    fn tonemap(&self, mode: TonemapMode) -> Result<Frame, String> {
        let mut out = Vec::with_capacity(self.layout.element_count());
        match self.buffer.as_ref() {
            PixelBuffer::U8(data) => {
                return Frame::rgba8(self.layout.width(), self.layout.height(), data.clone())
                    .map_err(|error| error.to_string());
            }
            PixelBuffer::F16(data) => {
                for value in data {
                    out.push(float_to_u8(value.to_f32(), mode));
                }
            }
            PixelBuffer::F32(data) => {
                for &value in data {
                    out.push(float_to_u8(value, mode));
                }
            }
        }
        Frame::rgba8(self.layout.width(), self.layout.height(), out)
            .map_err(|error| error.to_string())
    }

    fn to_rgb24(&self) -> Result<Vec<u8>, String> {
        let rgba = match self.buffer.as_ref() {
            PixelBuffer::U8(data) => data.clone(),
            PixelBuffer::F16(_) | PixelBuffer::F32(_) => {
                self.tonemap(TonemapMode::default())?.to_rgba8_vec()?
            }
        };
        let capacity = self
            .layout
            .elements_for(3, size_of::<u8>())
            .map_err(|error| error.to_string())?;
        let mut rgb = Vec::with_capacity(capacity);
        for pixel in rgba.chunks_exact(RGBA_CHANNELS) {
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
                for pixel in data.chunks_exact(RGBA_CHANNELS) {
                    rgb.push((pixel[0] as u16) * 257);
                    rgb.push((pixel[1] as u16) * 257);
                    rgb.push((pixel[2] as u16) * 257);
                }
            }
            PixelBuffer::F16(data) => {
                for pixel in data.chunks_exact(RGBA_CHANNELS) {
                    rgb.push(float_to_u16(pixel[0].to_f32()));
                    rgb.push(float_to_u16(pixel[1].to_f32()));
                    rgb.push(float_to_u16(pixel[2].to_f32()));
                }
            }
            PixelBuffer::F32(data) => {
                for pixel in data.chunks_exact(RGBA_CHANNELS) {
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
                .tonemap(TonemapMode::default())
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

fn float_to_u8(value: f32, mode: TonemapMode) -> u8 {
    (tonemap_value(value, mode) * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn float_to_u16(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65535.0).round() as u16
}

fn tonemap_value(value: f32, mode: TonemapMode) -> f32 {
    let x = value.max(0.0);
    match mode {
        TonemapMode::Clamp => x.clamp(0.0, 1.0),
        TonemapMode::Reinhard => (x / (1.0 + x)).clamp(0.0, 1.0),
        TonemapMode::ACES => {
            let a = 2.51;
            let b = 0.03;
            let c = 2.43;
            let d = 0.59;
            let e = 0.14;
            ((x * (a * x + b)) / (x * (c * x + d) + e)).clamp(0.0, 1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
