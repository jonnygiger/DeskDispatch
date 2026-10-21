use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisterWorkerRequest {
    pub registration_token: Option<String>,
    pub token: Option<String>,
    pub psk: Option<String>,
    pub pre_shared_key: Option<String>,
    pub hostname: Option<String>,
    pub display_name: Option<String>,
    pub mac_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisterWorkerResponse {
    pub status: String,
    pub worker_id: i64,
    pub hostname: String,
    pub display_name: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeartbeatRequest {
    pub status: Option<String>,
    pub current_task_run_id: Option<i64>,
    pub screen_width: Option<i32>,
    pub screen_height: Option<i32>,
    pub os_info: Option<String>,
    pub agent_version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeartbeatResponse {
    pub status: String,
    pub cancel_requested: bool,
    pub poll_interval_secs: u64,
    pub heartbeat_interval_secs: u64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NextAssignmentResponse {
    None,
    ExecuteAutomation {
        task_run_id: i64,
        automation: serde_json::Value,
    },
    StartRecording {
        recording_session_id: i64,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRunResponse {
    pub task_run_id: i64,
    pub status: String,
    pub automation: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VariableUpdateItem {
    pub variable_id: i64,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepResultRequest {
    pub seq: Option<i32>,
    pub step_id: i64,
    pub result: String,
    pub captured_r: Option<i16>,
    pub captured_g: Option<i16>,
    pub captured_b: Option<i16>,
    pub captured_rgb: Option<Vec<i16>>,
    pub captured_found: Option<bool>,
    pub captured_x: Option<i32>,
    pub captured_y: Option<i32>,
    pub captured_xy: Option<Vec<i32>>,
    pub screenshot_object_key: Option<String>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    pub variable_updates: Option<Vec<VariableUpdateItem>>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepResultResponse {
    pub status: String,
    pub step_result_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompleteTaskRunRequest {
    pub status: String,
    pub error_message: Option<String>,
    pub output_type: Option<String>,
    pub output_value: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompleteTaskRunResponse {
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScreenshotUploadUrlResponse {
    pub upload_url: String,
    pub object_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecordingEventItem {
    pub sequence_number: i32,
    pub event_type: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub button: Option<String>,
    pub key_combo: Option<String>,
    pub screenshot_object_key: Option<String>,
    pub captured_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostRecordingEventsRequest {
    pub events: Vec<RecordingEventItem>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostRecordingEventsResponse {
    pub status: String,
    pub count: usize,
    pub stop_requested: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerStopRecordingResponse {
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitScreenshotRequest {
    pub step_id: Option<i64>,
    pub object_key: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitScreenshotResponse {
    pub status: String,
    pub object_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitBitmapRequest {
    pub automation_id: Option<i64>,
    pub name: String,
    pub object_key: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitBitmapResponse {
    pub status: String,
    pub bitmap_id: i64,
    pub object_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationVariablePayload {
    pub id: i64,
    pub name: String,
    pub var_type: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BranchConditionPayload {
    PixelRgb {
        x: Option<i32>,
        y: Option<i32>,
        expected_r: i16,
        expected_g: i16,
        expected_b: i16,
        expected_rgb: [i16; 3],
        tolerance: i16,
        timeout_ms: u64,
        retry_interval_ms: u64,
    },
    Bitmap {
        reference_bitmap_id: Option<i64>,
        reference_bitmap_key: Option<String>,
        search_x: Option<i32>,
        search_y: Option<i32>,
        search_width: Option<i32>,
        search_height: Option<i32>,
        match_threshold: Option<f32>,
        timeout_ms: u64,
        retry_interval_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepPayload {
    MouseClick {
        id: i64,
        label: Option<String>,
        post_delay_ms: i32,
        x: Option<i32>,
        y: Option<i32>,
        x_variable_id: Option<i64>,
        y_variable_id: Option<i64>,
        region_variable_id: Option<i64>,
        button: String,
        click_type: String,
    },
    KeyPress {
        id: i64,
        label: Option<String>,
        post_delay_ms: i32,
        key_combo: String,
    },
    FindPixelRgb {
        id: i64,
        label: Option<String>,
        post_delay_ms: i32,
        x: i32,
        y: i32,
        output_variable_id: Option<i64>,
        timeout_ms: u64,
        retry_interval_ms: u64,
    },
    FindBitmap {
        id: i64,
        label: Option<String>,
        post_delay_ms: i32,
        reference_bitmap_id: i64,
        reference_bitmap_key: Option<String>,
        search_x: Option<i32>,
        search_y: Option<i32>,
        search_width: Option<i32>,
        search_height: Option<i32>,
        match_threshold: f32,
        output_found_variable_id: Option<i64>,
        output_x_variable_id: Option<i64>,
        output_y_variable_id: Option<i64>,
        output_region_variable_id: Option<i64>,
        timeout_ms: u64,
        retry_interval_ms: u64,
    },
    Branch {
        id: i64,
        label: Option<String>,
        post_delay_ms: i32,
        condition: BranchConditionPayload,
        on_match_step_id: Option<i64>,
        on_no_match_step_id: Option<i64>,
        timeout_ms: u64,
        retry_interval_ms: u64,
    },
    OcrExtract {
        id: i64,
        label: Option<String>,
        post_delay_ms: i32,
        search_x: Option<i32>,
        search_y: Option<i32>,
        search_width: Option<i32>,
        search_height: Option<i32>,
        region_variable_id: Option<i64>,
        output_variable_id: Option<i64>,
        set_as_task_output: bool,
        timeout_ms: u64,
        retry_interval_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FullAutomationPayload {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub status: String,
    pub parameters: std::collections::HashMap<String, serde_json::Value>,
    pub variables: Vec<AutomationVariablePayload>,
    pub steps: Vec<StepPayload>,
}
