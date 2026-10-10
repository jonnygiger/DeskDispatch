use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use deskdispatch::{AppState, config::Config, routes::*};
use secrecy::ExposeSecret;
use sqlx::PgPool;
use tower::ServiceExt;

async fn get_test_pool() -> Option<PgPool> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });
    PgPool::connect(&db_url).await.ok()
}

async fn create_test_user(pool: &PgPool, username: &str, role: &str) -> (i64, String) {
    let row = sqlx::query(
        "INSERT INTO users (username, password_hash, display_name, role) VALUES ($1, 'hash', $2, $3) RETURNING id",
    )
    .bind(username)
    .bind(username)
    .bind(role)
    .fetch_one(pool)
    .await
    .unwrap();

    let user_id: i64 = sqlx::Row::get(&row, "id");

    let session_row = sqlx::query(
        "INSERT INTO sessions (user_id, expires_at) VALUES ($1, now() + interval '1 hour') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .unwrap();

    let session_id: uuid::Uuid = sqlx::Row::get(&session_row, "id");
    (user_id, session_id.to_string())
}

#[tokio::test]
async fn test_run_now_with_worker_group_and_parameter_overrides() {
    let Some(pool) = get_test_pool().await else {
        println!(
            "Database not available, skipping test_run_now_with_worker_group_and_parameter_overrides"
        );
        return;
    };

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

    let app = Router::new()
        .route(
            "/automations/{id}",
            axum::routing::get(get_automation_detail_handler),
        )
        .route(
            "/automations/{id}/run-now",
            axum::routing::post(post_run_now_automation_handler),
        )
        .route("/runs/{id}", axum::routing::get(get_run_detail_handler))
        .route(
            "/runs/{id}/status-frame",
            axum::routing::get(get_run_status_frame_handler),
        )
        .route(
            "/runs/{id}/cancel",
            axum::routing::post(post_cancel_run_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state);

    let (_admin_id, admin_session) = create_test_user(
        &pool,
        &format!("run_now_admin_{}", uuid::Uuid::new_v4().simple()),
        "admin",
    )
    .await;
    let csrf_token = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Create worker group
    let wg_row = sqlx::query(
        "INSERT INTO worker_groups (name, description) VALUES ($1, 'Test Group') RETURNING id",
    )
    .bind(format!("WG_{}", uuid::Uuid::new_v4().simple()))
    .fetch_one(&pool)
    .await
    .unwrap();
    let worker_group_id: i64 = sqlx::Row::get(&wg_row, "id");

    // 2. Create active automation
    let auto_row = sqlx::query("INSERT INTO automations (name, description, status, created_by) VALUES ('Overridden Automation', 'Desc', 'active', 1) RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    let automation_id: i64 = sqlx::Row::get(&auto_row, "id");

    // 3. Add parameter to automation
    sqlx::query("INSERT INTO automation_parameters (automation_id, name, param_type, default_value, description) VALUES ($1, 'max_retries', 'int', '3', 'Max Retry Count')")
        .bind(automation_id)
        .execute(&pool)
        .await
        .unwrap();

    // 4. Add step to automation (active step required for Run Now)
    let step_row = sqlx::query("INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, 10.0, 'key_press', 'Press Key', 0) RETURNING id")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let step_id: i64 = sqlx::Row::get(&step_row, "id");
    sqlx::query("INSERT INTO step_key_presses (step_id, key_combo) VALUES ($1, 'enter')")
        .bind(step_id)
        .execute(&pool)
        .await
        .unwrap();

    // 5. GET /automations/{id} detail page and check parameter and worker group inputs exist
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Run Now (Manual Trigger)"));
    assert!(body_str.contains("Target Worker Group"));
    assert!(body_str.contains("Parameter Overrides"));
    assert!(body_str.contains("max_retries"));

    // 6. POST /automations/{id}/run-now with target_worker_group_id and parameter override max_retries=10
    let run_now_body = format!(
        "worker_group_id={}&parameters[max_retries]=10&csrf_token={}",
        worker_group_id, csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/run-now", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(run_now_body))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let location = res
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        location.starts_with("/runs/"),
        "Expected redirect to /runs/{{id}}, got {}",
        location
    );

    let task_run_id: i64 = location.trim_start_matches("/runs/").parse().unwrap();

    // 7. Verify task_run DB row
    let run_db_row = sqlx::query("SELECT automation_id, target_worker_group_id, parameter_overrides, dispatched_automation_json, status FROM task_runs WHERE id = $1")
        .bind(task_run_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let db_auto_id: i64 = sqlx::Row::get(&run_db_row, "automation_id");
    let db_group_id: Option<i64> = sqlx::Row::get(&run_db_row, "target_worker_group_id");
    let db_overrides_json: Option<serde_json::Value> =
        sqlx::Row::get(&run_db_row, "parameter_overrides");
    let db_dispatched_json: Option<serde_json::Value> =
        sqlx::Row::get(&run_db_row, "dispatched_automation_json");
    let db_status: String = sqlx::Row::get(&run_db_row, "status");

    assert_eq!(db_auto_id, automation_id);
    assert_eq!(db_group_id, Some(worker_group_id));
    assert_eq!(db_status, "queued");

    let overrides_obj = db_overrides_json.expect("Expected parameter_overrides JSON");
    assert_eq!(
        overrides_obj.get("max_retries").and_then(|v| v.as_str()),
        Some("10")
    );

    let dispatched_obj = db_dispatched_json.expect("Expected dispatched_automation_json");
    let params_val = dispatched_obj
        .get("parameters")
        .expect("Expected parameters in dispatched json");
    assert_eq!(
        params_val.get("max_retries").and_then(|v| v.as_i64()),
        Some(10)
    );

    // 8. GET /runs/{task_run_id} and check execution parameters section rendered
    let req = Request::builder()
        .method("GET")
        .uri(format!("/runs/{}", task_run_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Execution Parameters"));
    assert!(body_str.contains("max_retries"));
    assert!(body_str.contains("10"));
    assert!(body_str.contains("Override"));

    // 9. Cancel task run (queued -> cancelled)
    let cancel_body = format!("csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/runs/{}/cancel", task_run_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(cancel_body.clone()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let cancel_status: String = sqlx::query_scalar("SELECT status FROM task_runs WHERE id = $1")
        .bind(task_run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(cancel_status, "cancelled");

    // 10. Test running -> cancelling state display
    let run2_row = sqlx::query("INSERT INTO task_runs (automation_id, status, queued_at, started_at) VALUES ($1, 'running', now(), now()) RETURNING id")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let task_run2_id: i64 = sqlx::Row::get(&run2_row, "id");

    // Cancel running task run -> transitions to 'cancelling'
    let req = Request::builder()
        .method("POST")
        .uri(format!("/runs/{}/cancel", task_run2_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(cancel_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let run2_status: String = sqlx::query_scalar("SELECT status FROM task_runs WHERE id = $1")
        .bind(task_run2_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(run2_status, "cancelling");

    // GET /runs/{id}/status-frame and verify "Cancelling..." is rendered
    let req = Request::builder()
        .method("GET")
        .uri(format!("/runs/{}/status-frame", task_run2_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Cancelling..."));
    assert!(body_str.contains("cancelling"));
}
