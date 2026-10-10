/// Server-side mapping logic for coordinate picking and refinement on screenshots.
/// Maps coarse image-input click coordinates (`click_x`, `click_y`) on a rendered display container
/// of size (`display_w`, `display_h`) to exact native image pixel coordinates (`native_x`, `native_y`).
///
/// This accounts for aspect-ratio scale-to-fit calculation (contain fitting), including centering
/// offsets (vertical letterboxing or horizontal pillarboxing) and boundary clamping.
pub fn map_coarse_click_to_native(
    click_x: u32,
    click_y: u32,
    display_w: f64,
    display_h: f64,
    native_w: u32,
    native_h: u32,
) -> (u32, u32) {
    if native_w == 0 || native_h == 0 || display_w <= 0.0 || display_h <= 0.0 {
        return (0, 0);
    }

    let scale_w = display_w / native_w as f64;
    let scale_h = display_h / native_h as f64;
    let scale = scale_w.min(scale_h);

    let rendered_w = native_w as f64 * scale;
    let rendered_h = native_h as f64 * scale;

    let offset_x = (display_w - rendered_w) / 2.0;
    let offset_y = (display_h - rendered_h) / 2.0;

    let img_x = (click_x as f64 - offset_x).clamp(0.0, rendered_w);
    let img_y = (click_y as f64 - offset_y).clamp(0.0, rendered_h);

    let native_x = (img_x / scale).floor().min((native_w - 1) as f64) as u32;
    let native_y = (img_y / scale).floor().min((native_h - 1) as f64) as u32;

    (native_x, native_y)
}

