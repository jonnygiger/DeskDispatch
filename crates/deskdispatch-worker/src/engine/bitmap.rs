use super::capture::CapturedImage;

#[derive(Debug, Clone, PartialEq)]
pub struct BitmapMatchResult {
    pub found: bool,
    pub x: u32,
    pub y: u32,
    pub score: f32,
}

const INLINE_SAMPLE_CAP: usize = 256;

#[derive(Debug, Clone, Copy, Default)]
struct SamplePixel {
    rel_byte_offset: u32,
    r: u8,
    g: u8,
    b: u8,
}

// Allowed: SampleBuffer is allocated directly on the local stack frame of search_bitmap
// to avoid heap allocations for templates <= 256 samples.
#[allow(clippy::large_enum_variant)]
enum SampleBuffer {
    Stack {
        buf: [SamplePixel; INLINE_SAMPLE_CAP],
        len: usize,
    },
    Heap(Vec<SamplePixel>),
}

impl SampleBuffer {
    #[inline]
    fn new(total_expected_samples: usize) -> Self {
        if total_expected_samples <= INLINE_SAMPLE_CAP {
            Self::Stack {
                buf: [SamplePixel::default(); INLINE_SAMPLE_CAP],
                len: 0,
            }
        } else {
            Self::Heap(Vec::with_capacity(total_expected_samples))
        }
    }

    #[inline]
    fn push(&mut self, sample: SamplePixel) {
        match self {
            Self::Stack { buf, len } => {
                if *len < INLINE_SAMPLE_CAP {
                    buf[*len] = sample;
                    *len += 1;
                }
            }
            Self::Heap(v) => v.push(sample),
        }
    }

    #[inline]
    fn as_slice(&self) -> &[SamplePixel] {
        match self {
            Self::Stack { buf, len } => &buf[..*len],
            Self::Heap(v) => v.as_slice(),
        }
    }

    #[inline]
    fn as_mut_slice(&mut self) -> &mut [SamplePixel] {
        match self {
            Self::Stack { buf, len } => &mut buf[..*len],
            Self::Heap(v) => v.as_mut_slice(),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        match self {
            Self::Stack { len, .. } => *len,
            Self::Heap(v) => v.len(),
        }
    }
}

/// Search for template bitmap in screen within optional bounding box.
///
/// Performance-tuned: Uses a zero-allocation stack buffer for up to 256 samples,
/// compact 8-byte sample representation, variance-based sample feature sorting for early pruning,
/// integer-only match count tracking in the inner search loop, and unchecked slice accesses
/// after pre-flight bounds validation.
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

    let samples_w = template.width.div_ceil(sample_step) as usize;
    let samples_h = template.height.div_ceil(sample_step) as usize;
    let total_expected_samples = samples_w * samples_h;

    let mut storage = SampleBuffer::new(total_expected_samples);

    let mut sum_r = 0u64;
    let mut sum_g = 0u64;
    let mut sum_b = 0u64;

    for ty in (0..template.height).step_by(sample_step as usize) {
        for tx in (0..template.width).step_by(sample_step as usize) {
            if let Some((tr, tg, tb)) = template.get_pixel_rgb(tx, ty) {
                let rel_byte_offset = (ty as usize) * screen_stride + (tx as usize) * 4;
                if rel_byte_offset <= u32::MAX as usize {
                    let sample = SamplePixel {
                        rel_byte_offset: rel_byte_offset as u32,
                        r: tr,
                        g: tg,
                        b: tb,
                    };
                    sum_r += tr as u64;
                    sum_g += tg as u64;
                    sum_b += tb as u64;

                    storage.push(sample);
                }
            }
        }
    }

    let sample_count = storage.len();
    if sample_count == 0 {
        return BitmapMatchResult {
            found: false,
            x: 0,
            y: 0,
            score: 0.0,
        };
    }

    // Sort samples by descending color variance from template average RGB
    // to evaluate high-contrast visual features first for immediate pruning
    let avg_r = (sum_r / sample_count as u64) as i16;
    let avg_g = (sum_g / sample_count as u64) as i16;
    let avg_b = (sum_b / sample_count as u64) as i16;

    storage.as_mut_slice().sort_by_key(|s| {
        let dr = (s.r as i16 - avg_r).abs();
        let dg = (s.g as i16 - avg_g).abs();
        let db = (s.b as i16 - avg_b).abs();
        std::cmp::Reverse(dr + dg + db)
    });

