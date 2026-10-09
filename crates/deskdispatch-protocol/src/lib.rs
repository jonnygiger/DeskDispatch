use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterWorkerRequest {
    pub registration_token: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegisterWorkerResponse {
    pub status: String,
    pub worker_id: i64,
    pub hostname: String,
    pub display_name: String,
    pub api_key: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct HeartbeatRequest {
    pub status: Option<String>,
    pub current_task_run_id: Option<i64>,
    pub screen_width: Option<i32>,
    pub screen_height: Option<i32>,
    pub os_info: Option<String>,
    pub agent_version: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub struct HeartbeatResponse {
    pub status: String,
    pub cancel_requested: bool,
    pub poll_interval_secs: u64,
    pub heartbeat_interval_secs: u64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NextAssignmentResponse {
    None,
    ExecuteAutomation {
        task_run_id: i64,
        #[schema(value_type = Object)]
        automation: serde_json::Value,
    },
    StartRecording {
        recording_session_id: i64,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct TaskRunResponse {
    pub task_run_id: i64,
    pub status: String,
    #[schema(value_type = Object)]
    pub automation: serde_json::Value,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct VariableUpdateItem {
    pub variable_id: i64,
    #[schema(value_type = Object)]
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize, ToSchema)]
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

#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub struct StepResultResponse {
    pub status: String,
    pub step_result_id: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CompleteTaskRunRequest {
    pub status: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub struct CompleteTaskRunResponse {
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct ScreenshotUploadUrlResponse {
    pub upload_url: String,
    pub object_key: String,
}

#[derive(Debug, Deserialize, ToSchema)]
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

#[derive(Debug, Deserialize, ToSchema)]
pub struct PostRecordingEventsRequest {
    pub events: Vec<RecordingEventItem>,
}

#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub struct PostRecordingEventsResponse {
    pub status: String,
    pub count: usize,
    pub stop_requested: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub struct WorkerStopRecordingResponse {
    pub status: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CommitScreenshotRequest {
    pub step_id: Option<i64>,
    pub object_key: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub struct CommitScreenshotResponse {
    pub status: String,
    pub object_key: String,
}
