use deskdispatch_worker::config::{CliArgs, WorkerConfig};
use deskdispatch_worker::engine::bitmap::search_bitmap;
use deskdispatch_worker::engine::capture::CapturedImage;
use deskdispatch_worker::service::SystemdServiceManager;
use std::path::Path;

#[test]
fn test_worker_config_default_and_loading() {
    let args = CliArgs {
        command: None,
        config: None,
        server_url: Some("http://192.168.1.50:3000".to_string()),
        psk: Some("secret_psk_123".to_string()),
        token: None,
        api_key: None,
        hostname: Some("debian-node-01".to_string()),
        display_name: Some("Debian Worker 01".to_string()),
        display: ":1".to_string(),
        log_level: "debug".to_string(),
    };

    let config = WorkerConfig::load(&args).expect("Failed to load worker config");
    assert_eq!(config.server_url, "http://192.168.1.50:3000");
    assert_eq!(config.psk.as_deref(), Some("secret_psk_123"));
    assert_eq!(config.hostname, "debian-node-01");
    assert_eq!(config.display_name, "Debian Worker 01");
    assert_eq!(config.display, ":1");
    assert_eq!(config.log_level, "debug");
}

#[test]
#[allow(clippy::chunks_exact_to_as_chunks)]
fn test_image_capture_and_bitmap_search() {
    // Construct 100x100 synthetic screen image
    let mut screen_data = vec![0u8; 100 * 100 * 4];
    // Fill background with black
    for chunk in screen_data.chunks_exact_mut(4) {
        chunk[0] = 10;
        chunk[1] = 10;
        chunk[2] = 10;
        chunk[3] = 255;
    }

    // Place a 10x10 red box at coordinate (20, 30)
    for y in 30..40 {
        for x in 20..30 {
            let idx = ((y * 100 + x) * 4) as usize;
            screen_data[idx] = 255;
            screen_data[idx + 1] = 0;
            screen_data[idx + 2] = 0;
            screen_data[idx + 3] = 255;
        }
    }

    let screen = CapturedImage::new(100, 100, screen_data);

    // Create 10x10 template image of red box
    let mut template_data = vec![0u8; 10 * 10 * 4];
    for chunk in template_data.chunks_exact_mut(4) {
        chunk[0] = 255;
        chunk[1] = 0;
        chunk[2] = 0;
        chunk[3] = 255;
    }
    let template = CapturedImage::new(10, 10, template_data);

    let match_res = search_bitmap(&screen, &template, None, None, None, None, 0.9);
    assert!(match_res.found);
    assert_eq!(match_res.x, 20);
    assert_eq!(match_res.y, 30);
    assert!(match_res.score >= 0.99);
}

#[test]
fn test_systemd_unit_file_generation() {
    let manager = SystemdServiceManager::new("deskdispatch-worker-test", "deskdispatch");
    let binary_path = Path::new("/usr/local/bin/deskdispatch-worker");
    let config_path = Path::new("/etc/deskdispatch-worker/config.json");

    let unit_content = manager.generate_unit_file(binary_path, Some(config_path));
    assert!(unit_content.contains("Description=DeskDispatch Task Worker PC Client Service"));
    assert!(unit_content.contains("User=deskdispatch"));
    assert!(unit_content.contains("ExecStart=/usr/local/bin/deskdispatch-worker --config /etc/deskdispatch-worker/config.json run"));
    assert!(unit_content.contains("Environment=\"DISPLAY=:0\""));
}
