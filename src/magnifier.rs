use askama::Template;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt::Write;

/// Represents aspect-ratio containment fitting of a native image `(native_w, native_h)`
/// inside a display container viewport `(display_w, display_h)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportFit {
    pub scale: f64,
    pub rendered_w: f64,
    pub rendered_h: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}

impl ViewportFit {
    /// Calculates containment scale and centering offsets for fitting a native image into a display viewport.
    #[inline]
    pub fn compute(display_w: f64, display_h: f64, native_w: u32, native_h: u32) -> Option<Self> {
        if native_w == 0 || native_h == 0 || display_w <= 0.0 || display_h <= 0.0 {
            return None;
        }

        let scale_w = display_w / native_w as f64;
        let scale_h = display_h / native_h as f64;
        let scale = scale_w.min(scale_h);

        let rendered_w = native_w as f64 * scale;
        let rendered_h = native_h as f64 * scale;

        let offset_x = (display_w - rendered_w) / 2.0;
        let offset_y = (display_h - rendered_h) / 2.0;

        Some(Self {
            scale,
            rendered_w,
            rendered_h,
            offset_x,
            offset_y,
        })
    }

    /// Maps coarse click coordinates on display container to native image pixel coordinates.
    #[inline]
    pub fn map_click(
        &self,
        click_x: u32,
        click_y: u32,
        native_w: u32,
        native_h: u32,
    ) -> (u32, u32) {
        let img_x = (click_x as f64 - self.offset_x).clamp(0.0, self.rendered_w);
        let img_y = (click_y as f64 - self.offset_y).clamp(0.0, self.rendered_h);

        let native_x = (img_x / self.scale).floor().min((native_w - 1) as f64) as u32;
        let native_y = (img_y / self.scale).floor().min((native_h - 1) as f64) as u32;

        (native_x, native_y)
    }

    /// Generates CSS marker position styling (`left: X.XXpx; top: Y.YYpx;`) for native target point `(tx, ty)`.
    #[inline]
    pub fn marker_style(&self, tx: f64, ty: f64) -> String {
        let marker_left = self.offset_x + (tx * self.scale);
        let marker_top = self.offset_y + (ty * self.scale);
        let mut style = String::with_capacity(36);
        let _ = write!(
            style,
            "left: {:.2}px; top: {:.2}px;",
            marker_left, marker_top
        );
        style
    }
}

/// Maps coarse image-input click coordinates (`click_x`, `click_y`) on a rendered display container
/// of size (`display_w`, `display_h`) to exact native image pixel coordinates (`native_x`, `native_y`).
pub fn map_coarse_click_to_native(
    click_x: u32,
    click_y: u32,
    display_w: f64,
    display_h: f64,
    native_w: u32,
    native_h: u32,
) -> (u32, u32) {
    match ViewportFit::compute(display_w, display_h, native_w, native_h) {
        Some(fit) => fit.map_click(click_x, click_y, native_w, native_h),
        None => (0, 0),
    }
}

/// Maps click coordinates on the 20x grid panel view (320x320 viewport centered at `center_x`, `center_y`)
/// to exact native image pixel coordinates.
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

    let target_x = (center_x as i64 + dx.floor() as i64).clamp(0, (native_w - 1) as i64) as u32;
    let target_y = (center_y as i64 + dy.floor() as i64).clamp(0, (native_h - 1) as i64) as u32;

    (target_x, target_y)
}

/// Helper to escape characters in URLs used inside CSS `url('...')` values with UTF-8 character preservation.
pub fn escape_css_url<'a>(url: &'a str) -> Cow<'a, str> {
    if !url.bytes().any(|b| {
        matches!(
            b,
            b'\\' | b'\'' | b'"' | b'\n' | b'\r' | 0x0C | b'(' | b')' | b'<' | b'>'
        )
    }) {
        return Cow::Borrowed(url);
    }
    let mut escaped = String::with_capacity(url.len() + 16);
    for c in url.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '\'' => escaped.push_str("\\'"),
            '"' => escaped.push_str("\\\""),
            '\n' | '\r' | '\x0C' => {}
            '(' => escaped.push_str("%28"),
            ')' => escaped.push_str("%29"),
            '<' => escaped.push_str("%3C"),
            '>' => escaped.push_str("%3E"),
            c => escaped.push(c),
        }
    }
    Cow::Owned(escaped)
}

