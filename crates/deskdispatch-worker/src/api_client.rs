use deskdispatch_protocol::*;
use reqwest::{Client, StatusCode};
use std::time::Duration;

#[derive(Clone)]
pub struct WorkerApiClient {
    pub client: Client,
    pub server_url: String,
    pub api_key: Option<String>,
}

impl WorkerApiClient {
    pub fn new(server_url: &str, api_key: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            server_url: server_url.trim_end_matches('/').to_string(),
            api_key,
        }
    }

    pub fn set_api_key(&mut self, api_key: String) {
        self.api_key = Some(api_key);
    }

    pub async fn register(
        &self,
        token: Option<&str>,
        psk: Option<&str>,
        hostname: &str,
        display_name: &str,
    ) -> Result<RegisterWorkerResponse, String> {
        let url = format!("{}/api/v1/workers/register", self.server_url);
        let req = RegisterWorkerRequest {
            registration_token: token.map(|s| s.to_string()),
            token: None,
            psk: psk.map(|s| s.to_string()),
            pre_shared_key: None,
            hostname: Some(hostname.to_string()),
            display_name: Some(display_name.to_string()),
            mac_address: None,
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| format!("Registration HTTP request failed: {}", e))?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(format!("Registration HTTP error: {}", err_body));
        }

        resp.json::<RegisterWorkerResponse>()
            .await
            .map_err(|e| format!("Failed to parse register response JSON: {}", e))
    }

    pub async fn send_heartbeat(
        &self,
        status: &str,
        current_task_run_id: Option<i64>,
        screen_width: u32,
        screen_height: u32,
        os_info: &str,
        agent_version: &str,
    ) -> Result<HeartbeatResponse, String> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| "API key required for heartbeat".to_string())?;

        let url = format!("{}/api/v1/workers/heartbeat", self.server_url);
        let req = HeartbeatRequest {
            status: Some(status.to_string()),
            current_task_run_id,
            screen_width: Some(screen_width as i32),
            screen_height: Some(screen_height as i32),
            os_info: Some(os_info.to_string()),
            agent_version: Some(agent_version.to_string()),
        };

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&req)
            .send()
            .await
            .map_err(|e| format!("Heartbeat HTTP request failed: {}", e))?;

        if resp.status() == StatusCode::UPGRADE_REQUIRED {
            return Err("426 Upgrade Required: Agent version is outdated".to_string());
        }

        if !resp.status().is_success() {
            return Err(format!("Heartbeat HTTP status {}", resp.status()));
        }

        resp.json::<HeartbeatResponse>()
            .await
            .map_err(|e| format!("Failed to parse HeartbeatResponse JSON: {}", e))
    }

    pub async fn poll_next_assignment(&self) -> Result<NextAssignmentResponse, String> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| "API key required for task assignment polling".to_string())?;

        let url = format!("{}/api/v1/workers/next-assignment", self.server_url);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await
            .map_err(|e| format!("Poll next assignment HTTP request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "Poll next assignment HTTP status {}",
                resp.status()
            ));
        }

        resp.json::<NextAssignmentResponse>()
            .await
            .map_err(|e| format!("Failed to parse NextAssignmentResponse JSON: {}", e))
    }

    pub async fn post_step_result(
        &self,
        task_run_id: i64,
        step_req: &StepResultRequest,
    ) -> Result<StepResultResponse, String> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| "API key required to post step result".to_string())?;

        let url = format!(
            "{}/api/v1/workers/task-runs/{}/step-result",
            self.server_url, task_run_id
        );

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(step_req)
            .send()
            .await
            .map_err(|e| format!("Post step result HTTP request failed: {}", e))?;

        if !resp.status().is_success() {
            let err_txt = resp.text().await.unwrap_or_default();
            return Err(format!("Post step result HTTP error: {}", err_txt));
        }

        resp.json::<StepResultResponse>()
            .await
            .map_err(|e| format!("Failed to parse StepResultResponse JSON: {}", e))
    }

    pub async fn complete_task_run(
        &self,
        task_run_id: i64,
        status: &str,
        error_message: Option<String>,
        output_type: Option<String>,
        output_value: Option<String>,
    ) -> Result<CompleteTaskRunResponse, String> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| "API key required to complete task run".to_string())?;

        let url = format!(
            "{}/api/v1/workers/task-runs/{}/complete",
            self.server_url, task_run_id
        );

        let req = CompleteTaskRunRequest {
            status: status.to_string(),
            error_message,
            output_type,
            output_value,
        };

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&req)
            .send()
            .await
            .map_err(|e| format!("Complete task run HTTP request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Complete task run HTTP status {}", resp.status()));
        }

        resp.json::<CompleteTaskRunResponse>()
            .await
            .map_err(|e| format!("Failed to parse CompleteTaskRunResponse JSON: {}", e))
    }

    pub async fn get_screenshot_upload_url(
        &self,
        task_run_id: i64,
    ) -> Result<ScreenshotUploadUrlResponse, String> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| "API key required for screenshot upload URL".to_string())?;

        let url = format!(
            "{}/api/v1/workers/task-runs/{}/screenshot-upload-url",
            self.server_url, task_run_id
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await
            .map_err(|e| format!("Get screenshot upload URL request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "Get screenshot upload URL HTTP status {}",
                resp.status()
            ));
        }

        resp.json::<ScreenshotUploadUrlResponse>()
            .await
            .map_err(|e| format!("Failed to parse ScreenshotUploadUrlResponse JSON: {}", e))
    }

    pub async fn commit_screenshot(
        &self,
        task_run_id: i64,
        step_id: i64,
        object_key: &str,
        width: u32,
        height: u32,
    ) -> Result<CommitScreenshotResponse, String> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| "API key required for screenshot commit".to_string())?;

        let url = format!(
            "{}/api/v1/workers/task-runs/{}/screenshots/commit",
            self.server_url, task_run_id
        );

        let req = CommitScreenshotRequest {
            step_id: Some(step_id),
            object_key: Some(object_key.to_string()),
            width: Some(width as i32),
            height: Some(height as i32),
        };

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&req)
            .send()
            .await
            .map_err(|e| format!("Commit screenshot request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Commit screenshot HTTP status {}", resp.status()));
        }

        resp.json::<CommitScreenshotResponse>()
            .await
            .map_err(|e| format!("Failed to parse CommitScreenshotResponse JSON: {}", e))
    }

    pub async fn upload_image_to_presigned_url(
        &self,
        upload_url: &str,
        image_bytes: Vec<u8>,
    ) -> Result<(), String> {
        let resp = self
            .client
            .put(upload_url)
            .header("Content-Type", "image/png")
            .body(image_bytes.clone())
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => Ok(()),
            Ok(r)
                if r.status() == StatusCode::FORBIDDEN || r.status() == StatusCode::BAD_REQUEST =>
            {
                let retry_resp = self
                    .client
                    .put(upload_url)
                    .body(image_bytes)
                    .send()
                    .await
                    .map_err(|e| format!("Failed retry upload to presigned URL: {}", e))?;

                if retry_resp.status().is_success() {
                    Ok(())
                } else {
                    Err(format!(
                        "Presigned URL upload retry failed with status {}",
                        retry_resp.status()
                    ))
                }
            }
            Ok(r) => Err(format!("Presigned URL upload status {}", r.status())),
            Err(e) => Err(format!("Presigned URL upload HTTP error: {}", e)),
        }
    }

    pub async fn download_image_bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Failed to download image from {}: {}", url, e))?;

        if !resp.status().is_success() {
            return Err(format!("Download image HTTP status {}", resp.status()));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("Failed to read image bytes: {}", e))
    }

    pub async fn post_recording_events(
        &self,
        recording_session_id: i64,
        events: Vec<RecordingEventItem>,
    ) -> Result<PostRecordingEventsResponse, String> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| "API key required for recording events".to_string())?;

        let url = format!(
            "{}/api/v1/workers/recordings/{}/events",
            self.server_url, recording_session_id
        );

        let req = PostRecordingEventsRequest { events };

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&req)
            .send()
            .await
            .map_err(|e| format!("Post recording events HTTP request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "Post recording events HTTP status {}",
                resp.status()
            ));
        }

        resp.json::<PostRecordingEventsResponse>()
            .await
            .map_err(|e| format!("Failed to parse PostRecordingEventsResponse JSON: {}", e))
    }
}
