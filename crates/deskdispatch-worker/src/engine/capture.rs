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
        let decoder = png::Decoder::new(cursor);
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

        let data = match output_info.color_type {
            png::ColorType::Rgb => {
                let mut rgba = Vec::with_capacity((width * height * 4) as usize);
                for chunk in buf[..output_info.buffer_size()].chunks(3) {
                    if chunk.len() == 3 {
                        rgba.push(chunk[0]);
                        rgba.push(chunk[1]);
                        rgba.push(chunk[2]);
                        rgba.push(255);
                    }
                }
                rgba
            }
            png::ColorType::Rgba => buf[..output_info.buffer_size()].to_vec(),
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
    let mut data = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            data[idx] = (x % 256) as u8;
            data[idx + 1] = (y % 256) as u8;
            data[idx + 2] = ((x + y) % 256) as u8;
            data[idx + 3] = 255;
        }
    }
    CapturedImage::new(width, height, data)
}
