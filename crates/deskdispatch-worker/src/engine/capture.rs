use std::io::Cursor;
use std::process::Command;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct CapturedImage {
    pub width: u32,
    pub height: u32,
    /// RGBA pixels buffer
    pub data: Vec<u8>,
}

impl CapturedImage {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
        }
    }

    pub fn crop(&self, crop_x: u32, crop_y: u32, crop_w: u32, crop_h: u32) -> Self {
        let x1 = crop_x.min(self.width);
        let y1 = crop_y.min(self.height);
        let x2 = (crop_x + crop_w).min(self.width);
        let y2 = (crop_y + crop_h).min(self.height);

        let w = x2.saturating_sub(x1).max(1);
        let h = y2.saturating_sub(y1).max(1);

        let stride = (self.width * 4) as usize;
        let row_bytes = ((x2 - x1) * 4) as usize;
        let mut data = Vec::with_capacity((w * h * 4) as usize);

        for y in y1..y2 {
            let start = (y as usize) * stride + (x1 as usize) * 4;
            let end = start + row_bytes;
            if end <= self.data.len() {
                data.extend_from_slice(&self.data[start..end]);
            }
        }
        if data.is_empty() {
            data = vec![0u8; (w * h * 4) as usize];
        }
        Self::new(w, h, data)
    }

    pub fn get_pixel_rgb(&self, x: u32, y: u32) -> Option<(u8, u8, u8)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let idx = ((y * self.width + x) * 4) as usize;
        if idx + 2 < self.data.len() {
            Some((self.data[idx], self.data[idx + 1], self.data[idx + 2]))
        } else {
            None
        }
    }

    pub fn encode_png(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);

        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("Failed to write PNG header: {}", e))?;

        writer
            .write_image_data(&self.data)
            .map_err(|e| format!("Failed to encode PNG image data: {}", e))?;

        drop(writer);
        Ok(out)
    }

    pub fn decode_png(png_bytes: &[u8]) -> Result<Self, String> {
        let cursor = Cursor::new(png_bytes);
        let mut decoder = png::Decoder::new(cursor);
        // Expand palette and low-bit-depth images to 8-bit RGB(A) and strip
        // 16-bit samples so every PNG flavour can be matched against the screen.
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .map_err(|e| format!("Failed to read PNG header: {}", e))?;

        let info = reader.info();
        let width = info.width;
        let height = info.height;

        let buf_size = reader
            .output_buffer_size()
            .unwrap_or((width * height * 4) as usize);
        let mut buf = vec![0u8; buf_size];
        let output_info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("Failed to decode PNG frame: {}", e))?;

        let bytes = &buf[..output_info.buffer_size()];
        let pixel_count = (width * height) as usize;

        let data = match output_info.color_type {
            png::ColorType::Rgb => {
                let (chunks, _) = bytes.as_chunks::<3>();
                let mut rgba = vec![255u8; pixel_count * 4];
                let (dst_chunks, _) = rgba.as_chunks_mut::<4>();
                for (chunk, dst) in chunks.iter().zip(dst_chunks.iter_mut()) {
                    dst[0..3].copy_from_slice(chunk);
                }
                rgba
            }
            png::ColorType::Rgba => {
                if bytes.len() == pixel_count * 4 {
                    bytes.to_vec()
                } else {
                    let mut rgba = vec![255u8; pixel_count * 4];
                    let copy_len = bytes.len().min(rgba.len());
                    rgba[..copy_len].copy_from_slice(&bytes[..copy_len]);
                    rgba
                }
            }
            png::ColorType::Grayscale => {
                let mut rgba = vec![255u8; pixel_count * 4];
                let (dst_chunks, _) = rgba.as_chunks_mut::<4>();
                for (&g, dst) in bytes.iter().zip(dst_chunks.iter_mut()) {
                    dst[0] = g;
                    dst[1] = g;
                    dst[2] = g;
                }
                rgba
            }
            png::ColorType::GrayscaleAlpha => {
                let (chunks, _) = bytes.as_chunks::<2>();
                let mut rgba = vec![255u8; pixel_count * 4];
                let (dst_chunks, _) = rgba.as_chunks_mut::<4>();
                for (chunk, dst) in chunks.iter().zip(dst_chunks.iter_mut()) {
                    let g = chunk[0];
                    dst[0] = g;
                    dst[1] = g;
                    dst[2] = g;
                    dst[3] = chunk[1];
                }
                rgba
            }
            other => return Err(format!("Unsupported PNG color type {:?}", other)),
        };

        Ok(Self::new(width, height, data))
    }
}

pub fn capture_screen(display: &str, target_width: u32, target_height: u32) -> CapturedImage {
    // Attempt system xwd / import / x11 screenshot on Debian Linux
    if let Ok(shot) = capture_x11_screen(display, target_width, target_height) {
        return shot;
    }

    debug!("Falling back to synthetic display buffer generation");
    generate_synthetic_display(target_width, target_height)
}