/// Maps click coordinates on the 20x grid panel view (320x320 viewport centered at `center_x`, `center_y`)
/// to exact native image pixel coordinates.
///
/// The grid view renders each native pixel as a 20px x 20px cell. Clicks within the 320x320 panel
/// are converted to relative native pixel offsets from `(center_x, center_y)` and clamped to
/// image boundaries `(0..native_w - 1, 0..native_h - 1)`.
pub fn map_grid_click_to_native(
    click_x: u32,
    click_y: u32,
    center_x: u32,
    center_y: u32,
    native_w: u32,
    native_h: u32,
) -> (u32, u32) {
    if native_w == 0 || native_h == 0 {
        return (0, 0);
    }

    let dx = (click_x as f64 - 160.0 + 10.0) / 20.0;
    let dy = (click_y as f64 - 160.0 + 10.0) / 20.0;

    let offset_x = dx.floor() as i64;
    let offset_y = dy.floor() as i64;

    let target_x = (center_x as i64 + offset_x).clamp(0, (native_w - 1) as i64) as u32;
    let target_y = (center_y as i64 + offset_y).clamp(0, (native_h - 1) as i64) as u32;

    (target_x, target_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_coarse_click_exact_aspect_ratio_fit() {
        // Native 1200 x 680 in 600 x 340 display -> scale = 0.5 exactly (1200/600 = 680/340 = 2.0)
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 1200, 680);
        assert_eq!((nx, ny), (600, 340));

        let (nx_top, ny_top) = map_coarse_click_to_native(0, 0, 600.0, 340.0, 1200, 680);
        assert_eq!((nx_top, ny_top), (0, 0));

        let (nx_bot, ny_bot) = map_coarse_click_to_native(600, 340, 600.0, 340.0, 1200, 680);
        assert_eq!((nx_bot, ny_bot), (1199, 679));
    }

    #[test]
    fn test_map_coarse_click_1080p_resolution() {
        // 1080p: 1920 x 1080 in 600 x 340 display container
        // scale_w = 600/1920 = 0.3125, scale_h = 340/1080 = 0.3148148...
        // scale = 0.3125, rendered_w = 600, rendered_h = 337.5, offset_x = 0, offset_y = 1.25
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 1920, 1080);
        assert_eq!((nx, ny), (960, 540));

        // Click top edge inside rendered region (y = 1) -> img_y = 0.0 -> native_y = 0
        let (nx_top, ny_top) = map_coarse_click_to_native(0, 1, 600.0, 340.0, 1920, 1080);
        assert_eq!((nx_top, ny_top), (0, 0));

        // Click bottom edge (y = 339) -> img_y = 337.5 -> clamped to 337.5 -> native_y = 1079
        let (nx_bot, ny_bot) = map_coarse_click_to_native(600, 339, 600.0, 340.0, 1920, 1080);
        assert_eq!((nx_bot, ny_bot), (1919, 1079));
    }

    #[test]
    fn test_map_coarse_click_1440p_resolution() {
        // 1440p: 2560 x 1440 in 600 x 340 display container
        // scale_w = 600/2560 = 0.234375, scale_h = 340/1440 = 0.2361111...
        // scale = 0.234375, rendered_w = 600, rendered_h = 337.5, offset_x = 0, offset_y = 1.25
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 2560, 1440);
        assert_eq!((nx, ny), (1280, 720));

        // Top left
        let (nx_tl, ny_tl) = map_coarse_click_to_native(0, 0, 600.0, 340.0, 2560, 1440);
        assert_eq!((nx_tl, ny_tl), (0, 0));

        // Bottom right
        let (nx_br, ny_br) = map_coarse_click_to_native(600, 340, 600.0, 340.0, 2560, 1440);
        assert_eq!((nx_br, ny_br), (2559, 1439));
    }

    #[test]
    fn test_map_coarse_click_4k_resolution() {
        // 4K UHD: 3840 x 2160 in 600 x 340 display container
        // scale_w = 600/3840 = 0.15625, scale_h = 340/2160 = 0.157407...
        // scale = 0.15625, rendered_w = 600, rendered_h = 337.5, offset_x = 0, offset_y = 1.25
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 3840, 2160);
        assert_eq!((nx, ny), (1920, 1080));

        // Quarter point: x = 150 -> 150 / 0.15625 = 960
        let (nx_q, ny_q) = map_coarse_click_to_native(150, 170, 600.0, 340.0, 3840, 2160);
        assert_eq!((nx_q, ny_q), (960, 1080));
    }

    #[test]
    fn test_map_coarse_click_720p_resolution() {
        // 720p: 1280 x 720 in 600 x 340 display container
        // scale_w = 600/1280 = 0.46875, scale_h = 340/720 = 0.472222...
        // scale = 0.46875, rendered_w = 600, rendered_h = 337.5
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 1280, 720);
        assert_eq!((nx, ny), (640, 360));
    }

    #[test]
    fn test_map_coarse_click_ultrawide_aspect_ratio() {
        // 21:9 Ultrawide: 2560 x 1080 in 600 x 340 display container
        // scale_w = 600/2560 = 0.234375, scale_h = 340/1080 = 0.314814...
        // scale = 0.234375, rendered_w = 600, rendered_h = 253.125
        // offset_x = 0, offset_y = (340 - 253.125)/2 = 43.4375
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 2560, 1080);
        assert_eq!((nx, ny), (1280, 540));

        // Click in top letterbox area (y = 10) -> img_y clamped to 0 -> native_y = 0
        let (nx_lb, ny_lb) = map_coarse_click_to_native(300, 10, 600.0, 340.0, 2560, 1080);
        assert_eq!((nx_lb, ny_lb), (1280, 0));
    }

    #[test]
    fn test_map_coarse_click_portrait_aspect_ratio() {
        // Portrait / Tall image: 600 x 1200 in 600 x 340 display container
        // scale_w = 600/600 = 1.0, scale_h = 340/1200 = 0.2833333333333333
        // scale = 0.2833333333333333, rendered_w = 170, rendered_h = 340
        // offset_x = (600 - 170) / 2 = 215, offset_y = 0
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 600, 1200);
        assert_eq!((nx, ny), (300, 600));

        // Click in left pillarbox margin (x = 50) -> img_x clamped to 0 -> native_x = 0
        let (nx_pb, ny_pb) = map_coarse_click_to_native(50, 170, 600.0, 340.0, 600, 1200);
        assert_eq!((nx_pb, ny_pb), (0, 600));
    }

    #[test]
    fn test_map_coarse_click_zero_and_invalid_dimensions() {
        let (nx, ny) = map_coarse_click_to_native(100, 100, 600.0, 340.0, 0, 0);
        assert_eq!((nx, ny), (0, 0));

        let (nx_disp, ny_disp) = map_coarse_click_to_native(100, 100, 0.0, 0.0, 1920, 1080);
        assert_eq!((nx_disp, ny_disp), (0, 0));

        let (nx_neg, ny_neg) = map_coarse_click_to_native(100, 100, -100.0, -100.0, 1920, 1080);
        assert_eq!((nx_neg, ny_neg), (0, 0));
    }

    #[test]
    fn test_map_grid_click_exact_center() {
        // Center click at (160, 160) on 320x320 grid panel should return exact (center_x, center_y)
        let (nx, ny) = map_grid_click_to_native(160, 160, 960, 540, 1920, 1080);
        assert_eq!((nx, ny), (960, 540));
    }

    #[test]
    fn test_map_grid_click_offsets() {
        // +20px right on 20x grid panel -> +1 native x pixel
        let (nx_r1, ny_r1) = map_grid_click_to_native(180, 160, 500, 300, 1920, 1080);
        assert_eq!((nx_r1, ny_r1), (501, 300));

        // +40px right -> +2 native x pixels
        let (nx_r2, ny_r2) = map_grid_click_to_native(200, 160, 500, 300, 1920, 1080);
        assert_eq!((nx_r2, ny_r2), (502, 300));

        // -20px left -> -1 native x pixel
        let (nx_l1, ny_l1) = map_grid_click_to_native(140, 160, 500, 300, 1920, 1080);
        assert_eq!((nx_l1, ny_l1), (499, 300));

        // +20px down -> +1 native y pixel
        let (nx_d1, ny_d1) = map_grid_click_to_native(160, 180, 500, 300, 1920, 1080);
        assert_eq!((nx_d1, ny_d1), (500, 301));

        // -20px up -> -1 native y pixel
        let (nx_u1, ny_u1) = map_grid_click_to_native(160, 140, 500, 300, 1920, 1080);
        assert_eq!((nx_u1, ny_u1), (500, 299));
    }

    #[test]
    fn test_map_grid_click_boundary_clamping() {
        // Clicking far top-left (0, 0) near corner pixel (2, 2)
        let (nx_zero, ny_zero) = map_grid_click_to_native(0, 0, 2, 2, 1920, 1080);
        assert_eq!((nx_zero, ny_zero), (0, 0));

        // Clicking far bottom-right (319, 319) near bottom-right edge (1918, 1078)
        let (nx_max, ny_max) = map_grid_click_to_native(319, 319, 1918, 1078, 1920, 1080);
        assert_eq!((nx_max, ny_max), (1919, 1079));
    }

    #[test]
    fn test_map_grid_click_zero_and_invalid_dimensions() {
        let (nx, ny) = map_grid_click_to_native(160, 160, 100, 100, 0, 0);
        assert_eq!((nx, ny), (0, 0));

        let (nx_w, ny_w) = map_grid_click_to_native(160, 160, 100, 100, 0, 1080);
        assert_eq!((nx_w, ny_w), (0, 0));

        let (nx_h, ny_h) = map_grid_click_to_native(160, 160, 100, 100, 1920, 0);
        assert_eq!((nx_h, ny_h), (0, 0));
    }
}
