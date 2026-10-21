use super::capture::CapturedImage;

#[derive(Debug, Clone, PartialEq)]
pub struct BitmapMatchResult {
    pub found: bool,
    pub x: u32,
    pub y: u32,
    pub score: f32,
}

#[derive(Debug, Clone, Copy)]
struct SamplePixel {
    rel_byte_offset: usize,
    r: u8,
    g: u8,
    b: u8,
}

/// Search for template bitmap in screen within optional bounding box.
///
/// Performance-tuned: Pre-calculates template sample pixel byte offsets relative to
/// candidate top-left positions, indexes `screen.data` directly without per-pixel bounds checks or
/// Option wrapping, and uses early pruning when remaining samples cannot exceed `best_score`.
pub fn search_bitmap(
    screen: &CapturedImage,
    template: &CapturedImage,
    search_x: Option<i32>,
    search_y: Option<i32>,
    search_w: Option<i32>,
    search_h: Option<i32>,
    match_threshold: f32,
) -> BitmapMatchResult {
    if template.width == 0 || template.height == 0 || screen.width == 0 || screen.height == 0 {
        return BitmapMatchResult {
            found: false,
            x: 0,
            y: 0,
            score: 0.0,
        };
    }

    if template.width > screen.width || template.height > screen.height {
        return BitmapMatchResult {
            found: false,
            x: 0,
            y: 0,
            score: 0.0,
        };
    }

    let min_x = search_x.unwrap_or(0).max(0) as u32;
    let min_y = search_y.unwrap_or(0).max(0) as u32;

    let max_x = match search_w {
        Some(w) if w > 0 => (min_x + w as u32).min(screen.width),
        _ => screen.width,
    };

    let max_y = match search_h {
        Some(h) if h > 0 => (min_y + h as u32).min(screen.height),
        _ => screen.height,
    };

    if min_x >= max_x || min_y >= max_y {
        return BitmapMatchResult {
            found: false,
            x: 0,
            y: 0,
            score: 0.0,
        };
    }

    if max_x < template.width || max_y < template.height {
        return BitmapMatchResult {
            found: false,
            x: 0,
            y: 0,
            score: 0.0,
        };
    }

    let stop_x = max_x - template.width;
    let stop_y = max_y - template.height;

    if min_x > stop_x || min_y > stop_y {
        return BitmapMatchResult {
            found: false,
            x: 0,
            y: 0,
            score: 0.0,
        };
    }

    let sample_step = 2u32;
    let screen_stride = (screen.width * 4) as usize;

    let sample_cap =
        ((template.height.div_ceil(sample_step)) * (template.width.div_ceil(sample_step))) as usize;
    let mut samples = Vec::with_capacity(sample_cap);

    for ty in (0..template.height).step_by(sample_step as usize) {
        for tx in (0..template.width).step_by(sample_step as usize) {
            if let Some((tr, tg, tb)) = template.get_pixel_rgb(tx, ty) {
                let rel_byte_offset = (ty as usize) * screen_stride + (tx as usize) * 4;
                samples.push(SamplePixel {
                    rel_byte_offset,
                    r: tr,
                    g: tg,
                    b: tb,
                });
            }
        }
    }

    if samples.is_empty() {
        return BitmapMatchResult {
            found: false,
            x: 0,
            y: 0,
            score: 0.0,
        };
    }

    let sample_count = samples.len();
    let mut best_score = 0.0f32;
    let mut best_x = 0u32;
    let mut best_y = 0u32;

    let screen_bytes = &screen.data;

    for sy in min_y..=stop_y {
        let row_start_idx = (sy as usize) * screen_stride;

        for sx in min_x..=stop_x {
            let start_idx = row_start_idx + (sx as usize) * 4;
            let mut matched_samples = 0usize;

            let min_required_matches = (best_score * sample_count as f32).floor() as usize;

            for (idx, sample) in samples.iter().enumerate() {
                let remaining_samples = sample_count - idx;
                if matched_samples + remaining_samples <= min_required_matches {
                    break;
                }

                let px_idx = start_idx + sample.rel_byte_offset;
                if px_idx + 2 < screen_bytes.len() {
                    let sr = screen_bytes[px_idx];
                    let sg = screen_bytes[px_idx + 1];
                    let sb = screen_bytes[px_idx + 2];

                    let diff_r = (sr as i16 - sample.r as i16).abs();
                    let diff_g = (sg as i16 - sample.g as i16).abs();
                    let diff_b = (sb as i16 - sample.b as i16).abs();

                    if diff_r <= 30 && diff_g <= 30 && diff_b <= 30 {
                        matched_samples += 1;
                    }
                }
            }

            let score = matched_samples as f32 / sample_count as f32;
            if score > best_score {
                best_score = score;
                best_x = sx;
                best_y = sy;

                if best_score >= 0.99 {
                    break;
                }
            }
        }

        if best_score >= 0.99 {
            break;
        }
    }

    let found = best_score >= match_threshold;

    BitmapMatchResult {
        found,
        x: best_x,
        y: best_y,
        score: best_score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_bitmap_zero_and_oversized_dimensions() {
        let empty_screen = CapturedImage::new(0, 0, vec![]);
        let template = CapturedImage::new(10, 10, vec![0; 400]);

        let res = search_bitmap(&empty_screen, &template, None, None, None, None, 0.9);
        assert!(!res.found);
        assert_eq!(res.score, 0.0);

        let small_screen = CapturedImage::new(5, 5, vec![0; 100]);
        let large_template = CapturedImage::new(10, 10, vec![0; 400]);

        let res_oversized =
            search_bitmap(&small_screen, &large_template, None, None, None, None, 0.9);
        assert!(!res_oversized.found);
        assert_eq!(res_oversized.score, 0.0);
    }

    #[test]
    fn test_search_bitmap_bounding_box_constraints() {
        let mut screen_data = vec![0u8; 50 * 50 * 4];
        // Target blue box at (30, 30)
        for y in 30..35 {
            for x in 30..35 {
                let idx = ((y * 50 + x) * 4) as usize;
                screen_data[idx + 2] = 255;
                screen_data[idx + 3] = 255;
            }
        }
        let screen = CapturedImage::new(50, 50, screen_data);

        let mut tmpl_data = vec![0u8; 5 * 5 * 4];
        for chunk in tmpl_data.as_chunks_mut::<4>().0 {
            chunk[2] = 255;
            chunk[3] = 255;
        }
        let template = CapturedImage::new(5, 5, tmpl_data);

        // Search within bounding box containing the target (25..40, 25..40)
        let res_found = search_bitmap(
            &screen,
            &template,
            Some(25),
            Some(25),
            Some(20),
            Some(20),
            0.9,
        );
        assert!(res_found.found);
        assert_eq!((res_found.x, res_found.y), (30, 30));

        // Search within bounding box NOT containing target (0..20, 0..20)
        let res_miss = search_bitmap(
            &screen,
            &template,
            Some(0),
            Some(0),
            Some(20),
            Some(20),
            0.9,
        );
        assert!(!res_miss.found);
    }
}
