use std::path::PathBuf;
use std::sync::Arc;

use half::f16 as F16;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum PixelBuffer {
    U8(Vec<u8>),
    F16(Vec<F16>),
    F32(Vec<f32>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8,
    RgbaF16,
    RgbaF32,
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
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
    file: Option<PathBuf>,
}

impl Frame {
    pub fn rgba8(width: usize, height: usize, data: Vec<u8>) -> Self {
        Self::new(width, height, PixelBuffer::U8(data), None)
    }

    pub fn rgba_f16(width: usize, height: usize, data: Vec<F16>) -> Self {
        Self::new(width, height, PixelBuffer::F16(data), None)
    }

    pub fn rgba_f32(width: usize, height: usize, data: Vec<f32>) -> Self {
        Self::new(width, height, PixelBuffer::F32(data), None)
    }

    pub fn new(width: usize, height: usize, buffer: PixelBuffer, file: Option<PathBuf>) -> Self {
        let pixel_format = match &buffer {
            PixelBuffer::U8(_) => PixelFormat::Rgba8,
            PixelBuffer::F16(_) => PixelFormat::RgbaF16,
            PixelBuffer::F32(_) => PixelFormat::RgbaF32,
        };
        Self {
            buffer: Arc::new(buffer),
            pixel_format,
            width,
            height,
            file,
        }
    }

    pub fn file(&self) -> Option<&PathBuf> {
        self.file.as_ref()
    }

    pub fn buffer(&self) -> Arc<PixelBuffer> {
        Arc::clone(&self.buffer)
    }

    pub fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    pub fn resolution(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub fn crop_copy(&self, new_w: usize, new_h: usize, align: CropAlign) -> Frame {
        if new_w == self.width && new_h == self.height {
            return self.clone();
        }

        let (src_x, src_y, dst_x, dst_y, copy_w, copy_h) =
            crop_window(self.width, self.height, new_w, new_h, align);

        match self.buffer.as_ref() {
            PixelBuffer::U8(src) => {
                let mut dst = vec![0u8; new_w * new_h * 4];
                copy_rows(
                    src, &mut dst, self.width, new_w, src_x, src_y, dst_x, dst_y, copy_w, copy_h, 4,
                );
                Frame::rgba8(new_w, new_h, dst)
            }
            PixelBuffer::F16(src) => {
                let mut dst = vec![F16::ZERO; new_w * new_h * 4];
                copy_rows(
                    src, &mut dst, self.width, new_w, src_x, src_y, dst_x, dst_y, copy_w, copy_h, 4,
                );
                Frame::rgba_f16(new_w, new_h, dst)
            }
            PixelBuffer::F32(src) => {
                let mut dst = vec![0.0f32; new_w * new_h * 4];
                copy_rows(
                    src, &mut dst, self.width, new_w, src_x, src_y, dst_x, dst_y, copy_w, copy_h, 4,
                );
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
        let mut out = Vec::with_capacity(self.width * self.height * 4);
        match self.buffer.as_ref() {
            PixelBuffer::U8(data) => {
                return Ok(Frame::rgba8(self.width, self.height, data.clone()));
            }
            PixelBuffer::F16(data) => {
                for v in data {
                    out.push(float_to_u8(v.to_f32(), mode));
                }
            }
            PixelBuffer::F32(data) => {
                for &v in data {
                    out.push(float_to_u8(v, mode));
                }
            }
        }
        Ok(Frame::rgba8(self.width, self.height, out))
    }

    fn to_rgb24(&self) -> Result<Vec<u8>, String> {
        let rgba = match self.buffer.as_ref() {
            PixelBuffer::U8(data) => data.clone(),
            PixelBuffer::F16(_) | PixelBuffer::F32(_) => {
                self.tonemap(TonemapMode::default())?.to_rgba8_vec()?
            }
        };
        let mut rgb = Vec::with_capacity(self.width * self.height * 3);
        for px in rgba.chunks_exact(4) {
            rgb.extend_from_slice(&px[0..3]);
        }
        Ok(rgb)
    }

    fn to_rgb48(&self) -> Result<Vec<u16>, String> {
        let mut rgb = Vec::with_capacity(self.width * self.height * 3);
        match self.buffer.as_ref() {
            PixelBuffer::U8(data) => {
                for px in data.chunks_exact(4) {
                    rgb.push((px[0] as u16) * 257);
                    rgb.push((px[1] as u16) * 257);
                    rgb.push((px[2] as u16) * 257);
                }
            }
            PixelBuffer::F16(data) => {
                for px in data.chunks_exact(4) {
                    rgb.push(float_to_u16(px[0].to_f32()));
                    rgb.push(float_to_u16(px[1].to_f32()));
                    rgb.push(float_to_u16(px[2].to_f32()));
                }
            }
            PixelBuffer::F32(data) => {
                for px in data.chunks_exact(4) {
                    rgb.push(float_to_u16(px[0]));
                    rgb.push(float_to_u16(px[1]));
                    rgb.push(float_to_u16(px[2]));
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
                .and_then(|f| f.to_rgba8_vec()),
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
    src_w: usize,
    dst_w: usize,
    src_x: usize,
    src_y: usize,
    dst_x: usize,
    dst_y: usize,
    copy_w: usize,
    copy_h: usize,
    channels: usize,
) {
    let row_len = copy_w * channels;
    for row in 0..copy_h {
        let src_start = ((src_y + row) * src_w + src_x) * channels;
        let dst_start = ((dst_y + row) * dst_w + dst_x) * channels;
        dst[dst_start..dst_start + row_len].copy_from_slice(&src[src_start..src_start + row_len]);
    }
}

fn float_to_u8(v: f32, mode: TonemapMode) -> u8 {
    (tonemap_value(v, mode) * 255.0).round().clamp(0.0, 255.0) as u8
}

fn float_to_u16(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 65535.0).round() as u16
}

fn tonemap_value(v: f32, mode: TonemapMode) -> f32 {
    let x = v.max(0.0);
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