/// ImageMagnifier struct representing the three-panel image magnifier component settings and calculations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageMagnifier {
    pub presigned_url: String,
    pub native_w: u32,
    pub native_h: u32,
    pub target_x: Option<u32>,
    pub target_y: Option<u32>,
}

impl ImageMagnifier {
    pub fn new(
        presigned_url: impl Into<String>,
        native_w: u32,
        native_h: u32,
        target_x: Option<u32>,
        target_y: Option<u32>,
    ) -> Self {
        Self {
            presigned_url: presigned_url.into(),
            native_w,
            native_h,
            target_x,
            target_y,
        }
    }

    #[inline]
    pub fn show_marker(&self) -> bool {
        self.target_x.is_some() && self.target_y.is_some() && self.native_w > 0 && self.native_h > 0
    }

    #[inline]
    pub fn escaped_url(&self) -> Cow<'_, str> {
        escape_css_url(&self.presigned_url)
    }

    #[inline]
    fn target_or_center(&self) -> (f64, f64) {
        match (self.target_x, self.target_y) {
            (Some(x), Some(y)) => (x as f64, y as f64),
            _ => (self.native_w as f64 / 2.0, self.native_h as f64 / 2.0),
        }
    }

    /// Background style for normal panel (1x scale, viewport 600x340 or calculated fit)
    pub fn normal_style(&self) -> String {
        format!(
            "background-image: url('{}'); background-size: contain; background-position: center;",
            self.escaped_url()
        )
    }

    /// Marker style for normal panel (returns static borrowed empty string when marker is disabled)
    pub fn normal_marker_style(&self) -> Cow<'static, str> {
        if self.show_marker() {
            let (tx, ty) = self.target_or_center();
            match ViewportFit::compute(600.0, 340.0, self.native_w, self.native_h) {
                Some(fit) => Cow::Owned(fit.marker_style(tx, ty)),
                None => Cow::Borrowed(""),
            }
        } else {
            Cow::Borrowed("")
        }
    }

    /// Background style for 400% (4x) zoom panel (viewport 320x320)
    pub fn zoom400_style(&self) -> String {
        let (tx, ty) = self.target_or_center();
        let (bg_w, bg_h) = (self.native_w * 4, self.native_h * 4);
        let (pan_x, pan_y) = if self.native_w > 0 && self.native_h > 0 {
            (160.0 - (tx * 4.0), 160.0 - (ty * 4.0))
        } else {
            (0.0, 0.0)
        };
        format!(
            "background-image: url('{}'); background-size: {}px {}px; background-position: {:.2}px {:.2}px;",
            self.escaped_url(),
            bg_w,
            bg_h,
            pan_x,
            pan_y
        )
    }

    /// Marker style for 400% (4x) zoom panel (viewport 320x320)
    #[inline]
    pub fn zoom400_marker_style(&self) -> &'static str {
        if self.show_marker() {
            "left: 160px; top: 160px;"
        } else {
            ""
        }
    }

    /// CSS custom properties style for 20x pixel grid panel (viewport 320x320)
    pub fn grid_style(&self) -> String {
        let (tx, ty) = self.target_or_center();
        let (bg_w, bg_h) = (self.native_w * 20, self.native_h * 20);
        let (pan_x, pan_y) = if self.native_w > 0 && self.native_h > 0 {
            (160.0 - (tx * 20.0 + 10.0), 160.0 - (ty * 20.0 + 10.0))
        } else {
            (0.0, 0.0)
        };
        format!(
            "--grid-bg-url: url('{}'); --grid-bg-w: {}px; --grid-bg-h: {}px; --grid-pan-x: {:.2}px; --grid-pan-y: {:.2}px;",
            self.escaped_url(),
            bg_w,
            bg_h,
            pan_x,
            pan_y
        )
    }

    /// Marker style for 20x pixel grid panel (viewport 320x320)
    #[inline]
    pub fn grid_marker_style(&self) -> &'static str {
        if self.show_marker() {
            "left: 160px; top: 160px;"
        } else {
            ""
        }
    }
}

