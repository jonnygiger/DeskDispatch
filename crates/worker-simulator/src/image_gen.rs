use image::{ImageBuffer, Rgb, RgbImage, codecs::png::PngEncoder};
use rand::Rng;
use std::io::Cursor;

/// Resolution options for random screenshot generation.
const COMMON_RESOLUTIONS: &[(u32, u32)] = &[
    (800, 600),
    (1024, 768),
    (1280, 720),
    (1366, 768),
    (1920, 1080),
];

/// Generates a random PNG screenshot with random colors and random geometries.
pub fn generate_random_screenshot() -> Vec<u8> {
    let mut rng = rand::thread_rng();
    let (width, height) = COMMON_RESOLUTIONS[rng.gen_range(0..COMMON_RESOLUTIONS.len())];
    generate_random_screenshot_with_dimensions(width, height)
}

/// Generates a random PNG screenshot with specific dimensions and random geometries.
pub fn generate_random_screenshot_with_dimensions(width: u32, height: u32) -> Vec<u8> {
    let mut rng = rand::thread_rng();
    let mut img: RgbImage = ImageBuffer::new(width, height);

    // 1. Fill canvas with a random background color or subtle gradient
    let bg_r = rng.gen_range(20..240);
    let bg_g = rng.gen_range(20..240);
    let bg_b = rng.gen_range(20..240);
    let bg_color = Rgb([bg_r, bg_g, bg_b]);

    let use_gradient: bool = rng.r#gen();
    if use_gradient {
        let bg_r2 = rng.gen_range(20..240);
        let bg_g2 = rng.gen_range(20..240);
        let bg_b2 = rng.gen_range(20..240);
        for y in 0..height {
            let factor = y as f32 / height.max(1) as f32;
            let r = (bg_r as f32 * (1.0 - factor) + bg_r2 as f32 * factor) as u8;
            let g = (bg_g as f32 * (1.0 - factor) + bg_g2 as f32 * factor) as u8;
            let b = (bg_b as f32 * (1.0 - factor) + bg_b2 as f32 * factor) as u8;
            let row_color = Rgb([r, g, b]);
            for x in 0..width {
                img.put_pixel(x, y, row_color);
            }
        }
    } else {
        for pixel in img.pixels_mut() {
            *pixel = bg_color;
        }
    }

    // 2. Draw random grid / UI layout lines
    if rng.gen_bool(0.7) {
        let line_color = Rgb([
            rng.gen_range(0..=255),
            rng.gen_range(0..=255),
            rng.gen_range(0..=255),
        ]);
        let grid_spacing = rng.gen_range(40..150);
        for x in (0..width).step_by(grid_spacing) {
            for y in 0..height {
                img.put_pixel(x, y, line_color);
            }
        }
        for y in (0..height).step_by(grid_spacing) {
            for x in 0..width {
                img.put_pixel(x, y, line_color);
            }
        }
    }

    // 3. Draw random rectangles / window shapes
    let rect_count = rng.gen_range(3..12);
    for _ in 0..rect_count {
        let rect_w = rng.gen_range(30..(width / 2).max(40));
        let rect_h = rng.gen_range(30..(height / 2).max(40));
        let rect_x = rng.gen_range(0..(width - rect_w).max(1));
        let rect_y = rng.gen_range(0..(height - rect_h).max(1));

        let rect_color = Rgb([
            rng.gen_range(0..=255),
            rng.gen_range(0..=255),
            rng.gen_range(0..=255),
        ]);
        let filled: bool = rng.r#gen();

        if filled {
            draw_filled_rect(&mut img, rect_x, rect_y, rect_w, rect_h, rect_color);
        } else {
            let thickness = rng.gen_range(1..5);
            draw_outline_rect(
                &mut img, rect_x, rect_y, rect_w, rect_h, thickness, rect_color,
            );
        }
    }

    // 4. Draw random circles (e.g. status lights, icons, buttons)
    let circle_count = rng.gen_range(2..8);
    for _ in 0..circle_count {
        let radius = rng.gen_range(10..60);
        let cx = rng.gen_range(radius..(width - radius).max(radius + 1));
        let cy = rng.gen_range(radius..(height - radius).max(radius + 1));
        let circle_color = Rgb([
            rng.gen_range(0..=255),
            rng.gen_range(0..=255),
            rng.gen_range(0..=255),
        ]);
        draw_filled_circle(&mut img, cx, cy, radius, circle_color);
    }

    // 5. Draw random crosshairs / target points
    let crosshair_count = rng.gen_range(1..5);
    for _ in 0..crosshair_count {
        let cx = rng.gen_range(10..(width - 10));
        let cy = rng.gen_range(10..(height - 10));
        let size = rng.gen_range(15..40);
        let ch_color = Rgb([
            rng.gen_range(0..=255),
            rng.gen_range(0..=255),
            rng.gen_range(0..=255),
        ]);

        draw_line_horizontal(&mut img, cx.saturating_sub(size), cx + size, cy, ch_color);
        draw_line_vertical(&mut img, cx, cy.saturating_sub(size), cy + size, ch_color);
    }

    // Encode to PNG format
    let mut bytes: Vec<u8> = Vec::new();
    let encoder = PngEncoder::new(Cursor::new(&mut bytes));
    if let Err(e) = img.write_with_encoder(encoder) {
        tracing::error!("Failed to encode generated image to PNG: {}", e);
    }

    bytes
}

