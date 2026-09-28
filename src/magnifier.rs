use askama::Template;
use serde::{Deserialize, Serialize};

/// ImageMagnifier struct representing the three-panel image magnifier component settings and calculations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

    pub fn show_marker(&self) -> bool {
        self.target_x.is_some() && self.target_y.is_some() && self.native_w > 0 && self.native_h > 0
    }

    /// Background style for normal panel (1x scale, viewport 600x340 or calculated fit)
    pub fn normal_style(&self) -> String {
        format!(
            "background-image: url('{}'); background-size: contain; background-position: center;",
            self.presigned_url
        )
    }

    /// Marker style for normal panel
    pub fn normal_marker_style(&self) -> String {
        let (tx, ty) = match (self.target_x, self.target_y) {
            (Some(x), Some(y)) => (x as f64, y as f64),
            _ => return String::new(),
        };

        if self.native_w == 0 || self.native_h == 0 {
            return String::new();
        }

        let scale_w = 600.0 / self.native_w as f64;
        let scale_h = 340.0 / self.native_h as f64;
        let scale = scale_w.min(scale_h);

        let rendered_w = self.native_w as f64 * scale;
        let rendered_h = self.native_h as f64 * scale;

        let offset_x = (600.0 - rendered_w) / 2.0;
        let offset_y = (340.0 - rendered_h) / 2.0;

        let marker_left = offset_x + (tx * scale);
        let marker_top = offset_y + (ty * scale);

        format!("left: {:.2}px; top: {:.2}px;", marker_left, marker_top)
    }

    /// Background style for 400% (4x) zoom panel (viewport 320x320)
    pub fn zoom400_style(&self) -> String {
        let bg_w = self.native_w * 4;
        let bg_h = self.native_h * 4;

        let (pan_x, pan_y) = match (self.target_x, self.target_y) {
            (Some(tx), Some(ty)) if self.native_w > 0 && self.native_h > 0 => {
                let px = 160.0 - (tx as f64 * 4.0);
                let py = 160.0 - (ty as f64 * 4.0);
                (px, py)
            }
            _ => (0.0, 0.0),
        };

        format!(
            "background-image: url('{}'); background-size: {}px {}px; background-position: {:.2}px {:.2}px;",
            self.presigned_url, bg_w, bg_h, pan_x, pan_y
        )
    }

    /// Marker style for 400% (4x) zoom panel (viewport 320x320)
    pub fn zoom400_marker_style(&self) -> String {
        if self.show_marker() {
            "left: 160px; top: 160px;".to_string()
        } else {
            String::new()
        }
    }

    /// Background style for 20x pixel grid panel (viewport 320x320)
    pub fn grid_style(&self) -> String {
        let bg_w = self.native_w * 20;
        let bg_h = self.native_h * 20;

        let (pan_x, pan_y) = match (self.target_x, self.target_y) {
            (Some(tx), Some(ty)) if self.native_w > 0 && self.native_h > 0 => {
                let px = 160.0 - (tx as f64 * 20.0 + 10.0);
                let py = 160.0 - (ty as f64 * 20.0 + 10.0);
                (px, py)
            }
            _ => (0.0, 0.0),
        };

        format!(
            "background-image: repeating-linear-gradient(to right, transparent 0 19px, rgba(128,128,128,.6) 19px 20px), repeating-linear-gradient(to bottom, transparent 0 19px, rgba(128,128,128,.6) 19px 20px), url('{}'); background-size: 20px 20px, 20px 20px, {}px {}px; background-position: 0 0, 0 0, {:.2}px {:.2}px;",
            self.presigned_url, bg_w, bg_h, pan_x, pan_y
        )
    }

    /// Marker style for 20x pixel grid panel (viewport 320x320)
    pub fn grid_marker_style(&self) -> String {
        if self.show_marker() {
            "left: 160px; top: 160px;".to_string()
        } else {
            String::new()
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
    fn test_image_magnifier_without_target() {
        let mag = ImageMagnifier::new("http://example.com/image.png", 1920, 1080, None, None);

        assert!(!mag.show_marker());
        assert!(mag.normal_marker_style().is_empty());
        assert!(mag.zoom400_marker_style().is_empty());
        assert!(mag.grid_marker_style().is_empty());

        assert!(mag.normal_style().contains("background-image: url('http://example.com/image.png')"));
        assert!(mag.zoom400_style().contains("background-size: 7680px 4320px"));
        assert!(mag.grid_style().contains("background-size: 20px 20px, 20px 20px, 38400px 21600px"));
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
        // pan_x = 160 - (100 * 4) = -240
        // pan_y = 160 - (200 * 4) = -640
        assert!(zoom400_style.contains("background-position: -240.00px -640.00px"));

        let grid_style = mag.grid_style();
        assert!(grid_style.contains("repeating-linear-gradient"));
        assert!(grid_style.contains("background-size: 20px 20px, 20px 20px, 20000px 10000px"));
        // pan_x = 160 - (100 * 20 + 10) = -1850
        // pan_y = 160 - (200 * 20 + 10) = -3850
        assert!(grid_style.contains("0 0, 0 0, -1850.00px -3850.00px"));
    }

    #[test]
    fn test_normal_and_zoom400_edge_cases() {
        // Zero dimensions with target
        let mag_zero = ImageMagnifier::new(
            "http://example.com/image.png",
            0,
            0,
            Some(100),
            Some(100),
        );
        assert!(!mag_zero.show_marker());
        assert_eq!(mag_zero.normal_marker_style(), "");
        assert_eq!(mag_zero.zoom400_marker_style(), "");
        assert!(mag_zero.zoom400_style().contains("background-size: 0px 0px"));
        assert!(mag_zero.zoom400_style().contains("background-position: 0.00px 0.00px"));

        // Exact math check for normal panel marker with 600x340 viewport
        // native 1200 x 680 (aspect ratio 600/340 = 1.7647...)
        // scale_w = 600/1200 = 0.5, scale_h = 340/680 = 0.5, scale = 0.5
        // rendered_w = 600, rendered_h = 340, offset_x = 0, offset_y = 0
        // target (400, 200) -> marker_left = 200.00, marker_top = 100.00
        let mag_normal = ImageMagnifier::new(
            "http://example.com/image.png",
            1200,
            680,
            Some(400),
            Some(200),
        );
        assert_eq!(mag_normal.normal_marker_style(), "left: 200.00px; top: 100.00px;");

        // Zoom 400% math check: native 800 x 600, target (100, 50)
        // bg_w = 3200, bg_h = 2400
        // pan_x = 160 - 100*4 = -240.00
        // pan_y = 160 - 50*4 = -40.00
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
    fn test_grid_panel_styling_and_overlays() {
        let css = include_str!("../static/style.css");
        assert!(css.contains("image-rendering: pixelated;"), "CSS must contain image-rendering: pixelated;");

        let mag = ImageMagnifier::new(
            "http://example.com/test.png",
            800,
            600,
            Some(10),
            Some(20),
        );

        let grid_style = mag.grid_style();
        assert!(grid_style.contains("repeating-linear-gradient(to right, transparent 0 19px, rgba(128,128,128,.6) 19px 20px)"));
        assert!(grid_style.contains("repeating-linear-gradient(to bottom, transparent 0 19px, rgba(128,128,128,.6) 19px 20px)"));
        assert!(grid_style.contains("background-size: 20px 20px, 20px 20px, 16000px 12000px"));
        assert!(grid_style.contains("0 0, 0 0, -50.00px -250.00px"));
    }

    #[test]
    fn test_template_rendering() {
        let mag = ImageMagnifier::new(
            "http://example.com/test.jpg",
            800,
            600,
            Some(50),
            Some(50),
        );

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
