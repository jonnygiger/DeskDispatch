use askama::Template;
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use chrono::Utc;
use deskdispatch::AppState;
use deskdispatch::auth::{AuthUser, UserRole, generate_csrf_token};
use deskdispatch::config::Config;
use deskdispatch::routes::workers::*;
use secrecy::ExposeSecret;
use tower::ServiceExt;
use uuid::Uuid;

#[test]
fn test_worker_status_badges() {
    let item = WorkerPcItem {
        id: 1,
        hostname: "host-1".to_string(),
        display_name: "Worker 1".to_string(),
        status: "online".to_string(),
        last_heartbeat_at: Some(Utc::now()),
        screen_width: Some(1920),
        screen_height: Some(1080),
        os_info: Some("Linux".to_string()),
        agent_version: Some("1.0.0".to_string()),
        created_at: Utc::now(),
        groups: vec!["Group A".to_string()],
    };
    assert_eq!(item.status_badge_class(), "badge-success");

    let busy_item = WorkerPcItem {
        status: "busy".to_string(),
        ..item.clone()
    };
    assert_eq!(busy_item.status_badge_class(), "badge-warning");

    let error_item = WorkerPcItem {
        status: "error".to_string(),
        ..item.clone()
    };
    assert_eq!(error_item.status_badge_class(), "badge-danger");

    let offline_item = WorkerPcItem {
        status: "offline".to_string(),
        ..item
    };
    assert_eq!(offline_item.status_badge_class(), "badge-neutral");
}

#[test]
fn test_worker_templates_rendering() {
    let user = AuthUser {
        id: 10,
        username: "adminuser".to_string(),
        display_name: "Admin User".to_string(),
        role: UserRole::Admin,
        session_id: Uuid::new_v4(),
        csrf_token: "test_csrf_token_123".to_string(),
        must_change_password: false,
    };

    let worker_item = WorkerPcItem {
        id: 101,
        hostname: "pc-warehouse-01".to_string(),
        display_name: "Warehouse Worker PC".to_string(),
        status: "online".to_string(),
        last_heartbeat_at: Some(Utc::now()),
        screen_width: Some(1920),
        screen_height: Some(1080),
        os_info: Some("Windows 11".to_string()),
        agent_version: Some("v2.1.0".to_string()),
        created_at: Utc::now(),
        groups: vec!["Warehouse Fleet".to_string()],
    };

    let group_item = WorkerGroupItem {
        id: 501,
        name: "Warehouse Fleet".to_string(),
        description: "PCs located on warehouse floor".to_string(),
        member_count: 1,
        member_names: vec!["Warehouse Worker PC".to_string()],
    };

    // 1. Index Template
    let index_tmpl = WorkersIndexTemplate {
        user: user.clone(),
        workers: vec![worker_item.clone()],
        worker_groups: vec![group_item],
    };
    let index_html = index_tmpl.render().unwrap();
    assert!(index_html.contains("Task Worker PCs"));
    assert!(index_html.contains("Warehouse Worker PC"));
    assert!(index_html.contains("pc-warehouse-01"));
    assert!(index_html.contains("Warehouse Fleet"));

    // 2. Detail Template
    let worker_detail = WorkerDetail {
        id: 101,
        hostname: "pc-warehouse-01".to_string(),
        display_name: "Warehouse Worker PC".to_string(),
        status: "online".to_string(),
        last_heartbeat_at: Some(Utc::now()),
        screen_width: Some(1920),
        screen_height: Some(1080),
        os_info: Some("Windows 11".to_string()),
        agent_version: Some("v2.1.0".to_string()),
        created_at: Utc::now(),
        groups: vec![WorkerGroupSimple {
            id: 501,
            name: "Warehouse Fleet".to_string(),
            description: "PCs located on warehouse floor".to_string(),
        }],
        registration_token: None,
    };

    let detail_tmpl = WorkerDetailTemplate {
        user: user.clone(),
        worker: worker_detail.clone(),
        error: None,
    };
    let detail_html = detail_tmpl.render().unwrap();
    assert!(detail_html.contains("Warehouse Worker PC"));
    assert!(detail_html.contains("1920 &times; 1080"));
    assert!(detail_html.contains("Windows 11"));

    // 3. Edit Template
    let edit_tmpl = WorkerEditTemplate {
        user: user.clone(),
        worker: worker_detail,
        all_groups: vec![WorkerGroupSimple {
            id: 501,
            name: "Warehouse Fleet".to_string(),
            description: "PCs located on warehouse floor".to_string(),
        }],
        error: None,
    };
    assert!(edit_tmpl.is_group_selected(&501));
    assert!(!edit_tmpl.is_group_selected(&999));
    let edit_html = edit_tmpl.render().unwrap();
    assert!(edit_html.contains("Edit Worker PC"));
    assert!(edit_html.contains("checked"));

    // 4. Group Form Template
    let group_form_tmpl = WorkerGroupFormTemplate {
        user,
        group_id: Some(501),
        name: "Warehouse Fleet".to_string(),
        description: "PCs located on warehouse floor".to_string(),
        member_worker_ids: vec![101],
        all_workers: vec![WorkerSimple {
            id: 101,
            hostname: "pc-warehouse-01".to_string(),
            display_name: "Warehouse Worker PC".to_string(),
        }],
        error: None,
        is_edit: true,
    };
    assert!(group_form_tmpl.is_worker_selected(&101));
    assert!(!group_form_tmpl.is_worker_selected(&999));
    let group_form_html = group_form_tmpl.render().unwrap();
    assert!(group_form_html.contains("Edit Worker Group: Warehouse Fleet"));
    assert!(group_form_html.contains("checked"));
}

