use super::capture::CapturedImage;

#[derive(Debug, Clone, PartialEq)]
pub struct BitmapMatchResult {
    pub found: bool,
    pub x: u32,
    pub y: u32,
    pub score: f32,
}

/// Search for template bitmap in screen within optional bounding box.
/// Returns highest scoring match position and score (0.0 to 1.0).
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

    let stop_x = max_x.saturating_sub(template.width);
    let stop_y = max_y.saturating_sub(template.height);

    let mut best_score = 0.0f32;
    let mut best_x = 0u32;
    let mut best_y = 0u32;

    // Sub-sample template pixels to keep image matching fast
    let sample_step = 2u32;
    let mut sample_count = 0u32;

    for _ty in (0..template.height).step_by(sample_step as usize) {
        for _tx in (0..template.width).step_by(sample_step as usize) {
            sample_count += 1;
        }
    }

    if sample_count == 0 {
        sample_count = 1;
    }

    for sy in min_y..=stop_y {
        for sx in min_x..=stop_x {
            let mut matched_samples = 0u32;

            for ty in (0..template.height).step_by(sample_step as usize) {
                for tx in (0..template.width).step_by(sample_step as usize) {
                    if let (Some((sr, sg, sb)), Some((tr, tg, tb))) = (
                        screen.get_pixel_rgb(sx + tx, sy + ty),
                        template.get_pixel_rgb(tx, ty),
                    ) {
                        let diff_r = (sr as i16 - tr as i16).abs();
                        let diff_g = (sg as i16 - tg as i16).abs();
                        let diff_b = (sb as i16 - tb as i16).abs();

                        if diff_r <= 30 && diff_g <= 30 && diff_b <= 30 {
                            matched_samples += 1;
                        }
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
    }

    let found = best_score >= match_threshold;

    BitmapMatchResult {
        found,
        x: best_x,
        y: best_y,
        score: best_score,
    }
}