    let screen_bytes = &screen.data;
    let screen_len = screen_bytes.len();

    let samples_slice = storage.as_slice();
    let max_rel_offset = samples_slice
        .iter()
        .map(|s| s.rel_byte_offset as usize)
        .max()
        .unwrap_or(0);

    let max_possible_idx =
        (stop_y as usize) * screen_stride + (stop_x as usize) * 4 + max_rel_offset + 2;
    let can_use_unchecked = max_possible_idx < screen_len;

    let target_matched_count =
        ((match_threshold * sample_count as f32).ceil() as usize).min(sample_count);
    let exact_matched_count = ((0.99 * sample_count as f32).ceil() as usize).min(sample_count);

    let mut best_matched_count = 0usize;
    let mut best_x = 0u32;
    let mut best_y = 0u32;

    for sy in min_y..=stop_y {
        let row_start_idx = (sy as usize) * screen_stride;

        for sx in min_x..=stop_x {
            let start_idx = row_start_idx + (sx as usize) * 4;
            let mut matched_samples = 0usize;

            for (idx, sample) in samples_slice.iter().enumerate() {
                let remaining_samples = sample_count - idx;
                if matched_samples + remaining_samples <= best_matched_count {
                    break;
                }

                let px_idx = start_idx + sample.rel_byte_offset as usize;

                let (sr, sg, sb) = if can_use_unchecked {
                    // SAFETY: Pre-flight check verified max_possible_idx < screen_len
                    unsafe {
                        (
                            *screen_bytes.get_unchecked(px_idx),
                            *screen_bytes.get_unchecked(px_idx + 1),
                            *screen_bytes.get_unchecked(px_idx + 2),
                        )
                    }
                } else {
                    if px_idx + 2 < screen_len {
                        (
                            screen_bytes[px_idx],
                            screen_bytes[px_idx + 1],
                            screen_bytes[px_idx + 2],
                        )
                    } else {
                        continue;
                    }
                };

                let diff_r = (sr as i16 - sample.r as i16).abs();
                let diff_g = (sg as i16 - sample.g as i16).abs();
                let diff_b = (sb as i16 - sample.b as i16).abs();

                if diff_r <= 30 && diff_g <= 30 && diff_b <= 30 {
                    matched_samples += 1;
                }
            }

            if matched_samples > best_matched_count {
                best_matched_count = matched_samples;
                best_x = sx;
                best_y = sy;

                if best_matched_count >= exact_matched_count {
                    break;
                }
            }
        }

        if best_matched_count >= exact_matched_count {
            break;
        }
    }

    let best_score = best_matched_count as f32 / sample_count as f32;
    let found = best_matched_count >= target_matched_count;

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

    #[test]
    fn test_search_bitmap_exact_and_partial_match() {
        let mut screen_data = vec![255u8; 20 * 20 * 4]; // White background
        // Place red square at (8, 12)
        for y in 12..16 {
            for x in 8..12 {
                let idx = ((y * 20 + x) * 4) as usize;
                screen_data[idx] = 255; // R
                screen_data[idx + 1] = 0; // G
                screen_data[idx + 2] = 0; // B
            }
        }
        let screen = CapturedImage::new(20, 20, screen_data);

        let mut tmpl_data = vec![0u8; 4 * 4 * 4];
        for chunk in tmpl_data.as_chunks_mut::<4>().0 {
            chunk[0] = 255; // R
            chunk[1] = 0; // G
            chunk[2] = 0; // B
            chunk[3] = 255; // A
        }
        let template = CapturedImage::new(4, 4, tmpl_data);

        let res = search_bitmap(&screen, &template, None, None, None, None, 0.95);
        assert!(res.found);
        assert_eq!((res.x, res.y), (8, 12));
        assert!(res.score >= 0.99);
    }

    #[test]
    fn test_search_bitmap_large_template_heap_fallback() {
        // Create a 40x40 template (400 samples at step 2, exceeding INLINE_SAMPLE_CAP 256)
        let screen_data = vec![128u8; 100 * 100 * 4];
        let screen = CapturedImage::new(100, 100, screen_data);

        let tmpl_data = vec![128u8; 40 * 40 * 4];
        let template = CapturedImage::new(40, 40, tmpl_data);

        let res = search_bitmap(&screen, &template, None, None, None, None, 0.9);
        assert!(res.found);
        assert_eq!((res.x, res.y), (0, 0));
        assert_eq!(res.score, 1.0);
    }
}