fn draw_filled_rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, color: Rgb<u8>) {
    let img_w = img.width();
    let img_h = img.height();

    for cur_y in y..(y + h).min(img_h) {
        for cur_x in x..(x + w).min(img_w) {
            img.put_pixel(cur_x, cur_y, color);
        }
    }
}

fn draw_outline_rect(
    img: &mut RgbImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    thickness: u32,
    color: Rgb<u8>,
) {
    draw_filled_rect(img, x, y, w, thickness, color);
    draw_filled_rect(img, x, y + h.saturating_sub(thickness), w, thickness, color);
    draw_filled_rect(img, x, y, thickness, h, color);
    draw_filled_rect(img, x + w.saturating_sub(thickness), y, thickness, h, color);
}

fn draw_filled_circle(img: &mut RgbImage, cx: u32, cy: u32, radius: u32, color: Rgb<u8>) {
    let img_w = img.width();
    let img_h = img.height();
    let r_i32 = radius as i32;
    let r_sq = r_i32 * r_i32;

    let min_x = (cx as i32 - r_i32).max(0) as u32;
    let max_x = (cx as i32 + r_i32).min(img_w as i32 - 1) as u32;
    let min_y = (cy as i32 - r_i32).max(0) as u32;
    let max_y = (cy as i32 + r_i32).min(img_h as i32 - 1) as u32;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let dx = x as i32 - cx as i32;
            let dy = y as i32 - cy as i32;
            if dx * dx + dy * dy <= r_sq {
                img.put_pixel(x, y, color);
            }
        }
    }
}

fn draw_line_horizontal(img: &mut RgbImage, x1: u32, x2: u32, y: u32, color: Rgb<u8>) {
    if y >= img.height() {
        return;
    }
    let img_w = img.width();
    for x in x1.min(img_w)..=x2.min(img_w - 1) {
        img.put_pixel(x, y, color);
    }
}

fn draw_line_vertical(img: &mut RgbImage, x: u32, y1: u32, y2: u32, color: Rgb<u8>) {
    if x >= img.width() {
        return;
    }
    let img_h = img.height();
    for y in y1.min(img_h)..=y2.min(img_h - 1) {
        img.put_pixel(x, y, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_random_screenshot_png_header() {
        let bytes = generate_random_screenshot();
        assert!(!bytes.is_empty(), "Screenshot bytes should not be empty");

        // Magic numbers for PNG header
        let png_header = &[137, 80, 78, 71, 13, 10, 26, 10];
        assert_eq!(
            &bytes[..8],
            png_header,
            "Generated image should have valid PNG magic header"
        );
    }

    #[test]
    fn test_generate_random_screenshot_with_custom_dimensions() {
        let (width, height) = (640, 480);
        let bytes = generate_random_screenshot_with_dimensions(width, height);
        assert!(!bytes.is_empty());

        let loaded = image::load_from_memory(&bytes).expect("Should load generated PNG");
        assert_eq!(loaded.width(), width);
        assert_eq!(loaded.height(), height);
    }

    #[test]
    fn test_draw_geometries_boundary_safety() {
        let mut img: RgbImage = ImageBuffer::new(100, 100);
        let color = Rgb([255, 0, 0]);

        draw_filled_rect(&mut img, 80, 80, 50, 50, color);
        draw_outline_rect(&mut img, 50, 50, 100, 100, 10, color);
        draw_filled_circle(&mut img, 10, 10, 30, color);
        draw_line_horizontal(&mut img, 0, 150, 99, color);
        draw_line_vertical(&mut img, 99, 0, 150, color);

        let mut bytes = Vec::new();
        let encoder = PngEncoder::new(Cursor::new(&mut bytes));
        img.write_with_encoder(encoder).expect("PNG encode success");
        assert!(!bytes.is_empty());
    }
}
