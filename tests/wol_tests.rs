#![allow(clippy::unwrap_used)]

use askama::Template;
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use deskdispatch::AppState;
use deskdispatch::auth::{AuthUser, UserRole, generate_csrf_token};
use deskdispatch::config::Config;
use deskdispatch::routes::schedules::*;
use deskdispatch::wol::{create_magic_packet, parse_mac_address};
use secrecy::ExposeSecret;
use tower::ServiceExt;
use uuid::Uuid;

#[test]
fn test_wol_mac_parsing_and_packet_building() {
    let mac_colon = parse_mac_address("00:11:22:33:44:55").unwrap();
    assert_eq!(mac_colon, [0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);

    let mac_hyphen = parse_mac_address("AA-BB-CC-DD-EE-FF").unwrap();
    assert_eq!(mac_hyphen, [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);

    let mac_raw = parse_mac_address("1234567890ab").unwrap();
    assert_eq!(mac_raw, [0x12, 0x34, 0x56, 0x78, 0x90, 0xAB]);

    assert!(parse_mac_address("invalid_mac").is_err());
    assert!(parse_mac_address("00:11:22").is_err());

    let packet = create_magic_packet(&mac_colon);
    assert_eq!(packet.len(), 102);
    assert_eq!(&packet[..6], &[0xFF; 6]);
    for i in 0..16 {
        assert_eq!(&packet[6 + i * 6..6 + i * 6 + 6], &mac_colon);
    }
}

#[test]
fn test_schedule_wol_template_rendering() {
    let user = AuthUser {
        id: 1,
        username: "editor".to_string(),
        display_name: "Editor User".to_string(),
        role: UserRole::Editor,
        session_id: Uuid::new_v4(),
        csrf_token: "test_token".to_string(),
        must_change_password: false,
    };

    let tmpl = ScheduleFormTemplate {
        user,
        is_edit: false,
        schedule_id: None,
        automation_id: Some(10),
        name: "Morning Backup with WOL".to_string(),
        cron_expression: "0 8 * * MON-FRI".to_string(),
        timezone: "UTC".to_string(),
        worker_group_id: None,
        overlap_policy: "allow".to_string(),
        max_queue_age_secs: Some(3600),
        wol_enabled: true,
        wol_warmup_secs: 600,
        is_enabled: true,
        automations: vec![],
        worker_groups: vec![],
        error: None,
    };

    let html = tmpl.render().expect("Schedule form template should render");
    assert!(html.contains("Enable Wake-on-LAN (WOL) Boot Warm-up"));
    assert!(html.contains("value=\"600\""));
    assert!(html.contains("checked"));
}

async fn get_test_pool() -> Option<sqlx::PgPool> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });
    sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(500))
        .connect(&db_url)
        .await
        .ok()
}

#[tokio::test]
async fn test_worker_mac_and_manual_wol_wake() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping WOL integration test");
        return;
    };

    let admin_username = format!("admin_wol_{}", Uuid::new_v4().simple());
    let admin_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO users (username, password_hash, display_name, role, is_active)
        VALUES ($1, 'hash', 'WOL Admin', 'admin', true)
        RETURNING id
        "#,
    )
    .bind(&admin_username)
    .fetch_one(&pool)
    .await
    .unwrap();

    let admin_session_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, now() + interval '1 day')",
    )
    .bind(admin_session_id)
    .bind(admin_id)
    .execute(&pool)
    .await
    .unwrap();

    let admin_cookie = format!("session_id={}", admin_session_id);

    // Create worker with MAC address
    let worker_hostname = format!("wol-pc-{}", Uuid::new_v4().simple());
    let worker_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO task_worker_pcs (hostname, display_name, mac_address, status)
        VALUES ($1, 'WOL Target PC', '00:11:22:33:44:55', 'offline')
        RETURNING id
        "#,
    )
    .bind(&worker_hostname)
    .fetch_one(&pool)
    .await
    .unwrap();

    let config = Config::from_env().unwrap();
    let storage = deskdispatch::storage::StorageService::new(&config.s3_bucket).with_credentials(
        "key",
        "secret",
        "us-east-1",
        None::<String>,
    );

    let state = AppState {
        db: pool.clone(),
        storage,
        config: config.clone(),
        rate_limiter: deskdispatch::auth::LoginRateLimiter::default(),
        task_queue_notifier: tokio::sync::broadcast::channel::<()>(100).0,
        wps_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    let app = deskdispatch::build_router(state);

    // POST /workers/{id}/wake
    let csrf_token = generate_csrf_token(admin_session_id, config.session_secret.expose_secret());
    let wake_form = format!("csrf_token={}", csrf_token);

    let req = Request::builder()
        .method("POST")
        .uri(format!("/workers/{}/wake", worker_id))
        .header(header::COOKIE, &admin_cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(wake_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.contains("wake_success=true"));

    // Clean up
    let _ = sqlx::query("DELETE FROM task_worker_pcs WHERE id = $1")
        .bind(worker_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(admin_id)
        .execute(&pool)
        .await;
}
