#![allow(clippy::unwrap_used)]
use deskdispatch_protocol::*;

#[test]
fn test_protocol_dto_serialization_roundtrip() {
    // 1. HeartbeatRequest roundtrip & field checks
    let hb_req = HeartbeatRequest {
        status: Some("online".to_string()),
        current_task_run_id: Some(100),
        screen_width: Some(1920),
        screen_height: Some(1080),
        os_info: Some("Linux x86_64".to_string()),
        agent_version: Some("1.2.3".to_string()),
    };

    let json_val = serde_json::to_value(&hb_req).unwrap();
    assert_eq!(json_val["status"], "online");
    assert_eq!(json_val["current_task_run_id"], 100);
    assert_eq!(json_val["screen_width"], 1920);
    assert_eq!(json_val["screen_height"], 1080);
    assert_eq!(json_val["os_info"], "Linux x86_64");
    assert_eq!(json_val["agent_version"], "1.2.3");

    let json_str = serde_json::to_string(&hb_req).unwrap();
    let hb_roundtrip: HeartbeatRequest = serde_json::from_str(&json_str).unwrap();
    assert_eq!(hb_roundtrip, hb_req);

    // 2. RegisterWorkerRequest & RegisterWorkerResponse roundtrips
    let reg_req = RegisterWorkerRequest {
        registration_token: Some("dd_reg_abcdef".to_string()),
        token: None,
        psk: Some("secret_psk_123".to_string()),
        pre_shared_key: None,
        hostname: Some("worker-node-01".to_string()),
        display_name: Some("Worker Node 01".to_string()),
        mac_address: Some("00:11:22:33:44:55".to_string()),
    };
    let reg_req_str = serde_json::to_string(&reg_req).unwrap();
    let reg_req_roundtrip: RegisterWorkerRequest = serde_json::from_str(&reg_req_str).unwrap();
    assert_eq!(reg_req_roundtrip, reg_req);

    let reg_resp = RegisterWorkerResponse {
        status: "ok".to_string(),
        worker_id: 42,
        hostname: "worker-node-01".to_string(),
        display_name: "Worker Node 01".to_string(),
        api_key: "dd_pk_123456".to_string(),
    };
    let reg_resp_str = serde_json::to_string(&reg_resp).unwrap();
    let reg_resp_roundtrip: RegisterWorkerResponse = serde_json::from_str(&reg_resp_str).unwrap();
    assert_eq!(reg_resp_roundtrip, reg_resp);

    // 3. StepResultRequest roundtrip
    let step_req = StepResultRequest {
        seq: Some(5),
        step_id: 12,
        result: "success".to_string(),
        captured_r: Some(255),
        captured_g: Some(128),
        captured_b: Some(0),
        captured_rgb: Some(vec![255, 128, 0]),
        captured_found: Some(true),
        captured_x: Some(960),
        captured_y: Some(540),
        captured_xy: Some(vec![960, 540]),
        screenshot_object_key: Some("screenshots/run_1/step_12.png".to_string()),
        started_at: None,
        completed_at: None,
        timestamp: None,
        variable_updates: Some(vec![VariableUpdateItem {
            variable_id: 101,
            value: serde_json::json!("updated_value"),
        }]),
    };
    let step_req_str = serde_json::to_string(&step_req).unwrap();
    let step_req_roundtrip: StepResultRequest = serde_json::from_str(&step_req_str).unwrap();
    assert_eq!(step_req_roundtrip, step_req);

    // 4. CompleteTaskRunRequest & Response roundtrips
    let complete_req = CompleteTaskRunRequest {
        status: "completed".to_string(),
        error_message: None,
        output_type: Some("ocr_text".to_string()),
        output_value: Some("Invoice #9821".to_string()),
    };
    let complete_req_str = serde_json::to_string(&complete_req).unwrap();
    let complete_req_roundtrip: CompleteTaskRunRequest =
        serde_json::from_str(&complete_req_str).unwrap();
    assert_eq!(complete_req_roundtrip, complete_req);

    // 5. CommitScreenshotRequest & CommitBitmapRequest roundtrips
    let commit_ss_req = CommitScreenshotRequest {
        step_id: Some(12),
        object_key: Some("screenshots/step_12.png".to_string()),
        width: Some(1920),
        height: Some(1080),
    };
    let commit_ss_str = serde_json::to_string(&commit_ss_req).unwrap();
    let commit_ss_roundtrip: CommitScreenshotRequest =
        serde_json::from_str(&commit_ss_str).unwrap();
    assert_eq!(commit_ss_roundtrip, commit_ss_req);

    let commit_bm_req = CommitBitmapRequest {
        automation_id: Some(1),
        name: "Login Button".to_string(),
        object_key: "bitmaps/login.png".to_string(),
        width: Some(120),
        height: Some(40),
    };
    let commit_bm_str = serde_json::to_string(&commit_bm_req).unwrap();
    let commit_bm_roundtrip: CommitBitmapRequest = serde_json::from_str(&commit_bm_str).unwrap();
    assert_eq!(commit_bm_roundtrip, commit_bm_req);

    // 6. PostRecordingEventsRequest roundtrip
    let post_rec_req = PostRecordingEventsRequest {
        events: vec![RecordingEventItem {
            sequence_number: 1,
            event_type: "click".to_string(),
            x: Some(100),
            y: Some(200),
            button: Some("left".to_string()),
            key_combo: None,
            screenshot_object_key: None,
            captured_at: None,
        }],
    };
    let post_rec_str = serde_json::to_string(&post_rec_req).unwrap();
    let post_rec_roundtrip: PostRecordingEventsRequest =
        serde_json::from_str(&post_rec_str).unwrap();
    assert_eq!(post_rec_roundtrip, post_rec_req);

    // 7. FullAutomationPayload roundtrip
    let mut params = std::collections::HashMap::new();
    params.insert("timeout".to_string(), serde_json::json!(5000));
    let full_auto = FullAutomationPayload {
        id: 7,
        name: "E2E Test Flow".to_string(),
        description: "Automated test flow".to_string(),
        status: "active".to_string(),
        parameters: params,
        variables: vec![AutomationVariablePayload {
            id: 1,
            name: "success".to_string(),
            var_type: "boolean".to_string(),
            description: "Default success status".to_string(),
        }],
        steps: vec![
            StepPayload::KeyPress {
                id: 10,
                label: Some("Press Enter".to_string()),
                post_delay_ms: 100,
                key_combo: "Enter".to_string(),
            },
            StepPayload::OcrExtract {
                id: 11,
                label: Some("Extract Amount".to_string()),
                post_delay_ms: 200,
                search_x: Some(100),
                search_y: Some(200),
                search_width: Some(300),
                search_height: Some(150),
                region_variable_id: None,
                output_variable_id: Some(1),
                set_as_task_output: true,
                timeout_ms: 5000,
                retry_interval_ms: 250,
            },
            StepPayload::TypeText {
                id: 11,
                label: Some("Type Welcome".to_string()),
                post_delay_ms: 200,
                text_to_type: "Hello World".to_string(),
                delay_between_keys_ms: 50,
            },
            StepPayload::WindowControl {
                id: 12,
                label: Some("Focus Window".to_string()),
                post_delay_ms: 150,
                action: "activate".to_string(),
                match_pattern: "Firefox".to_string(),
                match_type: "contains".to_string(),
                timeout_ms: 5000,
            },
            StepPayload::ClipboardControl {
                id: 13,
                label: Some("Set Clipboard".to_string()),
                post_delay_ms: 100,
                action: "set_text".to_string(),
                text_to_set: Some("Copied Text".to_string()),
                input_variable_id: None,
                output_variable_id: Some(2),
                set_as_task_output: false,
                timeout_ms: 5000,
            },
            StepPayload::MouseScroll {
                id: 14,
                label: Some("Scroll Content".to_string()),
                post_delay_ms: 100,
                direction: "down".to_string(),
                scroll_amount: 5,
                x: Some(400),
                y: Some(300),
                x_variable_id: None,
                y_variable_id: None,
                region_variable_id: None,
            },
        ],
    };
    let full_auto_str = serde_json::to_string(&full_auto).unwrap();
    let full_auto_roundtrip: FullAutomationPayload = serde_json::from_str(&full_auto_str).unwrap();
    assert_eq!(full_auto_roundtrip, full_auto);
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
        automation: FullAutomationPayload {
            id: 1,
            name: "Test Automation".to_string(),
            description: String::new(),
            status: "active".to_string(),
            parameters: std::collections::HashMap::new(),
            variables: Vec::new(),
            steps: Vec::new(),
        },
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