/// Standalone Askama template struct for rendering the image magnifier component partial
#[derive(Template)]
#[template(path = "image_magnifier.html")]
pub struct ImageMagnifierTemplate<'a> {
    pub magnifier: &'a ImageMagnifier,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serde_roundtrip_and_minimal_json() {
        let mag = ImageMagnifier::new(
            "http://example.com/test.jpg",
            1920,
            1080,
            Some(100),
            Some(200),
        );

        let json_str = serde_json::to_string(&mag).expect("Serialization succeeds");

        // Ensure serialized JSON contains only the minimal 5 fields and no pre-computed CSS fields
        assert!(json_str.contains("presigned_url"));
        assert!(json_str.contains("native_w"));
        assert!(!json_str.contains("escaped_url"));
        assert!(!json_str.contains("zoom400_pan_x"));

        // Deserialize back into ImageMagnifier
        let deserialized: ImageMagnifier =
            serde_json::from_str(&json_str).expect("Deserialization succeeds");

        assert_eq!(mag, deserialized);
        assert!(
            deserialized
                .zoom400_style()
                .contains("background-size: 7680px 4320px")
        );
        assert!(
            deserialized
                .grid_style()
                .contains("--grid-bg-w: 38400px; --grid-bg-h: 21600px;")
        );
        assert!(deserialized.show_marker());
    }

    #[test]
    fn test_map_coarse_click_exact_aspect_ratio_fit() {
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 1200, 680);
        assert_eq!((nx, ny), (600, 340));

        let (nx_top, ny_top) = map_coarse_click_to_native(0, 0, 600.0, 340.0, 1200, 680);
        assert_eq!((nx_top, ny_top), (0, 0));

        let (nx_bot, ny_bot) = map_coarse_click_to_native(600, 340, 600.0, 340.0, 1200, 680);
        assert_eq!((nx_bot, ny_bot), (1199, 679));
    }

    #[test]
    fn test_map_coarse_click_1080p_resolution() {
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 1920, 1080);
        assert_eq!((nx, ny), (960, 540));

        let (nx_top, ny_top) = map_coarse_click_to_native(0, 1, 600.0, 340.0, 1920, 1080);
        assert_eq!((nx_top, ny_top), (0, 0));

        let (nx_bot, ny_bot) = map_coarse_click_to_native(600, 339, 600.0, 340.0, 1920, 1080);
        assert_eq!((nx_bot, ny_bot), (1919, 1079));
    }

    #[test]
    fn test_map_coarse_click_1440p_resolution() {
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 2560, 1440);
        assert_eq!((nx, ny), (1280, 720));

        let (nx_tl, ny_tl) = map_coarse_click_to_native(0, 0, 600.0, 340.0, 2560, 1440);
        assert_eq!((nx_tl, ny_tl), (0, 0));

        let (nx_br, ny_br) = map_coarse_click_to_native(600, 340, 600.0, 340.0, 2560, 1440);
        assert_eq!((nx_br, ny_br), (2559, 1439));
    }

    #[test]
    fn test_map_coarse_click_4k_resolution() {
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 3840, 2160);
        assert_eq!((nx, ny), (1920, 1080));

        let (nx_q, ny_q) = map_coarse_click_to_native(150, 170, 600.0, 340.0, 3840, 2160);
        assert_eq!((nx_q, ny_q), (960, 1080));
    }

    #[test]
    fn test_map_coarse_click_720p_resolution() {
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 1280, 720);
        assert_eq!((nx, ny), (640, 360));
    }

    #[test]
    fn test_map_coarse_click_ultrawide_aspect_ratio() {
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 2560, 1080);
        assert_eq!((nx, ny), (1280, 540));

        let (nx_lb, ny_lb) = map_coarse_click_to_native(300, 10, 600.0, 340.0, 2560, 1080);
        assert_eq!((nx_lb, ny_lb), (1280, 0));
    }

    #[test]
    fn test_map_coarse_click_portrait_aspect_ratio() {
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 600, 1200);
        assert_eq!((nx, ny), (300, 600));

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
        let (nx, ny) = map_grid_click_to_native(160, 160, 960, 540, 1920, 1080);
        assert_eq!((nx, ny), (960, 540));
    }

    #[test]
    fn test_map_grid_click_offsets() {
        let (nx_r1, ny_r1) = map_grid_click_to_native(180, 160, 500, 300, 1920, 1080);
        assert_eq!((nx_r1, ny_r1), (501, 300));

        let (nx_r2, ny_r2) = map_grid_click_to_native(200, 160, 500, 300, 1920, 1080);
        assert_eq!((nx_r2, ny_r2), (502, 300));

        let (nx_l1, ny_l1) = map_grid_click_to_native(140, 160, 500, 300, 1920, 1080);
        assert_eq!((nx_l1, ny_l1), (499, 300));

        let (nx_d1, ny_d1) = map_grid_click_to_native(160, 180, 500, 300, 1920, 1080);
        assert_eq!((nx_d1, ny_d1), (500, 301));

        let (nx_u1, ny_u1) = map_grid_click_to_native(160, 140, 500, 300, 1920, 1080);
        assert_eq!((nx_u1, ny_u1), (500, 299));
    }

    #[test]
    fn test_map_grid_click_boundary_clamping() {
        let (nx_zero, ny_zero) = map_grid_click_to_native(0, 0, 2, 2, 1920, 1080);
        assert_eq!((nx_zero, ny_zero), (0, 0));

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

    #[test]
    fn test_image_magnifier_without_target() {
        let mag = ImageMagnifier::new("http://example.com/image.png", 1920, 1080, None, None);

        assert!(!mag.show_marker());
        assert!(mag.normal_marker_style().is_empty());
        assert!(mag.zoom400_marker_style().is_empty());
        assert!(mag.grid_marker_style().is_empty());

        assert!(
            mag.normal_style()
                .contains("background-image: url('http://example.com/image.png')")
        );
        let z_style = mag.zoom400_style();
        assert!(z_style.contains("background-size: 7680px 4320px"));
        assert!(z_style.contains("background-position: -3680.00px -2000.00px"));

        let g_style = mag.grid_style();
        assert!(g_style.contains("--grid-bg-w: 38400px; --grid-bg-h: 21600px;"));
        assert!(g_style.contains("--grid-pan-x: -19050.00px; --grid-pan-y: -10650.00px;"));
    }

    #[test]
    fn test_image_magnifier_with_target() {
        let mag = ImageMagnifier::new(
            "http://example.com/image.png",
            1000,
            500,
            Some(100),
            Some(200),
        );

        assert!(mag.show_marker());
        assert_eq!(mag.zoom400_marker_style(), "left: 160px; top: 160px;");
        assert_eq!(mag.grid_marker_style(), "left: 160px; top: 160px;");

        let normal_marker = mag.normal_marker_style();
        assert!(normal_marker.contains("left:"));
        assert!(normal_marker.contains("top:"));

        let zoom400_style = mag.zoom400_style();
        assert!(zoom400_style.contains("background-size: 4000px 2000px"));
        assert!(zoom400_style.contains("background-position: -240.00px -640.00px"));

        let grid_style = mag.grid_style();
        assert!(grid_style.contains("--grid-bg-url: url('http://example.com/image.png')"));
        assert!(grid_style.contains("--grid-bg-w: 20000px; --grid-bg-h: 10000px;"));
        assert!(grid_style.contains("--grid-pan-x: -1850.00px; --grid-pan-y: -3850.00px;"));
    }

    #[test]
    fn test_marker_coordinate_positioning_aspect_ratios() {
        let mag_wide = ImageMagnifier::new(
            "http://example.com/wide.png",
            1200,
            340,
            Some(200),
            Some(100),
        );
        assert_eq!(
            mag_wide.normal_marker_style(),
            "left: 100.00px; top: 135.00px;"
        );

        let mag_tall = ImageMagnifier::new(
            "http://example.com/tall.png",
            600,
            680,
            Some(100),
            Some(200),
        );
        assert_eq!(
            mag_tall.normal_marker_style(),
            "left: 200.00px; top: 100.00px;"
        );
    }

    #[test]
    fn test_marker_coordinate_positioning_partial_targets() {
        let mag_no_x =
            ImageMagnifier::new("http://example.com/test.png", 800, 600, None, Some(100));
        assert!(!mag_no_x.show_marker());
        assert_eq!(mag_no_x.normal_marker_style(), "");
        assert_eq!(mag_no_x.zoom400_marker_style(), "");
        assert_eq!(mag_no_x.grid_marker_style(), "");

        let mag_no_y =
            ImageMagnifier::new("http://example.com/test.png", 800, 600, Some(100), None);
        assert!(!mag_no_y.show_marker());
        assert_eq!(mag_no_y.normal_marker_style(), "");
        assert_eq!(mag_no_y.zoom400_marker_style(), "");
        assert_eq!(mag_no_y.grid_marker_style(), "");
    }

    #[test]
    fn test_normal_and_zoom400_edge_cases() {
        let mag_zero =
            ImageMagnifier::new("http://example.com/image.png", 0, 0, Some(100), Some(100));
        assert!(!mag_zero.show_marker());
        assert_eq!(mag_zero.normal_marker_style(), "");
        assert_eq!(mag_zero.zoom400_marker_style(), "");
        assert!(
            mag_zero
                .zoom400_style()
                .contains("background-size: 0px 0px")
        );
        assert!(
            mag_zero
                .zoom400_style()
                .contains("background-position: 0.00px 0.00px")
        );

        let mag_normal = ImageMagnifier::new(
            "http://example.com/image.png",
            1200,
            680,
            Some(400),
            Some(200),
        );
        assert_eq!(
            mag_normal.normal_marker_style(),
            "left: 200.00px; top: 100.00px;"
        );

        let mag_zoom = ImageMagnifier::new(
            "http://example.com/image.png",
            800,
            600,
            Some(100),
            Some(50),
        );
        let zoom_style = mag_zoom.zoom400_style();
        assert!(zoom_style.contains("background-size: 3200px 2400px"));
        assert!(zoom_style.contains("background-position: -240.00px -40.00px"));
        assert_eq!(mag_zoom.zoom400_marker_style(), "left: 160px; top: 160px;");
    }

    #[test]
    fn test_escape_css_url() {
        let malicious = "http://example.com/img.png'); body { background: red; } /*";
        let escaped = escape_css_url(malicious);
        assert_eq!(
            escaped,
            "http://example.com/img.png\\'%29; body { background: red; } /*"
        );

        let mag = ImageMagnifier::new(malicious, 800, 600, None, None);
        assert!(!mag.normal_style().contains("img.png');"));
        assert!(mag.normal_style().contains("img.png\\'"));

        // Unicode non-ASCII test
        let unicode_url = "http://example.com/image_图片.jpg";
        let unicode_escaped = escape_css_url(unicode_url);
        assert_eq!(unicode_escaped, unicode_url);
    }

    #[test]
    fn test_escape_css_url_and_style_generation_edge_cases() {
        // 1. Safe URLs borrow original string slice without allocation
        let safe_url = "http://example.com/media/screenshots/step_100.png?v=2&auth=abc_123";
        let safe_escaped = escape_css_url(safe_url);
        assert!(matches!(safe_escaped, Cow::Borrowed(_)));
        assert_eq!(safe_escaped, safe_url);

        // 2. Unsafe characters: backslash (\), double quotes ("), newlines (\n, \r), form feed (\x0C), parentheses ( ( ) ), angle brackets (< >)
        let unsafe_url =
            "http://example.com/path\\file\"name.png?a=(1)\r\n&b=\x0C<script>alert(1)</script>";
        let unsafe_escaped = escape_css_url(unsafe_url);
        assert!(matches!(unsafe_escaped, Cow::Owned(_)));
        assert_eq!(
            unsafe_escaped,
            "http://example.com/path\\\\file\\\"name.png?a=%281%29&b=%3Cscript%3Ealert%281%29%3C/script%3E"
        );

        // 3. Verify style generation methods with malicious / CSS-unsafe input
        let malicious_url =
            "http://example.com/test\\path'\"_1\r\n\x0C.png?query=(val)&tag=<script>";
        let mag = ImageMagnifier::new(malicious_url, 1000, 500, Some(100), Some(200));

        let normal = mag.normal_style();
        assert!(normal.contains(
            "url('http://example.com/test\\\\path\\'\\\"_1.png?query=%28val%29&tag=%3Cscript%3E')"
        ));
        assert!(!normal.contains('\r'));
        assert!(!normal.contains('\n'));

        let zoom = mag.zoom400_style();
        assert!(zoom.contains(
            "url('http://example.com/test\\\\path\\'\\\"_1.png?query=%28val%29&tag=%3Cscript%3E')"
        ));
        assert!(!zoom.contains('\r'));
        assert!(!zoom.contains('\n'));

        let grid = mag.grid_style();
        assert!(grid.contains("--grid-bg-url: url('http://example.com/test\\\\path\\'\\\"_1.png?query=%28val%29&tag=%3Cscript%3E');"));
        assert!(!grid.contains('\r'));
        assert!(!grid.contains('\n'));
    }

    #[test]
    fn test_template_rendering() {
        let mag = ImageMagnifier::new("http://example.com/test.jpg", 800, 600, Some(50), Some(50));

        let tmpl = ImageMagnifierTemplate { magnifier: &mag };
        let rendered = tmpl.render().expect("Template should render successfully");

        assert!(rendered.contains("class=\"shot-container\""));
        assert!(rendered.contains("class=\"shot shot--normal\""));
        assert!(rendered.contains("class=\"shot shot--zoom400\""));
        assert!(rendered.contains("class=\"shot shot--grid\""));
        assert!(rendered.contains("class=\"marker\""));
        assert!(rendered.contains("http://example.com/test.jpg"));
    }
}
