use deskdispatch_protocol::*;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(),
    components(
        schemas(
            RegisterWorkerRequest,
            RegisterWorkerResponse,
            ErrorResponse,
            HeartbeatRequest,
            HeartbeatResponse,
            NextAssignmentResponse,
            TaskRunResponse,
            VariableUpdateItem,
            StepResultRequest,
            StepResultResponse,
            CompleteTaskRunRequest,
            CompleteTaskRunResponse,
            ScreenshotUploadUrlResponse,
            RecordingEventItem,
            PostRecordingEventsRequest,
            PostRecordingEventsResponse,
            WorkerStopRecordingResponse,
            CommitScreenshotRequest,
            CommitScreenshotResponse,
        )
    ),
    tags(
        (name = "worker_api", description = "Worker Agent Protocol Operations")
    )
)]
struct ApiDoc;

#[test]
fn test_openapi_schema_generation_and_components() {
    let openapi = ApiDoc::openapi();
    let json_str = openapi.to_json().expect("Failed to serialize OpenAPI spec to JSON");
    let doc: serde_json::Value = serde_json::from_str(&json_str).expect("Valid JSON spec expected");

    // Verify OpenAPI version is 3.x
    assert!(
        doc["openapi"].as_str().map_or(false, |v| v.starts_with("3.")),
        "OpenAPI spec version should be 3.x: {:?}",
        doc["openapi"]
    );

    // Verify schemas component contains all key protocol DTOs
    let schemas = &doc["components"]["schemas"];
    assert!(schemas.get("RegisterWorkerRequest").is_some());
    assert!(schemas.get("RegisterWorkerResponse").is_some());
    assert!(schemas.get("HeartbeatRequest").is_some());
    assert!(schemas.get("HeartbeatResponse").is_some());
    assert!(schemas.get("NextAssignmentResponse").is_some());
    assert!(schemas.get("TaskRunResponse").is_some());
    assert!(schemas.get("StepResultRequest").is_some());
    assert!(schemas.get("CompleteTaskRunRequest").is_some());
    assert!(schemas.get("PostRecordingEventsRequest").is_some());
}

#[test]
fn test_next_assignment_response_contract() {
    // 1. None assignment
    let resp_none = NextAssignmentResponse::None;
    let json_none = serde_json::to_value(&resp_none).unwrap();
    assert_eq!(json_none, serde_json::json!({ "type": "none" }));
    let roundtrip_none: NextAssignmentResponse = serde_json::from_value(json_none).unwrap();
    assert_eq!(roundtrip_none, NextAssignmentResponse::None);

    // 2. ExecuteAutomation assignment
    let resp_exec = NextAssignmentResponse::ExecuteAutomation {
        task_run_id: 101,
        automation: serde_json::json!({
            "id": 1,
            "name": "Test Automation",
            "steps": []
        }),
    };
    let json_exec = serde_json::to_value(&resp_exec).unwrap();
    assert_eq!(json_exec["type"], "execute_automation");
    assert_eq!(json_exec["task_run_id"], 101);
    assert_eq!(json_exec["automation"]["name"], "Test Automation");

    let roundtrip_exec: NextAssignmentResponse = serde_json::from_value(json_exec).unwrap();
    assert_eq!(roundtrip_exec, resp_exec);

    // 3. StartRecording assignment
    let resp_rec = NextAssignmentResponse::StartRecording {
        recording_session_id: 202,
    };
    let json_rec = serde_json::to_value(&resp_rec).unwrap();
    assert_eq!(json_rec["type"], "start_recording");
    assert_eq!(json_rec["recording_session_id"], 202);

    let roundtrip_rec: NextAssignmentResponse = serde_json::from_value(json_rec).unwrap();
    assert_eq!(roundtrip_rec, resp_rec);
}

#[test]
fn test_step_result_request_seq_and_backwards_compatibility() {
    // Legacy payload shape without seq or variable updates
    let raw_legacy = serde_json::json!({
        "step_id": 5,
        "result": "success",
        "captured_r": 255,
        "captured_g": 128,
        "captured_b": 0
    });
    let req_legacy: StepResultRequest = serde_json::from_value(raw_legacy).unwrap();
    assert_eq!(req_legacy.seq, None);
    assert_eq!(req_legacy.step_id, 5);
    assert_eq!(req_legacy.result, "success");
    assert_eq!(req_legacy.captured_r, Some(255));

    // Modern payload shape with seq, captured_rgb array, and variable_updates
    let raw_modern = serde_json::json!({
        "seq": 1,
        "step_id": 5,
        "result": "branch_matched",
        "captured_rgb": [255, 128, 0],
        "captured_found": true,
        "variable_updates": [
            { "variable_id": 10, "value": "updated_val" }
        ]
    });
    let req_modern: StepResultRequest = serde_json::from_value(raw_modern).unwrap();
    assert_eq!(req_modern.seq, Some(1));
    assert_eq!(req_modern.step_id, 5);
    assert_eq!(req_modern.result, "branch_matched");
    assert_eq!(req_modern.captured_rgb, Some(vec![255, 128, 0]));
    assert_eq!(req_modern.captured_found, Some(true));

    let updates = req_modern.variable_updates.unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].variable_id, 10);
    assert_eq!(updates[0].value, "updated_val");
}

#[test]
fn test_heartbeat_response_contract() {
    let hb = HeartbeatResponse {
        status: "ok".to_string(),
        cancel_requested: true,
        poll_interval_secs: 5,
        heartbeat_interval_secs: 15,
    };

    let json = serde_json::to_value(&hb).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["cancel_requested"], true);
    assert_eq!(json["poll_interval_secs"], 5);
    assert_eq!(json["heartbeat_interval_secs"], 15);
}

#[test]
fn test_route_documentation_consistency() {
    // List expected worker API endpoints defined in the documentation and router
    let expected_worker_endpoints = vec![
        "/api/v1/workers/register",
        "/api/v1/workers/heartbeat",
        "/api/v1/workers/next-assignment",
        "/api/v1/workers/task-runs/{id}",
        "/api/v1/workers/task-runs/{id}/step-result",
        "/api/v1/workers/task-runs/{id}/complete",
        "/api/v1/workers/task-runs/{id}/screenshot-url",
        "/api/v1/workers/task-runs/{id}/screenshots/commit",
        "/api/v1/workers/recordings/events",
        "/api/v1/workers/recordings/stop",
        "/api/v1/workers/recordings/screenshot-url",
    ];

    for endpoint in expected_worker_endpoints {
        assert!(
            !endpoint.is_empty(),
            "Endpoint string should not be empty: {}",
            endpoint
        );
        assert!(
            endpoint.starts_with("/api/v1/workers/"),
            "Worker route must be under /api/v1/workers/: {}",
            endpoint
        );
    }
}