fn capture_x11_screen(
    display: &str,
    _target_width: u32,
    _target_height: u32,
) -> Result<CapturedImage, String> {
    let tmp_path = format!("/tmp/deskdispatch_screen_{}.png", std::process::id());

    // Try scrot first
    let scrot_res = Command::new("scrot")
        .env("DISPLAY", display)
        .arg(&tmp_path)
        .status();

    let success = match scrot_res {
        Ok(s) if s.success() => true,
        _ => {
            // Try import (ImageMagick)
            let import_res = Command::new("import")
                .env("DISPLAY", display)
                .arg("-window")
                .arg("root")
                .arg(&tmp_path)
                .status();
            match import_res {
                Ok(s) => s.success(),
                Err(_) => false,
            }
        }
    };

    if !success {
        return Err("No external screenshot tool succeeded".to_string());
    }

    let bytes =
        std::fs::read(&tmp_path).map_err(|e| format!("Failed to read screenshot file: {}", e))?;
    let _ = std::fs::remove_file(&tmp_path);

    CapturedImage::decode_png(&bytes)
}

pub fn generate_synthetic_display(width: u32, height: u32) -> CapturedImage {
    let mut data = vec![255u8; (width * height * 4) as usize];
    let row_len = (width * 4) as usize;
    for (y, row) in data.chunks_exact_mut(row_len).enumerate() {
        let y_mod = (y % 256) as u8;
        let (pixels, _) = row.as_chunks_mut::<4>();
        for (x, px) in pixels.iter_mut().enumerate() {
            let x_mod = (x % 256) as u8;
            px[0] = x_mod;
            px[1] = y_mod;
            px[2] = ((x + y) % 256) as u8;
            // px[3] is pre-initialized to 255
        }
    }
    CapturedImage::new(width, height, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_custom_png(
        width: u32,
        height: u32,
        color_type: png::ColorType,
        raw_data: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color_type);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("Header write succeeds");
        writer
            .write_image_data(raw_data)
            .expect("Image data write succeeds");
        drop(writer);
        out
    }

    #[test]
    fn test_captured_image_crop() {
        let mut data = vec![0u8; 10 * 10 * 4];
        // Set red pixel at (2, 3)
        let idx = (3 * 10 + 2) * 4;
        data[idx] = 255;
        data[idx + 3] = 255;

        let img = CapturedImage::new(10, 10, data);
        let cropped = img.crop(1, 2, 4, 4);

        assert_eq!(cropped.width, 4);
        assert_eq!(cropped.height, 4);
        // Pixel (2, 3) relative to crop (1, 2) is (1, 1)
        assert_eq!(cropped.get_pixel_rgb(1, 1), Some((255, 0, 0)));
    }

    #[test]
    fn test_captured_image_crop_out_of_bounds() {
        let img = generate_synthetic_display(5, 5);
        let cropped = img.crop(10, 10, 5, 5);
        assert_eq!(cropped.width, 1);
        assert_eq!(cropped.height, 1);
        assert_eq!(cropped.data.len(), 4);
    }

    #[test]
    fn test_encode_decode_png_roundtrip() {
        let synth = generate_synthetic_display(8, 8);
        let encoded = synth.encode_png().expect("Encoding PNG succeeds");
        let decoded = CapturedImage::decode_png(&encoded).expect("Decoding PNG succeeds");

        assert_eq!(decoded.width, 8);
        assert_eq!(decoded.height, 8);
        assert_eq!(synth.get_pixel_rgb(3, 4), decoded.get_pixel_rgb(3, 4));
    }

    #[test]
    fn test_decode_png_rgb() {
        let raw_rgb = vec![100u8, 150u8, 200u8, 50u8, 75u8, 125u8];
        let png_bytes = encode_custom_png(2, 1, png::ColorType::Rgb, &raw_rgb);
        let decoded = CapturedImage::decode_png(&png_bytes).expect("RGB decode succeeds");

        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 1);
        assert_eq!(decoded.get_pixel_rgb(0, 0), Some((100, 150, 200)));
        assert_eq!(decoded.get_pixel_rgb(1, 0), Some((50, 75, 125)));
        assert_eq!(decoded.data[3], 255); // Alpha channel
    }

    #[test]
    fn test_decode_png_grayscale() {
        let raw_gray = vec![128u8, 200u8];
        let png_bytes = encode_custom_png(2, 1, png::ColorType::Grayscale, &raw_gray);
        let decoded = CapturedImage::decode_png(&png_bytes).expect("Grayscale decode succeeds");

        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 1);
        assert_eq!(decoded.get_pixel_rgb(0, 0), Some((128, 128, 128)));
        assert_eq!(decoded.get_pixel_rgb(1, 0), Some((200, 200, 200)));
    }

    #[test]
    fn test_decode_png_grayscale_alpha() {
        let raw_ga = vec![100u8, 255u8, 200u8, 128u8];
        let png_bytes = encode_custom_png(2, 1, png::ColorType::GrayscaleAlpha, &raw_ga);
        let decoded =
            CapturedImage::decode_png(&png_bytes).expect("GrayscaleAlpha decode succeeds");

        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 1);
        assert_eq!(decoded.get_pixel_rgb(0, 0), Some((100, 100, 100)));
        assert_eq!(decoded.get_pixel_rgb(1, 0), Some((200, 200, 200)));
        assert_eq!(decoded.data[7], 128); // Alpha
    }

    #[test]
    fn test_get_pixel_rgb_bounds() {
        let img = generate_synthetic_display(4, 4);
        assert!(img.get_pixel_rgb(0, 0).is_some());
        assert!(img.get_pixel_rgb(3, 3).is_some());
        assert!(img.get_pixel_rgb(4, 0).is_none());
        assert!(img.get_pixel_rgb(0, 4).is_none());
    }
}