async fn get_test_pool() -> Option<sqlx::PgPool> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });
    sqlx::PgPool::connect(&db_url).await.ok()
}

#[tokio::test]
async fn test_admin_workers_crud_and_rbac() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping workers CRUD integration test");
        return;
    };

    // 1. Seed Admin user and Non-Admin (Editor) user
    let admin_username = format!("admin_{}", Uuid::new_v4().simple());
    let editor_username = format!("editor_{}", Uuid::new_v4().simple());

    let admin_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO users (username, password_hash, display_name, role, is_active)
        VALUES ($1, 'hash', 'Admin Tester', 'admin', true)
        RETURNING id
        "#,
    )
    .bind(&admin_username)
    .fetch_one(&pool)
    .await
    .unwrap();

    let editor_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO users (username, password_hash, display_name, role, is_active)
        VALUES ($1, 'hash', 'Editor Tester', 'editor', true)
        RETURNING id
        "#,
    )
    .bind(&editor_username)
    .fetch_one(&pool)
    .await
    .unwrap();

    // Create active sessions in DB
    let admin_session_id = Uuid::new_v4();
    let editor_session_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, now() + interval '1 day')",
    )
    .bind(admin_session_id)
    .bind(admin_id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, now() + interval '1 day')",
    )
    .bind(editor_session_id)
    .bind(editor_id)
    .execute(&pool)
    .await
    .unwrap();

    let admin_cookie = format!("session_id={}", admin_session_id);
    let editor_cookie = format!("session_id={}", editor_session_id);

    // Create a test worker PC directly in database
    let worker_hostname = format!("test-pc-{}", Uuid::new_v4().simple());
    let worker_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO task_worker_pcs (hostname, display_name, api_key_hash, status)
        VALUES ($1, 'Initial Worker', '\x00'::bytea, 'online')
        RETURNING id
        "#,
    )
    .bind(&worker_hostname)
    .fetch_one(&pool)
    .await
    .unwrap();

    let config = Config::from_env().unwrap();
    let credentials = aws_sdk_s3::config::Credentials::new("key", "secret", None, None, "static");
    let s3_config = aws_sdk_s3::config::Builder::new()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .credentials_provider(credentials)
        .region(aws_sdk_s3::config::Region::new("us-east-1"))
        .build();
    let s3_client = aws_sdk_s3::Client::from_conf(s3_config);

    let state = AppState {
        db: pool.clone(),
        s3_client,
        config: config.clone(),
        rate_limiter: deskdispatch::auth::LoginRateLimiter::default(),
        task_queue_notifier: tokio::sync::broadcast::channel::<()>(100).0,
    };

    let app = deskdispatch::build_router(state);

    // --- RBAC Test: Non-admin editor user attempts to access /workers ---
    let req = Request::builder()
        .uri("/workers")
        .header(header::COOKIE, &editor_cookie)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // --- Admin access: GET /workers ---
    let req = Request::builder()
        .uri("/workers")
        .header(header::COOKIE, &admin_cookie)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body_bytes);
    assert!(html.contains("Initial Worker"));

    // --- Admin access: GET /workers/{id} ---
    let req = Request::builder()
        .uri(format!("/workers/{}", worker_id))
        .header(header::COOKIE, &admin_cookie)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // --- Admin access: POST /worker-groups (Create group) ---
    let csrf_token = generate_csrf_token(admin_session_id, config.session_secret.expose_secret());
    let group_form = format!(
        "csrf_token={}&name=Test+Group+Alpha&description=Testing+group&worker_ids={}",
        csrf_token, worker_id
    );

    let req = Request::builder()
        .method("POST")
        .uri("/worker-groups")
        .header(header::COOKIE, &admin_cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(group_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    // Fetch created group ID
    let group_id: i64 = sqlx::query_scalar("SELECT id FROM worker_groups WHERE name = $1")
        .bind("Test Group Alpha")
        .fetch_one(&pool)
        .await
        .unwrap();

    // Check group member assigned
    let member_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM worker_group_members WHERE group_id = $1 AND worker_id = $2",
    )
    .bind(group_id)
    .bind(worker_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(member_count, 1);

    // --- Admin access: POST /workers/{id}/edit ---
    let edit_form = format!(
        "csrf_token={}&hostname=updated-host-01&display_name=Updated+Worker&group_ids={}",
        csrf_token, group_id
    );

    let req = Request::builder()
        .method("POST")
        .uri(format!("/workers/{}/edit", worker_id))
        .header(header::COOKIE, &admin_cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(edit_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    // Verify worker updated in DB
    let updated_display: String =
        sqlx::query_scalar("SELECT display_name FROM task_worker_pcs WHERE id = $1")
            .bind(worker_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(updated_display, "Updated Worker");

    // --- Admin access: POST /worker-groups/{id}/delete ---
    let del_group_form = format!("csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/worker-groups/{}/delete", group_id))
        .header(header::COOKIE, &admin_cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(del_group_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    // Verify group deleted
    let group_exists: Option<i64> =
        sqlx::query_scalar("SELECT id FROM worker_groups WHERE id = $1")
            .bind(group_id)
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert!(group_exists.is_none());

    // --- Admin access: POST /workers/{id}/delete ---
    let del_worker_form = format!("csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/workers/{}/delete", worker_id))
        .header(header::COOKIE, &admin_cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(del_worker_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    // Verify worker deleted
    let worker_exists: Option<i64> =
        sqlx::query_scalar("SELECT id FROM task_worker_pcs WHERE id = $1")
            .bind(worker_id)
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert!(worker_exists.is_none());

    // Cleanup users
    let _ = sqlx::query("DELETE FROM users WHERE id IN ($1, $2)")
        .bind(admin_id)
        .bind(editor_id)
        .execute(&pool)
        .await;
}
